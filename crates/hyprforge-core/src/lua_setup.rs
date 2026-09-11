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

/// The closing marker of a Hyprforge-managed require block, shared by both
/// placements — which block a given `-- end...` line closes is determined
/// by which start marker precedes it, not by its own text.
const MARKER_END: &str = "-- end Hyprforge-managed requires";

/// The opening marker for `placement`'s block. Two placements, two blocks:
/// grouping every `BeforeUserRequires` module's line together (and likewise
/// for `AtEnd`) is what stops a second module landing in an arbitrary
/// position relative to the first — see [`detect`].
fn marker_start(placement: Placement) -> &'static str {
    match placement {
        Placement::BeforeUserRequires => {
            "-- Hyprforge-managed requires (evaluated first) — do not edit by hand."
        }
        Placement::AtEnd => "-- Hyprforge-managed requires (evaluated last) — do not edit by hand.",
    }
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
///
/// A require already grouped under `placement`'s marker block is found
/// first and reuses that block (see [`apply`]) — so a second
/// `BeforeUserRequires` module lands inside the first one's block, not in
/// an arbitrary position determined by whichever require happened to be
/// textually first at install time.
/// The first line that isn't a comment or blank, 1-indexed — where a block
/// belongs when it has to go "at the top" without displacing a file's header
/// comment. Falls back to line 1 for a file that is nothing but comments.
fn first_content_line(lines: &[&str]) -> usize {
    lines
        .iter()
        .position(|l| {
            let t = l.trim_start();
            !t.is_empty() && !t.starts_with("--")
        })
        .map(|i| i + 1)
        .unwrap_or(1)
}

pub fn detect(hyprland_lua: &str, line: &str, placement: Placement) -> SetupPlan {
    if hyprland_lua
        .lines()
        .any(|l| l.trim_start().starts_with("require(") && l.contains(line))
    {
        return SetupPlan::AlreadyPresent;
    }

    let lines: Vec<&str> = hyprland_lua.lines().collect();
    if let Some(start_idx) = lines.iter().position(|l| l.trim() == marker_start(placement)) {
        if let Some(offset) = lines[start_idx..].iter().position(|l| l.trim() == MARKER_END) {
            let end_idx = start_idx + offset;
            return SetupPlan::NeedsInsert { insert_before_line: end_idx + 1 };
        }
    }

    let end = lines.len() + 1;
    let insert_before_line = match placement {
        Placement::AtEnd => end,
        Placement::BeforeUserRequires => lines
            .iter()
            .enumerate()
            .find(|(_, l)| l.trim_start().starts_with("require("))
            .map(|(i, _)| i + 1)
            // No requires at all. "The end" is the tempting fallback and it
            // is exactly backwards: Hyprland applies the *last* matching
            // rule, verified against 0.56.1, so this placement only delivers
            // what it promises — the user's own config winning — by landing
            // early. Falling back to the end inverted that for every config
            // without a `require()` of its own, which is most of them.
            .unwrap_or_else(|| first_content_line(&lines)),
    };
    SetupPlan::NeedsInsert { insert_before_line }
}

/// Applies `plan` to `hyprland_lua`, returning the new full contents.
/// Returns the input unchanged for [`SetupPlan::AlreadyPresent`].
///
/// When `plan`'s insertion point sits right before an existing
/// `-- end Hyprforge-managed requires` line, only `line` itself is
/// inserted — [`detect`] already found `placement`'s block and is asking
/// for `line` to join it. Otherwise there's no block yet, so this creates
/// one: `line` gets its own opening/closing markers rather than being
/// dropped in bare.
pub fn apply(hyprland_lua: &str, plan: &SetupPlan, line: &str, placement: Placement) -> String {
    let SetupPlan::NeedsInsert { insert_before_line } = plan else {
        return hyprland_lua.to_string();
    };

    let mut lines: Vec<&str> = hyprland_lua.lines().collect();
    let idx = (*insert_before_line - 1).min(lines.len());
    let joins_existing_block = lines.get(idx).is_some_and(|l| l.trim() == MARKER_END);
    if joins_existing_block {
        lines.insert(idx, line);
    } else {
        for (offset, new_line) in [marker_start(placement), line, MARKER_END].into_iter().enumerate() {
            lines.insert(idx + offset, new_line);
        }
    }
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
    // `write_atomic`, not `fs::copy`: the overwrite below is atomic and
    // fsynced, so a copy that is neither leaves the backup as the half of
    // this pair that a power loss can lose. `contents` is already the
    // file's bytes, so there is nothing to re-read.
    let backup_path = path.with_extension("lua.hyprforge.bak");
    if !backup_path.exists() {
        crate::paths::write_atomic(&backup_path, &contents).map_err(|source| {
            SetupError::Backup {
                path: backup_path.display().to_string(),
                source,
            }
        })?;
    }

    let new_contents = apply(&contents, &plan, line, placement);
    crate::paths::write_atomic(path, &new_contents).map_err(|source| SetupError::Write {
        path: path.display().to_string(),
        source,
    })?;

    Ok(plan)
}

