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

use std::path::{Path, PathBuf};

pub const REQUIRE_LINE: &str = "require(\"hyprforge/window-rules\")";

/// Contents written by [`create_lua_config`] when the user has no Hyprland
/// config at all. Deliberately minimal: Hyprforge creates a file that does
/// nothing except source its own rules, leaving every other setting at
/// Hyprland's defaults for the user to fill in themselves.
const MINIMAL_LUA_CONFIG: &str = concat!(
    "-- Hyprland configuration.\n",
    "-- Created by Hyprforge because no hyprland.lua existed yet. This file is\n",
    "-- yours to edit — Hyprforge only ever manages the require() line below.\n",
    "\n",
    "require(\"hyprforge/window-rules\")\n",
);

/// Which Hyprland config the user actually has, established *before* any
/// attempt to insert the require line.
///
/// Hyprforge generates Lua (`window-rules.lua`) and sources it with
/// `require()`, which only exists in Hyprland's Lua config format. A user on
/// the older `hyprland.conf` therefore can't be served by inserting a line —
/// they need to know that up front rather than after a failed write.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HyprConfig {
    /// `hyprland.lua` exists — the normal path; pair with [`detect`].
    Lua(PathBuf),
    /// Only `hyprland.conf` exists. `require()` is not usable here.
    ConfOnly(PathBuf),
    /// Neither file exists; [`create_lua_config`] can make one.
    Missing,
}

/// Looks for `hyprland.lua`, then `hyprland.conf`, directly in `hypr_dir`.
/// `Lua` wins when both are present: Hyprland's config language is Lua as of
/// 0.55, so a user who has a `.lua` at all has migrated, and a leftover
/// `.conf` shouldn't drag them back onto the migration notice.
pub fn discover(hypr_dir: &Path) -> HyprConfig {
    let lua = hypr_dir.join("hyprland.lua");
    if lua.is_file() {
        return HyprConfig::Lua(lua);
    }
    let conf = hypr_dir.join("hyprland.conf");
    if conf.is_file() {
        return HyprConfig::ConfOnly(conf);
    }
    HyprConfig::Missing
}

/// Renders the file [`create_lua_config`] would write, so the GUI can show
/// it before anything touches the disk (vision pillar #4).
pub fn preview_lua_config() -> String {
    MINIMAL_LUA_CONFIG.to_string()
}

/// Creates a minimal `hyprland.lua` sourcing the Hyprforge rules file.
///
/// Refuses if `path` already exists: this is the [`HyprConfig::Missing`]
/// path only, and creating a config is not the same operation as editing
/// one — an existing file must go through [`install`], which backs up first.
pub fn create_lua_config(path: &Path) -> Result<(), SetupError> {
    if path.exists() {
        return Err(SetupError::AlreadyExists {
            path: path.display().to_string(),
        });
    }
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|source| SetupError::Write {
            path: parent.display().to_string(),
            source,
        })?;
    }
    std::fs::write(path, MINIMAL_LUA_CONFIG).map_err(|source| SetupError::Write {
        path: path.display().to_string(),
        source,
    })
}

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
    #[error("{path} already exists — it must be edited via install(), not recreated")]
    AlreadyExists { path: String },
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
    hyprforge_core::paths::write_atomic(path, &new_contents).map_err(|source| {
        SetupError::Write {
            path: path.display().to_string(),
            source,
        }
    })?;

    Ok(plan)
}
