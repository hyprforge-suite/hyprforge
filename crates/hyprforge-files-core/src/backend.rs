//! The boundary between this crate's logic and the real filesystem.
//!
//! Synchronous, deliberately: this is a browser over `std::fs`, not a
//! service behind a socket, and phase 2 (the iced view) calls it off the
//! UI thread the way any blocking work has to rather than making every
//! caller here `async` for a call that is a handful of syscalls. The
//! exact arrangement `hyprforge-network::backend` uses for
//! `NetworkBackend` — a trait, a real implementation, and a mock kept
//! behind a feature so this crate's own tests and its callers' tests can
//! both use it — for the same reason: the machine running tier 1 is not
//! guaranteed to have any particular directory tree, permission failure,
//! or ten-thousand-entry directory sitting around to test against, and
//! everything above this trait (sorting, filtering, preferences) should
//! be testable without arranging any of that on a real disk.
//!
//! A single entry inside an otherwise-readable directory failing to
//! resolve — permission lost between `readdir` and `stat`-ing it, or the
//! entry being removed in that same window — must never fail the whole
//! listing. The obvious `.collect::<Result<Vec<_>, _>>()` over a
//! directory iterator does exactly the wrong thing here: one bad entry
//! turns "here are the 999 other files" into "here is nothing", which is
//! a worse answer than showing 999 files and quietly dropping one that
//! could not be described. [`StdBackend::read_dir`] skips and logs
//! instead.

use crate::types::{Entry, EntryKind, EntrySize, FilesError};
use crate::users::UserNames;
use std::os::unix::fs::MetadataExt;
use std::path::{Path, PathBuf};

pub trait FsBackend: Send + Sync {
    /// Lists a directory's immediate children — never recursive, and
    /// never including `.` or `..`. An entry this call could not fully
    /// describe (see the module doc) is left out rather than failing the
    /// whole call; only the directory itself being unreadable is an
    /// `Err`.
    fn read_dir(&self, path: &Path) -> Result<Vec<Entry>, FilesError>;

    /// Full metadata for one path. Does **not** follow a symlink to
    /// decide success or failure — a link whose target is gone is a
    /// valid [`Entry`] with [`Entry::link_broken`] set, not an `Err`,
    /// for the same reason `hlconfig::storage` distinguishes "missing"
    /// from "present but broken" for a config file: collapsing the two
    /// would make a broken link indistinguishable from a typo'd path.
    fn stat(&self, path: &Path) -> Result<Entry, FilesError>;

    /// How many things are directly inside `path`.
    ///
    /// On the trait rather than left to a caller's own `std::fs`,
    /// because it is the second half of building a listing — a folder's
    /// Size cell — and a caller that reaches around the trait for it
    /// puts half the listing beyond reach of the mock.
    ///
    /// Deliberately *not* `read_dir(path).map(|e| e.len())`: that
    /// `stat`s every child and builds a complete [`Entry`] for each,
    /// only to throw all of it away and keep the length. A pinned
    /// Downloads folder with five thousand files is five thousand
    /// syscalls for one number that one `getdents` loop already has.
    fn count_children(&self, path: &Path) -> Result<usize, FilesError>;

    /// The user's home directory, as the browser's default starting
    /// place and the target of a "Home" shortcut.
    fn home_dir(&self) -> PathBuf;

    /// Resolves `.`, `..` and symlinks to get a canonical path — used
    /// when deciding whether two navigations landed on the same place
    /// (so a bookmark and a manually-typed path that happen to resolve
    /// identically are recognised as such), not for display: what is
    /// shown in the location bar is the path as navigated, symlinks and
    /// all.
    fn canonicalize(&self, path: &Path) -> Result<PathBuf, FilesError>;
}

/// The real filesystem, over `std::fs`.
pub struct StdBackend;

