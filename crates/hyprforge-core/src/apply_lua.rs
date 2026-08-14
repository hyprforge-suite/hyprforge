//! Writing a generated Lua file and getting Hyprland to load it.
//!
//! Shared because every Lua-owning module needs exactly this sequence and
//! getting it right took measurement against a real compositor — see
//! [`apply`]'s doc for what those measurements were. `hyprforge-shortcuts`
//! and `hyprforge-windowrules` each had their own copy, and they had already
//! diverged: rollback was added to one and not the other, leaving the module
//! that didn't happen to notice still able to strand a rejected file on disk.

use crate::paths::write_atomic;
use std::path::Path;
use std::process::Command;

#[derive(Debug, thiserror::Error)]
pub enum ApplyError {
    #[error("failed to write {path}: {source}")]
    Write {
        path: String,
        #[source]
        source: std::io::Error,
    },
    #[error("failed to run hyprctl reload: {0}")]
    Spawn(#[source] std::io::Error),
    #[error("hyprctl reload failed: {stderr}")]
    ReloadFailed { stderr: String },
    /// Hyprland loaded the file and refused part of it. Carries Hyprland's
    /// own wording, which names the file and line — far more useful than
    /// anything this crate could infer.
    #[error("Hyprland rejected the generated {subject}:\n{details}")]
    ConfigRejected {
        subject: &'static str,
        details: String,
    },
}

/// One module's generated Lua file, and what to call it when it's refused.
pub struct GeneratedFile<'a> {
    pub path: &'a Path,
    pub contents: &'a str,
    /// What to leave behind if the write is rejected and there was no
    /// previous version to restore — the module's own "no entries" output.
    ///
    /// It can't just be an empty file: the `require()` line pointing here
    /// may already be installed, and this is what the user is left running.
    pub empty: &'a str,
    /// Names the artifact in the rejection message: `"rules"`,
    /// `"shortcuts"`.
    pub subject: &'static str,
}

/// Writes `file` and triggers `hyprctl reload`, restoring the previous
/// version if Hyprland refuses it. Any failure is returned as an actionable
/// error the GUI can show directly — nothing is silently swallowed (vision
/// pillar #3: no dead ends).
///
/// Success is *not* the exit status alone. `hyprctl` decides per subcommand
/// whether a rejection is worth a non-zero exit — measured on 0.56.1, a bogus
/// `keyword` exits 0 with the complaint only in the body, while `eval` and
/// `dispatch` exit 7. Every one of them puts the message in the body, which
/// is why the revert prompt in the Settings app reads the body too. Trusting
/// the status alone here can report a file Hyprland refused as a clean save.
///
/// So three things are checked, in increasing order of what they can catch:
/// the exit status, the response body, and `hyprctl configerrors` — the
/// compositor's own answer to "is anything in the loaded config broken", and
/// the source its error overlay draws on.
///
/// The last two catch different things, measured on 0.56.1. A Lua *syntax*
/// error shows up in the response body and never reaches `configerrors`. A
/// rejected rule *property* — `unknown match property 'floating'` — shows up
/// in both, and lingers in `configerrors` until the next reload, which is
/// what puts Hyprland's error overlay on screen. Neither check subsumes the
/// other, so both are here.
pub fn apply(file: GeneratedFile<'_>) -> Result<(), ApplyError> {
    // Snapshot first: the user's own config may already have errors that
    // aren't ours, and failing their save over someone else's typo would be
    // its own dead end. Only errors that appear between here and the reload
    // are attributable to what we just wrote.
    let before = config_errors();
    let previous = last_good(file.path, file.empty);

    write_atomic(file.path, file.contents).map_err(|source| ApplyError::Write {
        path: file.path.display().to_string(),
        source,
    })?;

    let output = Command::new("hyprctl")
        .arg("reload")
        .output()
        .map_err(ApplyError::Spawn)?;

    let rejected = |details: String| ApplyError::ConfigRejected {
        subject: file.subject,
        details,
    };

    if !output.status.success() {
        let error = ApplyError::ReloadFailed {
            stderr: String::from_utf8_lossy(&output.stderr).into_owned(),
        };
        // A non-zero status means either "the compositor refused this" or
        // "there was no compositor to ask" — this command reports a
        // rejection in the status only sometimes, so the status alone can't
        // tell them apart. Whether the compositor gained an error can.
        // Reverting merely because Hyprland isn't running would throw away
        // a file that the next reload would have accepted quite happily.
        let refused = !added_errors(&before, &config_errors()).is_empty();
        return Err(if refused { roll_back(file.path, &previous, error) } else { error });
    }

    let body = String::from_utf8_lossy(&output.stdout);
    if let Some(message) = error_in_body(&body) {
        return Err(roll_back(file.path, &previous, rejected(message)));
    }

    let new_errors = added_errors(&before, &config_errors());
    if !new_errors.is_empty() {
        return Err(roll_back(file.path, &previous, rejected(new_errors.join("\n"))));
    }
    Ok(())
}

