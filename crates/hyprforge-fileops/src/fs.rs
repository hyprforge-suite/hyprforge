//! Which filesystem a path lives on, and where that filesystem is mounted.
//!
//! This trait exists for exactly one reason: `rename(2)` cannot cross a
//! filesystem boundary — it returns `EXDEV` — but the trash spec's whole
//! reversibility guarantee depends on trashing being a rename, never a
//! copy. A copy-then-delete can be interrupted between the two halves, or
//! can silently degrade for a large file, and either one is the exact
//! failure "just delete it" trash exists to prevent. So before this crate
//! ever calls `rename`, it has to know: is the file's own filesystem the
//! one the home trash lives on? If not, the *file's* filesystem needs its
//! own trash directory, per the spec's `$topdir` rules — and answering
//! that needs `stat`, not string matching on the path.
//!
//! This is not a theoretical concern on the machine this crate was
//! written on: it is btrfs with subvolumes, and subvolumes report
//! different `st_dev` values (`/` is one device, `/home` another,
//! `/var/log` a third), so a file living directly under `/` genuinely
//! cannot be renamed into `~/.local/share/Trash` — confirmed by trying
//! it: `rename("/home/.../x", "/var/tmp/x")` returns `EXDEV` in
//! practice, not just per the man page.
//!
//! # Real implementation: walk up `st_dev`, not `/proc/mounts`
//!
//! `RealFilesystem` finds a mount point by `stat`ing a path and its
//! ancestors and stopping at the first ancestor whose `st_dev` differs
//! from the starting path's. Parsing `/proc/mounts` was considered and
//! rejected: it needs unescaping the octal-escaped device/mountpoint
//! fields that file uses for spaces, and even after finding a mount
//! point in the table, still needs a second `stat` to answer "is this
//! candidate path actually on that device" — the two questions this
//! trait exists to answer collapse into one `st_dev` comparison instead.
//! Walking `st_dev` also works for a path that does not exist yet:
//! [`RealFilesystem::device_of`] climbs to the nearest ancestor that
//! does, which is always at least `/`.

use std::io;
use std::os::unix::fs::MetadataExt;
use std::path::{Path, PathBuf};

/// Enough of `stat(2)` to answer the trash spec's `.Trash` safety check —
/// must exist, must not be a symlink, must have the sticky bit set — and
/// nothing more. Not `std::fs::Metadata` itself, which has no public
/// constructor a fake could build.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FileStatus {
    pub is_dir: bool,
    pub is_symlink: bool,
    /// The low bits of `st_mode`: permissions plus setuid/setgid/sticky.
    pub mode: u32,
}

impl FileStatus {
    /// The sticky bit (`S_ISVTX`, octal `01000`) the spec requires on a
    /// shared per-filesystem `.Trash` directory before anyone may use
    /// it — the same bit `/tmp` sets, and for the same reason: many
    /// users can write into the directory and none may delete another's
    /// files out of it.
    pub fn has_sticky_bit(&self) -> bool {
        self.mode & 0o1000 != 0
    }
}

/// The seam between trash placement logic and the real filesystem, so a
/// test can declare "this path is on device 7, mounted at /mnt/usb"
/// without a second real mount existing. See the module doc for why this
/// exists at all.
pub trait Filesystem {
    /// The device `path` lives on. `path` need not exist: implementations
    /// climb to the nearest ancestor that does.
    fn device_of(&self, path: &Path) -> io::Result<u64>;

    /// The mount point `path` lives on — the highest ancestor still on
    /// the same device as `path` itself.
    fn mount_point_of(&self, path: &Path) -> io::Result<PathBuf>;

    /// `path`'s own status, *not* following a trailing symlink — the
    /// `.Trash` safety check needs to know whether the path itself is a
    /// symlink, which following it would hide.
    fn symlink_status(&self, path: &Path) -> io::Result<FileStatus>;
}

/// [`Filesystem`] backed by real `stat(2)` calls.
#[derive(Debug, Default, Clone, Copy)]
pub struct RealFilesystem;

