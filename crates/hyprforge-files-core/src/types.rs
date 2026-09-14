//! What a directory listing looks like to this crate, independent of how
//! it was read.
//!
//! Kept free of `std::fs` types in the public shape (a raw
//! `std::fs::Metadata` is not `Clone`, is not `PartialEq`, and carries a
//! platform-specific amount of detail this crate does not need) so the
//! model above [`crate::backend::FsBackend`] can be built and asserted on
//! against the mock, the same reasoning `hyprforge-network::types` gives
//! for keeping `zbus` out of its own shapes.

use std::path::PathBuf;
use std::time::SystemTime;

/// One row in a directory listing.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Entry {
    /// The final path component — what the list shows, not the full path.
    pub name: String,
    pub path: PathBuf,
    pub is_dir: bool,
    /// In bytes. `0` for a directory: this crate never sums a directory's
    /// contents to get one, because doing that during a listing turns an
    /// O(entries) operation into O(everything under every subdirectory),
    /// and a size column that means "apparent size of this one inode" for
    /// files and "recursive total" for directories is also two different
    /// numbers wearing one label.
    pub size: u64,
    /// `None` when the filesystem would not say — `stat` failing on the
    /// timestamp specifically is rarer than the whole call failing, but
    /// treating it as "epoch" would sort a genuinely-unknown-time entry
    /// as the oldest thing in the directory, which is a claim this crate
    /// has no basis for making.
    pub modified: Option<SystemTime>,
    pub is_symlink: bool,
    /// Only meaningful when `is_symlink` is true: the link exists but its
    /// target does not (or a component of the target's path does not).
    /// A broken link is a *state* the listing shows, not a reason to fail
    /// the entry or the read — see [`crate::backend`]'s module doc.
    pub link_broken: bool,
    /// A dotfile by the usual Unix convention (name starts with `.`,
    /// excluding `.` and `..` themselves, which [`crate::backend::FsBackend::read_dir`]
    /// never emits in the first place). [`crate::filter`] also honours a
    /// `.hidden` file naming further entries — that is a per-directory
    /// concern layered on top, not part of what makes one `Entry` hidden
    /// on its own.
    pub hidden: bool,
    pub kind: EntryKind,
}

/// A coarse category, used to pick a fallback icon badge when no
/// thumbnail is available and to decide whether a preview is even worth
/// attempting. Deliberately short: every variant here has to earn a
/// distinct icon and a distinct "can phase 2 preview this" answer, or it
/// is not worth telling apart from [`EntryKind::Other`]. A directory is
/// its own variant rather than folded into `Other` because it is the one
/// category [`crate::sort`]'s directories-first toggle keys on.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EntryKind {
    Folder,
    Image,
    Document,
    Archive,
    Code,
    Audio,
    Video,
    Other,
}

impl EntryKind {
    /// Classifies by extension. Content-sniffing is deliberately not done
    /// here: `read_dir` walks every entry in a directory, and opening
    /// each file to read a magic number turns a listing into as many
    /// opens as there are files, on a directory that may have thousands.
    /// Extension is wrong for a renamed or extensionless file and that is
    /// an accepted cost — it only ever affects which fallback badge is
    /// shown, never whether a file can be opened.
    pub fn classify(is_dir: bool, file_name: &str) -> EntryKind {
        if is_dir {
            return EntryKind::Folder;
        }
        let ext = file_name
            .rsplit_once('.')
            // A name with no `.`, or one that is only a leading dot
            // (`.bashrc`) has no extension to classify by — `rsplit_once`
            // on `.bashrc` would otherwise read the whole name minus the
            // dot as the "extension".
            .filter(|(stem, _)| !stem.is_empty())
            .map(|(_, ext)| ext.to_ascii_lowercase());
        let Some(ext) = ext else {
            return EntryKind::Other;
        };
        match ext.as_str() {
            "png" | "jpg" | "jpeg" | "gif" | "webp" | "bmp" | "svg" | "avif" | "heic" | "tiff" => {
                EntryKind::Image
            }
            "pdf" | "doc" | "docx" | "odt" | "txt" | "md" | "rtf" | "xls" | "xlsx" | "ods"
            | "ppt" | "pptx" | "odp" => EntryKind::Document,
            "zip" | "tar" | "gz" | "bz2" | "xz" | "zst" | "7z" | "rar" => EntryKind::Archive,
            "rs" | "py" | "js" | "ts" | "jsx" | "tsx" | "c" | "h" | "cpp" | "hpp" | "go"
            | "java" | "rb" | "sh" | "lua" | "toml" | "yaml" | "yml" | "json" | "html"
            | "css" => EntryKind::Code,
            "mp3" | "flac" | "wav" | "ogg" | "opus" | "m4a" => EntryKind::Audio,
            "mp4" | "mkv" | "webm" | "avi" | "mov" => EntryKind::Video,
            _ => EntryKind::Other,
        }
    }
}

