//! The freedesktop.org trash specification: trash, restore, list.
//!
//! Scope for this pass is trash only — no copy, move or rename; those
//! belong to `hyprforge-files-core`, later.
//!
//! # Placement
//!
//! A file on the same filesystem as `$XDG_DATA_HOME` goes to the home
//! trash, `$XDG_DATA_HOME/Trash`. A file on any other filesystem cannot
//! be `rename()`d there (see the module doc on [`crate::fs`] for why that
//! matters), so the spec routes it to a trash directory on its *own*
//! filesystem instead: `$topdir/.Trash/$uid` if `$topdir/.Trash` exists,
//! is not a symlink, and has the sticky bit set: otherwise
//! `$topdir/.Trash-$uid`, created if missing.
//!
//! One case neither of those can serve: `$topdir` itself might not be
//! writable by this user at all (this machine's own `/` is one example —
//! root-owned, mode `755` — and files living directly under it, `/srv`
//! say, have `$topdir` of `/`). Creating `/.Trash-$uid` then fails with
//! `EACCES`, and there is no fallback further than that: the spec gives
//! no third location. [`TrashError::PerFilesystemTrashUnavailable`]
//! names this so a caller can tell "trash it" apart from "there is
//! nowhere to trash this to" and offer permanent deletion instead —
//! never a silent failure, and never a silent copy-then-delete.
//!
//! # `.trashinfo`
//!
//! `[Trash Info]`, `Path=`, `DeletionDate=` in local time with no
//! timezone suffix (`YYYY-MM-DDThh:mm:ss` — see [`crate::localtime`]).
//! `Path` is percent-encoded (see [`crate::percent`]) and is **absolute**
//! for the home trash but **relative to `$topdir`** for a per-filesystem
//! one — confirmed against this machine's own `~/.local/share/Trash`,
//! whose entries store an absolute `Path=`, matching this crate.
//!
//! The info file is written *before* the file is moved. A crash between
//! the two must never produce a `files/` entry with no matching info
//! file — the spec calls that an orphan nothing can restore, because
//! nothing records where it came from. Writing the info file first means
//! the worst case of a crash here is an info file describing an entry
//! that never arrived: inert, and obviously wrong to a human looking at
//! the directory, rather than silently unrestoreable.
//!
//! # Name collisions
//!
//! Two files both called `notes.txt`, trashed the same day or any other
//! day, must not collide in `files/`. This implementation disambiguates
//! by inserting `.N` before the extension — `notes.txt` -> `notes.2.txt`
//! — matching the convention already found in 29 real `.trashinfo`
//! entries on this machine, written by gvfs (`Monkey Around.mp4`,
//! `Monkey Around.2.mp4`). The spec itself does not mandate a particular
//! disambiguation string, only that one be used; matching the existing
//! convention means this crate's trash directory interoperates with one
//! a file manager already wrote into, rather than picking its own scheme
//! and risking two implementations racing to the same collision
//! differently. The name stored in `files/` is **not** percent-encoded —
//! only the `Path=` value inside the info file is; that matches the real
//! entries too (a literal space in the filename on disk).

use crate::fs::{Filesystem, RealFilesystem};
use crate::localtime::LocalDateTime;
use crate::percent;
use serde::Serialize;
use std::ffi::{OsStr, OsString};
use std::io;
use std::os::unix::ffi::{OsStrExt, OsStringExt};
use std::path::{Path, PathBuf};

#[derive(Debug, thiserror::Error)]
pub enum TrashError {
    #[error("{path} does not exist")]
    SourceNotFound { path: PathBuf },

    #[error("could not tell which filesystem {path} is on: {source}")]
    Filesystem {
        path: PathBuf,
        #[source]
        source: io::Error,
    },

