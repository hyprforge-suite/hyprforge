//! Owning a slice of a hyprlang config file, the way [`crate::lua_setup`]
//! owns a slice of `hyprland.lua`.
//!
//! Hyprland itself moved to Lua in 0.55, but the ecosystem daemons —
//! hyprpaper, hyprsunset, hypridle, hyprlock — still read hyprlang. They
//! have the same escape hatch Lua does: `source = <path>` pulls in another
//! file, and the wiki is explicit that parsing is **linear** ("lines above
//! the `source =` will be parsed first, then lines inside … then lines
//! below"). So the same arrangement works — Hyprforge owns one generated
//! file, the user's own config keeps its comments and hand edits, and one
//! line joins them.
//!
//! The `source` line always goes **last**. These daemons resolve a
//! repeated setting by last-one-wins, so appending is what makes a value
//! saved in the app actually take effect; inserting it first would let the
//! user's own config silently override every save, which is the failure
//! the Lua modules already ran into.
//!
//! Applying is per-daemon and deliberately not modelled here. hyprpaper
//! and hyprsunset take live IPC through `hyprctl`; hypridle has none at
//! all (`hyprctl hypridle` → "unknown request") and has to be restarted.
//! A shared "reload" that pretended those were the same would be wrong for
//! two of the three.

use crate::lua_setup::SetupError;
use std::path::Path;

/// Opening marker of a Hyprforge-managed block. Comments are `#` here,
/// not `--`.
const MARKER_START: &str = "# Hyprforge-managed — do not edit by hand.";
const MARKER_END: &str = "# end Hyprforge-managed";

/// Whether the `source` line is already installed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SourcePlan {
    AlreadyPresent,
    NeedsAppend,
}

/// The exact line that sources `generated`.
///
/// An absolute path rather than `~`: hyprlang expands tildes, but the
/// path is built from `$XDG_CONFIG_HOME`, which needn't be under `$HOME`
/// at all — and a wrong `~` expansion fails silently as "no wallpaper".
pub fn source_line(generated: &Path) -> String {
    format!("source = {}", generated.display())
}

/// Detects whether `contents` already sources the generated file.
///
/// Matches the line anywhere, not just inside a marker block, so a line a
/// user moved or wrote themselves is recognised rather than duplicated.
pub fn detect(contents: &str, source_line: &str) -> SourcePlan {
    let wanted = source_line.trim();
    let present = contents.lines().any(|l| {
        let l = l.trim();
        // `source=x` and `source = x` are the same line to hyprlang.
        !l.starts_with('#') && normalise(l) == normalise(wanted)
    });
    if present {
        SourcePlan::AlreadyPresent
    } else {
        SourcePlan::NeedsAppend
    }
}

fn normalise(line: &str) -> String {
    line.chars().filter(|c| !c.is_whitespace()).collect()
}

/// Returns `contents` with the source line appended in a marked block, or
/// unchanged if it's already there.
pub fn apply(contents: &str, plan: &SourcePlan, source_line: &str) -> String {
    if *plan == SourcePlan::AlreadyPresent {
        return contents.to_string();
    }
    let mut out = contents.to_string();
    if !out.is_empty() && !out.ends_with('\n') {
        out.push('\n');
    }
    if !out.is_empty() {
        out.push('\n');
    }
    out.push_str(MARKER_START);
    out.push('\n');
    out.push_str(source_line);
    out.push('\n');
    out.push_str(MARKER_END);
    out.push('\n');
    out
}

/// Backs up and appends the source line, if it isn't already there.
///
/// A missing target is created rather than treated as an error: a user
/// who has never configured hyprpaper has no `hyprpaper.conf`, and
/// refusing to help them is worse than writing the one line that makes
/// the daemon read ours.
pub fn install(target: &Path, source_line: &str) -> Result<SourcePlan, SetupError> {
    let contents = match std::fs::read_to_string(target) {
        Ok(contents) => contents,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => String::new(),
        Err(source) => {
            return Err(SetupError::Read {
                path: target.display().to_string(),
                source,
            })
        }
    };
    let plan = detect(&contents, source_line);
    if plan == SourcePlan::AlreadyPresent {
        return Ok(plan);
    }
    // Only back up a file that exists and that we're about to change.
    if !contents.is_empty() {
        let backup = target.with_extension(format!(
            "{}.hyprforge.bak",
            target.extension().and_then(|e| e.to_str()).unwrap_or("conf")
        ));
        if !backup.exists() {
            crate::paths::write_atomic(&backup, &contents).map_err(|source| SetupError::Write {
                path: backup.display().to_string(),
                source,
            })?;
        }
    }
    crate::paths::write_atomic(target, &apply(&contents, &plan, source_line)).map_err(
        |source| SetupError::Write {
            path: target.display().to_string(),
            source,
        },
    )?;
    Ok(plan)
}

/// Renders `name = value`.
pub fn keyword(name: &str, value: impl std::fmt::Display) -> String {
    format!("{name} = {value}\n")
}

/// Renders a `name { … }` block from already-rendered `key = value`
/// pairs, skipping the block entirely when it has no fields — an empty
/// block is not the same as no block to every one of these daemons.
pub fn block(name: &str, fields: &[(&str, String)]) -> String {
    if fields.is_empty() {
        return String::new();
    }
    let mut out = format!("{name} {{\n");
    for (key, value) in fields {
        out.push_str(&format!("    {key} = {value}\n"));
    }
    out.push_str("}\n");
    out
}

