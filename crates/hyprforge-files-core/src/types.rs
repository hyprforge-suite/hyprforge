//! What a directory listing looks like to this crate, independent of how
//! it was read.
//!
//! Kept free of `std::fs` types in the public shape (a raw
//! `std::fs::Metadata` is not `Clone`, is not `PartialEq`, and carries a
//! platform-specific amount of detail this crate does not need) so the
//! model above [`crate::backend::FsBackend`] can be built and asserted on
//! against the mock, the same reasoning `hyprforge-network::types` gives
//! for keeping `zbus` out of its own shapes.

use std::cmp::Ordering;
use std::path::PathBuf;
use std::time::SystemTime;

/// One row in a directory listing.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Entry {
    /// The final path component — what the list shows, not the full path.
    pub name: String,
    pub path: PathBuf,
    pub is_dir: bool,
    /// What the Size column is about for this entry — bytes for a file,
    /// a count of children for a directory. See [`EntrySize`].
    pub size: EntrySize,
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
    /// The permission bits (`mode & 0o7777`), as the Permissions column
    /// shows them. Only the permissions: whether this is a directory or
    /// a link is already `is_dir`/`is_symlink`, and keeping the file-type
    /// bits here too would be the same fact in two places, free to
    /// disagree.
    pub mode: u32,
    /// The owning user's id, always — `stat` answers with this and
    /// nothing else.
    pub uid: u32,
    /// The owning user's login name, when this system can resolve one.
    ///
    /// `None` is ordinary rather than exceptional: a file copied from
    /// another machine, or one inside a container whose ids do not map
    /// here, genuinely has an owner this system cannot name. Both are
    /// kept because the *number* is the useful thing to show in that
    /// case — see [`crate::users`] and `format::format_owner`.
    pub owner: Option<String>,
}

/// What "size" means for one entry, as one value.
///
/// A file has a byte count and a directory has a number of things in it,
/// and those are different quantities — this crate never sums a
/// directory's contents to get bytes, because doing that during a
/// listing turns an O(entries) operation into O(everything under every
/// subdirectory), and a column meaning "apparent size of this inode" for
/// files and "recursive total" for directories is two numbers wearing
/// one label.
///
/// The two used to be stored apart: `size: u64` (always `0` for a
/// directory) on the entry, and the count the column actually *showed*
/// in a separate map on `Browser`, filled by a later pass. The cost was
/// that sorting and rendering disagreed — clicking the Size header
/// ordered folders by their `0`, i.e. by name, while the column plainly
/// showed counts — and a caller had to decode an `Option<Option<usize>>`
/// to tell "not counted yet" from "could not be read". One value, so the
/// thing shown and the thing sorted cannot come apart.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EntrySize {
    /// A file's apparent size, in bytes.
    Bytes(u64),
    /// A directory's child count. Arrives after the listing does — see
    /// [`ItemCount`].
    Items(ItemCount),
}

/// How many things are in a directory, including the two states that are
/// not a number.
///
/// `Pending` and `Unreadable` are deliberately distinct, and neither is
/// `Known(0)`. A folder that has not been counted yet renders blank; one
/// whose read failed renders an em dash; one that is genuinely empty
/// renders "0 items". A folder you have no permission to open must never
/// read as an empty one — CLAUDE.md's rule about never collapsing "could
/// not be read" into "there is nothing there", applied to a table cell.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ItemCount {
    /// The second pass has not answered for this folder yet — or never
    /// will, because the listing was longer than the counting budget.
    Pending,
    Known(usize),
    /// The directory could not be read.
    Unreadable,
}

impl EntrySize {
    /// A directory's size before anything has counted it.
    pub const UNCOUNTED: EntrySize = EntrySize::Items(ItemCount::Pending);
}

/// Ordering for the Size column.
///
/// Every directory sorts before every file, because "three items" and
/// "three kilobytes" are not quantities that can be ranked against each
/// other and pretending otherwise would put a folder somewhere its
/// number does not justify. Within directories, a count orders by the
/// number; the two non-numbers sort together at the low end, so a
/// listing being counted does not reshuffle as each answer lands.
impl Ord for EntrySize {
    fn cmp(&self, other: &Self) -> Ordering {
        fn rank(size: &EntrySize) -> (u8, u64) {
            match size {
                // `Pending`/`Unreadable` share a rank: neither is a
                // number, and giving them different ones would order
                // folders by *why* they have no count.
                EntrySize::Items(ItemCount::Pending | ItemCount::Unreadable) => (0, 0),
                EntrySize::Items(ItemCount::Known(n)) => (1, *n as u64),
                EntrySize::Bytes(n) => (2, *n),
            }
        }
        rank(self).cmp(&rank(other))
    }
}

impl PartialOrd for EntrySize {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
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
    fn a_folder_with_no_count_yet_is_not_the_same_size_as_an_empty_one() {
        // The distinction the old `Option<Option<usize>>` at the call
        // site existed to preserve, now in the type.
        assert_ne!(EntrySize::UNCOUNTED, EntrySize::Items(ItemCount::Known(0)));
        assert_ne!(EntrySize::Items(ItemCount::Unreadable), EntrySize::Items(ItemCount::Known(0)));
    }

    #[test]
    fn every_folder_sorts_before_every_file_by_size() {
        // Not a preference — "four items" cannot be ranked against
        // "four bytes", so they are kept in separate bands rather than
        // interleaved on a number that means different things.
        assert!(EntrySize::Items(ItemCount::Known(9_000)) < EntrySize::Bytes(0));
        assert!(EntrySize::UNCOUNTED < EntrySize::Bytes(0));
    }

    #[test]
    fn a_counted_folder_sorts_by_its_count() {
        assert!(EntrySize::Items(ItemCount::Known(2)) < EntrySize::Items(ItemCount::Known(10)));
    }

    #[test]
    fn an_uncounted_and_an_unreadable_folder_sort_together() {
        // Otherwise a listing visibly reshuffles as each count lands,
        // and folders end up ordered by *why* they have no number.
        assert_eq!(
            EntrySize::UNCOUNTED.cmp(&EntrySize::Items(ItemCount::Unreadable)),
            Ordering::Equal
        );
    }

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
