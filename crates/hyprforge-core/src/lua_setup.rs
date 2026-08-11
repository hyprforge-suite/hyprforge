//! Inserting one `require()` line into the user's `hyprland.lua`, once.
//!
//! Every Hyprforge component that generates Lua follows the same shape: it
//! owns a file under `hypr/hyprforge/`, and the user's own config sources it
//! with a single `require()` that Hyprforge inserts once and never touches
//! again. That mechanism lives here, parameterised by which module is being
//! installed, so a second component doesn't reimplement the backup-and-write
//! dance — or get it subtly different.
//!
//! What *isn't* generic is where the line goes relative to the user's own
//! requires; that depends on the evaluation-order semantics of whatever the
//! module generates, so callers decide and document it.

use std::path::{Path, PathBuf};

/// The `require("...")` line for a module living at `hyprforge/<name>`.
pub fn require_line(module: &str) -> String {
    format!("require(\"hyprforge/{module}\")")
}

/// Which Hyprland config the user actually has, established *before* any
/// attempt to insert a require line.
///
/// Hyprforge generates Lua and sources it with `require()`, which only exists
/// in Hyprland's Lua config format. A user still on the older
/// `hyprland.conf` therefore can't be served by inserting a line — they need
/// to know that up front rather than after a failed write.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HyprConfig {
    /// `hyprland.lua` exists — the normal path.
    Lua(PathBuf),
    /// Only `hyprland.conf` exists. `require()` is not usable here.
    ConfOnly(PathBuf),
    /// Neither file exists; [`create_lua_config`] can make one.
    Missing,
}

/// Looks for `hyprland.lua`, then `hyprland.conf`, directly in `hypr_dir`.
///
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

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SetupPlan {
    /// The require line is already present; nothing to do.
    AlreadyPresent,
    /// Not present. `insert_before_line` is the 1-indexed line the new line
    /// goes before.
    NeedsInsert { insert_before_line: usize },
}

/// Where a module's require line belongs among the user's own.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Placement {
    /// Before the user's first `require()`, so this module is evaluated
    /// earliest and the user's own config can override it.
    BeforeUserRequires,
    /// After everything, so this module has the last word.
    AtEnd,
}

/// Detects whether `hyprland_lua` already sources `line`, and where it would
/// go if not.
pub fn detect(hyprland_lua: &str, line: &str, placement: Placement) -> SetupPlan {
    if hyprland_lua
        .lines()
        .any(|l| l.trim_start().starts_with("require(") && l.contains(line))
    {
        return SetupPlan::AlreadyPresent;
    }

    let end = hyprland_lua.lines().count() + 1;
    let insert_before_line = match placement {
        Placement::AtEnd => end,
        Placement::BeforeUserRequires => hyprland_lua
            .lines()
            .enumerate()
            .find(|(_, l)| l.trim_start().starts_with("require("))
            .map(|(i, _)| i + 1)
            // No requires at all, so "before the first one" is the end.
            .unwrap_or(end),
    };
    SetupPlan::NeedsInsert { insert_before_line }
}

