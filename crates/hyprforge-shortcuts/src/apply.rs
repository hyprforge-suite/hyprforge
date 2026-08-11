//! Writing `keybinds.lua` and getting Hyprland to load it.
//!
//! Mirrors `hyprforge-windowrules`'s apply, including why it doesn't trust
//! the exit status: hyprctl reports a rejection in the response body, and
//! only sometimes in the status. See that crate for the measurements.

use crate::codegen::generate;
use crate::model::Shortcut;
use std::path::Path;
use std::process::Command;

#[derive(Debug, thiserror::Error)]
pub enum ApplyError {
    #[error("failed to write {path}: {source}")]
    Write { path: String, #[source] source: std::io::Error },
    #[error("failed to run hyprctl reload: {0}")]
    Spawn(#[source] std::io::Error),
    #[error("hyprctl reload failed: {stderr}")]
    ReloadFailed { stderr: String },
    #[error("Hyprland rejected the generated shortcuts:\n{details}")]
    ConfigRejected { details: String },
}

pub fn apply(lua_path: &Path, shortcuts: &[Shortcut]) -> Result<(), ApplyError> {
    let before = config_errors();

    let lua = generate(shortcuts);
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
    if let Some(message) = error_in_body(&String::from_utf8_lossy(&output.stdout)) {
        return Err(ApplyError::ConfigRejected { details: message });
    }
    let added = added_errors(&before, &config_errors());
    if !added.is_empty() {
        return Err(ApplyError::ConfigRejected { details: added.join("\n") });
    }
    Ok(())
}

fn error_in_body(body: &str) -> Option<String> {
    let body = body.trim();
    if body.is_empty() || body.eq_ignore_ascii_case("ok") {
        return None;
    }
    body.lines()
        .any(|l| l.trim_start().to_lowercase().starts_with("error:"))
        .then(|| body.to_string())
}

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

/// Errors in `after` that weren't in `before`, so a user's pre-existing
/// config error doesn't fail their save.
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

    #[test]
    fn a_success_body_is_not_an_error() {
        assert_eq!(error_in_body("ok"), None);
    }

    #[test]
    fn an_error_body_is_reported() {
        assert!(error_in_body("error: keybinds.lua:3: bad").is_some());
    }

    #[test]
    fn a_preexisting_error_is_not_blamed_on_us() {
        let same = vec!["theirs".to_string()];
        assert!(added_errors(&same, &same).is_empty());
    }
}
