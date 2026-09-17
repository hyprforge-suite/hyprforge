//! The trash as a *backend*, not as a branch in whoever reads a
//! directory.
//!
//! The trash's `files/` directory can be read with a plain `read_dir`,
//! and doing so is wrong in a way that is invisible: it shows the
//! *stored* names. `hyprland.conf.bak` appears with nothing saying it
//! came from `~/.config/hypr/`, and a `Monkey Around.2.mp4` was never
//! called that — the `.2` is the trash's own collision suffix, not part
//! of anyone's filename. The listing has to come from the `.trashinfo`
//! records instead, which is what [`hyprforge_fileops::list`] reads.
//!
//! That used to be an `if is_trash_dir(path) { .. }` in the host's
//! directory-reading function, with its own entry builder beside it. The
//! trouble with that shape is what comes next: a mounted share, a remote
//! location, "Recent", a set of search results — each one is a second
//! `if` in the same function, and the function is nobody's idea of where
//! "what is this place" gets decided.
//!
//! A place that lists differently is a different [`FsBackend`]. The
//! decision then lives in one router ([`RoutingBackend`]) that everything
//! above it goes through without knowing there was a decision.

use crate::backend::{FsBackend, StdBackend};
use crate::types::{Entry, EntryKind, FilesError};
use std::path::{Path, PathBuf};

/// Lists the freedesktop.org trash by its records rather than by its
/// storage directory.
pub struct TrashBackend {
    /// The trash root — `$XDG_DATA_HOME/Trash`. A field rather than a
    /// call to [`hyprforge_fileops::home_trash_dir`] so tests can point
    /// one at a throwaway directory instead of the real trash.
    root: PathBuf,
}

impl Default for TrashBackend {
    fn default() -> Self {
        TrashBackend::at(hyprforge_fileops::home_trash_dir())
    }
}

impl TrashBackend {
    pub fn at(root: impl Into<PathBuf>) -> Self {
        TrashBackend { root: root.into() }
    }

    /// The directory this backend claims: the trash's `files/`, which is
    /// what the sidebar's Trash item navigates to.
    pub fn listing_dir(&self) -> PathBuf {
        self.root.join("files")
    }

    /// Whether `candidate` is this backend's directory.
    ///
    /// Through `canonicalize` first, so a symlink to it or a spelling
    /// with a redundant `..` still matches, falling back to plain
    /// equality when either side cannot be canonicalized. That fallback
    /// is load-bearing: the commonest reason `files/` cannot be
    /// canonicalized is that the trash has never been used and the
    /// directory does not exist yet, and answering "not the trash" there
    /// would fall through to an ordinary `read_dir` of a missing
    /// directory — which reads to a user as "your trash is empty" for
    /// entirely the wrong reason.
    pub fn claims(&self, candidate: &Path) -> bool {
        let listing_dir = self.listing_dir();
        match (std::fs::canonicalize(candidate), std::fs::canonicalize(&listing_dir)) {
            (Ok(a), Ok(b)) => a == b,
            _ => candidate == listing_dir,
        }
    }

    /// One [`hyprforge_fileops::TrashedItem`] as a listing [`Entry`].
    ///
    /// `None` for an item whose trashed file has itself vanished since
    /// `list()` walked `info/` — the same "one bad entry must not sink
    /// the whole listing" rule [`StdBackend::read_dir`] already follows,
    /// applied here for the same reason.
    fn entry(item: &hyprforge_fileops::TrashedItem) -> Option<Entry> {
        let name = item.original_path.file_name()?.to_string_lossy().into_owned();
        // `StdBackend::stat` and not a second hand-rolled metadata read:
        // kind, size, dir-ness through a link and broken-link detection
        // are all decisions `build_entry` already makes, and making them
        // again here is how two listings drift apart.
        let mut entry = StdBackend.stat(&item.trashed_file).ok()?;
        // Three fields the trash overrides, and only three.
        //
        // The name is the one the file *had* — `hyprland.conf.bak`, not
        // the trash's stored spelling of it. The date is when it left,
        // which is what the row is actually about; the trashed file's
        // own mtime stays as the fallback for a `DeletionDate` that will
        // not parse. `hyprforge_fileops` both writes and parses that
        // field, and its `to_system_time` folds the value back through
        // the host's real timezone with `mktime`, so something trashed
        // this morning reads as this morning.
        entry.modified = hyprforge_fileops::localtime::LocalDateTime::parse(&item.deleted_at)
            .and_then(|dt| dt.to_system_time())
            .or(entry.modified);
        entry.hidden = name.starts_with('.');
        entry.kind = EntryKind::classify(entry.is_dir, &name);
        entry.name = name;
        Some(entry)
    }
}