    /// The file's own filesystem needs its own trash directory (it
    /// cannot be `rename()`d into the home trash), and neither
    /// `$topdir/.Trash/$uid` nor `$topdir/.Trash-$uid` could be used —
    /// most commonly because this user has no write access to `$topdir`
    /// at all. There is no further fallback the spec defines; a caller
    /// has to offer something else (permanent deletion, with
    /// confirmation), never treat this as transient.
    #[error(
        "{path} could not be trashed: no usable trash directory exists on its filesystem (tried {tried}): {source}"
    )]
    PerFilesystemTrashUnavailable {
        path: PathBuf,
        tried: PathBuf,
        #[source]
        source: io::Error,
    },

    #[error("could not create the trash directory {path}: {source}")]
    CreateTrashDir {
        path: PathBuf,
        #[source]
        source: io::Error,
    },

    #[error("could not move {from} into the trash at {to}: {source}")]
    Move {
        from: PathBuf,
        to: PathBuf,
        #[source]
        source: io::Error,
    },

    #[error("could not write the trash info file {path}: {source}")]
    WriteInfo {
        path: PathBuf,
        #[source]
        source: io::Error,
    },

    #[error("could not read the trash info file {path}: {source}")]
    ReadInfo {
        path: PathBuf,
        #[source]
        source: io::Error,
    },

    /// The file exists and will not parse as a `.trashinfo` file — never
    /// collapsed into "there is nothing here". See the crate-level rule
    /// in CLAUDE.md this exists to satisfy.
    #[error("{path} is not a valid .trashinfo file: {reason}")]
    InvalidInfo { path: PathBuf, reason: String },

    #[error("could not list the trash directory {path}: {source}")]
    ListDir {
        path: PathBuf,
        #[source]
        source: io::Error,
    },

    #[error("could not restore {path}: something already exists there")]
    RestoreWouldOverwrite { path: PathBuf },

    #[error("could not move the trashed file back to {path}: {source}")]
    Restore {
        path: PathBuf,
        #[source]
        source: io::Error,
    },

    #[error("could not remove the trash info file {path}: {source}")]
    RemoveInfo {
        path: PathBuf,
        #[source]
        source: io::Error,
    },

    #[error("could not delete {path}: {source}")]
    Erase {
        path: PathBuf,
        #[source]
        source: io::Error,
    },

    /// Asked to erase something by its stored path, and no record in the
    /// trash names it.
    #[error("{path} is not in the trash")]
    NotInTrash { path: PathBuf },
}

/// Where one trashed file ended up, and everything [`restore`] needs to
/// put it back. Returned by [`trash_into`] and [`list`]/[`list_per_filesystem`]
/// so a caller never has to know which kind of trash an entry came from.
#[derive(Debug, Clone, Serialize)]
pub struct TrashedItem {
    /// Absolute, already resolved against `$topdir` for a per-filesystem
    /// entry — the exact path [`restore`] writes back to.
    pub original_path: PathBuf,
    /// `DeletionDate=`, verbatim: local time, `YYYY-MM-DDThh:mm:ss`.
    pub deleted_at: String,
    pub trashed_file: PathBuf,
    pub info_file: PathBuf,
}

/// Which directory a file was placed into, and whether `Path=` should be
/// written absolute (home trash) or relative to `topdir` (per-filesystem).
struct TrashDir {
    base: PathBuf,
    /// `None` for the home trash. `Some(topdir)` for a per-filesystem
    /// trash, `topdir` being the mount point that trash lives under.
    topdir: Option<PathBuf>,
}

/// `$XDG_DATA_HOME/Trash` — the default location `trash()`/`list()` use.
pub fn home_trash_dir() -> PathBuf {
    hyprforge_paths::data_home().join("Trash")
}

/// Trashes `path` using the real filesystem and the default home trash
/// location.
pub fn trash(path: &Path) -> Result<TrashedItem, TrashError> {
    trash_into(&RealFilesystem, &home_trash_dir(), path)
}

/// [`trash`], parameterised over the filesystem and the home trash
/// location — the seam a test uses to fake "a different device" and a
/// throwaway directory instead of the real `$XDG_DATA_HOME`.
pub fn trash_into<F: Filesystem>(
    fs: &F,
    home_trash: &Path,
    path: &Path,
) -> Result<TrashedItem, TrashError> {
    // `symlink_metadata`, not `metadata`: trashing a symlink must trash
    // the symlink itself, never silently follow it and trash whatever it
    // points at.
    std::fs::symlink_metadata(path).map_err(|source| {
        if source.kind() == io::ErrorKind::NotFound {
            TrashError::SourceNotFound { path: path.to_path_buf() }
        } else {
            TrashError::Filesystem { path: path.to_path_buf(), source }
        }
    })?;

    let absolute = absolute_path(path)?;
    let dir = locate(fs, home_trash, path)?;
    create_trash_dirs(&dir, path)?;

    let original_name = path
        .file_name()
        .ok_or_else(|| TrashError::Filesystem {
            path: path.to_path_buf(),
            source: io::Error::new(io::ErrorKind::InvalidInput, "path has no file name"),
        })?;
    let files_dir = dir.base.join("files");
    let name = unique_name(&files_dir, original_name).map_err(|source| TrashError::Filesystem {
        path: files_dir.clone(),
        source,
    })?;
    let files_dest = files_dir.join(&name);
    let mut info_name = name.clone();
    info_name.push(".trashinfo");
    let info_dest = dir.base.join("info").join(&info_name);

    let path_field = match &dir.topdir {
        None => percent::encode_path(&absolute),
        Some(topdir) => {
            let relative = absolute.strip_prefix(topdir).map_err(|_| TrashError::Filesystem {
                path: absolute.clone(),
                source: io::Error::other("source path is not under its own mount point"),
            })?;
            percent::encode_path(relative)
        }
    };
    let deleted_at = LocalDateTime::now().to_iso8601();
    let info_contents = format!("[Trash Info]\nPath={path_field}\nDeletionDate={deleted_at}\n");

    // Written before the move — see the module doc's "`.trashinfo`"
    // section for why the ordering is this way round and not the other.
    hyprforge_paths::write_atomic(&info_dest, &info_contents)
        .map_err(|source| TrashError::WriteInfo { path: info_dest.clone(), source })?;

    std::fs::rename(path, &files_dest).map_err(|source| TrashError::Move {
        from: path.to_path_buf(),
        to: files_dest.clone(),
        source,
    })?;

    Ok(TrashedItem { original_path: absolute, deleted_at, trashed_file: files_dest, info_file: info_dest })
}

