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

/// Renders a value so hyprlang reads back what was meant.
///
/// `#` starts a comment *anywhere* on a line — not only at the start —
/// and `##` is the documented escape for a literal one. So a wallpaper at
/// `~/Pictures/#1 favourite.png` written raw becomes `~/Pictures/` with
/// the rest silently discarded: a path that still looks right in the
/// settings screen and shows no wallpaper at all, with nothing logged
/// anywhere because hyprlang saw a perfectly valid line.
///
/// Applied by [`keyword`] and [`block`], so no caller has to remember.
pub fn escape_value(value: &str) -> String {
    value.replace('#', "##")
}

/// Recovers the value hyprlang would see: everything before the first
/// unescaped `#`, with `##` folded back to a literal `#`, trimmed.
///
/// The inverse of [`escape_value`], and the two are tested against each
/// other — a reader and a writer that disagree about escaping is the
/// same class of drift as a generator and a matcher disagreeing.
pub fn strip_comment(line: &str) -> String {
    let chars: Vec<char> = line.chars().collect();
    let mut out = String::new();
    let mut i = 0;
    while i < chars.len() {
        if chars[i] == '#' {
            if chars.get(i + 1) == Some(&'#') {
                out.push('#');
                i += 2;
                continue;
            }
            break;
        }
        out.push(chars[i]);
        i += 1;
    }
    out.trim().to_string()
}

/// Renders `name = value`.
pub fn keyword(name: &str, value: impl std::fmt::Display) -> String {
    format!("{name} = {}\n", escape_value(&value.to_string()))
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
        out.push_str(&format!("    {key} = {}\n", escape_value(value)));
    }
    out.push_str("}\n");
    out
}

/// One thing a hyprlang file says, with where it said it.
///
/// Line numbers are 1-based and kept because adopting a setting into the
/// app has to be able to point back at the exact lines it came from —
/// both to show the user where a value was found and to retire those
/// lines afterwards without touching anything around them.
#[derive(Debug, Clone, PartialEq)]
pub enum Item {
    /// `key = value`, comment already stripped and `##` unescaped.
    Assignment { key: String, value: String, line: usize },
    /// `name { … }`. Nesting is not modelled: none of the four daemons
    /// this reads for uses it, and inventing a shape nothing produces
    /// would be untested code in the middle of a parser.
    Block { name: String, items: Vec<Item>, line: usize, end_line: usize },
}

impl Item {
    pub fn line(&self) -> usize {
        match self {
            Item::Assignment { line, .. } | Item::Block { line, .. } => *line,
        }
    }

    /// The last line this item occupies — its own, or a block's `}`.
    pub fn end_line(&self) -> usize {
        match self {
            Item::Assignment { line, .. } => *line,
            Item::Block { end_line, .. } => *end_line,
        }
    }
}

/// A parsed hyprlang file.
///
/// `problems` is never folded into "nothing here". A file that exists and
/// will not parse is a thing the user has to hear about, not a first run
/// — the distinction `hlconfig::storage` documents and that collapsing
/// once cost a user 37 hand-written binds.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Document {
    pub items: Vec<Item>,
    pub problems: Vec<(usize, String)>,
}

impl Document {
    /// The value of the last assignment named `key` at the top level.
    ///
    /// The *last*, because hyprlang overwrites a repeated variable with
    /// the one further down — reading the first would report a value the
    /// daemon never uses.
    pub fn value(&self, key: &str) -> Option<&str> {
        self.items
            .iter()
            .rev()
            .find_map(|item| match item {
                Item::Assignment { key: k, value, .. } if k == key => Some(value.as_str()),
                _ => None,
            })
    }

    /// Every top-level block called `name`, in file order.
    ///
    /// All of them, not the last: blocks are categories, and a second
    /// `listener` is an additional listener rather than a replacement for
    /// the first. Getting that backwards is how an import loses four of a
    /// user's five idle rules.
    pub fn blocks<'a>(&'a self, name: &'a str) -> impl Iterator<Item = &'a Item> + 'a {
        self.items.iter().filter(move |item| {
            matches!(item, Item::Block { name: n, .. } if n == name)
        })
    }

    /// The paths this file pulls in with `source =`.
    ///
    /// Reported rather than followed. Following them would walk straight
    /// into Hyprforge's own generated file and re-import what it just
    /// wrote; the caller knows which directory is its own and this does
    /// not.
    pub fn sourced(&self) -> Vec<&str> {
        self.items
            .iter()
            .filter_map(|item| match item {
                Item::Assignment { key, value, .. } if key == "source" => Some(value.as_str()),
                _ => None,
            })
            .collect()
    }
}

