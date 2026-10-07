//! Extended attributes: the `user.*` ones a copy carries across.
//!
//! A file's tags (`user.xdg.tags`, which Dolphin, Baloo and Files read),
//! where it was downloaded from (`user.xdg.origin.url`), a comment — all
//! live in extended attributes, and a copy that dropped them would lose
//! things the person put there. A move by `rename(2)` keeps them by
//! itself; this is for the copies, including the copy half of a move
//! between filesystems.
//!
//! Only `user.*`. `security.*` and `trusted.*` are the system's — an
//! SELinux label belongs to where a file *is*, and copying one across
//! would be wrong even where it is allowed — and `system.*` holds ACLs,
//! which a copy deliberately does not reproduce, as it does not
//! reproduce ownership.
//!
//! Best-effort, like the permissions and times beside it: a destination
//! that has no extended attributes at all (FAT, most network shares) is
//! still a destination, and the copy is a copy.
//!
//! Declared against the C library directly, as `batch.rs` declares
//! `renameat2`: every binary links it already.

use std::ffi::CString;
use std::fs::File;
use std::io;
use std::os::fd::AsRawFd;
use std::os::raw::{c_char, c_int, c_void};
use std::os::unix::ffi::OsStrExt;
use std::path::Path;

extern "C" {
    fn llistxattr(path: *const c_char, list: *mut c_char, size: usize) -> isize;
    fn lgetxattr(path: *const c_char, name: *const c_char, value: *mut c_void, size: usize) -> isize;
    fn lsetxattr(path: *const c_char, name: *const c_char, value: *const c_void, size: usize, flags: c_int) -> c_int;
    fn lremovexattr(path: *const c_char, name: *const c_char) -> c_int;
    fn fsetxattr(fd: c_int, name: *const c_char, value: *const c_void, size: usize, flags: c_int) -> c_int;
}

/// The most any one attribute value may be: Linux's own limit
/// (`XATTR_SIZE_MAX`). A value claiming more is not read.
const VALUE_MAX: usize = 64 * 1024;

fn c_path(path: &Path) -> io::Result<CString> {
    CString::new(path.as_os_str().as_bytes()).map_err(|_| io::Error::from(io::ErrorKind::InvalidInput))
}

fn c_name(name: &str) -> io::Result<CString> {
    CString::new(name).map_err(|_| io::Error::from(io::ErrorKind::InvalidInput))
}

/// Every extended attribute name on `path`, not following a symlink.
pub fn names(path: &Path) -> io::Result<Vec<String>> {
    let p = c_path(path)?;
    // SAFETY: a NUL-terminated path; a null buffer of size 0 asks only
    // for the size, which is what the kernel documents.
    let size = unsafe { llistxattr(p.as_ptr(), std::ptr::null_mut(), 0) };
    if size < 0 {
        return Err(io::Error::last_os_error());
    }
    let mut buf = vec![0u8; size as usize];
    if buf.is_empty() {
        return Ok(Vec::new());
    }
    // SAFETY: `buf` is `size` bytes, as passed. The list can grow
    // between the two calls; that is ERANGE, reported, not an overrun.
    let got = unsafe { llistxattr(p.as_ptr(), buf.as_mut_ptr().cast(), buf.len()) };
    if got < 0 {
        return Err(io::Error::last_os_error());
    }
    buf.truncate(got as usize);
    Ok(buf
        .split(|b| *b == 0)
        .filter(|n| !n.is_empty())
        .filter_map(|n| std::str::from_utf8(n).ok().map(str::to_string))
        .collect())
}

/// One attribute's value. `Ok(None)` when it is not set — which is not
/// the same as a filesystem that cannot hold any, an `Err` with
/// `ErrorKind::Unsupported`.
pub fn get(path: &Path, name: &str) -> io::Result<Option<Vec<u8>>> {
    let (p, n) = (c_path(path)?, c_name(name)?);
    // SAFETY: as in `names` — size first, then the read into that size.
    let size = unsafe { lgetxattr(p.as_ptr(), n.as_ptr(), std::ptr::null_mut(), 0) };
    if size < 0 {
        let e = io::Error::last_os_error();
        return match e.raw_os_error() {
            Some(code) if code == ENODATA => Ok(None),
            _ => Err(e),
        };
    }
    let size = size as usize;
    if size > VALUE_MAX {
        return Err(io::Error::from(io::ErrorKind::InvalidData));
    }
    let mut buf = vec![0u8; size];
    // SAFETY: `buf` is `size` bytes.
    let got = unsafe { lgetxattr(p.as_ptr(), n.as_ptr(), buf.as_mut_ptr().cast(), buf.len()) };
    if got < 0 {
        return Err(io::Error::last_os_error());
    }
    buf.truncate(got as usize);
    Ok(Some(buf))
}