/// Decides the home trash vs. a per-filesystem one, per the spec.
fn locate<F: Filesystem>(fs: &F, home_trash: &Path, source: &Path) -> Result<TrashDir, TrashError> {
    let source_dev = fs
        .device_of(source)
        .map_err(|err| TrashError::Filesystem { path: source.to_path_buf(), source: err })?;
    let home_dev = fs
        .device_of(home_trash)
        .map_err(|err| TrashError::Filesystem { path: home_trash.to_path_buf(), source: err })?;

    if source_dev == home_dev {
        return Ok(TrashDir { base: home_trash.to_path_buf(), topdir: None });
    }

    let topdir = fs
        .mount_point_of(source)
        .map_err(|err| TrashError::Filesystem { path: source.to_path_buf(), source: err })?;
    let uid = current_uid(source)?;

    let shared_trash = topdir.join(".Trash");
    let base = if shared_dir_is_safe(fs, &shared_trash)? {
        shared_trash.join(uid.to_string())
    } else {
        topdir.join(format!(".Trash-{uid}"))
    };
    Ok(TrashDir { base, topdir: Some(topdir) })
}

/// The spec's safety check on a shared per-filesystem `.Trash`: must
/// exist, must not be a symlink, must have the sticky bit set. Any
/// failure of that check — including the directory simply not existing —
/// means "not safe to use", never an error: the fallback
/// (`.Trash-$uid`) is the spec's own answer to exactly this case.
fn shared_dir_is_safe<F: Filesystem>(fs: &F, dot_trash: &Path) -> Result<bool, TrashError> {
    match fs.symlink_status(dot_trash) {
        Ok(status) => Ok(status.is_dir && !status.is_symlink && status.has_sticky_bit()),
        Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(false),
        Err(source) => Err(TrashError::Filesystem { path: dot_trash.to_path_buf(), source }),
    }
}

/// This process's real uid, read without a `libc` dependency: `/proc/self`
/// is a symlink to `/proc/<pid>`, and Linux sets that directory's owner
/// to the process's real uid — a standard trick for exactly this, no
/// FFI required, only a `stat`. `source_path` is carried through purely
/// to attribute a failure here to the file being trashed, not to the
/// unrelated-looking `/proc/self`.
fn current_uid(source_path: &Path) -> Result<u32, TrashError> {
    use std::os::unix::fs::MetadataExt;
    std::fs::metadata("/proc/self")
        .map(|m| m.uid())
        .map_err(|source| TrashError::Filesystem { path: source_path.to_path_buf(), source })
}

/// Creates `files/` and `info/` under `dir.base`, translating a failure
/// into [`TrashError::PerFilesystemTrashUnavailable`] when this is a
/// per-filesystem trash — see the module doc's placement section for why
/// that case is named separately from an ordinary I/O error.
fn create_trash_dirs(dir: &TrashDir, source_path: &Path) -> Result<(), TrashError> {
    for sub in ["files", "info"] {
        let path = dir.base.join(sub);
        std::fs::create_dir_all(&path).map_err(|source| match &dir.topdir {
            Some(_) => TrashError::PerFilesystemTrashUnavailable {
                path: source_path.to_path_buf(),
                tried: dir.base.clone(),
                source,
            },
            None => TrashError::CreateTrashDir { path, source },
        })?;
    }
    Ok(())
}

