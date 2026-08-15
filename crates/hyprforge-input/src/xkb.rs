//! The keyboard layouts, variants and models installed on this machine.
//!
//! `kb_layout` is the field most worth not typing. The value is a code
//! (`us`, `gb`, `de`), the thing a user knows is a name ("English (UK)"),
//! and a wrong code is accepted silently — XKB falls back and the
//! keyboard simply doesn't change, which looks exactly like the setting
//! failing to save.
//!
//! Read from `/usr/share/X11/xkb/rules/evdev.lst`, which ships with
//! xkeyboard-config and carries both halves:
//!
//! ```text
//! ! layout
//!   us              English (US)
//! ! variant
//!   colemak         us: English (Colemak)
//! ```
//!
//! `localectl list-x11-keymap-layouts` gives the same codes without the
//! descriptions, so the file is preferred and localectl isn't used at
//! all: a list of bare codes is barely better than a text box.
//!
//! Variants are tagged with the layout they belong to, which is what
//! makes filtering possible — the file lists over 400 of them, and
//! showing all of them under a `us` layout would be worse than useless.

use std::path::{Path, PathBuf};

/// Where xkeyboard-config installs its rules. The `evdev` ruleset is what
/// every Wayland compositor uses; `base` is the X11-era name for the same
/// data and is checked as a fallback for older installs.
const RULES_DIRS: &[&str] = &["/usr/share/X11/xkb/rules", "/usr/local/share/X11/xkb/rules"];
const RULESETS: &[&str] = &["evdev.lst", "base.lst"];

/// One selectable XKB value: the code Hyprland wants and the name a
/// person recognises.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Entry {
    pub code: String,
    pub description: String,
}

/// Everything one rules file describes.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Catalogue {
    pub layouts: Vec<Entry>,
    /// `(layout, variant)` — a variant only means anything under its own
    /// layout, so the pairing has to survive parsing.
    pub variants: Vec<(String, Entry)>,
    pub models: Vec<Entry>,
    pub options: Vec<Entry>,
}

impl Catalogue {
    /// The variants belonging to `layout`, in file order.
    ///
    /// Empty for an unknown layout, which is the honest answer: this
    /// machine's XKB data doesn't describe it, so there is nothing to
    /// offer.
    pub fn variants_for(&self, layout: &str) -> Vec<Entry> {
        self.variants
            .iter()
            .filter(|(l, _)| l == layout)
            .map(|(_, e)| e.clone())
            .collect()
    }
}

fn rules_file() -> Option<PathBuf> {
    RULES_DIRS
        .iter()
        .flat_map(|dir| RULESETS.iter().map(move |f| Path::new(dir).join(f)))
        .find(|p| p.is_file())
}

/// Reads the installed XKB catalogue.
///
/// A missing rules file yields an empty catalogue rather than an error:
/// the caller's answer is the same either way — offer a text field
/// instead of a picker — and there is nothing a user could do about it
/// from a settings screen.
pub fn catalogue() -> Catalogue {
    let Some(path) = rules_file() else {
        return Catalogue::default();
    };
    match std::fs::read_to_string(&path) {
        Ok(contents) => parse(&contents),
        Err(_) => Catalogue::default(),
    }
}

