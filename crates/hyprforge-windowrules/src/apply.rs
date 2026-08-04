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
}

/// Regenerates `window-rules.lua` from `rules` and triggers `hyprctl
/// reload`. Any failure is returned as an actionable error the GUI can show
/// directly — nothing is silently swallowed (vision pillar #3: no dead
/// ends).
pub fn apply(lua_path: &Path, rules: &[Rule]) -> Result<(), ApplyError> {
    let lua = generate(rules);
    if let Some(parent) = lua_path.parent() {
        std::fs::create_dir_all(parent).map_err(|source| ApplyError::Write {
            path: parent.display().to_string(),
            source,
        })?;
    }
    let tmp = lua_path.with_extension("lua.tmp");
    std::fs::write(&tmp, &lua).map_err(|source| ApplyError::Write {
        path: tmp.display().to_string(),
        source,
    })?;
    std::fs::rename(&tmp, lua_path).map_err(|source| ApplyError::Write {
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
    Ok(())
}
