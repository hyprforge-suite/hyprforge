//! Making sure only one instance of a given popup is ever open at once.
//!
//! Pressing the keybind twice must not start a second process — but the
//! usual ways of checking "is one already running" are exactly the two
//! CLAUDE.md warns about:
//!
//! - `pgrep -x`/`pkill -x` match against `/proc/<pid>/comm`, which is
//!   truncated at 15 characters. `hyprforge-clipmenu` is 18; a name-based
//!   check would match *nothing*, report success finding no match, and a
//!   second popup would open right on top of the first.
//! - `pkill -f` matches the whole command line, which includes the shell
//!   that ran it — killing the shell that launched this, not a second
//!   instance of it.
//! - A PID file has to be cleaned up by whoever wrote it, which a
//!   process that is killed never gets the chance to do — a crashed
//!   popup would wedge every future one shut.
//!
//! An `flock` on a file in `$XDG_RUNTIME_DIR` has none of those problems:
//! it does not care what the holder is named, and the kernel releases it
//! the instant the holding file descriptor closes — normal exit, a panic
//! unwinding, or a kill — with nothing for this process to remember to
//! do on the way out.
//!
//! `std` has no `flock` binding, and adding a crate for one syscall is
//! not worth a new dependency here, so it is declared directly against
//! libc — every target this ships on already links libc for its C
//! library regardless (glibc, on this suite's one supported platform),
//! so this adds no new library dependency, only a declaration of a
//! function that is already there.
//!
//! Parameterised by lock *name* rather than one hardcoded file, since
//! this is shared across every popup this crate serves — `hyprforge-clipmenu`,
//! `hyprforge-emojimenu` and `hyprforge-traymenu` each take their own lock,
//! not one shared between them (opening the emoji picker must not be refused
//! because the clipboard popup happens to be open, or vice versa).

use std::fs::{File, OpenOptions};
use std::io;
use std::os::unix::io::AsRawFd;
use std::path::{Path, PathBuf};

const LOCK_EX: i32 = 2;
const LOCK_NB: i32 = 4;

extern "C" {
    fn flock(fd: i32, operation: i32) -> i32;
}

/// Where a lock file named `name` (e.g. `"hyprforge-clipmenu.lock"`)
/// lives: `$XDG_RUNTIME_DIR` when it is set (every real session has one),
/// falling back to the system temp directory only so this never panics
/// on a machine with neither — a fallback that is never actually
/// exercised outside a broken environment.
pub fn lock_path(name: &str) -> PathBuf {
    let base = std::env::var_os("XDG_RUNTIME_DIR").map(PathBuf::from).unwrap_or_else(std::env::temp_dir);
    base.join(name)
}

/// Holds the exclusive lock for as long as this value lives. Dropping it
/// (including via an unwinding panic, or the process being killed, which
/// closes every file descriptor the kernel was tracking for it) is what
/// releases the lock — there is deliberately nothing to call by hand.
pub struct Lock(#[allow(dead_code)] File);

/// Tries to become the one popup of this kind allowed to run.
///
/// `Ok(Some(lock))`: this process holds the lock and should proceed —
/// keep `lock` alive for as long as the popup is open. `Ok(None)`: some
/// other process already holds it, so this one should exit immediately,
/// quietly and successfully — a keybind pressed twice is not an error.
/// `Err`: the lock file itself could not even be opened (an unwritable
/// runtime directory, say) — treated by the caller as "couldn't check,
/// so don't block on it", never as a reason to refuse to show the
/// popup: a broken lock must never lock out every future one.
pub fn acquire(path: &Path) -> io::Result<Option<Lock>> {
    // `truncate(false)`: the lock's meaning is entirely in the flock
    // state, never the file's contents (there are none), so there is
    // nothing to preserve or discard either way — explicit so this
    // doesn't read as an oversight.
    let file = OpenOptions::new().create(true).write(true).truncate(false).open(path)?;
    // SAFETY: `flock` is called with a valid, open file descriptor this
    // function owns for the duration of the call, and its only effect is
    // on that descriptor's lock state — no memory safety hazard, only
    // the usual "this is a syscall FFI has to declare by hand" one.
    let result = unsafe { flock(file.as_raw_fd(), LOCK_EX | LOCK_NB) };
    if result == 0 {
        Ok(Some(Lock(file)))
    } else {
        let err = io::Error::last_os_error();
        match err.kind() {
            // `LOCK_NB` makes a held lock fail with EWOULDBLOCK/EAGAIN
            // instead of waiting — never wait on another process
            // without a bound, and there is no bound shorter than "not
            // at all" for finding out someone else already has it.
            io::ErrorKind::WouldBlock => Ok(None),
            _ => Err(err),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_first_process_to_ask_gets_the_lock() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("test.lock");
        let lock = acquire(&path).unwrap();
        assert!(lock.is_some());
    }

    /// The property this module exists for: a second attempt while the
    /// first is still held must be told "no", not made to wait.
    #[test]
    fn a_second_attempt_while_the_first_is_held_is_told_no() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("test.lock");
        let first = acquire(&path).unwrap();
        assert!(first.is_some(), "the first attempt must succeed");

        let second = acquire(&path).unwrap();
        assert!(second.is_none(), "a second attempt must be refused, not blocked on");
    }

    /// The other half of the guarantee: dropping the lock (what happens
    /// on any exit, including a crash or a kill) must free it up for the
    /// next popup — a crashed one must never wedge every future one
    /// shut.
    #[test]
    fn dropping_the_lock_frees_it_for_the_next_attempt() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("test.lock");
        let first = acquire(&path).unwrap();
        drop(first);

        let second = acquire(&path).unwrap();
        assert!(second.is_some(), "the lock must be free once the holder is gone");
    }

    /// Two different lock files never contend with each other — this is
    /// not a global "only one popup-shaped thing" lock, only "only one
    /// holder of this exact path".
    #[test]
    fn locks_on_different_paths_never_contend() {
        let dir = tempfile::tempdir().unwrap();
        let a = acquire(&dir.path().join("a.lock")).unwrap();
        let b = acquire(&dir.path().join("b.lock")).unwrap();
        assert!(a.is_some());
        assert!(b.is_some());
    }

    /// A directory that does not exist cannot be opened for the lock
    /// file — reported as `Err`, not silently treated as "lock
    /// acquired" or "lock held by someone else".
    #[test]
    fn a_missing_directory_is_reported_as_an_error_not_a_lock_state() {
        let path = Path::new("/nonexistent-hyprforge-popup-test-dir/test.lock");
        assert!(acquire(path).is_err());
    }
}