/// The file as it stands before a write — or `empty` if there isn't one yet.
///
/// A file that doesn't exist still has a "previous" state worth restoring,
/// because the `require()` line pointing at it may already be installed and
/// a missing target is its own reload error.
fn last_good(path: &Path, empty: &str) -> String {
    std::fs::read_to_string(path).unwrap_or_else(|_| empty.to_string())
}

/// Puts the last-good file back and reloads, then returns `error` unchanged.
///
/// Without this a rejected save leaves the broken file on disk: Hyprland
/// keeps the last config it accepted in memory, so the failure looks
/// contained — until the user's next `hyprctl reload` (or next login) picks
/// the bad file up and takes every other entry down with it. The rejection
/// is still reported either way; this only decides what the user's config is
/// left in.
///
/// Best-effort by design. A failure to restore can't be reported without
/// discarding the error that says *why* the save failed, which is the more
/// useful of the two.
fn roll_back(path: &Path, previous: &str, error: ApplyError) -> ApplyError {
    if write_atomic(path, previous).is_ok() {
        let _ = Command::new("hyprctl").arg("reload").output();
    }
    error
}

/// The complaint in an `hyprctl` response body, if it is one.
///
/// A success is the literal `ok`; a failure is prefixed `error:` — the shape
/// measured across `eval`, `dispatch` and `keyword` on 0.56.1. Anything else
/// is passed over rather than guessed at, since treating unrecognised output
/// as failure would break saves on the next release that adds a notice line.
fn error_in_body(body: &str) -> Option<String> {
    let body = body.trim();
    if body.is_empty() || body.eq_ignore_ascii_case("ok") {
        return None;
    }
    body.lines()
        .any(|l| l.trim_start().to_lowercase().starts_with("error:"))
        .then(|| body.to_string())
}

/// Hyprland's current config parse errors, one per line, or an empty list if
/// `hyprctl` can't be reached at all.
///
/// An unreachable `hyprctl` is not treated as an error: the reload call above
/// already reports that case, and inventing a second failure here would turn
/// "not running Hyprland" into a save failure.
fn config_errors() -> Vec<String> {
    let Ok(out) = Command::new("hyprctl").arg("configerrors").output() else {
        return Vec::new();
    };
    String::from_utf8_lossy(&out.stdout)
        .lines()
        .map(str::trim)
        .filter(|l| !l.is_empty())
        .map(str::to_string)
        .collect()
}

/// Errors present in `after` that weren't in `before`.
///
/// Compares as a multiset rather than a set: the same message appearing twice
/// where it appeared once is a new error too, and dropping the duplicate would
/// hide it.
fn added_errors(before: &[String], after: &[String]) -> Vec<String> {
    let mut remaining: Vec<&String> = before.iter().collect();
    let mut added = Vec::new();
    for err in after {
        match remaining.iter().position(|b| *b == err) {
            Some(i) => {
                remaining.remove(i);
            }
            None => added.push(err.clone()),
        }
    }
    added
}

