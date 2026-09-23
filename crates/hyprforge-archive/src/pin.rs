//! Holding one version of an archive across every read of it.
//!
//! Nothing here reads an archive once. Every entry point sniffs the
//! format with one open and reads with another, an extraction lists the
//! archive before it unpacks it, a tar is reopened for each member, and
//! an edit does all of that and then writes. Every one of those opens
//! goes by *path*, and a path is not a file: a rewrite — this crate's
//! own, or nearly any other tool's — replaces an archive by renaming a
//! new one over its name, and can land between any two of them.
//! Measured with two concurrent edits of one zip under load: the listing
//! came from the old archive and the unpacked files from the new one,
//! and the edit failed repacking a member that had never been unpacked —
//! about three runs in two hundred, and never on an idle machine.
//!
//! [`Pinned`] opens the archive once and hands out
//! `/proc/self/fd/<n>` for every later open. Opening that reopens the
//! file the descriptor holds, not whatever has the name now, so every
//! read inside one call sees one archive — measured, not assumed: after
//! a rename over the name, the name reads the new contents and the
//! descriptor path the old, with the same inode behind it.
//!
//! Why not a hard link or a copy: `edit` briefly used a hard link beside the
//! archive, and that cannot serve extraction, which is routinely
//! from somewhere this user cannot write — a read-only mount, someone
//! else's shared folder — where there is nowhere to put a link, and a
//! copy costs the archive's whole size just to read it. A descriptor
//! needs neither.
//!
//! Without `/proc` (a sandbox that does not mount it), there is no path
//! that names a descriptor, and this falls back to the archive's own
//! path — the behaviour before this module existed, unpinned but no
//! worse.

use crate::backend::ExtractReport;
use crate::error::{ArchiveError, Result};
use std::fs::File;
use std::os::unix::fs::MetadataExt;
use std::os::unix::io::AsRawFd;
use std::path::{Path, PathBuf};

/// One archive, open, with a path that keeps naming it.
pub(crate) struct Pinned {
    /// Held for as long as `path` must keep resolving — the descriptor
    /// *is* what `/proc/self/fd/<n>` names, and closing it would let the
    /// number be reused for some other file entirely.
    _file: Option<File>,
    path: PathBuf,
    archive: PathBuf,
}

impl Pinned {
    pub(crate) fn open(archive: &Path) -> Result<Pinned> {
        let file = File::open(archive).map_err(|e| ArchiveError::io(archive, e))?;
        let via = PathBuf::from(format!("/proc/self/fd/{}", file.as_raw_fd()));
        // Asked of the path itself rather than of whether `/proc` is
        // mounted: what matters is that opening `via` reaches this file,
        // and comparing inodes is the one check that says so directly.
        let reaches = match (std::fs::metadata(&via), file.metadata()) {
            (Ok(through), Ok(held)) => through.dev() == held.dev() && through.ino() == held.ino(),
            _ => false,
        };
        if reaches {
            Ok(Pinned {
                _file: Some(file),
                path: via,
                archive: archive.to_path_buf(),
            })
        } else {
            Ok(Pinned {
                _file: None,
                path: archive.to_path_buf(),
                archive: archive.to_path_buf(),
            })
        }
    }

    /// Runs `read` against the pinned path, and puts the archive's own
    /// name back into any error it returns.
    ///
    /// The descriptor path is an implementation detail with a number in
    /// it, and these messages reach a status bar verbatim: "/proc/self/
    /// fd/9 is a damaged zip archive" names nothing the person chose.
    pub(crate) fn read<T>(&self, read: impl FnOnce(&Path) -> Result<T>) -> Result<T> {
        read(&self.path).map_err(|e| e.renaming(&self.path, &self.archive))
    }

    /// [`Pinned::read`] for an extraction, whose per-member failures
    /// are already sentences by the time they reach the report — so the
    /// name has to go back into the text as well as into the error.
    pub(crate) fn extract(
        &self,
        read: impl FnOnce(&Path) -> Result<ExtractReport>,
    ) -> Result<ExtractReport> {
        let mut report = self.read(read)?;
        if self.path != self.archive {
            let (from, to) = (self.path.to_string_lossy(), self.archive.to_string_lossy());
            for failure in &mut report.failed {
                failure.message = replace_path(&failure.message, &from, &to);
            }
        }
        Ok(report)
    }
}

/// Replaces `from` with `to` wherever it is a whole path in `text`.
///
/// "Whole" is the part a plain `str::replace` gets wrong:
/// `/proc/self/fd/7` is a prefix of `/proc/self/fd/71`, and an edit
/// holds one pin while the calls inside it take their own, so both
/// numbers can be live at once.
fn replace_path(text: &str, from: &str, to: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(at) = rest.find(from) {
        let after = &rest[at + from.len()..];
        out.push_str(&rest[..at]);
        if after.starts_with(|c: char| c.is_ascii_digit()) {
            out.push_str(from);
        } else {
            out.push_str(to);
        }
        rest = after;
    }
    out.push_str(rest);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The property the whole module rests on, asked of the kernel
    /// rather than assumed from documentation.
    #[test]
    fn a_pinned_archive_still_reads_as_it_was_after_a_rename_over_its_name() {
        let dir = tempfile::tempdir().unwrap();
        let archive = dir.path().join("a.zip");
        std::fs::write(&archive, b"old").unwrap();
        let pin = Pinned::open(&archive).unwrap();

        let newer = dir.path().join("newer");
        std::fs::write(&newer, b"new").unwrap();
        std::fs::rename(&newer, &archive).unwrap();

        assert_ne!(pin.path, archive, "no /proc here — nothing was pinned");
        assert_eq!(std::fs::read(&archive).unwrap(), b"new");
        assert_eq!(std::fs::read(&pin.path).unwrap(), b"old");
    }

    #[test]
    fn an_error_from_a_pinned_read_names_the_archive() {
        let dir = tempfile::tempdir().unwrap();
        let archive = dir.path().join("photo.zip");
        std::fs::write(&archive, b"not an archive").unwrap();
        let pin = Pinned::open(&archive).unwrap();

        let error = pin
            .read(|path| -> Result<()> { Err(ArchiveError::NotAnArchive { path: path.to_path_buf() }) })
            .unwrap_err();
        assert!(matches!(error, ArchiveError::NotAnArchive { path } if path == archive));
    }

    #[test]
    fn a_missing_archive_is_named_in_the_failure_to_open_it() {
        let archive = Path::new("/nonexistent/a.zip");
        let error = Pinned::open(archive).err().expect("there is nothing to open");
        assert!(matches!(error, ArchiveError::Io { path, .. } if path == archive));
    }

    #[test]
    fn a_path_is_replaced_only_where_it_is_the_whole_path() {
        assert_eq!(
            replace_path("/proc/self/fd/7 is damaged", "/proc/self/fd/7", "/home/a.zip"),
            "/home/a.zip is damaged"
        );
        assert_eq!(
            replace_path("/proc/self/fd/71 is damaged", "/proc/self/fd/7", "/home/a.zip"),
            "/proc/self/fd/71 is damaged",
            "fd 71 is a different pin, not fd 7 followed by a one"
        );
        assert_eq!(
            replace_path("(/proc/self/fd/7).", "/proc/self/fd/7", "x"),
            "(x).",
        );
    }
}
