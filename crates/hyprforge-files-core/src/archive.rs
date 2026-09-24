//! An archive as a place you can be.
//!
//! `~/Downloads/linux-6.6.tar.xz/kernel/sched/core.c` is not a path the
//! kernel has ever heard of, and it is exactly the path this browser
//! navigates to. The archive file is a real thing on disk; everything
//! after it names a member inside. [`split`] is the whole trick, and
//! [`ArchiveFsBackend`] is a second [`FsBackend`] that claims those
//! paths — the arrangement [`crate::trash`]'s module doc already
//! describes for the trash: *a place that lists differently is a
//! different backend*, and the decision lives in one router.
//!
//! # Why the index is cached
//!
//! A tar has no table of contents (see `hyprforge_archive::read`), so
//! listing one means decompressing the whole file. Walking three levels
//! down into a 900MB `.tar.xz` would do that three times — once per
//! `read_dir`, plus once more for each folder the Size column counts.
//! So an archive is read once and the result is kept, keyed on the
//! archive's own mtime and length: an archive rewritten under the
//! browser reads as a different file and is re-indexed, which is what
//! makes "edit an archive and see the change" work without anything
//! having to remember to invalidate anything.
//!
//! The cache is deliberately small. An index is roughly a hundred bytes
//! per member, so one kernel tarball is several megabytes of it — a
//! browser that kept every archive it had ever opened would grow without
//! bound in a process that also holds decoded thumbnails. Four is enough
//! for "back out of this archive and into the one next to it" and small
//! enough not to matter.

use crate::backend::FsBackend;
use crate::types::{Entry, EntryKind, EntrySize, FilesError, ItemCount};
use hyprforge_archive::{ArchiveError, Format, Index, Keyring, Member, StdArchives};
use std::collections::VecDeque;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

/// How many archives' indexes are kept at once — see the module doc.
const CACHED_ARCHIVES: usize = 4;

/// Splits a browser path into the archive file it goes through and the
/// member inside it.
///
/// `None` when no component of the path is an archive, which is the
/// answer for very nearly every path and is why the cheap test comes
/// first: only a component whose *name* could be an archive is ever
/// `stat`ed, so routing an ordinary directory costs no syscalls at all.
///
/// The member is `""` when the path *is* the archive — the archive's own
/// root, which is where navigating into one lands.
pub fn split(path: &Path) -> Option<(PathBuf, String)> {
    let mut prefix = PathBuf::new();
    let mut components = path.components();

    while let Some(component) = components.next() {
        prefix.push(component);
        if !Format::plausible_by_name(&prefix) {
            continue;
        }
        // A *file*, not a directory: a folder someone has called
        // `backup.zip` is a folder, and browsing into it must keep
        // working. `is_file` follows symlinks, which is right — a link
        // to an archive is a way to reach that archive.
        if !prefix.is_file() {
            continue;
        }
        let member = components
            .map(|c| c.as_os_str().to_string_lossy().into_owned())
            .collect::<Vec<_>>()
            .join("/");
        return Some((prefix, member));
    }
    None
}

/// Whether a *name* is one this suite could open as an archive.
///
/// The name and nothing else — no filesystem access at all, which is
/// what lets [`crate::browser::Browser`] use it while doing no I/O. A
/// caller that also knows the entry is not a directory (every listing
/// does) has the whole answer; one that does not must go through
/// [`split`], which checks.
pub fn looks_browsable(name: &str) -> bool {
    Format::plausible_by_name(Path::new(name))
}

/// The path a browser uses for `member` inside `archive`.
///
/// The inverse of [`split`], and the only place the two halves are
/// joined — a caller doing it by hand is how one of them ends up using
/// a separator the other does not.
pub fn join(archive: &Path, member: &str) -> PathBuf {
    if member.is_empty() {
        return archive.to_path_buf();
    }
    archive.join(member)
}

/// Lists the inside of an archive as though it were a directory tree.
pub struct ArchiveFsBackend {
    archives: Box<dyn hyprforge_archive::ArchiveBackend>,
    cache: Mutex<VecDeque<Cached>>,
    /// Passwords this process has been told, one per archive.
    ///
    /// Shared with whoever owns this backend, because the *prompt* is
    /// the host's and the *listing* is this backend's, and a password
    /// typed into the one has to reach the other. See
    /// [`hyprforge_archive::Keyring`], which also says why it is never
    /// written anywhere.
    keyring: Arc<Keyring>,
}