/// `path`, made absolute against the current directory if it is not
/// already — never resolved through `canonicalize`, which would follow a
/// symlink at any component and change which file `Path=` refers to.
fn absolute_path(path: &Path) -> Result<PathBuf, TrashError> {
    if path.is_absolute() {
        Ok(path.to_path_buf())
    } else {
        let cwd = std::env::current_dir()
            .map_err(|source| TrashError::Filesystem { path: path.to_path_buf(), source })?;
        Ok(cwd.join(path))
    }
}

/// Splits `name` into stem and extension the way a disambiguating suffix
/// needs — `"notes.txt"` -> `("notes", Some("txt"))` — and treats a name
/// with no `.` (or a dotfile, whose only `.` is its first byte) as having
/// no extension, so `.bashrc` disambiguates to `.bashrc.2`, not something
/// that splits it into a hidden empty stem and an extension `bashrc`.
fn split_stem_ext(name: &[u8]) -> (&[u8], Option<&[u8]>) {
    match name.iter().rposition(|&b| b == b'.') {
        Some(0) | None => (name, None),
        Some(idx) => (&name[..idx], Some(&name[idx + 1..])),
    }
}

/// Picks a name under `files_dir` that does not already exist there,
/// starting from `original` and inserting `.2`, `.3`, ... before the
/// extension on each collision — see the module doc's "Name collisions"
/// section for why this particular convention.
///
/// `pub(crate)` rather than private: `ops`'s keep-both collision policy
/// needs exactly this disambiguation and reuses it rather than growing a
/// second implementation of the same `stem.N.ext` scheme.
pub(crate) fn unique_name(files_dir: &Path, original: &OsStr) -> io::Result<OsString> {
    let bytes = original.as_bytes();
    if !files_dir.join(original).exists() {
        return Ok(original.to_os_string());
    }
    let (stem, ext) = split_stem_ext(bytes);
    let mut n = 2;
    loop {
        let mut candidate = stem.to_vec();
        candidate.extend_from_slice(format!(".{n}").as_bytes());
        if let Some(ext) = ext {
            candidate.push(b'.');
            candidate.extend_from_slice(ext);
        }
        let candidate = OsString::from_vec(candidate);
        if !files_dir.join(&candidate).exists() {
            return Ok(candidate);
        }
        n += 1;
    }
}

/// Everything in the home trash.
pub fn list(home_trash: &Path) -> Result<Vec<TrashedItem>, TrashError> {
    list_dir(home_trash, None)
}

/// Everything in a per-filesystem trash at `base` (either
/// `$topdir/.Trash/$uid` or `$topdir/.Trash-$uid` — whichever the caller
/// used), whose entries store `Path=` relative to `topdir`.
pub fn list_per_filesystem(base: &Path, topdir: &Path) -> Result<Vec<TrashedItem>, TrashError> {
    list_dir(base, Some(topdir))
}

fn list_dir(base: &Path, topdir: Option<&Path>) -> Result<Vec<TrashedItem>, TrashError> {
    let info_dir = base.join("info");
    let entries = match std::fs::read_dir(&info_dir) {
        Ok(entries) => entries,
        // A trash directory that was never created is first-run, not an
        // error — nothing has been trashed here yet.
        Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(source) => return Err(TrashError::ListDir { path: info_dir, source }),
    };

    let mut items = Vec::new();
    for entry in entries {
        let entry = entry.map_err(|source| TrashError::ListDir { path: info_dir.clone(), source })?;
        let info_path = entry.path();
        if info_path.extension().and_then(|e| e.to_str()) != Some("trashinfo") {
            continue;
        }

        let text = std::fs::read_to_string(&info_path)
            .map_err(|source| TrashError::ReadInfo { path: info_path.clone(), source })?;
        let parsed = parse_info(&text, &info_path)?;

        let original_path = match topdir {
            Some(topdir) => topdir.join(&parsed.path),
            None => parsed.path,
        };
        let stem = info_path.file_stem().ok_or_else(|| TrashError::InvalidInfo {
            path: info_path.clone(),
            reason: "info file has no name".to_string(),
        })?;
        let trashed_file = base.join("files").join(stem);

        items.push(TrashedItem {
            original_path,
            deleted_at: parsed.deletion_date,
            trashed_file,
            info_file: info_path,
        });
    }
    Ok(items)
}

struct ParsedInfo {
    path: PathBuf,
    deletion_date: String,
}