/// Applies `plan` to `hyprland_lua`, returning the new full contents.
/// Returns the input unchanged for [`SetupPlan::AlreadyPresent`].
pub fn apply(hyprland_lua: &str, plan: &SetupPlan, line: &str) -> String {
    let SetupPlan::NeedsInsert { insert_before_line } = plan else {
        return hyprland_lua.to_string();
    };

    let mut lines: Vec<&str> = hyprland_lua.lines().collect();
    let idx = (*insert_before_line - 1).min(lines.len());
    lines.insert(idx, line);
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
    #[error("failed to back up to {path}: {source}")]
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

/// Detects, then — only if a change is needed — backs up the current file to
/// `<path>.hyprforge.bak` and writes the modified contents atomically.
///
/// Callers must have shown the user the exact change and obtained explicit
/// confirmation first: this edits a file Hyprforge does not own.
pub fn install(path: &Path, line: &str, placement: Placement) -> Result<SetupPlan, SetupError> {
    let contents = std::fs::read_to_string(path).map_err(|source| SetupError::Read {
        path: path.display().to_string(),
        source,
    })?;
    let plan = detect(&contents, line, placement);
    if plan == SetupPlan::AlreadyPresent {
        return Ok(plan);
    }

    // One backup name across modules, and only ever written when a change is
    // actually being made — so installing a second module doesn't overwrite
    // the record of what the file looked like before Hyprforge touched it
    // with a copy that already includes the first module's line.
    let backup_path = path.with_extension("lua.hyprforge.bak");
    if !backup_path.exists() {
        std::fs::copy(path, &backup_path).map_err(|source| SetupError::Backup {
            path: backup_path.display().to_string(),
            source,
        })?;
    }

    let new_contents = apply(&contents, &plan, line);
    crate::paths::write_atomic(path, &new_contents).map_err(|source| SetupError::Write {
        path: path.display().to_string(),
        source,
    })?;

    Ok(plan)
}

/// The file [`create_lua_config`] would write, so a GUI can show it before
/// anything touches the disk.
pub fn preview_lua_config(lines: &[String]) -> String {
    let mut out = String::from(
        "-- Hyprland configuration.\n\
         -- Created by Hyprforge because no hyprland.lua existed yet. This file is\n\
         -- yours to edit — Hyprforge only ever manages the require() lines below.\n\n",
    );
    for line in lines {
        out.push_str(line);
        out.push('\n');
    }
    out
}

/// Creates a minimal `hyprland.lua` sourcing the given require lines.
///
/// Refuses if `path` exists: this is the [`HyprConfig::Missing`] path only,
/// and creating a config is not the same operation as editing one — an
/// existing file must go through [`install`], which backs up first.
pub fn create_lua_config(path: &Path, lines: &[String]) -> Result<(), SetupError> {
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
    std::fs::write(path, preview_lua_config(lines)).map_err(|source| SetupError::Write {
        path: path.display().to_string(),
        source,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_module_name_becomes_a_require_line() {
        assert_eq!(require_line("keybinds"), "require(\"hyprforge/keybinds\")");
    }

    #[test]
    fn placement_before_user_requires_goes_first() {
        let cfg = "-- header\nrequire(\"mine\")\nrequire(\"other\")\n";
        let line = require_line("keybinds");
        let plan = detect(cfg, &line, Placement::BeforeUserRequires);
        assert_eq!(plan, SetupPlan::NeedsInsert { insert_before_line: 2 });
        let out = apply(cfg, &plan, &line);
        assert!(out.lines().nth(1).unwrap().contains("hyprforge/keybinds"));
    }

    #[test]
    fn placement_at_end_goes_last() {
        let cfg = "require(\"mine\")\n";
        let line = require_line("keybinds");
        let plan = detect(cfg, &line, Placement::AtEnd);
        let out = apply(cfg, &plan, &line);
        assert!(out.lines().last().unwrap().contains("hyprforge/keybinds"));
    }

    /// With no requires at all, both placements mean "the end" — there's
    /// nothing to go before.
    #[test]
    fn a_config_with_no_requires_appends_either_way() {
        let cfg = "hl.config({})\n";
        let line = require_line("keybinds");
        for placement in [Placement::BeforeUserRequires, Placement::AtEnd] {
            let out = apply(cfg, &detect(cfg, &line, placement), &line);
            assert!(out.lines().last().unwrap().contains("hyprforge/keybinds"));
        }
    }

    /// Two modules must not see each other as "already present".
    #[test]
    fn modules_are_detected_independently() {
        let cfg = format!("{}\n", require_line("window-rules"));
        assert_eq!(
            detect(&cfg, &require_line("window-rules"), Placement::AtEnd),
            SetupPlan::AlreadyPresent
        );
        assert!(matches!(
            detect(&cfg, &require_line("keybinds"), Placement::AtEnd),
            SetupPlan::NeedsInsert { .. }
        ));
    }

    #[test]
    fn installing_is_idempotent() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("hyprland.lua");
        std::fs::write(&path, "hl.config({})\n").unwrap();
        let line = require_line("keybinds");

        install(&path, &line, Placement::AtEnd).unwrap();
        let once = std::fs::read_to_string(&path).unwrap();
        assert_eq!(
            install(&path, &line, Placement::AtEnd).unwrap(),
            SetupPlan::AlreadyPresent
        );
        assert_eq!(std::fs::read_to_string(&path).unwrap(), once);
    }

    /// The backup must record the file as it was before *Hyprforge* touched
    /// it, not before the most recent module was added — otherwise
    /// installing a second module destroys the only copy without the first.
    #[test]
    fn the_backup_is_taken_once_across_modules() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("hyprland.lua");
        std::fs::write(&path, "original\n").unwrap();

        install(&path, &require_line("window-rules"), Placement::AtEnd).unwrap();
        install(&path, &require_line("keybinds"), Placement::AtEnd).unwrap();

        let backup = std::fs::read_to_string(path.with_extension("lua.hyprforge.bak")).unwrap();
        assert_eq!(backup, "original\n", "the backup should predate both modules");
    }

    #[test]
    fn creating_a_config_refuses_to_clobber_one() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("hyprland.lua");
        std::fs::write(&path, "theirs\n").unwrap();
        assert!(create_lua_config(&path, &[require_line("keybinds")]).is_err());
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "theirs\n");
    }

    #[test]
    fn a_created_config_sources_every_module_asked_for() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("nested").join("hyprland.lua");
        let lines = [require_line("window-rules"), require_line("keybinds")];
        create_lua_config(&path, &lines).unwrap();
        let out = std::fs::read_to_string(&path).unwrap();
        assert!(out.contains("hyprforge/window-rules"));
        assert!(out.contains("hyprforge/keybinds"));
    }
}