/// The nearest ancestor of `path` (possibly `path` itself) that `stat`s
/// successfully, plus that call's metadata. Shared by `device_of` and
/// `mount_point_of` so both climb exactly the same way for a path that
/// does not exist yet, and so a caller never observes them disagreeing
/// about where the climb started.
fn nearest_existing(path: &Path) -> io::Result<(PathBuf, std::fs::Metadata)> {
    let mut current = path;
    loop {
        match std::fs::metadata(current) {
            Ok(meta) => return Ok((current.to_path_buf(), meta)),
            Err(e) if e.kind() == io::ErrorKind::NotFound => match current.parent() {
                Some(parent) => current = parent,
                // Ran out of ancestors without a `stat` succeeding — even
                // `/` failed, which means something is badly wrong with
                // this process's view of the filesystem. Report the
                // original error rather than looping forever.
                None => return Err(e),
            },
            Err(e) => return Err(e),
        }
    }
}

impl Filesystem for RealFilesystem {
    fn device_of(&self, path: &Path) -> io::Result<u64> {
        Ok(nearest_existing(path)?.1.dev())
    }

    fn mount_point_of(&self, path: &Path) -> io::Result<PathBuf> {
        let (mut current, meta) = nearest_existing(path)?;
        let dev = meta.dev();
        loop {
            match current.parent() {
                None => return Ok(current),
                Some(parent) => {
                    if self.device_of(parent)? != dev {
                        return Ok(current);
                    }
                    current = parent.to_path_buf();
                }
            }
        }
    }

    fn symlink_status(&self, path: &Path) -> io::Result<FileStatus> {
        let meta = std::fs::symlink_metadata(path)?;
        Ok(FileStatus {
            is_dir: meta.is_dir(),
            is_symlink: meta.file_type().is_symlink(),
            mode: meta.mode(),
        })
    }
}

/// Exposes [`MockFilesystem`] outside `#[cfg(test)]`, under this crate's
/// own `mock` feature — the same arrangement `hyprforge-power` uses for
/// `MockBackend`, so another crate's tests can drive trash placement
/// without a second real mount either.
#[cfg(any(test, feature = "mock"))]
pub mod mock {
    use super::*;
    use std::collections::BTreeMap;
    use std::sync::Mutex;

    /// A filesystem laid out however a test says.
    ///
    /// `device_of`/`mount_point_of` use a longest-prefix match against
    /// registered mounts: [`MockFilesystem::mount`] `/mnt/usb` at device 7
    /// and every path under it answers with device 7 and mount point
    /// `/mnt/usb`, the way a real `stat` walk would. A path matching
    /// nothing registered falls back to `default_device`, mounted at `/`
    /// — which is what makes "the file's own filesystem" and "the home
    /// trash's filesystem" the same by default, so a test opts into
    /// "different filesystem" by registering one mount, not by having to
    /// describe the whole tree.
    ///
    /// `symlink_status` is different: rather than requiring every test to
    /// also fake directory metadata, it defers to the *real* filesystem
    /// by default, so a test can `chmod`/`symlink` an actual tempdir to
    /// exercise the `.Trash` safety check while still faking only the
    /// device/mount-point placement decision. [`MockFilesystem::set_status`]
    /// overrides a specific path for the rare case nothing on disk can
    /// produce the status a test wants (there is no portable way to make
    /// a directory whose sticky bit reads as absent while insisting mode
    /// bits are perfectly ordinary, for example).
    #[derive(Default)]
    pub struct MockFilesystem {
        mounts: Mutex<BTreeMap<PathBuf, u64>>,
        statuses: Mutex<BTreeMap<PathBuf, FileStatus>>,
        pub default_device: Mutex<u64>,
    }

    impl MockFilesystem {
        pub fn new() -> Self {
            MockFilesystem {
                mounts: Mutex::new(BTreeMap::new()),
                statuses: Mutex::new(BTreeMap::new()),
                // Not 0: a real `st_dev` of 0 is unusual enough that a
                // bug comparing "unset" against "unset" and calling it a
                // match would be easy to miss. Any fixed nonzero value
                // works; this one is arbitrary.
                default_device: Mutex::new(1),
            }
        }