impl FsBackend for StdBackend {
    fn read_dir(&self, path: &Path) -> Result<Vec<Entry>, FilesError> {
        let iter =
            std::fs::read_dir(path).map_err(|e| FilesError::from_io(path.to_path_buf(), e))?;
        let mut entries = Vec::new();
        // One cache for the whole listing — see `build_entry_with`.
        let mut users = UserNames::new();
        for item in iter {
            let dir_entry = match item {
                Ok(d) => d,
                Err(e) => {
                    // The OS can fail mid-iteration as well as on a
                    // specific entry below — either way this is the
                    // "one entry disappeared" case the mock pins in
                    // `backend::tests`, and it must not sink a listing
                    // that is otherwise fine.
                    tracing::warn!(
                        path = %path.display(),
                        error = %e,
                        "skipping a directory entry that could not be read"
                    );
                    continue;
                }
            };
            let name = dir_entry.file_name().to_string_lossy().into_owned();
            let entry_path = dir_entry.path();
            // `DirEntry::metadata` reads the entry's own metadata without
            // following a symlink it points to — the same thing
            // `symlink_metadata` gives for a path built by hand — which
            // is what lets a broken link become a state in
            // `build_entry` instead of an `Err` here.
            let meta = match dir_entry.metadata() {
                Ok(m) => m,
                Err(e) => {
                    tracing::warn!(
                        path = %entry_path.display(),
                        error = %e,
                        "skipping an entry whose metadata could not be read"
                    );
                    continue;
                }
            };
            entries.push(build_entry_with(entry_path, name, &meta, &mut users));
        }
        Ok(entries)
    }

    fn stat(&self, path: &Path) -> Result<Entry, FilesError> {
        let meta = std::fs::symlink_metadata(path)
            .map_err(|e| FilesError::from_io(path.to_path_buf(), e))?;
        let name = path
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_else(|| path.to_string_lossy().into_owned());
        Ok(build_entry(path.to_path_buf(), name, &meta))
    }

    fn count_children(&self, path: &Path) -> Result<usize, FilesError> {
        let iter = std::fs::read_dir(path).map_err(|e| FilesError::from_io(path.to_path_buf(), e))?;
        // The entries themselves are never built — see the trait's doc.
        Ok(iter.count())
    }

    fn home_dir(&self) -> PathBuf {
        // No `expect`: a missing `$HOME` should give the browser
        // *somewhere* to start (root) rather than panic before a window
        // has even opened. `hyprforge-paths::config_home` gets away with
        // `expect` because every Hyprforge process already assumes a
        // normal login session; this trait's mock is also used to model
        // exactly the case where that assumption is wrong.
        std::env::var_os("HOME")
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from("/"))
    }

    fn canonicalize(&self, path: &Path) -> Result<PathBuf, FilesError> {
        std::fs::canonicalize(path).map_err(|e| FilesError::from_io(path.to_path_buf(), e))
    }
}

/// A directory's size is a child count nobody has taken yet; a file's is
/// the byte count `stat` just answered with. One place, so "a directory
/// has no byte size" is stated once rather than at each construction.
fn entry_size(is_dir: bool, len: u64) -> EntrySize {
    if is_dir {
        EntrySize::UNCOUNTED
    } else {
        EntrySize::Bytes(len)
    }
}

/// Builds an [`Entry`] from a path, its final name component, and its
/// **un-followed** metadata (`symlink_metadata`, or the equivalent
/// `DirEntry::metadata`). Shared by [`StdBackend::read_dir`] and
/// [`StdBackend::stat`] so the symlink-following logic exists in exactly
/// one place.
fn build_entry(path: PathBuf, name: String, meta: &std::fs::Metadata) -> Entry {
    build_entry_with(path, name, meta, &mut UserNames::new())
}