#[cfg(test)]
mod tests {
    use super::*;

    fn v(items: &[&str]) -> Vec<String> {
        items.iter().map(|s| s.to_string()).collect()
    }

    /// The exact strings hyprctl 0.56.1 returns, captured from the running
    /// compositor rather than imagined.
    #[test]
    fn a_success_body_is_not_an_error() {
        assert_eq!(error_in_body("ok"), None);
        assert_eq!(error_in_body("ok\n"), None);
        assert_eq!(error_in_body(""), None);
    }

    #[test]
    fn an_error_body_is_reported_verbatim() {
        let body = "error: ...probe_bad.lua:1: syntax error near 'is'";
        assert_eq!(error_in_body(body).as_deref(), Some(body));
    }

    /// hyprctl appends explanatory notes after the error line; the whole
    /// body is what helps, not just the first line.
    #[test]
    fn a_multi_line_error_keeps_its_explanation() {
        let body = "error: hl.dispatch: expected a dispatcher\n\n → Note: dispatch in lua is a shorthand";
        assert!(error_in_body(body).unwrap().contains("Note:"));
    }

    /// Output that is neither `ok` nor an error must not fail the save — a
    /// future release adding a banner line shouldn't break saving.
    #[test]
    fn unrecognised_output_is_not_treated_as_failure() {
        assert_eq!(error_in_body("reloaded 3 monitors"), None);
    }

    #[test]
    fn an_error_the_user_already_had_is_not_blamed_on_us() {
        let before = v(&["config error: line 12 of theirs"]);
        let after = v(&["config error: line 12 of theirs"]);
        assert!(added_errors(&before, &after).is_empty());
    }

    #[test]
    fn an_error_that_appears_after_the_write_is_reported() {
        let before = v(&["config error: line 12 of theirs"]);
        let after = v(&[
            "config error: line 12 of theirs",
            "window-rules.lua:3: unexpected symbol",
        ]);
        assert_eq!(added_errors(&before, &after), v(&["window-rules.lua:3: unexpected symbol"]));
    }

    /// A clean config that becomes broken is the case this exists for.
    #[test]
    fn the_first_error_on_a_clean_config_is_reported() {
        let after = v(&["window-rules.lua:1: '}' expected"]);
        assert_eq!(added_errors(&[], &after), after);
    }

    /// Two copies of one message where there was one means a second thing
    /// broke; collapsing them would swallow it.
    #[test]
    fn a_repeated_message_counts_as_new() {
        let before = v(&["same complaint"]);
        let after = v(&["same complaint", "same complaint"]);
        assert_eq!(added_errors(&before, &after), v(&["same complaint"]));
    }

    /// Fixing an unrelated error while saving must not read as an addition.
    #[test]
    fn errors_that_went_away_are_not_reported() {
        let before = v(&["a", "b"]);
        let after = v(&["b"]);
        assert!(added_errors(&before, &after).is_empty());
    }

    /// What a rejected save is rolled back *to*. A file that was never
    /// written still has a correct answer — the module's own empty output,
    /// which the already-installed `require()` line can load.
    #[test]
    fn a_file_that_never_existed_rolls_back_to_the_empty_version() {
        let dir = tempfile::tempdir().unwrap();
        let missing = dir.path().join("keybinds.lua");
        assert_eq!(last_good(&missing, "-- empty\n"), "-- empty\n");

        std::fs::write(&missing, "-- real\n").unwrap();
        assert_eq!(last_good(&missing, "-- empty\n"), "-- real\n");
    }

    /// The rejection message names the artifact, so a user reading it knows
    /// which module refused rather than just "something".
    #[test]
    fn a_rejection_names_what_was_refused() {
        let error = ApplyError::ConfigRejected {
            subject: "shortcuts",
            details: "keybinds.lua:3: bad".to_string(),
        };
        assert!(error.to_string().starts_with("Hyprland rejected the generated shortcuts:"));
    }
}