impl FsBackend for TrashBackend {
    fn read_dir(&self, _path: &Path) -> Result<Vec<Entry>, FilesError> {
        let items = hyprforge_fileops::list(&self.root).map_err(|e| FilesError::Io {
            path: self.root.clone(),
            source: std::io::Error::other(e.to_string()),
        })?;
        Ok(items.iter().filter_map(TrashBackend::entry).collect())
    }

    fn count_children(&self, path: &Path) -> Result<usize, FilesError> {
        // Through `read_dir`, not a `getdents` count of `files/`: an
        // item whose record is unreadable is not shown, so counting the
        // storage directory would disagree with the listing beside it.
        self.read_dir(path).map(|entries| entries.len())
    }

    // The trash is a listing, not a filesystem. Everything that is not
    // "what is in here" is the ordinary filesystem's answer — a trashed
    // file really does live at a real path, which is what makes
    // activating one work at all.
    fn stat(&self, path: &Path) -> Result<Entry, FilesError> {
        StdBackend.stat(path)
    }

    fn home_dir(&self) -> PathBuf {
        StdBackend.home_dir()
    }

    fn canonicalize(&self, path: &Path) -> Result<PathBuf, FilesError> {
        StdBackend.canonicalize(path)
    }
}

/// Sends each path to the backend that can describe it.
///
/// One place where "what kind of place is this" is decided, so nothing
/// above it has to ask. A new kind of location — a mounted share, a
/// remote host, search results — becomes a backend and a line here,
/// rather than another branch inside somebody's read function.
pub struct RoutingBackend {
    trash: TrashBackend,
    std: StdBackend,
}

impl Default for RoutingBackend {
    fn default() -> Self {
        RoutingBackend { trash: TrashBackend::default(), std: StdBackend }
    }
}

impl RoutingBackend {
    /// With the trash rooted somewhere specific — the seam tests use to
    /// point this at a throwaway directory rather than the real trash.
    pub fn with_trash(trash: TrashBackend) -> Self {
        RoutingBackend { trash, std: StdBackend }
    }

    fn route(&self, path: &Path) -> &dyn FsBackend {
        if self.trash.claims(path) {
            &self.trash
        } else {
            &self.std
        }
    }
}

impl FsBackend for RoutingBackend {
    fn read_dir(&self, path: &Path) -> Result<Vec<Entry>, FilesError> {
        self.route(path).read_dir(path)
    }

    fn count_children(&self, path: &Path) -> Result<usize, FilesError> {
        self.route(path).count_children(path)
    }

    fn stat(&self, path: &Path) -> Result<Entry, FilesError> {
        self.route(path).stat(path)
    }

    fn home_dir(&self) -> PathBuf {
        self.std.home_dir()
    }

