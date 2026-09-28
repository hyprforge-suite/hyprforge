//! Turning a numeric uid into the name a person recognises.
//!
//! `stat` answers with a uid and nothing else, and "1000" in an Owner
//! column is not information — the whole point of the column is to tell
//! *your* files from root's at a glance.
//!
//! `getpwuid_r` and not `/etc/passwd`, which is the tempting shortcut and
//! is wrong on any machine whose users come from somewhere else: LDAP,
//! SSSD, systemd-homed, a container's id mapping. The C library is the
//! only thing that knows what the name service switch is configured to
//! ask, exactly as `hyprforge_fileops::localtime` binds `localtime_r`
//! rather than shipping a timezone database.
//!
//! This module never reads `pw_passwd`. It is a legacy field that is
//! `x` on every modern system, but reading it at all would put a
//! credential-shaped value into a struct this code copies around, and
//! CLAUDE.md's rule about what must never reach a log or a debug dump is
//! easiest to keep by never holding the value in the first place.

use std::collections::HashMap;
use std::ffi::CStr;
use std::os::raw::{c_char, c_int};

/// POSIX's `struct passwd`. Field order and types are fixed by POSIX and
/// identical on glibc and musl; only the first three fields are read,
/// but every field has to be declared for the struct's size and the
/// offsets of the ones after it to match what `getpwuid_r` writes.
#[repr(C)]
struct Passwd {
    pw_name: *const c_char,
    /// Never read — see the module doc.
    pw_passwd: *const c_char,
    pw_uid: u32,
    pw_gid: u32,
    pw_gecos: *const c_char,
    pw_dir: *const c_char,
    pw_shell: *const c_char,
}

extern "C" {
    fn getpwuid_r(
        uid: u32,
        pwd: *mut Passwd,
        buf: *mut c_char,
        buflen: usize,
        result: *mut *mut Passwd,
    ) -> c_int;
}

/// glibc's own starting suggestion for the scratch buffer. Grown on
/// `ERANGE` rather than assumed sufficient — a directory-service entry
/// can be far larger than a local one.
const INITIAL_BUF: usize = 1024;

/// Past this the buffer stops growing and the lookup gives up. A name
/// needing more than a quarter of a megabyte of `struct passwd` is not a
/// name; refusing is better than doubling forever on a machine whose
/// name service is answering nonsense.
const MAX_BUF: usize = 256 * 1024;

/// The login name for `uid`, or `None` if the system has no entry for it.
///
/// `None` is a real and ordinary answer, not an error: a file copied
/// from another machine, or one inside a container whose ids do not map
/// to anything here, genuinely has an owner this system cannot name. The
/// caller shows the number in that case, which is more use than a dash —
/// an unmapped uid is a fact worth seeing.
pub fn name_for_uid(uid: u32) -> Option<String> {
    let mut len = INITIAL_BUF;
    loop {
        let mut buf = vec![0 as c_char; len];
        // SAFETY: all-zero bytes are a valid `Passwd` — its pointer fields
        // become null and its two `u32`s zero — and nothing reads it
        // until `getpwuid_r` has filled it and `result` is non-null.
        let mut pwd: Passwd = unsafe { std::mem::zeroed() };
        let mut result: *mut Passwd = std::ptr::null_mut();
        // SAFETY: `pwd` and `result` are live stack values, and `buf` is
        // a live allocation of exactly `len` elements — the three things
        // `getpwuid_r` writes into, each valid for the whole call.
        // `getpwuid_r` is the reentrant form precisely so it retains no
        // pointer past the call and shares no static buffer with another
        // thread; every pointer it writes into `pwd` points into `buf`,
        // which outlives the `CStr` read below.
        let code = unsafe { getpwuid_r(uid, &mut pwd, buf.as_mut_ptr(), len, &mut result) };
        if code == ERANGE {
            // The documented "your buffer was too small" answer, and the
            // only one worth retrying. Doubling rather than asking for a
            // size, because there is no call that reports one.
            len = len.saturating_mul(2);
            if len > MAX_BUF {
                return None;
            }
            continue;
        }
        // A non-zero code is a lookup failure; a null `result` with code
        // zero is the documented "no such user", which is not a failure
        // at all. Both leave `pwd` meaningless, so neither may read it.
        if code != 0 || result.is_null() {
            return None;
        }
        if pwd.pw_name.is_null() {
            return None;
        }
        // SAFETY: `pw_name` is non-null and points into `buf`, which is
        // still alive here; `getpwuid_r` guarantees it is NUL-terminated.
        let name = unsafe { CStr::from_ptr(pwd.pw_name) };
        // Lossy, not a rejection: a login name is conventionally ASCII
        // but nothing enforces it, and a file's owner column should not
        // go blank because someone's name has an odd byte in it.
        return Some(name.to_string_lossy().into_owned());
    }
}

