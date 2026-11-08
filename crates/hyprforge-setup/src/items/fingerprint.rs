//! Fingerprint for administrator prompts: pam_fprintd in polkit's PAM
//! stack.
//!
//! An agent cannot ask the reader itself. polkit authenticates through
//! its own helper, over the `polkit-1` PAM stack, and the agent only
//! relays what PAM says — so the finger works exactly when that stack has
//! pam_fprintd in it, which it does not as shipped (Arch's
//! `/usr/lib/pam.d/polkit-1` includes `system-auth`, and nothing else).
//!
//! This writes `/etc/pam.d/polkit-1`, which PAM reads in preference to
//! the vendor file: the stack that applies now, with one line put first —
//!
//! ```text
//! auth       sufficient   pam_fprintd.so timeout=10
//! ```
//!
//! `sufficient`: a finger is enough, and anything else — no finger in ten
//! seconds, no reader, fprintd not running — falls through to the
//! password exactly as before. Ten seconds is pam_fprintd's shortest
//! timeout, and it is how long a password waits when nobody touches the
//! reader: pam_fprintd waits before PAM asks for anything, and no agent
//! can interrupt it. The user chose that wait, 2026-10-09.
//!
//! The danger in this item is its own undo. A polkit-1 that does not
//! work breaks `pkexec`, and `pkexec` is how undo runs. So the stack is
//! never written from scratch: it is the one already in effect with one
//! line added, and the tests hold every other line to coming through
//! unchanged. Undo takes that line back out of the file as it is now, or
//! removes the file when what is left is the vendor's stack.
//!
//! Offered only when it can do something: pam_fprintd installed, and a
//! finger enrolled for this user. Off by default — a password prompt in
//! the middle of setting up everything else is not a default.

use super::{Applied, Cx};
use crate::record::Change;
use crate::state::State;
use std::path::PathBuf;

/// The stack this item changes.
pub const STACK: &str = "polkit-1";

/// The line it adds.
pub const LINE: &str = "auth       sufficient   pam_fprintd.so timeout=10";

const MARK: &str = "# Added by Hyprforge's Set up: a finger first, then the password as before.";

fn local(cx: &Cx<'_>) -> PathBuf {
    cx.env.pam_dir.join(STACK)
}

fn read(path: &std::path::Path) -> Result<Option<String>, String> {
    match std::fs::read_to_string(path) {
        Ok(text) => Ok(Some(text)),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(format!("couldn't read {}: {e}", path.display())),
    }
}

/// The stack PAM uses now: the local copy if there is one, otherwise the
/// vendor's.
fn in_effect(cx: &Cx<'_>) -> Result<Option<String>, String> {
    match read(&local(cx))? {
        Some(text) => Ok(Some(text)),
        None => read(&cx.env.vendor_pam_dir.join(STACK)),
    }
}

/// Whether an `auth` line already asks pam_fprintd.
fn has_fingerprint(stack: &str) -> bool {
    stack.lines().any(|l| {
        let l = l.trim_start();
        !l.starts_with('#') && l.starts_with("auth") && l.contains("pam_fprintd.so")
    })
}

/// `stack` with [`LINE`] before its first `auth` line — every other line
/// as it was, in order.
pub(crate) fn with_fingerprint(stack: &str) -> String {
    let mut out = String::with_capacity(stack.len() + LINE.len() + MARK.len() + 2);
    let mut added = false;
    for line in stack.lines() {
        let t = line.trim_start();
        if !added && !t.starts_with('#') && t.starts_with("auth") {
            out.push_str(MARK);
            out.push('\n');
            out.push_str(LINE);
            out.push('\n');
            added = true;
        }
        out.push_str(line);
        out.push('\n');
    }
    if !added {
        // No auth line at all would be a stack that authenticates nobody;
        // the line goes last rather than nowhere.
        out.push_str(MARK);
        out.push('\n');
        out.push_str(LINE);
        out.push('\n');
    }
    out
}

pub(super) fn check(cx: &Cx<'_>) -> State {
    if !cx.env.pam_modules.join("pam_fprintd.so").exists() {
        return State::Unavailable { why: "pam_fprintd isn't installed".into() };
    }
    match cx.sys.fingerprint_enrolled(&cx.env.user) {
        Some(false) => return State::Unavailable { why: "no fingerprint is enrolled — fprintd-enroll adds one".into() },
        None => return State::Unknown { why: "fprintd didn't say whether a finger is enrolled".into() },
        Some(true) => {}
    }
    match in_effect(cx) {
        Err(e) => State::Unknown { why: e },
        Ok(None) => State::Unavailable { why: "polkit has no PAM stack here".into() },
        Ok(Some(stack)) if has_fingerprint(&stack) => State::Done,
        Ok(Some(_)) => State::Todo { what: "Use your fingerprint for administrator prompts (asks for your password)".into() },
    }
}

