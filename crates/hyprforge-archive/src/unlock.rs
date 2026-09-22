//! The password for an encrypted archive, and where one is kept while
//! the window is open.
//!
//! # Why a password is a parameter and not a field
//!
//! Reading an encrypted archive needs a password; reading the one beside
//! it does not. A backend holding "the password" as state would be
//! answering for whichever archive was asked about last, which is the
//! kind of bug that shows up as one archive's password unlocking
//! another's listing. So [`Unlock`] travels with the call, and the
//! *keeping* of passwords is a separate thing ([`Keyring`]) that maps
//! each archive to its own.
//!
//! # What is never done with one
//!
//! Written down. A [`Keyring`] is memory for the life of the process and
//! nothing else — no file, no keyring daemon, no cache next to the
//! archive. Someone who closes the window and opens it again is asked
//! again, which is the correct amount of surprise: this suite has
//! nowhere it could put an archive password that would be safer than
//! asking, and inventing one is a decision for a person, not a default.
//!
//! The value itself is a [`Secret<String>`], so a `Debug` of anything
//! holding one renders a length and never the password — CLAUDE.md's
//! rule about a keystroke never reaching a log, applied to the one field
//! in this crate it could apply to.
//!
//! Not zeroized on drop. The same gap `hyprforge-lock` documents for the
//! typed password, named here rather than left to be discovered: the
//! `String` is freed like any other and its bytes stay in the allocator
//! until something reuses them.

use hyprforge_secret::Secret;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

/// What this call knows about opening an encrypted archive.
///
/// [`Unlock::none`] is the ordinary case, and every format here reads a
/// plain archive with it — asking for a password that is not needed is
/// not an error, and supplying one that is not needed is ignored.
#[derive(Debug, Clone, Default)]
pub struct Unlock {
    password: Option<Secret<String>>,
}

impl Unlock {
    /// No password — what nearly every call passes.
    pub fn none() -> Unlock {
        Unlock { password: None }
    }

    pub fn with(password: Secret<String>) -> Unlock {
        Unlock {
            password: Some(password),
        }
    }

    /// The password, if there is one.
    ///
    /// The single place a password leaves its wrapper in this crate, so
    /// `grep expose` finds it — which is the whole point of
    /// [`Secret`]'s accessor being named that.
    pub fn expose(&self) -> Option<&str> {
        self.password.as_ref().map(|p| p.expose().as_str())
    }

    pub fn is_some(&self) -> bool {
        self.password.is_some()
    }
}

/// The passwords this process has been told, one per archive.
///
/// Shared: the listing comes from `hyprforge-files-core`'s backend and
/// the extraction from the app's own job, and both have to reach the
/// same answer, so both hold an `Arc` of this rather than each keeping
/// a map that the other's prompt would not fill in.
#[derive(Debug, Default)]
pub struct Keyring {
    by_archive: Mutex<HashMap<PathBuf, Secret<String>>>,
}

impl Keyring {
    pub fn new() -> Keyring {
        Keyring::default()
    }

    /// Remembers a password for this archive, replacing any earlier one.
    pub fn remember(&self, archive: &Path, password: Secret<String>) {
        self.by_archive
            .lock()
            .unwrap()
            .insert(archive.to_path_buf(), password);
    }

    /// What is known about opening this archive.
    pub fn unlock_for(&self, archive: &Path) -> Unlock {
        match self.by_archive.lock().unwrap().get(archive) {
            Some(password) => Unlock::with(password.clone()),
            None => Unlock::none(),
        }
    }

    /// Forgets one archive's password — after a wrong one, so the next
    /// attempt asks again instead of retrying the answer that just
    /// failed forever.
    pub fn forget(&self, archive: &Path) {
        self.by_archive.lock().unwrap().remove(archive);
    }

    pub fn forget_all(&self) {
        self.by_archive.lock().unwrap().clear();
    }

    pub fn knows(&self, archive: &Path) -> bool {
        self.by_archive.lock().unwrap().contains_key(archive)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn one_archives_password_does_not_unlock_another() {
        let keyring = Keyring::new();
        keyring.remember(Path::new("/a.zip"), Secret::new("first".to_string()));

        assert_eq!(keyring.unlock_for(Path::new("/a.zip")).expose(), Some("first"));
        assert_eq!(
            keyring.unlock_for(Path::new("/b.zip")).expose(),
            None,
            "a password is remembered against the archive it opens, not globally"
        );
    }

    #[test]
    fn a_wrong_password_can_be_forgotten_so_the_next_attempt_asks_again() {
        let keyring = Keyring::new();
        let archive = Path::new("/a.zip");
        keyring.remember(archive, Secret::new("wrong".to_string()));
        assert!(keyring.knows(archive));

        keyring.forget(archive);
        assert!(!keyring.knows(archive));
        assert!(!keyring.unlock_for(archive).is_some());
    }

    /// The rule this crate inherits: a password must never be renderable,
    /// and `Debug` is the way it escapes by accident — a `tracing` field,
    /// a panic message, an `{:?}` in a test that fails.
    #[test]
    fn nothing_holding_a_password_renders_it() {
        let unlock = Unlock::with(Secret::new("hunter2".to_string()));
        let rendered = format!("{unlock:?}");
        assert!(!rendered.contains("hunter2"), "{rendered}");

        let keyring = Keyring::new();
        keyring.remember(Path::new("/a.zip"), Secret::new("hunter2".to_string()));
        let rendered = format!("{keyring:?}");
        assert!(!rendered.contains("hunter2"), "{rendered}");
    }

    #[test]
    fn no_password_is_the_default_and_costs_nothing_to_pass() {
        assert!(!Unlock::default().is_some());
        assert_eq!(Unlock::none().expose(), None);
    }
}