/// The file [`create_lua_config`] would write, so a GUI can show it before
/// anything touches the disk.
pub fn preview_lua_config(line: &str, placement: Placement) -> String {
    let mut out = String::from(
        "-- Hyprland configuration.\n\
         -- Created by Hyprforge because no hyprland.lua existed yet. This file is\n\
         -- yours to edit — Hyprforge only ever manages the require() lines below.\n\n",
    );
    out.push_str(marker_start(placement));
    out.push('\n');
    out.push_str(line);
    out.push('\n');
    out.push_str(MARKER_END);
    out.push('\n');
    out
}

/// Creates a minimal `hyprland.lua` sourcing `line`.
///
/// Refuses if `path` exists: this is the [`HyprConfig::Missing`] path only,
/// and creating a config is not the same operation as editing one — an
/// existing file must go through [`install`], which backs up first.
pub fn create_lua_config(path: &Path, line: &str, placement: Placement) -> Result<(), SetupError> {
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
    std::fs::write(path, preview_lua_config(line, placement)).map_err(|source| SetupError::Write {
        path: path.display().to_string(),
        source,
    })
}

/// Removes specific lines from specific files, after a successful import
/// (see `hyprforge-lua-import`) — but only lines that, right now, still
/// look exactly like a single-line `hl.*(...)` call: trimmed, starts with
/// `hl.` and ends with `)`. `targets` is `(file, 1-indexed line)` pairs.
///
/// This is deliberately conservative rather than clever. The importer
/// only ever knows where a call *starts* (see `RecordedCall::line`'s doc
/// comment), so a multi-line call's recorded line never satisfies "starts
/// with `hl.` *and* ends with `)`" on its own — it's excluded by the same
/// check that confirms a single-line call, not by a separate case. A file
/// edited since the import ran is handled the same way: re-reading and
/// re-checking here (rather than trusting what was true at import time)
/// means a line that no longer matches is just skipped, never force
/// -removed.
///
/// Multiple targets in the same file are handled together — each file is
/// read and rewritten at most once — and a fresh backup
/// (`<file>.hyprforge-import.bak`, always overwritten) is written first
/// for any file that has at least one real match. Returns how many lines
/// were actually removed, which may be less than `targets.len()`.
pub fn remove_matched_lines(targets: &[(PathBuf, usize)]) -> std::io::Result<usize> {
    let mut by_file: std::collections::HashMap<&Path, Vec<usize>> = std::collections::HashMap::new();
    for (path, line) in targets {
        by_file.entry(path.as_path()).or_default().push(*line);
    }

    let mut total_removed = 0;
    for (path, target_lines) in by_file {
        let Ok(contents) = std::fs::read_to_string(path) else {
            continue;
        };
        let original: Vec<&str> = contents.lines().collect();
        let mut kept = Vec::with_capacity(original.len());
        let mut removed_here = 0;
        for (i, text) in original.iter().enumerate() {
            let line_no = i + 1;
            let trimmed = text.trim();
            let matches = target_lines.contains(&line_no)
                && trimmed.starts_with("hl.")
                && trimmed.ends_with(')');
            if matches {
                removed_here += 1;
            } else {
                kept.push(*text);
            }
        }
        if removed_here == 0 {
            continue;
        }
        // The backup goes through `write_atomic`, like the one in
        // `hyprlang::install`, and not `fs::copy`. This is a
        // read-modify-write that *destroys* lines: the overwrite below is
        // atomic and fsynced, so on a power loss it is the write that
        // survives. A backup made with a plain copy is not fsynced, so it
        // is the half that does not — leaving the removed `hl.*` lines in
        // neither file. That is the shape of the failure that once cost a
        // user 37 hand-written binds, and this is the path that removes
        // imported lines from their config.
        let backup = path.with_extension("lua.hyprforge-import.bak");
        crate::paths::write_atomic(&backup, &contents)?;

        let mut out = kept.join("\n");
        if contents.ends_with('\n') {
            out.push('\n');
        }
        // Only after the backup is durably on disk.
        crate::paths::write_atomic(path, &out)?;
        total_removed += removed_here;
    }
    Ok(total_removed)
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
        let out = apply(cfg, &plan, &line, Placement::BeforeUserRequires);
        // A fresh block: opening marker, the line itself, closing marker —
        // all before the user's own first require.
        let out_lines: Vec<&str> = out.lines().collect();
        assert_eq!(out_lines[1], marker_start(Placement::BeforeUserRequires));
        assert!(out_lines[2].contains("hyprforge/keybinds"));
        assert_eq!(out_lines[3], MARKER_END);
        assert_eq!(out_lines[4], "require(\"mine\")");
    }

    #[test]
    fn placement_at_end_goes_last() {
        let cfg = "require(\"mine\")\n";
        let line = require_line("keybinds");
        let plan = detect(cfg, &line, Placement::AtEnd);
        let out = apply(cfg, &plan, &line, Placement::AtEnd);
        let out_lines: Vec<&str> = out.lines().collect();
        assert_eq!(out_lines[1], marker_start(Placement::AtEnd));
        assert!(out_lines[2].contains("hyprforge/keybinds"));
        assert_eq!(out_lines[3], MARKER_END);
    }

    /// With no requires at all, both placements mean "the end" — there's
    /// nothing to go before.
    #[test]
    fn a_config_with_no_requires_still_honours_its_placement() {
        let cfg = "hl.config({})\n";
        let line = require_line("keybinds");

        // `AtEnd` means what it says.
        let at_end = apply(cfg, &detect(cfg, &line, Placement::AtEnd), &line, Placement::AtEnd);
        assert!(at_end.contains("hyprforge/keybinds"));
        assert_eq!(at_end.lines().last().unwrap(), MARKER_END);

        // `BeforeUserRequires` must *not* also append. Hyprland applies the
        // last matching rule, so appending would make Hyprforge override the
        // user's own config — the exact opposite of what this placement
        // exists to guarantee. A config with no `require()` of its own is
        // the common case, not an edge case.
        let before = apply(
            cfg,
            &detect(cfg, &line, Placement::BeforeUserRequires),
            &line,
            Placement::BeforeUserRequires,
        );
        assert!(before.contains("hyprforge/keybinds"));
        assert_eq!(
            before.lines().next().unwrap(),
            marker_start(Placement::BeforeUserRequires),
            "the block belongs above the user's content, not below it: {before}"
        );
        assert_eq!(before.lines().last().unwrap(), "hl.config({})");
    }

    /// ...but a leading comment block is a header, and the require line
    /// shouldn't shove itself above it.
    #[test]
    fn a_leading_comment_block_keeps_its_place_at_the_top() {
        let cfg = "-- my hyprland config\n-- second line\n\nhl.config({})\n";
        let line = require_line("monitors");
        let out = apply(
            cfg,
            &detect(cfg, &line, Placement::BeforeUserRequires),
            &line,
            Placement::BeforeUserRequires,
        );
        let lines: Vec<&str> = out.lines().collect();
        assert_eq!(lines[0], "-- my hyprland config");
        assert_eq!(lines[1], "-- second line");
        assert!(
            out.find("hyprforge/monitors").unwrap() < out.find("hl.config").unwrap(),
            "still has to land before the user's own content: {out}"
        );
    }

    /// A second module sharing a placement must join the first module's
    /// block, not create a new one above/below it based on whichever
    /// require happened to be textually first.
    #[test]
    fn a_second_module_at_the_same_placement_joins_the_first_ones_block() {
        let cfg = "require(\"mine\")\n";
        let first = require_line("window-rules");
        let plan1 = detect(cfg, &first, Placement::BeforeUserRequires);
        let out = apply(cfg, &plan1, &first, Placement::BeforeUserRequires);

        let second = require_line("monitors");
        let plan2 = detect(&out, &second, Placement::BeforeUserRequires);
        let out = apply(&out, &plan2, &second, Placement::BeforeUserRequires);

        let out_lines: Vec<&str> = out.lines().collect();
        assert_eq!(out_lines[0], marker_start(Placement::BeforeUserRequires));
        assert!(out_lines[1].contains("hyprforge/window-rules"));
        assert!(out_lines[2].contains("hyprforge/monitors"));
        assert_eq!(out_lines[3], MARKER_END);
        assert_eq!(out_lines[4], "require(\"mine\")");
        // Only one block, not two.
        assert_eq!(
            out.lines().filter(|l| *l == marker_start(Placement::BeforeUserRequires)).count(),
            1
        );
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
        assert!(create_lua_config(&path, &require_line("keybinds"), Placement::AtEnd).is_err());
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "theirs\n");
    }

    /// A freshly created config already uses the marker-block format, so it
    /// looks the same whether it was created empty or grown from an
    /// existing file.
    #[test]
    fn a_created_config_wraps_its_line_in_a_marker_block() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("nested").join("hyprland.lua");
        create_lua_config(&path, &require_line("window-rules"), Placement::BeforeUserRequires).unwrap();
        let out = std::fs::read_to_string(&path).unwrap();
        assert!(out.contains(marker_start(Placement::BeforeUserRequires)));
        assert!(out.contains("hyprforge/window-rules"));
        assert!(out.contains(MARKER_END));
    }

    /// A second module, installed later into a config `create_lua_config`
    /// made, still joins its own block correctly rather than treating the
    /// freshly-created file's marker block as unrecognised text.
    #[test]
    fn a_module_installed_after_creation_joins_the_created_block() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("hyprland.lua");
        create_lua_config(&path, &require_line("window-rules"), Placement::BeforeUserRequires).unwrap();

        install(&path, &require_line("monitors"), Placement::BeforeUserRequires).unwrap();
        install(&path, &require_line("keybinds"), Placement::AtEnd).unwrap();

        let out = std::fs::read_to_string(&path).unwrap();
        assert_eq!(
            out.matches(marker_start(Placement::BeforeUserRequires)).count(),
            1,
            "window-rules and monitors should share one block"
        );
        assert!(out.contains("hyprforge/monitors"));
        assert!(out.contains("hyprforge/keybinds"));
    }

    #[test]
    fn remove_matched_lines_deletes_an_exact_single_line_call() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("binds.lua");
        std::fs::write(
            &path,
            "local mainMod = \"SUPER\"\nhl.bind(mainMod .. \" + Q\", hl.dsp.window.close())\n",
        )
        .unwrap();

        let removed = remove_matched_lines(&[(path.clone(), 2)]).unwrap();
        assert_eq!(removed, 1);
        let out = std::fs::read_to_string(&path).unwrap();
        assert_eq!(out, "local mainMod = \"SUPER\"\n");
    }

    #[test]
    fn remove_matched_lines_skips_a_line_that_no_longer_matches() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("hyprland.lua");
        // The user edited this line since the import ran — it's no longer
        // a bare `hl.*(...)` call, so it must survive.
        std::fs::write(&path, "-- hl.bind(\"SUPER + Q\", hl.dsp.window.close()) disabled\n").unwrap();

        let removed = remove_matched_lines(&[(path.clone(), 1)]).unwrap();
        assert_eq!(removed, 0);
        assert!(std::fs::read_to_string(&path).unwrap().contains("disabled"));
    }

    /// A multi-line call's recorded line is only its start — which never
    /// ends with `)` on its own — so it's left alone rather than having
    /// just its opening line deleted and the rest orphaned.
    #[test]
    fn remove_matched_lines_never_touches_a_multi_line_calls_start_line() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("hyprland.lua");
        std::fs::write(&path, "hl.window_rule({\n  class = \"^discord$\",\n})\n").unwrap();

        let removed = remove_matched_lines(&[(path.clone(), 1)]).unwrap();
        assert_eq!(removed, 0);
        assert!(std::fs::read_to_string(&path).unwrap().contains("hl.window_rule({"));
    }

    #[test]
    fn remove_matched_lines_writes_a_fresh_backup() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("hyprland.lua");
        let original = "hl.bind(\"SUPER + Q\", hl.dsp.window.close())\n";
        std::fs::write(&path, original).unwrap();

        remove_matched_lines(&[(path.clone(), 1)]).unwrap();

        let backup = path.with_extension("lua.hyprforge-import.bak");
        assert_eq!(std::fs::read_to_string(&backup).unwrap(), original);
    }

    #[test]
    fn remove_matched_lines_handles_multiple_files_independently() {
        let dir = tempfile::tempdir().unwrap();
        let a = dir.path().join("a.lua");
        let b = dir.path().join("b.lua");
        std::fs::write(&a, "hl.bind(\"SUPER + Q\", hl.dsp.window.close())\n").unwrap();
        std::fs::write(&b, "hl.bind(\"SUPER + C\", hl.dsp.window.close())\nrequire(\"x\")\n").unwrap();

        let removed = remove_matched_lines(&[(a.clone(), 1), (b.clone(), 1)]).unwrap();
        assert_eq!(removed, 2);
        assert_eq!(std::fs::read_to_string(&a).unwrap().trim(), "");
        assert_eq!(std::fs::read_to_string(&b).unwrap(), "require(\"x\")\n");
    }

    #[test]
    fn remove_matched_lines_is_a_noop_for_an_unreadable_file() {
        let removed = remove_matched_lines(&[(PathBuf::from("/nonexistent/path.lua"), 1)]).unwrap();
        assert_eq!(removed, 0);
    }
}

