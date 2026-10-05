//! How much room is left on the filesystem you are looking at.
//!
//! Explorer shows it under every drive and Dolphin and Nemo in their
//! status bars, because "will this fit" is asked right before a paste and
//! nowhere else in the window answers it.
//!
//! The browser does no I/O, so this is the pure half: *which* path the
//! host should ask about, and what to say once it has. The host asks with
//! `statvfs`, off the UI thread, and answers with
//! [`crate::browser::Message::SpaceMeasured`].

use std::path::{Path, PathBuf};

/// What `statvfs` said about one filesystem, in bytes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Space {
    /// What this user can still write — `f_bavail`, not `f_bfree`: the
    /// blocks reserved for root are not room an ordinary paste can use,
    /// and counting them would promise space that refuses the write.
    pub free: u64,
    /// The filesystem's size.
    pub total: u64,
}

/// The path whose filesystem a listing of `dir` lives on.
///
/// The folder itself, almost always. Inside an archive the browser's path
/// runs on past the archive file into members that are not on any
/// filesystem, so the question goes to the folder holding the archive —
/// which is where an extraction from it would land, and so the free space
/// that matters there.
pub fn measured_at(dir: &Path) -> PathBuf {
    match crate::archive::split(dir) {
        Some((archive, _member)) => archive.parent().map(Path::to_path_buf).unwrap_or(archive),
        None => dir.to_path_buf(),
    }
}

/// The status bar's words for `space`: "212.4 GiB free of 931.5 GiB".
///
/// `None` when the filesystem reports no size at all. Some FUSE mounts
/// and virtual filesystems answer `statvfs` with zeroes, and "0 B free"
/// there would read as a full disk on a share with plenty of room — a
/// blank is honest where a zero is wrong.
pub fn label(space: Space) -> Option<String> {
    if space.total == 0 {
        return None;
    }
    Some(format!(
        "{} free of {}",
        crate::format::human_readable_size(space.free),
        crate::format::human_readable_size(space.total)
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_filesystem_reporting_no_size_says_nothing_rather_than_full() {
        assert_eq!(label(Space { free: 0, total: 0 }), None);
    }

    #[test]
    fn the_label_names_what_is_free_and_out_of_how_much() {
        let gib = 1024 * 1024 * 1024;
        assert_eq!(
            label(Space { free: 212 * gib, total: 931 * gib }).as_deref(),
            Some("212.0 GiB free of 931.0 GiB")
        );
    }

    #[test]
    fn an_ordinary_folder_is_asked_about_itself() {
        let dir = Path::new("/home/someone/Documents");
        assert_eq!(measured_at(dir), dir);
    }

    #[test]
    fn inside_an_archive_the_folder_holding_it_is_asked() {
        let tmp = tempfile::tempdir().unwrap();
        let archive = tmp.path().join("bundle.zip");
        std::fs::write(&archive, b"PK\x05\x06\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0").unwrap();
        assert_eq!(measured_at(&archive.join("docs/guide")), tmp.path());
        assert_eq!(measured_at(&archive), tmp.path());
    }
}