/// [`build_entry`], sharing one uid-to-name cache across a whole
/// listing.
///
/// A directory of ten thousand files almost always has one or two
/// distinct owners, so the name-service lookup that matters is the
/// repeated one — see [`UserNames`]'s own doc for why the cache is
/// per-listing rather than global.
fn build_entry_with(
    path: PathBuf,
    name: String,
    meta: &std::fs::Metadata,
    users: &mut UserNames,
) -> Entry {
    let hidden = name.starts_with('.') && name != "." && name != "..";
    let is_symlink = meta.file_type().is_symlink();
    // From the *un-followed* metadata, like everything else here: a
    // symlink's own permissions and owner are what `ls -l` shows for it,
    // and following the link would report the target's instead.
    let mode = meta.mode() & 0o7777;
    let uid = meta.uid();
    let owner = users.get(uid).map(str::to_owned);

    if !is_symlink {
        let is_dir = meta.is_dir();
        return Entry {
            kind: EntryKind::classify(is_dir, &name),
            name,
            path,
            is_dir,
            size: entry_size(is_dir, meta.len()),
            modified: meta.modified().ok(),
            is_symlink: false,
            link_broken: false,
            hidden,
            mode,
            uid,
            owner,
            origin: None,
            packed: None,
        };
    }

    // Following the link is the only way to know what it points at, and
    // is exactly the step that fails for a broken one. Any failure here
    // — not found, a symlink loop, a permission this process cannot
    // cross — reads the same to someone browsing: the link doesn't lead
    // anywhere reachable right now. That is a state this `Entry` can
    // hold (`link_broken`), never a reason to fail the caller.
    match std::fs::metadata(&path) {
        Ok(target) => {
            let is_dir = target.is_dir();
            Entry {
                kind: EntryKind::classify(is_dir, &name),
                name,
                path,
                is_dir,
                size: entry_size(is_dir, target.len()),
                modified: target.modified().ok(),
                is_symlink: true,
                link_broken: false,
                hidden,
                mode,
                uid,
                owner,
                origin: None,
                packed: None,
            }
        }
        Err(_) => Entry {
            kind: EntryKind::Other,
            name,
            path,
            is_dir: false,
            size: EntrySize::UNCOUNTED,
            // The symlink's own mtime (when it was last re-pointed),
            // since the target's is unreachable.
            modified: meta.modified().ok(),
            is_symlink: true,
            link_broken: true,
            hidden,
            mode,
            uid,
            owner,
            origin: None,
            packed: None,
        },
    }
}

// The Files app's browser view is generic over `FsBackend` so its own
// tests (phase 2) can drive it with this mock instead of a real
// directory tree — the same reason `hyprforge-network`'s `MockBackend`
// compiles as ordinary code under a feature, not just `#[cfg(test)]`
// inside this crate.
#[cfg(any(test, feature = "mock"))]
pub mod mock {
    use super::*;
    use std::collections::{HashMap, HashSet};
    use std::sync::Mutex;
    use std::time::SystemTime;

    /// A filesystem that does what the test says.
    ///
    /// Seeded as a flat map from a directory's path to the full
    /// [`Entry`] list `read_dir` should hand back for it — there is no
    /// simulated inode table underneath, because nothing above
    /// [`FsBackend`] needs one.
    pub struct MockBackend {
        tree: Mutex<HashMap<PathBuf, Vec<Entry>>>,
        /// Directories on which `read_dir` reports permission denied,
        /// regardless of what (if anything) is seeded for them.
        unreadable: Mutex<HashSet<PathBuf>>,
        /// Entry paths that `read_dir` silently leaves out of its
        /// listing — as if the entry's own `stat` lost a race with
        /// something removing it — and that a direct `stat` on the same
        /// path also reports as gone. One flag models both pinned
        /// properties ("an entry that vanishes between listing and stat"
        /// and "one unreadable entry does not lose the other entries")
        /// because they are the same event in the real backend: an
        /// entry's metadata call failing is what both `read_dir`'s
        /// per-entry skip and a direct `stat` would hit.
        vanished: Mutex<HashSet<PathBuf>>,
        home: Mutex<PathBuf>,
    }

