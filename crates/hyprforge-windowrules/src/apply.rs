use crate::codegen::generate;
use crate::model::Rule;
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
    #[error("Hyprland rejected the generated rules:\n{details}")]
    ConfigRejected { details: String },
}

/// Regenerates `window-rules.lua` from `rules` and triggers `hyprctl
/// reload`. Any failure is returned as an actionable error the GUI can show
/// directly — nothing is silently swallowed (vision pillar #3: no dead
/// ends).
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
/// The two catch different things, measured on 0.56.1. A Lua *syntax* error
/// shows up in the response body and never reaches `configerrors`. A rejected
/// rule *property* — `unknown match property 'floating'` — shows up in both,
/// and lingers in `configerrors` until the next reload, which is what puts
/// Hyprland's error overlay on screen. Neither check subsumes the other, so
/// both are here.
pub fn apply(lua_path: &Path, rules: &[Rule]) -> Result<(), ApplyError> {
    // Snapshot first: the user's own config may already have errors that
    // aren't ours, and failing their save over someone else's typo would be
    // its own dead end. Only errors that appear between here and the reload
    // are attributable to what we just wrote.
    let before = config_errors();

    let lua = generate(rules);
    hyprforge_core::paths::write_atomic(lua_path, &lua).map_err(|source| ApplyError::Write {
        path: lua_path.display().to_string(),
        source,
    })?;

    let output = Command::new("hyprctl")
        .arg("reload")
        .output()
        .map_err(ApplyError::Spawn)?;

    if !output.status.success() {
        return Err(ApplyError::ReloadFailed {
            stderr: String::from_utf8_lossy(&output.stderr).into_owned(),
        });
    }

    let body = String::from_utf8_lossy(&output.stdout);
    if let Some(message) = error_in_body(&body) {
        return Err(ApplyError::ConfigRejected { details: message });
    }

    let new_errors = added_errors(&before, &config_errors());
    if !new_errors.is_empty() {
        return Err(ApplyError::ConfigRejected {
            details: new_errors.join("\n"),
        });
    }
    Ok(())
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
    use super::{added_errors, error_in_body};

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
}
