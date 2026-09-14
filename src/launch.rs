//! Opening a file with whatever the desktop thinks should open it.
//!
//! This is the app's job and deliberately not `hyprforge-files-core`'s:
//! the portal's open/save dialog browses the same tree with the same
//! view, but double-clicking there *chooses* a file and returns it to
//! whichever application asked. A dialog that launched a media player
//! when you picked an attachment would be a bug, so the two hosts
//! interpret the same `Activated` outcome differently and only this one
//! launches anything.
//!
//! `xdg-open` rather than reading `mimeapps.list` ourselves. It is the
//! freedesktop entry point every other application already uses, which
//! means the user's existing "always open .md in this editor" choice is
//! honoured without this suite reimplementing the lookup and then
//! disagreeing with the rest of the desktop about it. Reimplementing it
//! would also mean owning the whole desktop-entry spec — `Exec` field
//! codes, `TryExec`, terminal applications — to end up where `xdg-open`
//! already is.
//!
//! # Why this does not wait
//!
//! CLAUDE.md is emphatic that nothing here may wait on another process
//! without a bound, and the bound is usually
//! `hyprforge_process::output(.., TIMEOUT)`. That is the wrong tool
//! here, and the distinction is worth stating because reaching for it
//! would look correct: `output()` waits for a child to *exit* and hands
//! back what it printed. The child here is a text editor the user is
//! about to spend an hour in. There is nothing to collect and nothing
//! to wait for — the useful part is over the moment the process exists.
//!
//! So this spawns and never waits at all, which is the strongest form
//! of "bounded": no wait can be too long if there is no wait. What that
//! costs is that a program which fails *after* exec — a missing shared
//! library, a corrupt desktop entry — cannot be reported here, only a
//! failure to start at all. That is a real gap and it is the reason the
//! failure path below says what was attempted rather than pretending to
//! know why nothing appeared.

use std::path::Path;

/// What became of one attempt to open a file.
///
/// `Spawned` deliberately does **not** mean "it worked" — see this
/// module's own doc. It means a process was created; whether a window
/// ever appears is beyond what this can observe.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Opened {
    Spawned,
    /// `xdg-open` is not installed. Its own package (`xdg-utils`) is a
    /// dependency of essentially every desktop, so this is rare — but
    /// "rare" is not "impossible", and a file manager that silently does
    /// nothing when you double-click is indistinguishable from one that
    /// is broken. The UI says this sentence instead.
    NoOpener,
    /// It exists and could not be started.
    Failed(String),
}

impl Opened {
    /// The sentence a user should see, or `None` when there is nothing
    /// to say because it worked.
    ///
    /// Every variant that is not `Spawned` has to name what to do about
    /// it: "no dead ends" (vision pillar 3) means an error in this suite
    /// is an actionable message in the window, never a log line the user
    /// is expected to go and find.
    pub fn message(&self, path: &Path) -> Option<String> {
        let name = path.file_name().unwrap_or(path.as_os_str()).to_string_lossy();
        match self {
            Opened::Spawned => None,
            Opened::NoOpener => Some(format!(
                "Couldn't open {name} because xdg-open isn't installed. \
                 Install the xdg-utils package, or open it from an application directly."
            )),
            Opened::Failed(why) => Some(format!("Couldn't open {name}: {why}")),
        }
    }
}

/// Hands `path` to the desktop's own opener.
pub fn open(path: &Path) -> Opened {
    open_with("xdg-open", path)
}

/// [`open`], with the opener as a parameter so a test can point the
/// whole sequence at something that is not the real one — the same
/// arrangement `hyprforge_tray::launch` uses for `hyprforge-traymenu`.
pub fn open_with(opener: &str, path: &Path) -> Opened {
    match std::process::Command::new(opener)
        .arg(path)
        // The child's output goes nowhere rather than inheriting this
        // window's. Same reasoning as `hyprforge-trayd`'s
        // `spawn_settings`: a GUI started from here prints its own
        // renderer chatter, and inheriting it buries anything this app
        // actually logged. The child keeps its own logging.
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
    {
        Ok(_child) => {
            // The handle is dropped without being waited on, which on
            // Unix leaves the child a zombie until this process exits.
            // For a file manager that is a handful of entries in the
            // process table over a session, not a leak worth a reaper
            // thread — but it is deliberate rather than overlooked, and
            // if this ever becomes a long-lived daemon that opens
            // thousands of files, it is the thing to revisit.
            Opened::Spawned
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Opened::NoOpener,
        Err(e) => Opened::Failed(e.to_string()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    /// The sibling-binary-absent case CLAUDE.md requires every component
    /// to survive: a missing opener is a state with a message, never a
    /// crash and never silence.
    #[test]
    fn a_missing_opener_is_reported_rather_than_failing_silently() {
        let outcome = open_with("xdg-open-does-not-exist-xyz", Path::new("/tmp/x.txt"));
        assert_eq!(outcome, Opened::NoOpener);

        let message = outcome.message(Path::new("/tmp/x.txt")).expect("a failure must say something");
        assert!(message.contains("x.txt"), "the message names the file the user clicked");
        assert!(message.contains("xdg-utils"), "and names the package that fixes it");
    }

    /// Success is silent. A file manager that popped up "opened!" every
    /// time you double-clicked would be unusable.
    #[test]
    fn opening_something_successfully_says_nothing_at_all() {
        // `true` ignores its argument and exits 0 — a stand-in for an
        // opener that starts fine, without launching a real application
        // on the machine running the tests.
        let outcome = open_with("true", Path::new("/tmp/x.txt"));
        assert_eq!(outcome, Opened::Spawned);
        assert_eq!(outcome.message(Path::new("/tmp/x.txt")), None);
    }

    /// A path with no file name at all (the filesystem root) must still
    /// produce a sentence rather than panicking on an `unwrap` of
    /// `file_name()` — which returns `None` for `/`.
    #[test]
    fn a_path_with_no_file_name_still_produces_a_message() {
        let outcome = Opened::NoOpener;
        let message = outcome.message(&PathBuf::from("/")).expect("still a message");
        assert!(!message.is_empty());
    }
}