/// Parses a `.trashinfo` file's contents. A file that will not parse is
/// reported all the way out to the caller of `list`/`list_per_filesystem`
/// — never skipped, per CLAUDE.md's rule on collapsing "could not be
/// read" into "there is nothing there".
fn parse_info(text: &str, info_path: &Path) -> Result<ParsedInfo, TrashError> {
    let mut lines = text.lines();
    let header = lines.next().ok_or_else(|| TrashError::InvalidInfo {
        path: info_path.to_path_buf(),
        reason: "file is empty".to_string(),
    })?;
    if header.trim() != "[Trash Info]" {
        return Err(TrashError::InvalidInfo {
            path: info_path.to_path_buf(),
            reason: format!("expected a [Trash Info] header, found {header:?}"),
        });
    }

    let mut path_field = None;
    let mut date_field = None;
    for line in lines {
        if let Some(v) = line.strip_prefix("Path=") {
            path_field = Some(v.to_string());
        } else if let Some(v) = line.strip_prefix("DeletionDate=") {
            date_field = Some(v.to_string());
        }
    }

    let path_field = path_field.ok_or_else(|| TrashError::InvalidInfo {
        path: info_path.to_path_buf(),
        reason: "missing Path=".to_string(),
    })?;
    let date_field = date_field.ok_or_else(|| TrashError::InvalidInfo {
        path: info_path.to_path_buf(),
        reason: "missing DeletionDate=".to_string(),
    })?;

    let decoded = percent::decode_path(&path_field).map_err(|e| TrashError::InvalidInfo {
        path: info_path.to_path_buf(),
        reason: format!("Path= is not valid percent-encoding: {e}"),
    })?;
    if LocalDateTime::parse(&date_field).is_none() {
        return Err(TrashError::InvalidInfo {
            path: info_path.to_path_buf(),
            reason: format!("DeletionDate={date_field:?} is not YYYY-MM-DDThh:mm:ss"),
        });
    }

    Ok(ParsedInfo { path: decoded, deletion_date: date_field })
}

/// Puts a trashed file back at [`TrashedItem::original_path`] and removes
/// its info file. Refuses — leaving both the trashed file and the info
/// file untouched — if something already exists at the destination,
/// rather than overwriting it.
pub fn restore(item: &TrashedItem) -> Result<(), TrashError> {
    // `symlink_metadata`, not `exists()`: a broken symlink already sitting
    // at the destination must still block the restore, even though
    // `exists()` would report it as absent.
    if std::fs::symlink_metadata(&item.original_path).is_ok() {
        return Err(TrashError::RestoreWouldOverwrite { path: item.original_path.clone() });
    }
    if let Some(parent) = item.original_path.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|source| TrashError::Restore { path: item.original_path.clone(), source })?;
    }
    std::fs::rename(&item.trashed_file, &item.original_path)
        .map_err(|source| TrashError::Restore { path: item.original_path.clone(), source })?;
    std::fs::remove_file(&item.info_file)
        .map_err(|source| TrashError::RemoveInfo { path: item.info_file.clone(), source })?;
    Ok(())
}

/// Deletes a trashed item for good — its content and its record.
///
/// Content first, record second. If removing the content fails partway
/// through a large folder, the record stays, so the item is still listed
/// and can be tried again; the other order would leave content nothing
/// lists, filling the disk invisibly.
pub fn erase(item: &TrashedItem) -> Result<(), TrashError> {
    delete_permanently(&item.trashed_file)
        .map_err(|source| TrashError::Erase { path: item.trashed_file.clone(), source })?;
    std::fs::remove_file(&item.info_file)
        .map_err(|source| TrashError::RemoveInfo { path: item.info_file.clone(), source })
}

/// [`erase`], for a caller that only has the stored path — a listing of
/// the trash shows those, not the records behind them.
pub fn erase_stored(home_trash: &Path, trashed_file: &Path) -> Result<(), TrashError> {
    let item = list(home_trash)?
        .into_iter()
        .find(|item| item.trashed_file == trashed_file)
        .ok_or_else(|| TrashError::NotInTrash { path: trashed_file.to_path_buf() })?;
    erase(&item)
}

