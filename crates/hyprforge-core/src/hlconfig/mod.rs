//! Editing Hyprland's `hl.config` settings, for any category.
//!
//! Every `hl.config` category behaves the same way, and the behaviour is
//! not obvious, so it is worth stating once here rather than rediscovering
//! it per module. Measured on Hyprland 0.56.1:
//!
//! 1. **A later call beats an earlier one** for the keys it passes.
//! 2. **A call updates only the keys it passes**, leaving everything else
//!    to Hyprland's default or the user's own config.
//!
//! Together those make a generated file an *overlay*: it names only the
//! settings a module has taken ownership of, and it must be sourced
//! **last** so that ownership means anything. Sourced first, a user's own
//! `hl.config` block silently wins and the app reports saves that changed
//! nothing.
//!
//! This module is everything that follows from those two facts, with the
//! category-specific part — which options exist, their types and ranges —
//! supplied by a [`Catalog`]. `hyprforge-input` and `hyprforge-appearance`
//! are each a catalog plus a require line; nothing else about them
//! differs.
//!
//! Sharing it is not tidiness. The pieces here encode defects that were
//! expensive to find once: a non-finite float rendering as `NaN.0` and
//! taking the whole config down, `[[EMPTY]]` sentinels imported as if they
//! were values, an unreadable store treated as an empty one. Three copies
//! of `lua_string` each got the same bracket rule wrong in this codebase
//! before it was shared; this is the same lesson applied earlier.

pub mod codegen;
pub mod import;
pub mod model;
pub mod storage;

pub use model::{Invalid, Settings, Value};

/// What a setting accepts, and what Hyprland does with it when nothing
/// sets it.
///
/// The default matters for more than display: [`model`] uses it to tell
/// "the user chose the default" from "the user chose nothing", which is
/// the difference between writing the key and staying out of the way.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Kind {
    Bool {
        default: bool,
    },
    Int {
        default: i64,
        min: Option<i64>,
        max: Option<i64>,
    },
    /// An integer whose values are named states rather than a quantity —
    /// a dropdown, not a spinner.
    IntEnum {
        default: i64,
        choices: &'static [(i64, &'static str)],
    },
    Float {
        default: f64,
        min: Option<f64>,
        max: Option<f64>,
    },
    Text {
        default: &'static str,
    },
    /// Text with a closed set of accepted values. Only for sets that
    /// really are closed: Hyprland's `accel_profile` looks like one but
    /// also takes `custom <step> <points...>`, so it stays [`Kind::Text`].
    TextEnum {
        default: &'static str,
        choices: &'static [&'static str],
    },
    /// A colour, written as Hyprland's `rgba(rrggbbaa)`/`rgb(rrggbb)`.
    ///
    /// Stored as text, but distinct from [`Kind::Text`] so an editor can
    /// offer a colour control and so the format is validated: a malformed
    /// colour is refused by Hyprland and takes the generated file with it.
    ///
    /// Hyprland calls the type `gradient` and reads it back in a different
    /// form than it accepts — `rgba(bd93f9ff)` going in comes out as
    /// `ffbd93f9 0deg`, which is `AARRGGBB` plus an angle. Reading a live
    /// value therefore needs converting, not copying; see
    /// [`import::read_gradient`].
    Color {
        default: &'static str,
    },
    /// A colour Hyprland stores as a plain number rather than a
    /// gradient — its `color` type, as opposed to `gradient`.
    ///
    /// Written exactly like [`Kind::Color`] (`rgba(rrggbbaa)`), but read
    /// back as an integer: `rgba(112233ff)` in comes out as
    /// `4279312947`, which is `0xFF112233` — AARRGGBB, the same byte
    /// order a gradient uses in its hex form. Measured, not assumed.
    ColorInt {
        default: &'static str,
    },
    /// A gap size. Hyprland's type is `css_gap`, and in Lua it takes
    /// "an integer or a table with optional top/right/bottom/left fields"
    /// — its own words, from the error it returns for anything else. A
    /// string is refused, despite the wiki calling the type `css_gaps`.
    ///
    /// Only the uniform integer form is editable here. Per-side gaps need
    /// the table form, which an overlay of scalar values can't express;
    /// a config using one imports as nothing rather than as a wrong
    /// number.
    Gaps {
        default: i64,
        min: Option<i64>,
        max: Option<i64>,
    },
}

impl Kind {
    /// The name `hyprctl getoption -j` uses for this type's value field.
    /// Comparing against this is what stops a catalog claiming an int
    /// where Hyprland has a float.
    pub fn hyprctl_field(&self) -> &'static str {
        match self {
            Kind::Bool { .. } => "bool",
            Kind::Int { .. } | Kind::IntEnum { .. } => "int",
            Kind::Float { .. } => "float",
            Kind::Text { .. } | Kind::TextEnum { .. } => "str",
            Kind::Color { .. } => "gradient",
            Kind::ColorInt { .. } => "int",
            Kind::Gaps { .. } => "css",
        }
    }
}