/// Split out from [`catalogue`] so the parsing is testable without the
/// file — the part that can actually be wrong.
pub fn parse(contents: &str) -> Catalogue {
    let mut out = Catalogue::default();
    let mut section = "";
    for line in contents.lines() {
        if let Some(name) = line.strip_prefix('!') {
            // `! model`, `! layout`, `! variant`, `! option`, and others
            // this doesn't use (`! option` is followed by `! ...` groups
            // in some versions).
            section = match name.trim() {
                "model" => "model",
                "layout" => "layout",
                "variant" => "variant",
                "option" => "option",
                _ => "",
            };
            continue;
        }
        let line = line.trim();
        if line.is_empty() || section.is_empty() {
            continue;
        }
        let Some((code, description)) = line.split_once(char::is_whitespace) else {
            continue;
        };
        let (code, description) = (code.trim(), description.trim());
        if code.is_empty() || description.is_empty() {
            continue;
        }
        match section {
            "layout" => out.layouts.push(Entry {
                code: code.to_string(),
                description: description.to_string(),
            }),
            "model" => out.models.push(Entry {
                code: code.to_string(),
                description: description.to_string(),
            }),
            "option" => out.options.push(Entry {
                code: code.to_string(),
                description: description.to_string(),
            }),
            "variant" => {
                // `colemak  us: English (Colemak)` — the layout is the
                // prefix before the colon, and without it a variant
                // can't be filtered to the layout it belongs to.
                let Some((layout, rest)) = description.split_once(':') else {
                    continue;
                };
                out.variants.push((
                    layout.trim().to_string(),
                    Entry {
                        code: code.to_string(),
                        description: rest.trim().to_string(),
                    },
                ));
            }
            _ => {}
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Real lines from this machine's `evdev.lst`.
    const SAMPLE: &str = "\
! model
  pc101           Generic 101-key PC
  pc105           Generic 105-key PC

! layout
  us              English (US)
  al              Albanian
  ara             Arabic

! variant
  colemak         us: English (Colemak)
  dvorak          us: English (Dvorak)
  plisi           al: Albanian (Plisi)

! option
  grp:switch      Right Alt (while pressed)
";

    #[test]
    fn every_section_is_read_with_its_descriptions() {
        let c = parse(SAMPLE);
        assert_eq!(c.layouts.len(), 3);
        assert_eq!(c.layouts[0], Entry { code: "us".into(), description: "English (US)".into() });
        assert_eq!(c.models.len(), 2);
        assert_eq!(c.options.len(), 1);
        assert_eq!(c.variants.len(), 3);
    }

    /// The file lists over 400 variants. Showing all of them under a `us`
    /// layout would be worse than a text box.
    #[test]
    fn variants_are_filtered_to_their_own_layout() {
        let c = parse(SAMPLE);
        let us = c.variants_for("us");
        assert_eq!(us.len(), 2);
        assert_eq!(us[0].code, "colemak");
        assert_eq!(us[0].description, "English (Colemak)", "the layout prefix is stripped");
        assert_eq!(c.variants_for("al").len(), 1);
    }

    /// An unknown layout has nothing to offer, which is the honest
    /// answer rather than every variant on the system.
    #[test]
    fn an_unknown_layout_has_no_variants() {
        assert!(parse(SAMPLE).variants_for("nonesuch").is_empty());
    }

    /// A variant line without the `layout:` prefix can't be attributed,
    /// and guessing would file it under the wrong keyboard.
    #[test]
    fn a_variant_without_a_layout_prefix_is_skipped() {
        let c = parse("! variant\n  orphan          No layout here\n");
        assert!(c.variants.is_empty());
    }

    /// Sections this module doesn't use must not leak into the ones it
    /// does — some rulesets carry extra `!` groups.
    #[test]
    fn unknown_sections_are_ignored() {
        let c = parse("! layout\n  us  English (US)\n! something_else\n  xx  Not a layout\n");
        assert_eq!(c.layouts.len(), 1);
    }

    #[test]
    fn a_missing_rules_file_yields_an_empty_catalogue() {
        assert_eq!(parse(""), Catalogue::default());
        assert_eq!(parse("! layout\n\n"), Catalogue::default());
    }

    /// Runs against whatever this machine really has, so a format change
    /// in xkeyboard-config shows up here.
    #[test]
    fn the_installed_catalogue_parses() {
        let c = catalogue();
        if rules_file().is_some() {
            assert!(!c.layouts.is_empty(), "a rules file exists but no layouts parsed");
            assert!(
                c.layouts.iter().any(|l| l.code == "us"),
                "every install has a `us` layout"
            );
            assert!(!c.variants_for("us").is_empty(), "`us` always has variants");
        }
    }
}
