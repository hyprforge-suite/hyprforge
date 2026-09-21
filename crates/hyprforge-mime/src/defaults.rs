//! Which application opens a type, and changing it.
//!
//! `mimeapps.list` is the freedesktop file that records the choice. It
//! is read from several places (the user's own, then the system's) and
//! written in exactly one: `$XDG_CONFIG_HOME/mimeapps.list`.
//!
//! # This file is hand-edited, so it is edited by hand
//!
//! CLAUDE.md's rule about `files-config.toml` applies here with more
//! force, because this file is not ours: people put comments in it,
//! group related types together, and keep sections this code has never
//! heard of. [`with_default`] therefore changes one line and copies
//! every other byte through — no parse-and-reserialise, which would
//! silently reorder a person's file and drop their notes the first time
//! they changed a default from a menu.
//!
//! `[Added Associations]` and `[Removed Associations]` are *read* —
//! they change what a chooser may offer — and never written: adding or
//! removing an association is not something this suite has a reason to
//! do on anyone's behalf. A section beyond those three is neither read
//! nor touched, which is the only safe thing to do with it.

use std::collections::BTreeMap;

const DEFAULTS_SECTION: &str = "[Default Applications]";
/// Applications a user added for a type beyond what the desktop entries
/// themselves registered.
pub const ADDED_SECTION: &str = "[Added Associations]";
/// Applications a user does not want offered for a type, whatever the
/// desktop entry claims.
pub const REMOVED_SECTION: &str = "[Removed Associations]";

/// The `[Default Applications]` entries of one `mimeapps.list`.
///
/// The value is a list because the spec allows fallbacks: the first
/// entry that is actually installed wins.
pub fn parse(text: &str) -> BTreeMap<String, Vec<String>> {
    parse_section(text, DEFAULTS_SECTION)
}

/// Any one section of a `mimeapps.list` — or of `mimeinfo.cache`,
/// which has the same shape and is read by this too (see
/// [`crate::apps::parse_cache`]).
///
/// The three that matter here are the defaults, the associations a
/// person added, and the ones they removed — see [`ADDED_SECTION`] and
/// [`REMOVED_SECTION`]. A removal is not a smaller kind of default: it
/// says "never offer this for that type", and a chooser that ignores it
/// keeps putting back something somebody deliberately took away.
///
/// A key that appears twice in one section keeps the entries of both,
/// first occurrence first — the same "the earlier rule wins" the glob
/// matcher settled on, so that a caller taking `.first()` gets the
/// answer it would get from two files in directory order.
pub fn parse_section(text: &str, section: &str) -> BTreeMap<String, Vec<String>> {
    let mut defaults: BTreeMap<String, Vec<String>> = BTreeMap::new();
    let mut in_section = false;
    for line in text.lines() {
        let line = line.trim();
        if line.starts_with('[') {
            in_section = line == section;
            continue;
        }
        if !in_section || line.is_empty() || line.starts_with('#') {
            continue;
        }
        let Some((mime, entries)) = line.split_once('=') else { continue };
        let entries = entries.split(';').map(str::trim).filter(|e| !e.is_empty());
        let known: &mut Vec<String> = defaults.entry(mime.trim().to_string()).or_default();
        for entry in entries {
            if !known.iter().any(|seen| seen == entry) {
                known.push(entry.to_string());
            }
        }
    }
    defaults
}

/// `text` with `mime` set to open in `app` — the whole file back, one
/// line different.
///
/// Three cases, in the order they are met in the wild:
///
/// - The type already has a line: that line's value is replaced, where
///   it sits. Moving it to the bottom would reorder a file someone
///   grouped on purpose.
/// - There is a `[Default Applications]` section but no line for this
///   type: the line is added at the end of that section, before any
///   blank line that separates it from the next one.
/// - There is no such section: it is appended, after a blank line.
///
/// The file's own line endings are not preserved, because every writer
/// of this format uses `\n` and pretending otherwise would mean carrying
/// a guess through every branch above.
pub fn with_default(text: &str, mime: &str, app: &str) -> String {
    let new_line = format!("{mime}={app}");
    let mut lines: Vec<String> = text.lines().map(str::to_string).collect();

    // Where the section starts, and where it ends (the next section, or
    // the end of the file).
    let start = lines.iter().position(|line| line.trim() == DEFAULTS_SECTION);
    let Some(start) = start else {
        if !lines.is_empty() && !lines.last().is_some_and(|l| l.trim().is_empty()) {
            lines.push(String::new());
        }
        lines.push(DEFAULTS_SECTION.to_string());
        lines.push(new_line);
        return finish(lines);
    };
    let end = lines[start + 1..]
        .iter()
        .position(|line| line.trim_start().starts_with('['))
        .map(|offset| start + 1 + offset)
        .unwrap_or(lines.len());

    let existing = lines[start + 1..end].iter().position(|line| {
        line.split_once('=').is_some_and(|(key, _)| key.trim() == mime) && !line.trim_start().starts_with('#')
    });
    match existing {
        Some(offset) => lines[start + 1 + offset] = new_line,
        // After the last line with something on it, so the new entry
        // joins the section rather than landing past the blank line that
        // ends it.
        None => {
            let last = lines[start + 1..end]
                .iter()
                .rposition(|line| !line.trim().is_empty())
                .map(|offset| start + 2 + offset)
                .unwrap_or(start + 1);
            lines.insert(last, new_line);
        }
    }
    finish(lines)
}