/// Everything that can go wrong reading the filesystem, phrased so the UI
/// can turn each variant directly into a sentence — never "check the
/// logs", which is pillar 3's own wording for the thing this enum exists
/// to prevent.
#[derive(Debug, thiserror::Error)]
pub enum FilesError {
    /// Deliberately **not** how an empty, readable directory is
    /// represented — that is `Ok(vec![])`. Collapsing "could not read"
    /// into "nothing here" is the exact mistake `hlconfig::storage`
    /// already made once for a missing-vs-unparseable config file; a
    /// listing has the same two states and must not blur them either.
    #[error("You don't have permission to open {path}.")]
    PermissionDenied { path: PathBuf },
    #[error("{path} doesn't exist.")]
    NotFound { path: PathBuf },
    #[error("{path} is not a directory.")]
    NotADirectory { path: PathBuf },
    /// The directory (or the entry being `stat`-ed) was there when this
    /// crate started looking and was gone by the time it looked again —
    /// distinct from `NotFound` because it is a race, not a typo in a
    /// path the caller built, and the UI's sentence for it should say
    /// so rather than imply the path was always wrong.
    #[error("{path} was removed while it was being read.")]
    VanishedMidRead { path: PathBuf },
    #[error("{path} couldn't be read: {source}")]
    Io {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
}

impl FilesError {
    /// Builds the right variant from an [`std::io::Error`] encountered
    /// while operating on `path`, so every call site does not have to
    /// re-derive this `match` by hand.
    pub fn from_io(path: PathBuf, source: std::io::Error) -> FilesError {
        match source.kind() {
            std::io::ErrorKind::PermissionDenied => FilesError::PermissionDenied { path },
            std::io::ErrorKind::NotFound => FilesError::NotFound { path },
            std::io::ErrorKind::NotADirectory => FilesError::NotADirectory { path },
            _ => FilesError::Io { path, source },
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_dotfile_with_no_further_extension_is_not_misread_as_all_extension() {
        // `.bashrc` is a hidden file with no extension, not a file whose
        // extension is `bashrc` — `rsplit_once('.')` alone would say the
        // latter, which would classify it as `Other` for the wrong
        // reason and would classify `.png`, say, as an Image when it is
        // really just a hidden, nameless file.
        assert_eq!(EntryKind::classify(false, ".bashrc"), EntryKind::Other);
    }

    #[test]
    fn a_normal_extension_is_classified_case_insensitively() {
        assert_eq!(EntryKind::classify(false, "Photo.JPG"), EntryKind::Image);
    }

    #[test]
    fn a_directory_is_folder_regardless_of_its_name() {
        assert_eq!(EntryKind::classify(true, "archive.zip"), EntryKind::Folder);
    }

    #[test]
    fn an_extensionless_name_is_other() {
        assert_eq!(EntryKind::classify(false, "Makefile"), EntryKind::Other);
    }
}