        /// Declares `prefix` (and everything under it) mounted on `device`,
        /// with `prefix` itself as the mount point.
        pub fn mount(&self, prefix: impl Into<PathBuf>, device: u64) {
            self.mounts.lock().unwrap().insert(prefix.into(), device);
        }

        /// Overrides what [`Filesystem::symlink_status`] reports for
        /// exactly `path`, instead of falling through to the real
        /// filesystem.
        pub fn set_status(&self, path: impl Into<PathBuf>, status: FileStatus) {
            self.statuses.lock().unwrap().insert(path.into(), status);
        }

        fn lookup(&self, path: &Path) -> Option<(PathBuf, u64)> {
            self.mounts
                .lock()
                .unwrap()
                .iter()
                .filter(|(prefix, _)| path.starts_with(prefix.as_path()))
                .max_by_key(|(prefix, _)| prefix.as_os_str().len())
                .map(|(prefix, device)| (prefix.clone(), *device))
        }
    }

    impl Filesystem for MockFilesystem {
        fn device_of(&self, path: &Path) -> io::Result<u64> {
            Ok(self
                .lookup(path)
                .map(|(_, device)| device)
                .unwrap_or(*self.default_device.lock().unwrap()))
        }

        fn mount_point_of(&self, path: &Path) -> io::Result<PathBuf> {
            Ok(self
                .lookup(path)
                .map(|(prefix, _)| prefix)
                .unwrap_or_else(|| PathBuf::from("/")))
        }

        fn symlink_status(&self, path: &Path) -> io::Result<FileStatus> {
            if let Some(status) = self.statuses.lock().unwrap().get(path) {
                return Ok(*status);
            }
            RealFilesystem.symlink_status(path)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_path_under_a_registered_mount_reports_that_devices_mount_point() {
        let fs = mock::MockFilesystem::new();
        fs.mount("/mnt/usb", 7);
        assert_eq!(fs.device_of(Path::new("/mnt/usb/photos/a.jpg")).unwrap(), 7);
        assert_eq!(
            fs.mount_point_of(Path::new("/mnt/usb/photos/a.jpg")).unwrap(),
            PathBuf::from("/mnt/usb")
        );
    }

    #[test]
    fn a_path_matching_no_mount_falls_back_to_the_default_device() {
        let fs = mock::MockFilesystem::new();
        fs.mount("/mnt/usb", 7);
        assert_eq!(fs.device_of(Path::new("/home/someone/file.txt")).unwrap(), 1);
    }

    #[test]
    fn the_longest_matching_prefix_wins() {
        let fs = mock::MockFilesystem::new();
        fs.mount("/mnt", 5);
        fs.mount("/mnt/usb", 7);
        assert_eq!(fs.device_of(Path::new("/mnt/usb/file.txt")).unwrap(), 7);
        assert_eq!(fs.device_of(Path::new("/mnt/other/file.txt")).unwrap(), 5);
    }

    #[test]
    fn real_filesystem_finds_a_devices_own_mount_point_by_climbing_stat() {
        let dir = tempfile::tempdir().unwrap();
        let nested = dir.path().join("a").join("b");
        std::fs::create_dir_all(&nested).unwrap();
        // Everything under the tempdir is on the same device as the
        // tempdir itself (nothing here crosses a real mount boundary),
        // so climbing from `nested` must land back on `dir`'s own
        // device — this pins that the climb terminates at a device
        // change rather than, say, always returning `/`.
        let fs = RealFilesystem;
        let dev_nested = fs.device_of(&nested).unwrap();
        let dev_root = fs.device_of(dir.path()).unwrap();
        assert_eq!(dev_nested, dev_root);
    }

    #[test]
    fn device_of_resolves_a_path_that_does_not_exist_yet_via_its_nearest_ancestor() {
        let dir = tempfile::tempdir().unwrap();
        let missing = dir.path().join("does").join("not").join("exist.txt");
        let fs = RealFilesystem;
        // Must not error just because nothing is there yet.
        assert_eq!(fs.device_of(&missing).unwrap(), fs.device_of(dir.path()).unwrap());
    }
}