    impl Default for MockBackend {
        fn default() -> Self {
            MockBackend {
                tree: Mutex::new(HashMap::new()),
                unreadable: Mutex::new(HashSet::new()),
                vanished: Mutex::new(HashSet::new()),
                home: Mutex::new(PathBuf::from("/home/mock")),
            }
        }
    }

    impl MockBackend {
        pub fn new() -> Self {
            Self::default()
        }

        /// Seeds one directory's listing.
        pub fn seed(&self, dir: impl Into<PathBuf>, entries: Vec<Entry>) {
            self.tree.lock().unwrap().insert(dir.into(), entries);
        }

        /// A plain file entry, for building a seeded tree quickly. Not
        /// every test needs every field, so the ones a test cares about
        /// are set on the returned `Entry` afterward.
        pub fn file(dir: &Path, name: &str, size: u64) -> Entry {
            Entry {
                name: name.to_string(),
                path: dir.join(name),
                is_dir: false,
                size: EntrySize::Bytes(size),
                modified: Some(SystemTime::UNIX_EPOCH),
                is_symlink: false,
                link_broken: false,
                hidden: name.starts_with('.'),
                kind: EntryKind::classify(false, name),
                // Ownership and permissions are fixtures here: these
                // helpers build entries for tests about names, sizes and
                // ordering, none of which read them.
                mode: 0o644,
                uid: 1000,
                owner: Some("alex".to_string()),
                origin: None,
                packed: None,
            }
        }

        pub fn dir(parent: &Path, name: &str) -> Entry {
            Entry {
                name: name.to_string(),
                path: parent.join(name),
                is_dir: true,
                size: EntrySize::UNCOUNTED,
                modified: Some(SystemTime::UNIX_EPOCH),
                is_symlink: false,
                link_broken: false,
                hidden: name.starts_with('.'),
                kind: EntryKind::Folder,
                // Ownership and permissions are fixtures here: these
                // helpers build entries for tests about names, sizes and
                // ordering, none of which read them.
                mode: 0o755,
                uid: 1000,
                owner: Some("alex".to_string()),
                origin: None,
                packed: None,
            }
        }

        /// Seeds `dir` with `count` plain files, for the "a directory
        /// with a very large number of entries" failure mode — the
        /// point is that nothing above `FsBackend` (sorting, filtering)
        /// chokes on or is quadratic in a directory this size, which is
        /// cheap to assert against a seeded list and expensive to
        /// arrange as an actual directory on whatever disk tier 1 runs
        /// on.
        pub fn seed_many(&self, dir: impl Into<PathBuf>, count: usize) {
            let dir = dir.into();
            let entries = (0..count)
                .map(|i| Self::file(&dir, &format!("file{i}.txt"), i as u64))
                .collect();
            self.seed(dir, entries);
        }

        /// After this, `read_dir` on `dir` reports permission denied.
        pub fn make_unreadable(&self, dir: impl Into<PathBuf>) {
            self.unreadable.lock().unwrap().insert(dir.into());
        }

        /// After this, `entry_path` is left out of any listing that
        /// contains it, and a direct `stat` on it fails as "not found".
        pub fn make_vanish(&self, entry_path: impl Into<PathBuf>) {
            self.vanished.lock().unwrap().insert(entry_path.into());
        }

        pub fn set_home(&self, home: impl Into<PathBuf>) {
            *self.home.lock().unwrap() = home.into();
        }
    }

    impl FsBackend for MockBackend {
        fn count_children(&self, path: &Path) -> Result<usize, FilesError> {
            // Through `read_dir` so an unreadable or absent directory
            // fails here exactly as it does there — a mock whose two
            // answers about the same directory disagree is worse than no
            // mock.
            self.read_dir(path).map(|entries| entries.len())
        }