/// Sets one attribute on `path`, not following a symlink.
pub fn set(path: &Path, name: &str, value: &[u8]) -> io::Result<()> {
    let (p, n) = (c_path(path)?, c_name(name)?);
    // SAFETY: NUL-terminated strings and a slice of `value.len()` bytes.
    let status = unsafe { lsetxattr(p.as_ptr(), n.as_ptr(), value.as_ptr().cast(), value.len(), 0) };
    if status != 0 {
        return Err(io::Error::last_os_error());
    }
    Ok(())
}

/// Removes one attribute. Not set already is not an error.
pub fn remove(path: &Path, name: &str) -> io::Result<()> {
    let (p, n) = (c_path(path)?, c_name(name)?);
    // SAFETY: NUL-terminated strings.
    let status = unsafe { lremovexattr(p.as_ptr(), n.as_ptr()) };
    if status != 0 {
        let e = io::Error::last_os_error();
        if e.raw_os_error() == Some(ENODATA) {
            return Ok(());
        }
        return Err(e);
    }
    Ok(())
}

/// `ENODATA` on Linux: "no such attribute".
const ENODATA: i32 = 61;

/// Copies every `user.*` attribute of `source` onto the open `dest` —
/// see the module doc. Best-effort: a failure is logged and the copy
/// goes on.
pub(crate) fn copy_user(source: &Path, dest: &File, dest_path: &Path) {
    let names = match names(source) {
        Ok(names) => names,
        Err(e) => {
            // No attributes on the source's filesystem: nothing to carry.
            if e.kind() != io::ErrorKind::Unsupported {
                tracing::debug!(path = %source.display(), error = %e, "could not list extended attributes");
            }
            return;
        }
    };
    for name in names.iter().filter(|n| n.starts_with("user.")) {
        let Ok(Some(value)) = get(source, name) else { continue };
        let Ok(n) = c_name(name) else { continue };
        // SAFETY: an open descriptor, a NUL-terminated name and a slice.
        let status = unsafe { fsetxattr(dest.as_raw_fd(), n.as_ptr(), value.as_ptr().cast(), value.len(), 0) };
        if status != 0 {
            tracing::warn!(
                path = %dest_path.display(),
                attribute = name.as_str(),
                error = %io::Error::last_os_error(),
                "could not carry an extended attribute across"
            );
        }
    }
}

/// Whether `error` means the filesystem has no extended attributes at
/// all, for a caller that says so in words.
pub fn unsupported(error: &io::Error) -> bool {
    error.kind() == io::ErrorKind::Unsupported || error.raw_os_error() == Some(95)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A temporary directory whose filesystem holds `user.*` attributes,
    /// or `None` (with the skip marker) where it does not.
    pub(crate) fn xattr_dir() -> Option<tempfile::TempDir> {
        let dir = tempfile::tempdir().ok()?;
        let probe = dir.path().join("probe");
        std::fs::write(&probe, "").ok()?;
        match set(&probe, "user.hyprforge.probe", b"1") {
            Ok(()) => Some(dir),
            Err(e) => {
                eprintln!("HYPRFORGE-SKIP: the temporary directory holds no user extended attributes ({e})");
                None
            }
        }
    }

    #[test]
    fn an_attribute_set_reads_back_and_a_missing_one_is_none() {
        let Some(dir) = xattr_dir() else { return };
        let file = dir.path().join("a");
        std::fs::write(&file, "x").unwrap();
        set(&file, "user.xdg.tags", b"work,draft").unwrap();
        assert_eq!(get(&file, "user.xdg.tags").unwrap().as_deref(), Some(&b"work,draft"[..]));
        assert_eq!(get(&file, "user.xdg.comment").unwrap(), None, "not set is not an error");
        assert!(names(&file).unwrap().contains(&"user.xdg.tags".to_string()));
        remove(&file, "user.xdg.tags").unwrap();
        remove(&file, "user.xdg.tags").unwrap();
        assert_eq!(get(&file, "user.xdg.tags").unwrap(), None);
    }
}