/// Everything a module needs to own a `require()`d Lua file.
///
/// The three Settings modules each ran the same four-step opening move —
/// discover the config, create or install, detect the resulting plan, write
/// an empty generated file if it doesn't exist yet — in about forty
/// near-identical lines apiece, comments included. Only three values ever
/// differed, and they're the three fields here.
pub struct ModuleSetup<'a> {
    /// The `require("…")` line this module owns.
    pub require_line: &'a str,
    pub placement: Placement,
    /// The generated file the require line points at, and the contents to
    /// give it if it doesn't exist yet.
    ///
    /// Writing it eagerly is not optional: the require line can be installed
    /// before the module has ever saved anything, and `require()` of a
    /// missing file is an error on the user's very next `hyprctl reload`.
    pub generated: (PathBuf, String),
}

/// What [`bootstrap`] worked out about the user's config.
pub struct SetupState {
    pub config: HyprConfig,
    pub plan: SetupPlan,
    /// Why automatic setup failed, if it did. Never fatal — a module stays
    /// usable and saves to its own TOML regardless (vision pillar #3).
    pub error: Option<String>,
}

/// Runs a module's opening move against the user's Hyprland config.
///
/// No confirmation is asked for anywhere in here: the require line only ever
/// points at Hyprforge's own generated file and never touches the user's own
/// content, so there is nothing to ask permission for beyond what installing
/// the module already implies. A real write failure still surfaces, in
/// [`SetupState::error`].
pub fn bootstrap(hypr_dir: &Path, hyprland_lua: &Path, setup: ModuleSetup<'_>) -> SetupState {
    let mut config = discover(hypr_dir);
    let mut error = None;
    match &config {
        HyprConfig::Missing => {
            match create_lua_config(hyprland_lua, setup.require_line, setup.placement) {
                Ok(()) => config = HyprConfig::Lua(hyprland_lua.to_path_buf()),
                Err(e) => error = Some(e.to_string()),
            }
        }
        HyprConfig::Lua(_) => {
            if let Err(e) = install(hyprland_lua, setup.require_line, setup.placement) {
                error = Some(e.to_string());
            }
        }
        // Nothing to automate: require() doesn't exist in that format.
        HyprConfig::ConfOnly(_) => {}
    }

    let plan = match &config {
        HyprConfig::Lua(path) => {
            let contents = std::fs::read_to_string(path).unwrap_or_default();
            detect(&contents, setup.require_line, setup.placement)
        }
        _ => SetupPlan::NeedsInsert { insert_before_line: 1 },
    };

    if matches!(config, HyprConfig::Lua(_)) {
        let (path, contents) = &setup.generated;
        if !path.exists() {
            // Reported, not `let _ =`. The field's own doc says writing
            // this eagerly is "not optional": the require line can be
            // installed before the module has ever saved anything, and
            // `require()` of a missing file is an error on the user's
            // very next `hyprctl reload`. Discarding the failure
            // produced exactly that — a require line pointing at
            // nothing, an error on a line the user never wrote, and no
            // message anywhere. `SetupState::error` is rendered in the
            // app, so the channel was there all along.
            if let Err(e) = crate::paths::write_atomic(path, contents) {
                // An earlier failure came first and explains more; this
                // one is its consequence, not a separate problem.
                error.get_or_insert_with(|| {
                    format!("couldn't write {}: {e}", path.display())
                });
            }
        }
    }

    SetupState { config, plan, error }
}