struct Cached {
    archive: PathBuf,
    /// What the archive file looked like when it was read. Not a
    /// timestamp on its own: an mtime has one-second resolution on some
    /// filesystems, and a rewrite that lands inside the same second
    /// would otherwise serve a stale index of a file that has changed.
    /// The length moving is the other half of the same answer.
    stamp: (Option<std::time::SystemTime>, u64),
    index: Arc<Index>,
}

impl Default for ArchiveFsBackend {
    fn default() -> Self {
        ArchiveFsBackend::with(Box::new(StdArchives))
    }
}

impl ArchiveFsBackend {
    /// With a specific archive backend — the seam this crate's tests use
    /// to browse a seeded archive rather than one on disk.
    pub fn with(archives: Box<dyn hyprforge_archive::ArchiveBackend>) -> Self {
        ArchiveFsBackend::with_keyring(archives, Arc::new(Keyring::new()))
    }

    pub fn with_keyring(
        archives: Box<dyn hyprforge_archive::ArchiveBackend>,
        keyring: Arc<Keyring>,
    ) -> Self {
        ArchiveFsBackend {
            archives,
            cache: Mutex::new(VecDeque::new()),
            keyring,
        }
    }

    /// The passwords this backend consults, so a host can add one.
    pub fn keyring(&self) -> &Arc<Keyring> {
        &self.keyring
    }

    /// Whether this backend is the one for `path`.
    pub fn claims(&self, path: &Path) -> bool {
        split(path).is_some()
    }

    fn stamp(archive: &Path) -> (Option<std::time::SystemTime>, u64) {
        match std::fs::metadata(archive) {
            Ok(meta) => (meta.modified().ok(), meta.len()),
            Err(_) => (None, 0),
        }
    }

    /// The archive's index, read if it is not already in hand.
    fn index(&self, archive: &Path) -> Result<Arc<Index>, FilesError> {
        let stamp = Self::stamp(archive);
        if let Some(hit) = self
            .cache
            .lock()
            .unwrap()
            .iter()
            .find(|c| c.archive == archive && c.stamp == stamp)
        {
            return Ok(hit.index.clone());
        }

        let index = Arc::new(
            self.archives
                .index_with(archive, &self.keyring.unlock_for(archive))
                .map_err(|e| to_files_error(archive, e))?,
        );

        let mut cache = self.cache.lock().unwrap();
        // Any stale entry for this same archive goes, so a rewritten
        // archive does not sit in the cache twice.
        cache.retain(|c| c.archive != archive);
        cache.push_back(Cached {
            archive: archive.to_path_buf(),
            stamp,
            index: index.clone(),
        });
        while cache.len() > CACHED_ARCHIVES {
            cache.pop_front();
        }
        Ok(index)
    }

    /// Who a member belongs to: whoever owns the archive file.
    ///
    /// Not the ids recorded *inside* the archive, which name users on
    /// whatever machine built it and very often do not exist here — a
    /// tarball from a build server is full of uid 1000 meaning somebody
    /// else entirely. And not the current user either, which was the
    /// first answer here and rendered as `0` on a machine where `$UID`
    /// is a shell variable rather than an environment one: an archive
    /// member displayed as owned by root, which is both wrong and
    /// alarming.
    ///
    /// The archive's own owner is the one claim that is true on this
    /// machine: these bytes are inside that file, and that file belongs
    /// to someone this system can name.
    fn owner_of(&self, archive: &Path) -> (u32, Option<String>) {
        match crate::backend::StdBackend.stat(archive) {
            Ok(entry) => (entry.uid, entry.owner),
            Err(_) => (0, None),
        }
    }