/// The "don't edit this" banner, naming the screen that owns the file.
pub fn header(subject: &str) -> String {
    format!(
        "# Generated by Hyprforge. Do not edit by hand — changes will be\n\
         # overwritten the next time {subject} are saved in the Hyprforge\n\
         # Settings app.\n"
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_source_line_is_appended_in_a_marked_block() {
        let out = apply("preload = /a.png\n", &SourcePlan::NeedsAppend, "source = /gen.conf");
        assert!(out.starts_with("preload = /a.png\n"), "{out}");
        assert!(out.contains(MARKER_START), "{out}");
        assert!(out.trim_end().ends_with(MARKER_END), "{out}");
        assert!(
            out.find("source = /gen.conf") > out.find("preload"),
            "the source line must come last: {out}"
        );
    }

    /// Last-one-wins is why the line goes last. Inserting it first would
    /// let the user's own config silently override every save.
    #[test]
    fn an_existing_config_keeps_its_content_and_comments() {
        let original = "# my notes\npreload = /a.png\n\nwallpaper {\n    monitor =\n}\n";
        let out = apply(original, &SourcePlan::NeedsAppend, "source = /gen.conf");
        assert!(out.starts_with(original), "{out}");
    }

    #[test]
    fn an_already_present_line_is_not_added_twice() {
        let cfg = "source = /gen.conf\n";
        assert_eq!(detect(cfg, "source = /gen.conf"), SourcePlan::AlreadyPresent);
        assert_eq!(apply(cfg, &SourcePlan::AlreadyPresent, "source = /gen.conf"), cfg);
    }

    /// `source=x` and `source = x` are the same line to hyprlang, so a
    /// user who wrote it without spaces must not get a duplicate.
    #[test]
    fn spacing_does_not_hide_an_existing_line() {
        assert_eq!(
            detect("source=/gen.conf\n", "source = /gen.conf"),
            SourcePlan::AlreadyPresent
        );
    }

    /// A commented-out line is not installed — treating it as present
    /// would leave the module silently doing nothing.
    #[test]
    fn a_commented_out_line_does_not_count_as_installed() {
        assert_eq!(
            detect("# source = /gen.conf\n", "source = /gen.conf"),
            SourcePlan::NeedsAppend
        );
    }

    #[test]
    fn an_empty_config_gets_the_block_without_leading_blank_lines() {
        let out = apply("", &SourcePlan::NeedsAppend, "source = /gen.conf");
        assert!(out.starts_with(MARKER_START), "{out}");
    }

    #[test]
    fn a_file_without_a_trailing_newline_still_appends_cleanly() {
        let out = apply("preload = /a.png", &SourcePlan::NeedsAppend, "source = /gen.conf");
        assert!(out.contains("preload = /a.png\n"), "{out}");
        assert!(out.contains("\nsource = /gen.conf\n"), "{out}");
    }

    #[test]
    fn installing_creates_a_missing_target_and_backs_up_an_existing_one() {
        let dir = tempfile::tempdir().unwrap();

        let fresh = dir.path().join("hyprpaper.conf");
        assert_eq!(install(&fresh, "source = /gen.conf").unwrap(), SourcePlan::NeedsAppend);
        assert!(std::fs::read_to_string(&fresh).unwrap().contains("source = /gen.conf"));

        let existing = dir.path().join("other.conf");
        std::fs::write(&existing, "preload = /a.png\n").unwrap();
        install(&existing, "source = /gen.conf").unwrap();
        let backup = existing.with_extension("conf.hyprforge.bak");
        assert_eq!(std::fs::read_to_string(&backup).unwrap(), "preload = /a.png\n");
    }

    /// Installing twice must not append twice, nor overwrite the backup
    /// with the already-modified file — that would destroy the only copy
    /// of the user's original.
    #[test]
    fn installing_twice_is_a_no_op_and_preserves_the_first_backup() {
        let dir = tempfile::tempdir().unwrap();
        let target = dir.path().join("hyprpaper.conf");
        std::fs::write(&target, "preload = /a.png\n").unwrap();

        install(&target, "source = /gen.conf").unwrap();
        let after_first = std::fs::read_to_string(&target).unwrap();
        assert_eq!(install(&target, "source = /gen.conf").unwrap(), SourcePlan::AlreadyPresent);
        assert_eq!(std::fs::read_to_string(&target).unwrap(), after_first);

        let backup = target.with_extension("conf.hyprforge.bak");
        assert_eq!(
            std::fs::read_to_string(&backup).unwrap(),
            "preload = /a.png\n",
            "the backup must still be the user's original"
        );
    }

    #[test]
    fn a_block_renders_its_fields_indented() {
        let out = block(
            "wallpaper",
            &[("monitor", "eDP-2".into()), ("path", "/a.png".into())],
        );
        assert_eq!(out, "wallpaper {\n    monitor = eDP-2\n    path = /a.png\n}\n");
    }

    /// An empty block is not the same as no block to these daemons — an
    /// empty `wallpaper {}` is a wallpaper with no image.
    #[test]
    fn an_empty_block_renders_nothing() {
        assert_eq!(block("wallpaper", &[]), "");
    }

    #[test]
    fn a_keyword_renders_on_its_own_line() {
        assert_eq!(keyword("temperature", 6500), "temperature = 6500\n");
    }
}