        fn read_dir(&self, path: &Path) -> Result<Vec<Entry>, FilesError> {
            if self.unreadable.lock().unwrap().contains(path) {
                return Err(FilesError::PermissionDenied {
                    path: path.to_path_buf(),
                });
            }
            let entries = self
                .tree
                .lock()
                .unwrap()
                .get(path)
                .cloned()
                .ok_or_else(|| FilesError::NotFound {
                    path: path.to_path_buf(),
                })?;
            let vanished = self.vanished.lock().unwrap();
            Ok(entries
                .into_iter()
                .filter(|e| !vanished.contains(&e.path))
                .collect())
        }

        fn stat(&self, path: &Path) -> Result<Entry, FilesError> {
            if self.vanished.lock().unwrap().contains(path) {
                return Err(FilesError::NotFound {
                    path: path.to_path_buf(),
                });
            }
            let tree = self.tree.lock().unwrap();
            if tree.contains_key(path) {
                // The path is itself a seeded directory — synthesise a
                // directory `Entry` for it rather than requiring every
                // test to seed the directory *and* its own entry in its
                // parent.
                let name = path
                    .file_name()
                    .map(|n| n.to_string_lossy().into_owned())
                    .unwrap_or_else(|| path.to_string_lossy().into_owned());
                return Ok(Entry {
                    name,
                    path: path.to_path_buf(),
                    is_dir: true,
                    size: EntrySize::UNCOUNTED,
                    modified: Some(SystemTime::UNIX_EPOCH),
                    is_symlink: false,
                    link_broken: false,
                    hidden: false,
                    kind: EntryKind::Folder,
                    // Ownership and permissions are fixtures here: these
                    // helpers build entries for tests about names, sizes and
                    // ordering, none of which read them.
                    mode: 0o644,
                    uid: 1000,
                    owner: Some("alex".to_string()),
                    origin: None,
                    packed: None,
                });
            }
            for entries in tree.values() {
                if let Some(entry) = entries.iter().find(|e| e.path == path) {
                    return Ok(entry.clone());
                }
            }
            Err(FilesError::NotFound {
                path: path.to_path_buf(),
            })
        }

        fn home_dir(&self) -> PathBuf {
            self.home.lock().unwrap().clone()
        }