    /// One member as a listing row.
    fn entry(archive: &Path, member: &Member, owner: &(u32, Option<String>)) -> Entry {
        let name = member.name().to_string();
        Entry {
            kind: EntryKind::classify(member.is_dir, &name),
            path: join(archive, &member.path),
            is_dir: member.is_dir,
            size: if member.is_dir {
                // Filled in by `count_children` the same way a real
                // folder's is — the browser's second pass does not know
                // or care that this folder is inside a zip.
                EntrySize::UNCOUNTED
            } else if member.size_known {
                EntrySize::Bytes(member.size)
            } else {
                // A size the archive never stated. `Bytes(0)` would
                // claim the file is empty, which is the "could not read"
                // collapsed into "there is nothing there" mistake
                // CLAUDE.md names, in a table cell.
                EntrySize::Items(ItemCount::Unreadable)
            },
            modified: member.modified,
            // A symlink *recorded in an archive* is not a symlink on
            // this filesystem: there is no link to follow and nothing to
            // be broken. Reporting it as one would put a broken-link
            // badge on every link in every tarball, since the target
            // does not exist until something extracts it.
            is_symlink: false,
            link_broken: false,
            hidden: name.starts_with('.'),
            mode: member.mode.unwrap_or(if member.is_dir { 0o755 } else { 0o644 }) & 0o7777,
            // An archive records a numeric owner at best, and very often
            // nothing at all. The current user is the honest answer for
            // "who owns this thing you are looking at", since extracting
            // it is what would give it an owner.
            uid: owner.0,
            owner: owner.1.clone(),
            origin: None,
            // What mockup `1j` calls "Packed". `None` for a tar, where
            // there is no per-member compressed size to report.
            packed: member.compressed.filter(|_| !member.is_dir),
            name,
        }
    }
}

/// An archive failure as a listing failure.
///
/// Every variant keeps its own sentence — see [`FilesError::Elsewhere`].
/// The two that map onto ordinary filesystem failures are mapped, so the
/// browser renders them the way it renders the same thing happening to a
/// directory.
fn to_files_error(path: &Path, error: ArchiveError) -> FilesError {
    let message = error.to_string();
    match error {
        ArchiveError::NotAnArchive { .. } => FilesError::NotAnArchive {
            path: path.to_path_buf(),
        },
        ArchiveError::PasswordRequired { .. } => FilesError::PasswordRequired {
            path: path.to_path_buf(),
        },
        ArchiveError::MemberNotFound { .. } => FilesError::NotFound {
            path: path.to_path_buf(),
        },
        ArchiveError::Io { source, .. } if source.kind() == std::io::ErrorKind::PermissionDenied => {
            FilesError::PermissionDenied {
                path: path.to_path_buf(),
            }
        }
        _ => FilesError::Elsewhere {
            path: path.to_path_buf(),
            message,
        },
    }
}

impl FsBackend for ArchiveFsBackend {
    fn read_dir(&self, path: &Path) -> Result<Vec<Entry>, FilesError> {
        let Some((archive, member)) = split(path) else {
            return Err(FilesError::NotFound {
                path: path.to_path_buf(),
            });
        };
        let index = self.index(&archive)?;
        // Once for the whole listing, not once per row — the same
        // reason `build_entry_with` shares one name cache across a
        // directory read.
        let owner = self.owner_of(&archive);
        let children = index.children(&member).ok_or_else(|| {
            // A member that is there but is a file is "not a directory";
            // one that is not there at all is "not found". Two different
            // sentences, and the browser draws them differently.
            if index.get(&member).is_some() {
                FilesError::NotADirectory {
                    path: path.to_path_buf(),
                }
            } else {
                FilesError::NotFound {
                    path: path.to_path_buf(),
                }
            }
        })?;
        Ok(children
            .into_iter()
            .map(|member| Self::entry(&archive, member, &owner))
            .collect())
    }

    fn stat(&self, path: &Path) -> Result<Entry, FilesError> {
        let Some((archive, member)) = split(path) else {
            return Err(FilesError::NotFound {
                path: path.to_path_buf(),
            });
        };

        if member.is_empty() {
            // The archive file itself, asked about as a *place*. It is a
            // real file on disk and every other answer comes from
            // `stat`, but `is_dir` has to be true: this is the path a
            // navigation is about to land on, and something that
            // answered "not a directory" here would refuse to open the
            // location it just offered. The kind stays `Archive` so the
            // row keeps its own icon rather than becoming a folder.
            let mut entry = crate::backend::StdBackend.stat(&archive)?;
            // Confirms it really is one, rather than trusting the name —
            // `split` already checked the name, this checks the bytes.
            self.index(&archive)?;
            entry.is_dir = true;
            entry.kind = EntryKind::Archive;
            entry.size = EntrySize::UNCOUNTED;
            return Ok(entry);
        }

        let index = self.index(&archive)?;
        let owner = self.owner_of(&archive);
        index
            .get(&member)
            .map(|m| Self::entry(&archive, m, &owner))
            .ok_or_else(|| FilesError::NotFound {
                path: path.to_path_buf(),
            })
    }