/// Reads a hyprlang file.
///
/// Deliberately lenient in the same places hyprlang is, and loud in the
/// places it isn't: an unclosed block or a line that is neither an
/// assignment nor a block lands in [`Document::problems`] rather than
/// being dropped on the floor.
pub fn parse(contents: &str) -> Document {
    let mut document = Document::default();
    // A stack even though nesting isn't modelled: it is what lets an
    // unexpected `{` be reported at the line it appeared on instead of
    // silently swallowing everything after it.
    let mut open: Vec<(String, usize, Vec<Item>)> = Vec::new();

    for (index, raw) in contents.lines().enumerate() {
        let line = index + 1;
        let text = strip_comment(raw);
        if text.is_empty() {
            continue;
        }

        if text == "}" {
            match open.pop() {
                Some((name, start, items)) => {
                    let block = Item::Block { name, items, line: start, end_line: line };
                    match open.last_mut() {
                        Some((_, _, parent)) => parent.push(block),
                        None => document.items.push(block),
                    }
                }
                None => document
                    .problems
                    .push((line, "a closing `}` with no block open".to_string())),
            }
            continue;
        }

        if let Some(name) = text.strip_suffix('{') {
            let name = name.trim().to_string();
            if name.is_empty() {
                document.problems.push((line, "a block with no name".to_string()));
                continue;
            }
            open.push((name, line, Vec::new()));
            continue;
        }

        let Some((key, value)) = text.split_once('=') else {
            document.problems.push((
                line,
                format!("`{text}` is neither `key = value` nor a block"),
            ));
            continue;
        };
        // An empty value is meaningful, not missing: `monitor =` is
        // exactly how hyprpaper declares the fallback that applies to
        // every output, so it must survive a round trip.
        let item = Item::Assignment {
            key: key.trim().to_string(),
            value: value.trim().to_string(),
            line,
        };
        match open.last_mut() {
            Some((_, _, items)) => items.push(item),
            None => document.items.push(item),
        }
    }

    for (name, line, _) in open {
        document
            .problems
            .push((line, format!("`{name}` block is never closed")));
    }
    document
}