/// `ERANGE`, the same value on every Linux ABI this suite targets.
const ERANGE: c_int = 34;

/// A uid-to-name cache for the span of one directory listing.
///
/// A directory of ten thousand files has, almost always, one or two
/// distinct owners — so the lookup that matters is the repeated one. Not
/// a global cache, deliberately: a process that ran for days would go on
/// reporting a name for a uid that had since been renamed or removed,
/// and a listing is exactly the right lifetime for "who owns this" to be
/// true for.
#[derive(Default)]
pub struct UserNames {
    seen: HashMap<u32, Option<String>>,
}

impl UserNames {
    pub fn new() -> Self {
        Self::default()
    }

    /// The name for `uid`, looked up once per distinct uid.
    ///
    /// A `None` answer is cached too — a uid this system cannot name
    /// will not start being nameable partway through one listing, and
    /// caching only the hits would mean every unmapped file paid for a
    /// full name-service round trip.
    pub fn get(&mut self, uid: u32) -> Option<&str> {
        self.seen.entry(uid).or_insert_with(|| name_for_uid(uid)).as_deref()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_current_user_resolves_to_a_name() {
        // SAFETY: `current_uid` has no preconditions; see its comment.
        let uid = unsafe { super::current_uid() };
        let name = name_for_uid(uid).expect("this process's own uid always has a passwd entry");
        assert!(!name.is_empty());
    }

    #[test]
    fn root_is_root() {
        // uid 0 is `root` on every Unix — the one name that can be
        // asserted as a constant rather than compared against something.
        assert_eq!(name_for_uid(0).as_deref(), Some("root"));
    }

    /// An unmapped uid is `None`, not an error and not an empty string —
    /// the caller shows the number, which is the useful answer for a
    /// file that came from another machine.
    #[test]
    fn a_uid_nothing_knows_about_is_none_rather_than_a_guess() {
        // Deliberately absurd: inside the 32-bit range `getpwuid_r`
        // accepts, far outside any range a system allocates.
        assert_eq!(name_for_uid(4_294_967_294), None);
    }

    #[test]
    fn the_cache_answers_the_same_thing_the_direct_lookup_does() {
        let mut names = UserNames::new();
        assert_eq!(names.get(0), Some("root"));
        // Twice, to exercise the cached path rather than only the miss.
        assert_eq!(names.get(0), Some("root"));
        assert_eq!(names.get(4_294_967_294), None);
    }

}

/// The FFI here declares `struct passwd` by hand, so the thing worth
/// testing is not that it parses but that it agrees with the C library
/// the rest of the system uses. A wrong field offset would still produce
/// a plausible-looking string — `pw_passwd` or `pw_dir` read as a name —
/// which is exactly why this compares against `id` rather than against a
/// constant someone typed. The same argument, and the same shape, as
/// `hyprforge_fileops::localtime`'s own audit against `date(1)`.
#[cfg(test)]
mod ffi_audit {
    use super::*;

    #[test]
    fn our_user_names_agree_with_the_id_command_the_rest_of_the_system_uses() {
        // SAFETY: `current_uid` has no preconditions; see its comment.
        for uid in [0_u32, unsafe { super::current_uid() }] {
            let ours = name_for_uid(uid);
            let theirs = std::process::Command::new("id")
                .args(["-nu", &uid.to_string()])
                .output()
                .expect("`id` is in coreutils and always present");
            let theirs = String::from_utf8_lossy(&theirs.stdout).trim().to_string();
            assert_eq!(
                ours.as_deref(),
                Some(theirs.as_str()),
                "disagreed with id(1) for uid {uid}"
            );
        }
    }
}

/// This process's own uid — test-only, so the audit below has a second
/// uid to check besides root's.
// SAFETY: this is `unsafe fn` only because its body is an FFI call; it
// asks nothing of its caller. `getuid` takes no arguments, cannot fail
// (POSIX: "no errors are defined"), touches no memory of ours and is
// always the same `uid_t`-sized answer — the same 32-bit `uid_t` this
// module's `getpwuid_r` binding is declared against.
#[cfg(test)]
unsafe fn current_uid() -> u32 {
    extern "C" {
        fn getuid() -> u32;
    }
    // SAFETY: as above — no arguments, no failure mode, no state of ours.
    unsafe { getuid() }
}