    fn count_children(&self, path: &Path) -> Result<usize, FilesError> {
        let Some((archive, member)) = split(path) else {
            return Err(FilesError::NotFound {
                path: path.to_path_buf(),
            });
        };
        // Through the cached index, so counting the folders in a listing
        // costs nothing beyond the one read that produced the listing.
        let index = self.index(&archive)?;
        index
            .children(&member)
            .map(|children| children.len())
            .ok_or_else(|| FilesError::NotFound {
                path: path.to_path_buf(),
            })
    }

    fn home_dir(&self) -> PathBuf {
        crate::backend::StdBackend.home_dir()
    }

    fn canonicalize(&self, path: &Path) -> Result<PathBuf, FilesError> {
        let Some((archive, member)) = split(path) else {
            return Err(FilesError::NotFound {
                path: path.to_path_buf(),
            });
        };
        // Only the real half can be canonicalized — there are no
        // symlinks or `..` left inside an archive, because
        // `hyprforge_archive::model::normalise` resolved them when the
        // index was built.
        let archive = crate::backend::StdBackend.canonicalize(&archive)?;
        Ok(join(&archive, &member))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use hyprforge_archive::backend::mock::MockArchives;

    fn mounted(archive: &Path) -> ArchiveFsBackend {
        let mock = MockArchives::new();
        mock.seed(
            archive,
            &[
                ("readme.md", b"# hello"),
                ("docs/guide.txt", b"a guide"),
                ("docs/deep/more.txt", b"more"),
                ("empty/", b""),
            ],
        );
        ArchiveFsBackend::with(Box::new(mock))
    }

    /// The one test that needs a real file, because `split` deliberately
    /// asks the filesystem whether the archive-looking component is a
    /// file.
    fn on_disk(dir: &Path, name: &str) -> PathBuf {
        let path = dir.join(name);
        std::fs::write(&path, b"stand-in").unwrap();
        path
    }

    #[test]
    fn a_path_through_an_archive_splits_into_the_file_and_the_member() {
        let dir = tempfile::tempdir().unwrap();
        let archive = on_disk(dir.path(), "sample.zip");

        assert_eq!(
            split(&archive.join("docs/guide.txt")),
            Some((archive.clone(), "docs/guide.txt".to_string()))
        );
        assert_eq!(
            split(&archive),
            Some((archive, String::new())),
            "the archive itself is its own root"
        );
    }

    #[test]
    fn an_ordinary_path_is_not_claimed() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir(dir.path().join("docs")).unwrap();
        assert_eq!(split(&dir.path().join("docs")), None);
    }

    /// A folder someone has named `backup.zip` is a folder, and browsing
    /// into it has to keep working — the name is a hint, and the
    /// filesystem is the answer.
    #[test]
    fn a_directory_named_like_an_archive_is_still_a_directory() {
        let dir = tempfile::tempdir().unwrap();
        let folder = dir.path().join("backup.zip");
        std::fs::create_dir(&folder).unwrap();
        std::fs::write(folder.join("inside.txt"), b"x").unwrap();

        assert_eq!(split(&folder), None);
        assert_eq!(split(&folder.join("inside.txt")), None);
    }

    #[test]
    fn listing_an_archives_root_shows_what_is_at_its_top_level() {
        let dir = tempfile::tempdir().unwrap();
        let archive = on_disk(dir.path(), "sample.zip");
        let backend = mounted(&archive);

        let mut names: Vec<String> = backend
            .read_dir(&archive)
            .unwrap()
            .into_iter()
            .map(|e| e.name)
            .collect();
        names.sort();
        assert_eq!(names, ["docs", "empty", "readme.md"]);
    }

    #[test]
    fn listing_a_folder_inside_an_archive_shows_only_its_own_children() {
        let dir = tempfile::tempdir().unwrap();
        let archive = on_disk(dir.path(), "sample.zip");
        let backend = mounted(&archive);

        let entries = backend.read_dir(&archive.join("docs")).unwrap();
        let mut names: Vec<&str> = entries.iter().map(|e| e.name.as_str()).collect();
        names.sort();
        assert_eq!(names, ["deep", "guide.txt"]);
        assert!(entries.iter().find(|e| e.name == "deep").unwrap().is_dir);
        assert_eq!(
            entries.iter().find(|e| e.name == "guide.txt").unwrap().size,
            EntrySize::Bytes(7)
        );
    }