    fn canonicalize(&self, path: &Path) -> Result<PathBuf, FilesError> {
        self.std.canonicalize(path)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    /// Builds a trash directory holding one item, and returns its root.
    fn trash_with(original: &str, stored: &str, contents: &str) -> tempfile::TempDir {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        fs::create_dir_all(root.join("files")).unwrap();
        fs::create_dir_all(root.join("info")).unwrap();
        fs::write(root.join("files").join(stored), contents).unwrap();
        fs::write(
            root.join("info").join(format!("{stored}.trashinfo")),
            format!("[Trash Info]\nPath={original}\nDeletionDate=2024-03-01T12:30:00\n"),
        )
        .unwrap();
        dir
    }

    /// The whole reason the trash is not an ordinary `read_dir`: the
    /// stored name carries a collision suffix nobody chose.
    #[test]
    fn a_trashed_file_is_listed_under_the_name_it_had_not_the_one_it_is_stored_as() {
        let dir = trash_with("/home/alex/Monkey Around.mp4", "Monkey Around.2.mp4", "x");
        let backend = TrashBackend::at(dir.path());
        let entries = backend.read_dir(&backend.listing_dir()).unwrap();
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].name, "Monkey Around.mp4");
        // The path still points at the real stored file, so activating
        // the row opens something that exists.
        assert_eq!(entries[0].path, dir.path().join("files").join("Monkey Around.2.mp4"));
        assert!(!entries[0].is_dir);
        assert_eq!(entries[0].size, crate::types::EntrySize::Bytes(1));
    }

    /// The brief's own wording: the trash must be recognised "including
    /// via a symlinked or non-canonical spelling".
    #[test]
    fn the_trash_is_claimed_through_a_symlink_and_a_noncanonical_spelling() {
        let dir = tempfile::tempdir().unwrap();
        let backend = TrashBackend::at(dir.path());
        fs::create_dir_all(backend.listing_dir()).unwrap();

        let symlink = dir.path().join("trash-link");
        std::os::unix::fs::symlink(backend.listing_dir(), &symlink).unwrap();
        assert!(backend.claims(&symlink), "a symlink to the trash must still match");

        let noncanonical = dir.path().join(".").join("files");
        assert!(
            backend.claims(&noncanonical),
            "a `.`-laden spelling of the same path must still match"
        );

        let unrelated = dir.path().join("Documents");
        fs::create_dir_all(&unrelated).unwrap();
        assert!(!backend.claims(&unrelated));
    }

    #[test]
    fn an_item_whose_stored_file_has_vanished_is_skipped_not_fatal() {
        let dir = trash_with("/home/alex/gone.txt", "gone.txt", "x");
        fs::remove_file(dir.path().join("files").join("gone.txt")).unwrap();
        let backend = TrashBackend::at(dir.path());
        assert!(backend.read_dir(&backend.listing_dir()).unwrap().is_empty());
    }

    #[test]
    fn a_trash_that_has_never_been_used_lists_empty_rather_than_failing() {
        let dir = tempfile::tempdir().unwrap();
        let backend = TrashBackend::at(dir.path().join("never-used"));
        assert!(backend.read_dir(&backend.listing_dir()).unwrap().is_empty());
    }

    /// The fallback in `claims` that matters: a trash directory that
    /// does not exist yet must still be recognised as the trash, or the
    /// router sends it to `StdBackend` and the user is told their trash
    /// is empty when the truth is that nothing has ever been put in it.
    #[test]
    fn the_trash_is_claimed_even_before_the_directory_exists() {
        let dir = tempfile::tempdir().unwrap();
        let backend = TrashBackend::at(dir.path().join("never-used"));
        assert!(backend.claims(&backend.listing_dir()));
    }

    #[test]
    fn the_router_sends_the_trash_to_the_trash_and_everything_else_to_the_filesystem() {
        let dir = trash_with("/home/alex/note.txt", "note.txt", "hello");
        let trash = TrashBackend::at(dir.path());
        let listing_dir = trash.listing_dir();
        let router = RoutingBackend::with_trash(trash);

        // Through the router, the trash directory lists by record.
        let entries = router.read_dir(&listing_dir).unwrap();
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].name, "note.txt");

        // Anywhere else is the ordinary filesystem: `info/` is a real
        // directory holding one real file, and the router must show it
        // as such rather than routing it to the trash listing.
        let info = router.read_dir(&dir.path().join("info")).unwrap();
        assert_eq!(info.len(), 1);
        assert_eq!(info[0].name, "note.txt.trashinfo");
    }
}