#[cfg(test)]
mod bootstrap_tests {
    use super::*;

    fn setup(dir: &Path) -> ModuleSetup<'static> {
        ModuleSetup {
            require_line: "require(\"hyprforge/keybinds\")",
            placement: Placement::AtEnd,
            generated: (dir.join("keybinds.lua"), "-- empty\n".to_string()),
        }
    }

    /// The gap that bit a real user: the require line exists before the
    /// module has ever saved, and `require()` of a missing file fails the
    /// next reload.
    #[test]
    fn the_generated_file_exists_before_anything_is_saved() {
        let dir = tempfile::tempdir().unwrap();
        let lua = dir.path().join("hyprland.lua");
        std::fs::write(&lua, "hl.config({})\n").unwrap();

        let state = bootstrap(dir.path(), &lua, setup(dir.path()));
        assert!(state.error.is_none(), "{:?}", state.error);
        assert!(matches!(state.config, HyprConfig::Lua(_)));
        assert_eq!(
            std::fs::read_to_string(dir.path().join("keybinds.lua")).unwrap(),
            "-- empty\n"
        );
        assert!(std::fs::read_to_string(&lua).unwrap().contains("hyprforge/keybinds"));
    }

    /// A second run must not clobber real content with the empty version.
    #[test]
    fn an_existing_generated_file_is_left_alone() {
        let dir = tempfile::tempdir().unwrap();
        let lua = dir.path().join("hyprland.lua");
        std::fs::write(&lua, "hl.config({})\n").unwrap();
        std::fs::write(dir.path().join("keybinds.lua"), "-- real content\n").unwrap();

        bootstrap(dir.path(), &lua, setup(dir.path()));
        assert_eq!(
            std::fs::read_to_string(dir.path().join("keybinds.lua")).unwrap(),
            "-- real content\n"
        );
    }

    /// A `.conf`-only user can't be served by a require line, and that has
    /// to be reported rather than attempted.
    #[test]
    fn a_conf_only_config_is_left_untouched() {
        let dir = tempfile::tempdir().unwrap();
        let conf = dir.path().join("hyprland.conf");
        std::fs::write(&conf, "monitor=,preferred,auto,1\n").unwrap();

        let state = bootstrap(dir.path(), &dir.path().join("hyprland.lua"), setup(dir.path()));
        assert!(matches!(state.config, HyprConfig::ConfOnly(_)));
        assert_eq!(std::fs::read_to_string(&conf).unwrap(), "monitor=,preferred,auto,1\n");
        assert!(!dir.path().join("hyprland.lua").exists());
    }
}