/// Removes `path` for good, file or folder, without following a symlink
/// — deleting a link removes the link, never what it points at.
///
/// `std::fs::remove_dir_all` is safe here: it does not follow symlinks
/// inside the tree either, including against a link swapped in while it
/// runs.
pub fn delete_permanently(path: &Path) -> io::Result<()> {
    let meta = std::fs::symlink_metadata(path)?;
    if meta.is_dir() {
        std::fs::remove_dir_all(path)
    } else {
        std::fs::remove_file(path)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fs::mock::MockFilesystem;
    use std::os::unix::fs::PermissionsExt;

    fn write_file(path: &Path, contents: &str) {
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, contents).unwrap();
    }

    #[test]
    fn a_file_on_the_same_filesystem_goes_to_the_home_trash() {
        let dir = tempfile::tempdir().unwrap();
        let home_trash = dir.path().join("home").join("Trash");
        let source = dir.path().join("docs").join("notes.txt");
        write_file(&source, "hello");

        // Default MockFilesystem: everything is the same device, so this
        // is exactly the "same filesystem" case.
        let fs = MockFilesystem::new();
        let item = trash_into(&fs, &home_trash, &source).unwrap();

        assert!(item.trashed_file.starts_with(&home_trash));
        assert!(!source.exists());
        assert_eq!(std::fs::read_to_string(&item.trashed_file).unwrap(), "hello");
    }

    #[test]
    fn a_file_on_another_filesystem_goes_to_that_filesystems_own_trash_never_the_home_one() {
        let dir = tempfile::tempdir().unwrap();
        let home_trash = dir.path().join("home").join("Trash");
        let usb = dir.path().join("mnt").join("usb");
        let source = usb.join("photo.jpg");
        write_file(&source, "bytes");

        let fs = MockFilesystem::new();
        fs.mount(&usb, 7); // a different device than the default (home's)

        let item = trash_into(&fs, &home_trash, &source).unwrap();

        assert!(!item.trashed_file.starts_with(&home_trash), "must never land in the home trash");
        assert!(item.trashed_file.starts_with(&usb), "must land on the file's own filesystem");
    }

    #[test]
    fn dot_trash_without_the_sticky_bit_is_refused_and_dot_trash_dash_uid_used_instead() {
        let dir = tempfile::tempdir().unwrap();
        let home_trash = dir.path().join("home").join("Trash");
        let usb = dir.path().join("mnt").join("usb");
        let source = usb.join("photo.jpg");
        write_file(&source, "bytes");

        // A real `.Trash` directory, but ordinary permissions — no
        // sticky bit.
        let shared = usb.join(".Trash");
        std::fs::create_dir_all(&shared).unwrap();
        std::fs::set_permissions(&shared, std::fs::Permissions::from_mode(0o755)).unwrap();

        let fs = MockFilesystem::new();
        fs.mount(&usb, 7);

        let item = trash_into(&fs, &home_trash, &source).unwrap();

        assert!(
            item.trashed_file.to_string_lossy().contains(".Trash-1000"),
            "expected the .Trash-$uid fallback, got {:?}",
            item.trashed_file
        );
        assert!(!item.trashed_file.starts_with(&shared), "an unsafe .Trash must never be used");
    }

    #[test]
    fn a_dot_trash_that_is_a_symlink_is_refused() {
        let dir = tempfile::tempdir().unwrap();
        let home_trash = dir.path().join("home").join("Trash");
        let usb = dir.path().join("mnt").join("usb");
        let source = usb.join("photo.jpg");
        write_file(&source, "bytes");
        std::fs::create_dir_all(&usb).unwrap();

        // `.Trash` is a symlink to a directory that itself has every
        // other property right (sticky bit, real directory at the far
        // end) — only the symlink-ness should disqualify it.
        let real_target = dir.path().join("elsewhere");
        std::fs::create_dir_all(&real_target).unwrap();
        std::fs::set_permissions(&real_target, std::fs::Permissions::from_mode(0o1777)).unwrap();
        std::os::unix::fs::symlink(&real_target, usb.join(".Trash")).unwrap();

        let fs = MockFilesystem::new();
        fs.mount(&usb, 7);

        let item = trash_into(&fs, &home_trash, &source).unwrap();

        assert!(
            item.trashed_file.to_string_lossy().contains(".Trash-1000"),
            "a symlinked .Trash must be refused even though the real target is safe"
        );
    }

    #[test]
    fn a_topdir_this_user_cannot_write_to_reports_the_named_error_not_a_generic_io_failure() {
        let dir = tempfile::tempdir().unwrap();
        let home_trash = dir.path().join("home").join("Trash");
        // Stands in for this machine's own `/`: root-owned, mode 755 —
        // unprivileged, so creating `.Trash-$uid` under it fails.
        let topdir = dir.path().join("root-owned");
        std::fs::create_dir_all(&topdir).unwrap();
        std::fs::set_permissions(&topdir, std::fs::Permissions::from_mode(0o555)).unwrap();
        let source = topdir.join("file.txt");
        // Can't actually create a file under a read-only dir; fake its
        // existence check by writing it before locking permissions down
        // would defeat the point, so instead trash a path that exists as
        // far as `symlink_metadata` is concerned via a bind — simplest is
        // to write it, then re-tighten permissions on the parent only.
        std::fs::set_permissions(&topdir, std::fs::Permissions::from_mode(0o755)).unwrap();
        write_file(&source, "bytes");
        std::fs::set_permissions(&topdir, std::fs::Permissions::from_mode(0o555)).unwrap();

        let fs = MockFilesystem::new();
        fs.mount(&topdir, 30);

        let result = trash_into(&fs, &home_trash, &source);
        // Restore writable so tempdir cleanup on drop can remove it.
        std::fs::set_permissions(&topdir, std::fs::Permissions::from_mode(0o755)).unwrap();

        assert!(
            matches!(result, Err(TrashError::PerFilesystemTrashUnavailable { .. })),
            "expected PerFilesystemTrashUnavailable, got {result:?}"
        );
    }

    #[test]
    fn two_files_with_the_same_name_both_survive_being_trashed() {
        let dir = tempfile::tempdir().unwrap();
        let home_trash = dir.path().join("home").join("Trash");
        let a = dir.path().join("a").join("notes.txt");
        let b = dir.path().join("b").join("notes.txt");
        write_file(&a, "from a");
        write_file(&b, "from b");

        let fs = MockFilesystem::new();
        let item_a = trash_into(&fs, &home_trash, &a).unwrap();
        let item_b = trash_into(&fs, &home_trash, &b).unwrap();

        assert_ne!(item_a.trashed_file, item_b.trashed_file);
        assert_eq!(std::fs::read_to_string(&item_a.trashed_file).unwrap(), "from a");
        assert_eq!(std::fs::read_to_string(&item_b.trashed_file).unwrap(), "from b");
    }

    #[test]
    fn the_info_files_path_is_absolute_for_the_home_trash_and_relative_for_a_per_filesystem_one() {
        let dir = tempfile::tempdir().unwrap();
        let home_trash = dir.path().join("home").join("Trash");

        let home_source = dir.path().join("docs").join("notes.txt");
        write_file(&home_source, "x");
        let fs = MockFilesystem::new();
        let home_item = trash_into(&fs, &home_trash, &home_source).unwrap();
        let home_info = std::fs::read_to_string(&home_item.info_file).unwrap();
        let home_path_line = home_info.lines().find(|l| l.starts_with("Path=")).unwrap();
        assert!(
            home_path_line["Path=".len()..].starts_with('/'),
            "home trash Path= must be absolute: {home_path_line}"
        );

        let usb = dir.path().join("mnt").join("usb");
        let usb_source = usb.join("sub").join("notes.txt");
        write_file(&usb_source, "y");
        fs.mount(&usb, 9);
        let usb_item = trash_into(&fs, &home_trash, &usb_source).unwrap();
        let usb_info = std::fs::read_to_string(&usb_item.info_file).unwrap();
        let usb_path_line = usb_info.lines().find(|l| l.starts_with("Path=")).unwrap();
        assert_eq!(
            &usb_path_line["Path=".len()..],
            "sub/notes.txt",
            "per-filesystem Path= must be relative to $topdir"
        );
    }

    #[test]
    fn trash_then_restore_round_trips_a_file_including_a_space_and_a_percent_in_its_name() {
        let dir = tempfile::tempdir().unwrap();
        let home_trash = dir.path().join("home").join("Trash");
        let source = dir.path().join("docs").join("100% done notes.txt");
        write_file(&source, "important");

        let fs = MockFilesystem::new();
        let item = trash_into(&fs, &home_trash, &source).unwrap();
        assert!(!source.exists());

        restore(&item).unwrap();
        assert_eq!(std::fs::read_to_string(&source).unwrap(), "important");
        assert!(!item.trashed_file.exists(), "the trashed copy is gone once restored");
        assert!(!item.info_file.exists(), "the info file is gone once restored");
    }

    #[test]
    fn restore_refuses_when_something_already_exists_at_the_original_path() {
        let dir = tempfile::tempdir().unwrap();
        let home_trash = dir.path().join("home").join("Trash");
        let source = dir.path().join("docs").join("notes.txt");
        write_file(&source, "original");

        let fs = MockFilesystem::new();
        let item = trash_into(&fs, &home_trash, &source).unwrap();

        // Something new now occupies the original path.
        write_file(&source, "someone else's file");

        let err = restore(&item).expect_err("must refuse rather than overwrite");
        assert!(matches!(err, TrashError::RestoreWouldOverwrite { .. }));
        assert_eq!(std::fs::read_to_string(&source).unwrap(), "someone else's file");
        assert!(item.trashed_file.exists(), "the trashed copy must be left alone on refusal");
    }

    #[test]
    fn a_trashinfo_that_will_not_parse_is_reported_by_list_not_skipped() {
        let dir = tempfile::tempdir().unwrap();
        let home_trash = dir.path().join("home").join("Trash");
        let info_dir = home_trash.join("info");
        std::fs::create_dir_all(&info_dir).unwrap();
        std::fs::write(info_dir.join("broken.trashinfo"), "this is not a trashinfo file at all").unwrap();

        let err = list(&home_trash).expect_err("a malformed .trashinfo must fail list(), not be skipped");
        assert!(matches!(err, TrashError::InvalidInfo { .. }));
    }

    #[test]
    fn a_missing_trash_directory_lists_as_empty_not_an_error() {
        let dir = tempfile::tempdir().unwrap();
        let home_trash = dir.path().join("never-created").join("Trash");
        assert_eq!(list(&home_trash).unwrap().len(), 0);
    }

    #[test]
    fn trashing_a_missing_source_reports_source_not_found() {
        let dir = tempfile::tempdir().unwrap();
        let home_trash = dir.path().join("home").join("Trash");
        let missing = dir.path().join("nope.txt");

        let fs = MockFilesystem::new();
        let err = trash_into(&fs, &home_trash, &missing).unwrap_err();
        assert!(matches!(err, TrashError::SourceNotFound { .. }));
    }

    #[test]
    fn erasing_removes_the_content_and_the_record() {
        let dir = tempfile::tempdir().unwrap();
        let home_trash = dir.path().join("home").join("Trash");
        let source = dir.path().join("docs").join("notes.txt");
        write_file(&source, "gone for good");
        let item = trash_into(&MockFilesystem::new(), &home_trash, &source).unwrap();

        erase_stored(&home_trash, &item.trashed_file).unwrap();
        assert!(!item.trashed_file.exists());
        assert!(!item.info_file.exists());
        assert!(list(&home_trash).unwrap().is_empty());
    }

    #[test]
    fn erasing_a_trashed_folder_removes_all_of_it() {
        let dir = tempfile::tempdir().unwrap();
        let home_trash = dir.path().join("home").join("Trash");
        let folder = dir.path().join("docs").join("project");
        write_file(&folder.join("deep").join("a.txt"), "x");
        let item = trash_into(&MockFilesystem::new(), &home_trash, &folder).unwrap();
        erase(&item).unwrap();
        assert!(!item.trashed_file.exists());
    }

    #[test]
    fn erasing_something_the_trash_has_no_record_of_is_refused() {
        let dir = tempfile::tempdir().unwrap();
        let home_trash = dir.path().join("home").join("Trash");
        let stray = home_trash.join("files").join("stray.txt");
        write_file(&stray, "x");
        let err = erase_stored(&home_trash, &stray).unwrap_err();
        assert!(matches!(err, TrashError::NotInTrash { .. }), "{err}");
        assert!(stray.exists(), "nothing is deleted on a refusal");
    }

    /// Deleting a link deletes the link — never what it points at.
    #[test]
    fn deleting_a_symlink_leaves_its_target_alone() {
        let dir = tempfile::tempdir().unwrap();
        let target = dir.path().join("keep");
        write_file(&target.join("precious.txt"), "x");
        let link = dir.path().join("link");
        std::os::unix::fs::symlink(&target, &link).unwrap();
        delete_permanently(&link).unwrap();
        assert!(std::fs::symlink_metadata(&link).is_err());
        assert!(target.join("precious.txt").exists());
    }

    #[test]
    fn deleting_a_folder_removes_the_tree() {
        let dir = tempfile::tempdir().unwrap();
        let folder = dir.path().join("tree");
        write_file(&folder.join("a").join("b.txt"), "x");
        delete_permanently(&folder).unwrap();
        assert!(!folder.exists());
    }

    #[test]
    fn listing_after_trashing_reports_the_original_path_and_deletion_date() {
        let dir = tempfile::tempdir().unwrap();
        let home_trash = dir.path().join("home").join("Trash");
        let source = dir.path().join("docs").join("notes.txt");
        write_file(&source, "hi");

        let fs = MockFilesystem::new();
        let trashed = trash_into(&fs, &home_trash, &source).unwrap();

        let items = list(&home_trash).unwrap();
        assert_eq!(items.len(), 1);
        assert_eq!(items[0].original_path, trashed.original_path);
        assert_eq!(items[0].deleted_at, trashed.deleted_at);
    }
}