#[derive(Debug, Clone, Copy)]
pub struct Setting {
    /// Hyprland's own colon-separated key, e.g. `decoration:blur:passes`.
    pub key: &'static str,
    /// Short human label for the editor.
    pub label: &'static str,
    /// One sentence on what it does, shown next to the control. Kept to
    /// what a user needs to decide, not a restatement of the label.
    pub help: &'static str,
    pub kind: Kind,
}

impl Setting {
    /// The `decoration:blur`-style prefix this setting groups under.
    pub fn category(&self) -> &'static str {
        match self.key.rfind(':') {
            Some(i) => &self.key[..i],
            None => self.key,
        }
    }

    /// The last path segment — the name as written inside the Lua table.
    pub fn leaf(&self) -> &'static str {
        match self.key.rfind(':') {
            Some(i) => &self.key[i + 1..],
            None => self.key,
        }
    }
}

/// A group of settings, in the order an editor shows them.
#[derive(Debug, Clone, Copy)]
pub struct Category {
    pub key: &'static str,
    pub label: &'static str,
    pub help: &'static str,
}

/// Everything a module claims about one slice of Hyprland's config
/// surface.
///
/// A catalog is the single source of truth for its module: the editor is
/// generated from it, codegen and import filter through it, and a live
/// test checks every claim in it against a running compositor. A
/// hand-written form beside it would be a second copy of the same facts,
/// drifting.
pub struct Catalog {
    pub settings: &'static [Setting],
    pub categories: &'static [Category],
    /// `(category, why)` for parts of the surface this module knowingly
    /// doesn't edit.
    ///
    /// Listed rather than omitted so a key from one is reported as
    /// unsupported instead of as a typo — otherwise a user goes hunting
    /// for a misspelling that isn't there.
    pub unsupported: &'static [(&'static str, &'static str)],
}

impl Catalog {
    pub fn get(&self, key: &str) -> Option<&'static Setting> {
        self.settings.iter().find(|s| s.key == key)
    }

    pub fn category(&self, key: &str) -> Option<&'static Category> {
        self.categories.iter().find(|c| c.key == key)
    }

    /// Settings in `category`, in catalog order.
    pub fn in_category<'a>(
        &'a self,
        category: &'a str,
    ) -> impl Iterator<Item = &'static Setting> + 'a {
        self.settings.iter().filter(move |s| s.category() == category)
    }

    /// Why a key isn't in this catalog — "belongs to a part we don't
    /// edit" and "is a typo" send a user to completely different places.
    ///
    /// Matches an unsupported entry either exactly or as a prefix,
    /// because both shapes occur: `decoration:wobble` names a whole
    /// subcategory, while `misc:bell_sound` names one option. Checking
    /// only the prefix form meant a stored `misc:bell_sound` — exactly
    /// what a user would have — fell through to "typo".
    pub fn unknown_key_reason(&self, key: &str) -> String {
        let key = key.trim();
        for (entry, why) in self.unsupported {
            if key == *entry || key.starts_with(&format!("{entry}:")) {
                return (*why).to_string();
            }
        }
        "not a setting Hyprforge knows about".to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn category_and_leaf_split_the_key() {
        let s = Setting {
            key: "decoration:blur:passes",
            label: "",
            help: "",
            kind: Kind::Int { default: 1, min: None, max: None },
        };
        assert_eq!(s.category(), "decoration:blur");
        assert_eq!(s.leaf(), "passes");
    }

    /// Both shapes occur in real catalogues: one names a whole
    /// subcategory, the other names a single option. Only handling the
    /// prefix form meant a stored key equal to the entry fell through to
    /// "typo", which sends the user looking for a misspelling that isn't
    /// there.
    #[test]
    fn an_unsupported_entry_is_matched_exactly_as_well_as_by_prefix() {
        let catalog = Catalog {
            settings: &[],
            categories: &[],
            unsupported: &[("misc:bell_sound", "not in 0.56"), ("decoration:wobble", "not yet")],
        };
        assert_eq!(catalog.unknown_key_reason("misc:bell_sound"), "not in 0.56");
        assert_eq!(catalog.unknown_key_reason("decoration:wobble"), "not yet");
        assert_eq!(catalog.unknown_key_reason("decoration:wobble:enabled"), "not yet");
        assert!(catalog.unknown_key_reason("misc:bell_sounds").contains("know"));
    }

    #[test]
    fn a_single_segment_key_is_its_own_category() {
        let s = Setting {
            key: "rounding",
            label: "",
            help: "",
            kind: Kind::Bool { default: false },
        };
        assert_eq!(s.category(), "rounding");
        assert_eq!(s.leaf(), "rounding");
    }
}