/// A text file ends with a newline.
fn finish(lines: Vec<String>) -> String {
    let mut text = lines.join("\n");
    text.push('\n');
    text
}

#[cfg(test)]
mod tests {
    use super::*;

    const REAL: &str = "\
# my associations
[Default Applications]
text/html=google-chrome.desktop
x-scheme-handler/http=google-chrome.desktop
model/stl=view3d.desktop

[Added Associations]
model/3mf=fstl.desktop;
";

    #[test]
    fn the_defaults_section_reads_as_type_to_applications() {
        let defaults = parse(REAL);
        assert_eq!(defaults.get("text/html").unwrap(), &["google-chrome.desktop"]);
        assert_eq!(defaults.get("model/stl").unwrap(), &["view3d.desktop"]);
        assert_eq!(defaults.get("model/3mf"), None, "that one is an association, not a default");
    }

    #[test]
    fn the_other_two_sections_read_the_same_way() {
        let added = parse_section(REAL, ADDED_SECTION);
        assert_eq!(added.get("model/3mf").unwrap(), &["fstl.desktop"]);
        assert!(parse_section(REAL, REMOVED_SECTION).is_empty(), "none in this file");
        assert!(
            !added.contains_key("text/html"),
            "a default is not an association; the sections stay apart"
        );
    }

    #[test]
    fn a_fallback_list_keeps_its_order() {
        let defaults = parse("[Default Applications]\nimage/png=a.desktop;b.desktop;\n");
        assert_eq!(defaults.get("image/png").unwrap(), &["a.desktop", "b.desktop"]);
    }

    /// A key written twice in one section keeps both, in the order
    /// written, without repeating an entry that appears in both.
    #[test]
    fn a_repeated_key_accumulates_rather_than_replacing() {
        let defaults = parse(
            "[Default Applications]\nimage/png=a.desktop\nimage/png=b.desktop;a.desktop\n",
        );
        assert_eq!(defaults.get("image/png").unwrap(), &["a.desktop", "b.desktop"]);
    }

    /// `mimeinfo.cache` is the same shape and is read by the same
    /// scanner — one implementation, one answer about repeats.
    #[test]
    fn the_registration_cache_reads_through_the_same_scanner() {
        let cache = crate::apps::parse_cache(
            "[MIME Cache]\nmodel/stl=viewer.desktop;slicer.desktop;\n",
        );
        assert_eq!(cache.get("model/stl").unwrap(), &["viewer.desktop", "slicer.desktop"]);
    }

    /// The property this module exists for: everything that is not the
    /// changed line survives, byte for byte.
    #[test]
    fn changing_a_default_leaves_every_other_line_alone() {
        let after = with_default(REAL, "model/stl", "BambuStudio.desktop");
        assert!(after.contains("# my associations"), "the comment stays");
        assert!(after.contains("[Added Associations]\nmodel/3mf=fstl.desktop;"), "so does a section we do not understand");
        assert!(after.contains("model/stl=BambuStudio.desktop"));
        assert!(!after.contains("model/stl=view3d.desktop"));
        // The changed line is where it was, not moved to the end.
        let lines: Vec<&str> = after.lines().collect();
        assert_eq!(lines[4], "model/stl=BambuStudio.desktop");
    }

    #[test]
    fn a_new_type_joins_the_section_rather_than_landing_after_it() {
        let after = with_default(REAL, "model/3mf", "view3d.desktop");
        let lines: Vec<&str> = after.lines().collect();
        let section = lines.iter().position(|l| *l == "[Default Applications]").unwrap();
        let added = lines.iter().position(|l| *l == "model/3mf=view3d.desktop").unwrap();
        let next_section = lines.iter().position(|l| *l == "[Added Associations]").unwrap();
        assert!(section < added && added < next_section, "{lines:?}");
        assert_eq!(lines[added + 1], "", "the blank line separating the sections survives");
    }

    #[test]
    fn a_file_without_the_section_gains_one() {
        let after = with_default("[Added Associations]\nmodel/3mf=fstl.desktop;\n", "image/png", "imv.desktop");
        assert!(after.starts_with("[Added Associations]\nmodel/3mf=fstl.desktop;\n"));
        assert!(after.ends_with("[Default Applications]\nimage/png=imv.desktop\n"), "{after}");
    }

    #[test]
    fn an_empty_file_becomes_a_valid_one() {
        assert_eq!(
            with_default("", "image/png", "imv.desktop"),
            "[Default Applications]\nimage/png=imv.desktop\n"
        );
        assert_eq!(parse(&with_default("", "image/png", "imv.desktop")).len(), 1);
    }

    /// A commented-out line is a note, not an entry: changing the
    /// default must add a real line rather than editing someone's note.
    #[test]
    fn a_commented_out_entry_is_left_as_a_comment() {
        let after = with_default("[Default Applications]\n#image/png=old.desktop\n", "image/png", "new.desktop");
        assert!(after.contains("#image/png=old.desktop"));
        assert!(after.contains("\nimage/png=new.desktop"));
    }

    /// Setting the same default twice is not two lines.
    #[test]
    fn setting_the_same_default_again_changes_nothing() {
        let once = with_default(REAL, "image/png", "imv.desktop");
        let twice = with_default(&once, "image/png", "imv.desktop");
        assert_eq!(once, twice);
    }
}