pub(super) fn apply(cx: &Cx<'_>) -> Result<Applied, String> {
    let previous = read(&local(cx))?;
    let base = match &previous {
        Some(text) => text.clone(),
        None => in_effect(cx)?.ok_or_else(|| "polkit has no PAM stack here".to_string())?,
    };
    cx.sys.write_as_root(&local(cx), Some(&with_fingerprint(&base)))?;
    Ok(Applied {
        change: Change::FingerprintPam { previous },
        note: Some("touch the reader when asked, or wait ten seconds to type your password".into()),
    })
}

/// `stack` with the two lines [`with_fingerprint`] added taken back out,
/// every other line as it is — or `None` when they are not there.
pub(crate) fn without_fingerprint(stack: &str) -> Option<String> {
    let lines: Vec<&str> = stack.lines().collect();
    let at = lines.windows(2).position(|pair| pair[0] == MARK && pair[1] == LINE)?;
    let mut out = String::with_capacity(stack.len());
    for (i, line) in lines.iter().enumerate() {
        if i != at && i != at + 1 {
            out.push_str(line);
            out.push('\n');
        }
    }
    Some(out)
}

/// Takes this item's line back out of the stack in effect — or, when
/// what is left is exactly the vendor's, removes the local file so the
/// vendor's stack applies again.
///
/// `_previous` is deliberately not what gets written. It comes from
/// `setup.toml`, which the user's own account can write, and this runs as
/// root: installing it would let anything running as the user put
/// `auth sufficient pam_permit.so` into polkit's stack and be root
/// without a password the next time anyone undid this item. The record
/// only says *that* the line was added; the file itself says what to
/// keep.
pub(super) fn undo(cx: &Cx<'_>, _previous: Option<&str>) -> Result<Option<String>, String> {
    let Some(current) = read(&local(cx))? else {
        return Ok(Some("polkit's stack had no local copy left to change.".into()));
    };
    let Some(restored) = without_fingerprint(&current) else {
        return Ok(Some("The fingerprint line was already gone from polkit's stack.".into()));
    };
    let vendor = read(&cx.env.vendor_pam_dir.join(STACK))?;
    if vendor.as_deref() == Some(restored.as_str()) {
        cx.sys.write_as_root(&local(cx), None)?;
    } else {
        cx.sys.write_as_root(&local(cx), Some(&restored))?;
    }
    Ok(None)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Arch's `/usr/lib/pam.d/polkit-1`, as read on the machine this was
    /// written on.
    const ARCH: &str = "#%PAM-1.0\n\nauth       include      system-auth\naccount    include      system-auth\npassword   include      system-auth\nsession    include      system-auth\n";

    /// The finger goes first, and every line that was there comes through
    /// unchanged and in order — the property that keeps `pkexec`, and so
    /// this item's own undo, working.
    #[test]
    fn the_finger_goes_first_and_nothing_else_changes() {
        let out = with_fingerprint(ARCH);
        let first_auth = out.lines().find(|l| l.starts_with("auth")).unwrap();
        assert_eq!(first_auth, LINE);
        let kept: Vec<&str> = out.lines().filter(|l| *l != LINE && *l != MARK).collect();
        assert_eq!(kept, ARCH.lines().collect::<Vec<_>>());
        assert!(out.starts_with("#%PAM-1.0\n"), "the header stays the first line");
    }

    #[test]
    fn taking_the_finger_out_gives_back_the_stack_it_went_into() {
        assert_eq!(without_fingerprint(&with_fingerprint(ARCH)).as_deref(), Some(ARCH));
        assert_eq!(without_fingerprint(ARCH), None, "nothing of ours to take out");
        // A line someone wrote themselves, identical to ours but without
        // the mark, is theirs and stays.
        let theirs = format!("{LINE}\n{ARCH}");
        assert_eq!(without_fingerprint(&theirs), None);
    }

    #[test]
    fn a_stack_that_already_asks_the_reader_is_seen_as_done() {
        assert!(has_fingerprint(&with_fingerprint(ARCH)));
        assert!(!has_fingerprint(ARCH));
        assert!(!has_fingerprint("# auth sufficient pam_fprintd.so\nauth include system-auth\n"), "a comment is not a line");
    }
}