    #[test]
    fn an_empty_folder_inside_an_archive_lists_as_empty_rather_than_missing() {
        let dir = tempfile::tempdir().unwrap();
        let archive = on_disk(dir.path(), "sample.zip");
        let backend = mounted(&archive);

        assert_eq!(backend.read_dir(&archive.join("empty")).unwrap().len(), 0);
        assert!(matches!(
            backend.read_dir(&archive.join("nowhere")),
            Err(FilesError::NotFound { .. })
        ));
    }

    #[test]
    fn a_file_inside_an_archive_is_not_a_directory_to_list() {
        let dir = tempfile::tempdir().unwrap();
        let archive = on_disk(dir.path(), "sample.zip");
        let backend = mounted(&archive);

        assert!(matches!(
            backend.read_dir(&archive.join("readme.md")),
            Err(FilesError::NotADirectory { .. })
        ));
    }

    /// The archive is a file on disk and a directory to navigate into,
    /// and `stat` is where those two facts have to be reconciled.
    #[test]
    fn the_archive_itself_stats_as_a_place_that_can_be_opened() {
        let dir = tempfile::tempdir().unwrap();
        let archive = on_disk(dir.path(), "sample.zip");
        let backend = mounted(&archive);

        let entry = backend.stat(&archive).unwrap();
        assert!(entry.is_dir, "a location that cannot be opened is not a location");
        assert_eq!(entry.kind, EntryKind::Archive, "and it keeps its own icon");
    }

    #[test]
    fn counting_a_folder_inside_an_archive_answers_from_the_index() {
        let dir = tempfile::tempdir().unwrap();
        let archive = on_disk(dir.path(), "sample.zip");
        let backend = mounted(&archive);

        assert_eq!(backend.count_children(&archive.join("docs")).unwrap(), 2);
        assert_eq!(backend.count_children(&archive.join("empty")).unwrap(), 0);
    }

    #[test]
    fn a_damaged_archive_reports_its_own_sentence_rather_than_an_empty_listing() {
        let dir = tempfile::tempdir().unwrap();
        let archive = on_disk(dir.path(), "sample.zip");
        let mock = MockArchives::new();
        mock.seed(&archive, &[("a.txt", b"x")]);
        mock.make_damaged(&archive, "the central directory is truncated");
        let backend = ArchiveFsBackend::with(Box::new(mock));

        let err = backend
            .read_dir(&archive)
            .expect_err("a damaged archive must not read as an empty one");
        assert!(
            err.to_string().contains("truncated"),
            "the reason has to survive: {err}"
        );
    }

    #[test]
    fn a_size_the_archive_never_stated_does_not_render_as_zero_bytes() {
        let dir = tempfile::tempdir().unwrap();
        let archive = on_disk(dir.path(), "dump.sql.gz");
        let mock = MockArchives::new();
        mock.seed(&archive, &[("dump.sql", b"")]);
        let backend = ArchiveFsBackend::with(Box::new(mock));

        // The mock states sizes, so this asserts the *mapping* rather
        // than the mock: a member with `size_known` false must not
        // become `Bytes(0)`.
        let mut member = Member::file("dump.sql", 0);
        member.size_known = false;
        let entry = ArchiveFsBackend::entry(&archive, &member, &(1000, Some("alex".into())));
        assert_ne!(entry.size, EntrySize::Bytes(0));
        let _ = backend;
    }
}

#[cfg(test)]
mod owner_tests {
    use super::*;

    /// The first version of this answered with the *current user*, read
    /// from `$UID` — a shell variable, not an environment one, so it
    /// resolved to `0` and every archive member showed as owned by root.
    /// Alarming, and wrong.
    #[test]
    fn a_member_is_owned_by_whoever_owns_the_archive_and_never_by_root_by_accident() {
        let dir = tempfile::tempdir().unwrap();
        let archive = dir.path().join("sample.zip");
        std::fs::write(&archive, b"stand-in").unwrap();

        let backend = ArchiveFsBackend::default();
        let (uid, _name) = backend.owner_of(&archive);

        let real = crate::backend::StdBackend.stat(&archive).unwrap();
        assert_eq!(uid, real.uid, "a member's owner is the archive's owner");
        assert_ne!(
            uid, 0,
            "a file this test just wrote is not owned by root, and must not display as though it were"
        );
    }
}