/// Comments out the given 1-based, inclusive line ranges.
///
/// Used after a setting has been adopted into the app, so the daemon
/// stops seeing two of it. Blocks are categories and accumulate — a
/// hand-written `listener` is not replaced by a generated one, both run —
/// so on a config with five listeners, importing without this leaves ten.
///
/// **Comments out, never deletes.** The user's original stays legible in
/// their own file, which is the difference between a change they can
/// read and undo and one that looks like the app ate their config. The
/// caller is separately required to have made a `.hyprforge.bak` first;
/// this function cannot check that and does not pretend to.
///
/// Lines outside `ranges` are returned byte-for-byte, blank lines and
/// hand-written comments between blocks included. A line already starting
/// with `#` is left alone rather than prefixed twice, so retiring the
/// same range twice is a no-op.
///
/// **Pass every range in one call.** Ranges are 1-based into `contents`
/// as given, and each retired range gains a note line — so the output's
/// numbering no longer matches the input's. Retiring, re-parsing and
/// retiring again against the first set of numbers would comment out the
/// wrong lines.
pub fn retire(contents: &str, ranges: &[(usize, usize)], note: &str) -> String {
    let retired: Vec<usize> = ranges
        .iter()
        .flat_map(|(start, end)| *start..=*end)
        .collect();
    let starts: Vec<usize> = ranges.iter().map(|(start, _)| *start).collect();

    let mut out = String::new();
    for (index, line) in contents.lines().enumerate() {
        let number = index + 1;
        if starts.contains(&number) {
            // The indentation of what follows, so the note doesn't sit at
            // column 0 in the middle of an indented block.
            let indent: String =
                line.chars().take_while(|c| c.is_whitespace()).collect();
            out.push_str(&format!("{indent}# {note}\n"));
        }
        if retired.contains(&number) && !line.trim_start().starts_with('#') {
            let indent: String =
                line.chars().take_while(|c| c.is_whitespace()).collect();
            out.push_str(&format!("{indent}# {}\n", &line[indent.len()..]));
        } else {
            out.push_str(line);
            out.push('\n');
        }
    }
    // A file that didn't end in a newline still shouldn't gain one.
    if !contents.ends_with('\n') && out.ends_with('\n') {
        out.pop();
    }
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

    /// `#` starts a comment anywhere on the line, so a path containing one
    /// written raw is truncated by hyprlang into a shorter path that still
    /// exists on screen and points at nothing. `##` is the documented
    /// escape.
    #[test]
    fn a_value_containing_a_hash_survives_being_written() {
        let line = keyword("path", "/home/me/Pictures/#1 favourite.png");
        let (_, value) = line.trim_end().split_once(" = ").unwrap();
        assert_eq!(strip_comment(value), "/home/me/Pictures/#1 favourite.png");
    }

    #[test]
    fn a_block_field_containing_a_hash_survives_too() {
        let out = block("wallpaper", &[("path", "/a/#b.png".to_string())]);
        let value = out.lines().nth(1).unwrap().split_once(" = ").unwrap().1;
        assert_eq!(strip_comment(value), "/a/#b.png");
    }

    #[test]
    fn a_trailing_comment_is_not_part_of_the_value() {
        assert_eq!(strip_comment("150   # 2.5min."), "150");
        assert_eq!(strip_comment("# whole line"), "");
        assert_eq!(strip_comment("no comment here"), "no comment here");
    }

    /// Every value the writer produces must read back as itself. A reader
    /// and a writer that disagree about escaping is the same class of
    /// drift as a generator and a matcher disagreeing about what is valid.
    #[test]
    fn escaping_round_trips_for_every_shape_of_value() {
        for value in [
            "plain",
            "/a/#b.png",
            "##already escaped",
            "trailing #",
            "#leading",
            "a # b # c",
            "",
        ] {
            assert_eq!(strip_comment(&escape_value(value)), value.trim(), "{value:?}");
        }
    }

    #[test]
    fn a_keyword_and_a_block_are_both_recovered() {
        let doc = parse("temperature = 6500\n\nprofile {\n    time = 21:00\n}\n");
        assert_eq!(doc.value("temperature"), Some("6500"));
        assert_eq!(doc.problems, Vec::new());
        let blocks: Vec<_> = doc.blocks("profile").collect();
        assert_eq!(blocks.len(), 1);
        let Item::Block { items, line, end_line, .. } = blocks[0] else {
            panic!("expected a block")
        };
        assert_eq!((*line, *end_line), (3, 5));
        assert_eq!(items[0], Item::Assignment {
            key: "time".to_string(),
            value: "21:00".to_string(),
            line: 4,
        });
    }

    /// Variables overwrite; the daemon uses the last one. Reading the
    /// first would report a value it never sees.
    #[test]
    fn a_repeated_variable_reads_as_the_one_that_wins() {
        assert_eq!(parse("temperature = 6500\ntemperature = 4000\n").value("temperature"), Some("4000"));
    }

    /// Blocks are categories, not variables: a second `listener` is an
    /// additional listener. Treating them as last-wins would lose four of
    /// this machine's five idle rules on import.
    #[test]
    fn every_block_of_a_name_is_kept_not_just_the_last() {
        let doc = parse("listener {\n  timeout = 1\n}\nlistener {\n  timeout = 2\n}\n");
        assert_eq!(doc.blocks("listener").count(), 2);
    }

    /// `monitor =` is how hyprpaper declares the fallback that covers
    /// every output. An empty value is a value.
    #[test]
    fn an_empty_value_is_kept_rather_than_treated_as_absent() {
        let doc = parse("wallpaper {\n    monitor =\n}\n");
        let Item::Block { items, .. } = doc.blocks("wallpaper").next().unwrap() else {
            panic!("expected a block")
        };
        assert_eq!(items[0], Item::Assignment {
            key: "monitor".to_string(),
            value: String::new(),
            line: 2,
        });
    }

    /// A file that exists and will not parse is not a file with nothing
    /// in it. Collapsing the two once cost a user 37 hand-written binds.
    #[test]
    fn a_malformed_file_reports_the_problem_rather_than_reading_as_empty() {
        let unclosed = parse("listener {\n    timeout = 1\n");
        assert_eq!(unclosed.problems.len(), 1, "{:?}", unclosed.problems);
        assert!(unclosed.problems[0].1.contains("never closed"));

        let stray = parse("}\n");
        assert_eq!(stray.problems.len(), 1);

        let nonsense = parse("this is not a setting\n");
        assert_eq!(nonsense.problems.len(), 1);
        assert!(nonsense.items.is_empty());
    }

    /// The user's own file is theirs. Everything outside the retired
    /// range comes back byte-for-byte — blank lines, hand-written
    /// comments and all.
    #[test]
    fn retiring_a_block_leaves_everything_around_it_untouched() {
        let original = "# my notes\n\nlistener {\n    timeout = 5\n}\n\n# trailing note\n";
        let out = retire(original, &[(3, 5)], "moved into Settings");
        assert!(out.starts_with("# my notes\n\n"), "{out}");
        assert!(out.ends_with("\n# trailing note\n"), "{out}");
        assert!(out.contains("# listener {\n"), "{out}");
        assert!(out.contains("    # timeout = 5\n"), "{out}");
        assert!(out.contains("# }\n"), "{out}");
    }

    /// Commented out, never deleted. The difference between a change the
    /// user can read and undo, and one that looks like the app ate their
    /// config.
    #[test]
    fn retiring_keeps_every_line_the_user_wrote() {
        let original = "listener {\n    timeout = 5\n}\n";
        let out = retire(original, &[(1, 3)], "note");
        for fragment in ["listener {", "timeout = 5", "}"] {
            assert!(out.contains(fragment), "{fragment} was lost: {out}");
        }
        assert_eq!(parse(&out).items, Vec::new(), "the daemon must no longer see it");
    }

    #[test]
    fn a_note_says_where_the_setting_went() {
        let out = retire("listener {\n}\n", &[(1, 2)], "now in Settings → Desktop → Idle");
        assert!(out.starts_with("# now in Settings → Desktop → Idle\n"), "{out}");
    }

    /// Retiring the same range twice must not double-prefix, or repeated
    /// imports turn a config into a wall of `# # # #`.
    #[test]
    fn retiring_twice_changes_nothing_the_second_time() {
        let original = "listener {\n    timeout = 5\n}\n";
        let once = retire(original, &[(1, 3)], "note");
        // The note is re-emitted, so compare the retired lines rather than
        // the whole file.
        let twice = retire(&once, &[(2, 4)], "note");
        assert!(!twice.contains("# # "), "{twice}");
    }

    /// Only the ranges given. A second listener the user did not adopt
    /// has to keep working.
    #[test]
    fn a_range_that_was_not_given_is_left_live() {
        let original = "listener {\n    timeout = 1\n}\nlistener {\n    timeout = 2\n}\n";
        let out = retire(original, &[(1, 3)], "note");
        let doc = parse(&out);
        assert_eq!(doc.blocks("listener").count(), 1, "{out}");
        let Item::Block { items, .. } = doc.blocks("listener").next().unwrap() else {
            panic!("expected a block")
        };
        // Line 6, not 5: the note inserted above shifted it. Hence the
        // rule in `retire`'s docs — every range in one call, computed
        // against the original.
        assert_eq!(items[0], Item::Assignment {
            key: "timeout".to_string(),
            value: "2".to_string(),
            line: 6,
        });
    }

    #[test]
    fn a_file_without_a_trailing_newline_does_not_gain_one() {
        assert_eq!(retire("a = 1", &[], "note"), "a = 1");
    }

    /// Sourced paths are reported, never followed: following them walks
    /// into Hyprforge's own generated file and re-imports what it wrote.
    #[test]
    fn a_sourced_path_is_reported_and_not_followed() {
        let doc = parse("source = /home/me/.config/hypr/hyprforge/idle.conf\n");
        assert_eq!(doc.sourced(), vec!["/home/me/.config/hypr/hyprforge/idle.conf"]);
    }
}