        fn canonicalize(&self, path: &Path) -> Result<PathBuf, FilesError> {
            // No `.`/`..`/symlinks in the seeded tree to resolve — a
            // mock path is canonical by construction, as long as it is
            // one the tree actually knows about.
            self.stat(path).map(|_| path.to_path_buf())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::mock::MockBackend;
    use super::*;

    #[test]
    fn an_unreadable_directory_is_an_error_carrying_a_usable_message() {
        let backend = MockBackend::new();
        let dir = PathBuf::from("/secret");
        backend.seed(dir.clone(), vec![MockBackend::file(&dir, "a.txt", 1)]);
        backend.make_unreadable(&dir);

        let err = backend
            .read_dir(&dir)
            .expect_err("a directory with permission revoked must not read as empty");
        assert!(matches!(err, FilesError::PermissionDenied { .. }));
        assert!(
            err.to_string().to_lowercase().contains("permission"),
            "the message must say what went wrong, not just that something did: {err}"
        );
    }

    /// The property `hlconfig::storage` already paid for once: "we could
    /// not read this" and "this has nothing in it" have to stay two
    /// different, distinguishable outcomes.
    #[test]
    fn an_empty_readable_directory_is_not_confused_with_an_unreadable_one() {
        let backend = MockBackend::new();
        let dir = PathBuf::from("/empty");
        backend.seed(dir.clone(), vec![]);

        let entries = backend.read_dir(&dir).expect("an empty directory reads fine");
        assert!(entries.is_empty());
    }

    #[test]
    fn an_entry_that_vanishes_between_listing_and_stat_does_not_fail_the_listing() {
        let backend = MockBackend::new();
        let dir = PathBuf::from("/home/mock/docs");
        let staying = MockBackend::file(&dir, "staying.txt", 10);
        let going = MockBackend::file(&dir, "going.txt", 20);
        backend.seed(dir.clone(), vec![staying.clone(), going.clone()]);
        backend.make_vanish(&going.path);

        let entries = backend.read_dir(&dir).expect("one vanished entry must not fail the read");
        assert_eq!(entries, vec![staying]);
        assert!(matches!(
            backend.stat(&going.path),
            Err(FilesError::NotFound { .. })
        ));
    }

    #[test]
    fn one_unreadable_entry_does_not_lose_the_other_entries() {
        let backend = MockBackend::new();
        let dir = PathBuf::from("/home/mock/mixed");
        let good_a = MockBackend::file(&dir, "a.txt", 1);
        let bad = MockBackend::file(&dir, "b.txt", 2);
        let good_c = MockBackend::file(&dir, "c.txt", 3);
        backend.seed(dir.clone(), vec![good_a.clone(), bad.clone(), good_c.clone()]);
        backend.make_vanish(&bad.path);

        let entries = backend.read_dir(&dir).unwrap();
        assert_eq!(entries.len(), 2, "the two good entries must survive the one bad one");
        assert!(entries.contains(&good_a));
        assert!(entries.contains(&good_c));
        assert!(!entries.iter().any(|e| e.path == bad.path));
    }

    #[test]
    fn a_very_large_directory_is_still_read_in_full() {
        let backend = MockBackend::new();
        let dir = PathBuf::from("/home/mock/huge");
        backend.seed_many(&dir, 50_000);

        let entries = backend.read_dir(&dir).unwrap();
        assert_eq!(entries.len(), 50_000);
    }

    #[test]
    fn a_broken_symlink_is_a_state_reported_alongside_the_others_not_an_error() {
        let backend = MockBackend::new();
        let dir = PathBuf::from("/home/mock/links");
        let mut link = MockBackend::file(&dir, "dangling", 0);
        link.is_symlink = true;
        link.link_broken = true;
        backend.seed(dir.clone(), vec![link.clone()]);

        let entries = backend.read_dir(&dir).unwrap();
        assert_eq!(entries.len(), 1);
        assert!(entries[0].is_symlink);
        assert!(entries[0].link_broken);
    }

    // --- against the real filesystem -------------------------------------

    #[test]
    fn the_real_backend_reads_a_tempdir() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("a.txt"), b"hi").unwrap();
        std::fs::create_dir(dir.path().join("sub")).unwrap();

        let entries = StdBackend.read_dir(dir.path()).unwrap();
        assert_eq!(entries.len(), 2);
        let file = entries.iter().find(|e| e.name == "a.txt").unwrap();
        assert!(!file.is_dir);
        assert_eq!(file.size, EntrySize::Bytes(2));
        let subdir = entries.iter().find(|e| e.name == "sub").unwrap();
        assert!(subdir.is_dir);
    }

    #[test]
    fn the_real_backend_reports_a_broken_symlink_as_a_state_not_an_error() {
        let dir = tempfile::tempdir().unwrap();
        let target = dir.path().join("gone");
        let link = dir.path().join("dangling");
        std::fs::write(&target, b"x").unwrap();
        #[cfg(unix)]
        std::os::unix::fs::symlink(&target, &link).unwrap();
        std::fs::remove_file(&target).unwrap();

        let entry = StdBackend.stat(&link).expect("a broken link must stat, not error");
        assert!(entry.is_symlink);
        assert!(entry.link_broken);
    }

    #[test]
    fn the_real_backend_reports_a_missing_path_as_not_found() {
        let dir = tempfile::tempdir().unwrap();
        let err = StdBackend
            .stat(&dir.path().join("nope"))
            .expect_err("a path that was never there is NotFound");
        assert!(matches!(err, FilesError::NotFound { .. }));
    }

    #[test]
    fn the_real_backend_reading_a_plain_file_as_a_directory_says_so() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("a.txt");
        std::fs::write(&file, b"x").unwrap();

        let err = StdBackend
            .read_dir(&file)
            .expect_err("a plain file is not a directory to read");
        assert!(matches!(err, FilesError::NotADirectory { .. }));
    }
}
