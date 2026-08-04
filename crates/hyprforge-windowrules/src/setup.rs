//! One-time insertion of `require("hyprforge/window-rules")` into the
//! user's `hyprland.lua`.
//!
//! Hyprland 0.56 evaluates all *named* rules first, then all anonymous
//! ones, and the **last** match wins. Hyprforge rules are always named (see
//! [`crate::model::Rule::name`]), so they are placed **first** among the
//! user's `require()` calls: this makes Hyprforge's named rules the
//! earliest-evaluated, so the user's own rules — named or anonymous —
//! override them by default. Appending last would invert that and let
//! Hyprforge silently beat the user's own named rules.

use std::path::Path;

pub const REQUIRE_LINE: &str = "require(\"hyprforge/window-rules\")";

#[derive(Debug, Clone, PartialEq)]
pub enum SetupPlan {
    /// The require line is already present; nothing to do.
    AlreadyPresent,
    /// Not present. `insert_before_line` is the 1-indexed line number of the
    /// first existing `require(` call the new line will be inserted before
    /// (or, if there are none, appended at the end of the file).
    NeedsInsert { insert_before_line: usize },
}

/// Detects whether `hyprland.lua` (or an equivalent Lua file the user
/// supplies for testing) already sources the Hyprforge window-rules file.
pub fn detect(hyprland_lua: &str) -> SetupPlan {
    if hyprland_lua
        .lines()
        .any(|l| l.trim_start().starts_with("require(") && l.contains(REQUIRE_LINE))
    {
        return SetupPlan::AlreadyPresent;
    }

    let first_require_line = hyprland_lua
        .lines()
        .enumerate()
        .find(|(_, l)| l.trim_start().starts_with("require("))
        .map(|(i, _)| i + 1);

    match first_require_line {
        Some(line) => SetupPlan::NeedsInsert {
            insert_before_line: line,
        },
        None => SetupPlan::NeedsInsert {
            insert_before_line: hyprland_lua.lines().count() + 1,
        },
    }
}

/// Renders the exact line to be inserted, for display in the GUI's
/// confirmation dialog.
pub fn preview_line() -> String {
    REQUIRE_LINE.to_string()
}

/// Applies `plan` to `hyprland_lua`, returning the new full file contents.
/// Does nothing (returns the input unchanged) if the plan is
/// [`SetupPlan::AlreadyPresent`].
pub fn apply(hyprland_lua: &str, plan: &SetupPlan) -> String {
    let SetupPlan::NeedsInsert { insert_before_line } = plan else {
        return hyprland_lua.to_string();
    };

    let mut lines: Vec<&str> = hyprland_lua.lines().collect();
    let idx = (*insert_before_line - 1).min(lines.len());
    lines.insert(idx, REQUIRE_LINE);
    let mut out = lines.join("\n");
    if hyprland_lua.ends_with('\n') || hyprland_lua.is_empty() {
        out.push('\n');
    }
    out
}

#[derive(Debug, thiserror::Error)]
pub enum SetupError {
    #[error("failed to read {path}: {source}")]
    Read {
        path: String,
        #[source]
        source: std::io::Error,
    },
    #[error("failed to back up {path}: {source}")]
    Backup {
        path: String,
        #[source]
        source: std::io::Error,
    },
    #[error("failed to write {path}: {source}")]
    Write {
        path: String,
        #[source]
        source: std::io::Error,
    },
}

/// Detects, then — only if a change is needed — backs up the current file
/// to `<path>.hyprforge.bak` and writes the modified contents atomically.
/// Callers (the GUI) must have already shown the user the exact change and
/// obtained explicit confirmation before calling this.
pub fn install(path: &Path) -> Result<SetupPlan, SetupError> {
    let contents = std::fs::read_to_string(path).map_err(|source| SetupError::Read {
        path: path.display().to_string(),
        source,
    })?;
    let plan = detect(&contents);
    if plan == SetupPlan::AlreadyPresent {
        return Ok(plan);
    }

    let backup_path = path.with_extension("lua.hyprforge.bak");
    std::fs::copy(path, &backup_path).map_err(|source| SetupError::Backup {
        path: backup_path.display().to_string(),
        source,
    })?;

    let new_contents = apply(&contents, &plan);
    let tmp = path.with_extension("lua.tmp");
    std::fs::write(&tmp, &new_contents).map_err(|source| SetupError::Write {
        path: tmp.display().to_string(),
        source,
    })?;
    std::fs::rename(&tmp, path).map_err(|source| SetupError::Write {
        path: path.display().to_string(),
        source,
    })?;

    Ok(plan)
}
