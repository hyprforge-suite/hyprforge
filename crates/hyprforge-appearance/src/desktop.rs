//! The desktop theme settings GTK and Qt apps read, which live in
//! gsettings rather than in any file Hyprforge generates.
//!
//! This is the half of "appearance" that isn't Hyprland. A user's window
//! borders and their GTK theme are one decision — on this machine both are
//! Dracula — but they're configured in two unrelated places and nothing
//! keeps them in step. Putting them on one screen is the point of the
//! module.
//!
//! **Ownership works differently here, and the difference matters.**
//! Hyprland settings are an overlay: Hyprforge writes a file, that file
//! wins, and clearing a key hands it straight back. gsettings is shared
//! desktop-wide state with exactly one value and no layering — writing it
//! *is* the change, every app sees it immediately, and there is no
//! generated file to delete to undo it. So:
//!
//! - Nothing is written unless the user changes that specific control.
//!   There is no "adopt these" import step, because there is nothing to
//!   adopt: the current value is already the live one.
//! - The previous value is returned by [`set`] so a caller can offer to
//!   put it back. That is the only undo available (vision pillar #4).
//!
//! Talking to `gsettings` by process, the same way the rest of the
//! codebase talks to `hyprctl`, rather than linking gio: one small
//! well-understood dependency-free call, and a missing binary degrades to
//! "this section is unavailable" instead of failing to build.

use std::process::Command;

/// The GNOME interface schema. Qt apps read it too via qt5ct/qt6ct
/// platform themes, and it is what `cursor:sync_gsettings_theme` writes
/// to, which is why the cursor theme appears on both halves of the screen.
const SCHEMA: &str = "org.gnome.desktop.interface";

#[derive(Debug, thiserror::Error)]
pub enum DesktopError {
    #[error("couldn't run gsettings: {0}")]
    Spawn(#[source] std::io::Error),
    #[error("gsettings refused {key} = {value}: {message}")]
    Refused {
        key: String,
        value: String,
        message: String,
    },
}

/// What a desktop key holds, and how an editor should offer it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DesktopKind {
    /// Free text — a name nothing on disk can be enumerated for.
    Text,
    /// A closed set the schema itself declares.
    Enum(&'static [&'static str]),
    Int { min: i64, max: i64 },
    /// A name that can be discovered by scanning the system, so it can be
    /// picked from a list instead of typed.
    ///
    /// Typing these by hand means knowing an exact directory name, and a
    /// typo produces no error anywhere — GTK just falls back to its
    /// default, which looks exactly like the setting failing to save.
    Installed(Catalogue),
    /// A font name: a family and a point size in one string, e.g.
    /// `"Noto Sans 10"`. Offered as a family list plus a size, because
    /// they are two decisions and only one of them is enumerable.
    Font,
}

/// Which set of installed things a [`DesktopKind::Installed`] draws from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Catalogue {
    GtkThemes,
    IconThemes,
    CursorThemes,
}

impl Catalogue {
    /// The names currently installed, sorted and de-duplicated.
    pub fn installed(self) -> Vec<String> {
        match self {
            Catalogue::GtkThemes => crate::themes::gtk_themes(),
            Catalogue::IconThemes => crate::themes::icon_themes(),
            Catalogue::CursorThemes => crate::themes::cursor_themes(),
        }
    }
}

#[derive(Debug, Clone, Copy)]
pub struct DesktopSetting {
    pub key: &'static str,
    pub label: &'static str,
    pub help: &'static str,
    pub kind: DesktopKind,
    /// A Hyprland option that also writes this key, and the warning to
    /// show while it's on.
    ///
    /// Only the cursor keys have one, and it is a real conflict rather
    /// than a theoretical one: with `cursor:sync_gsettings_theme` on —
    /// which is the default — Hyprland pushes its own xcursor theme and
    /// size into gsettings on every theme load. A value set here is
    /// overwritten on the next reload, and without saying so the screen
    /// would look broken rather than contested.
    pub contested_by: Option<Contested>,
}

/// A Hyprland option that writes a gsettings key behind the screen's back.
#[derive(Debug, Clone, Copy)]
pub struct Contested {
    /// The Hyprland option key, so the caller can read whether it's on.
    pub option: &'static str,
    pub warning: &'static str,
}

const CURSOR_SYNC: Contested = Contested {
    option: "cursor:sync_gsettings_theme",
    warning: "Hyprland is set to push its own cursor theme and size here, so this \
              will be overwritten on the next reload. Turn off \"Share cursor theme \
              with GTK apps\" under Windows to control it from here.",
};

/// The keys this screen offers.
///
/// Deliberately short. Every one of these is something a user picks to
/// make their desktop look consistent; the rest of the schema (blink
/// rates, hinting, rgba order) is font rendering and belongs elsewhere if
/// anywhere.
pub const SETTINGS: &[DesktopSetting] = &[
    DesktopSetting {
        key: "color-scheme",
        label: "Light or dark",
        help: "What apps should prefer. Most GTK4 and modern Qt apps follow this.",
        kind: DesktopKind::Enum(&["default", "prefer-dark", "prefer-light"]),
    contested_by: None,
    },
    DesktopSetting {
        key: "gtk-theme",
        label: "GTK theme",
        help: "Widget theme for GTK apps.",
        kind: DesktopKind::Installed(Catalogue::GtkThemes),
    contested_by: None,
    },
    DesktopSetting {
        key: "icon-theme",
        label: "Icon theme",
        help: "Icon set for GTK apps.",
        kind: DesktopKind::Installed(Catalogue::IconThemes),
    contested_by: None,
    },
    DesktopSetting {
        key: "cursor-theme",
        label: "Cursor theme",
        help: "Pointer theme.",
        kind: DesktopKind::Installed(Catalogue::CursorThemes),
        contested_by: Some(CURSOR_SYNC),
    },
    DesktopSetting {
        key: "cursor-size",
        label: "Cursor size",
        help: "Pointer size in pixels. 24 is the usual default.",
        kind: DesktopKind::Int { min: 8, max: 128 },
        contested_by: Some(CURSOR_SYNC),
    },
    DesktopSetting {
        key: "font-name",
        label: "Interface font",
        help: "Font and size for app interfaces.",
        kind: DesktopKind::Font,
    contested_by: None,
    },
    DesktopSetting {
        key: "document-font-name",
        label: "Document font",
        help: "Font and size for document text.",
        kind: DesktopKind::Font,
    contested_by: None,
    },
    DesktopSetting {
        key: "monospace-font-name",
        label: "Monospace font",
        help: "Font and size for terminals and code.",
        kind: DesktopKind::Font,
    contested_by: None,
    },
];

pub fn get_setting(key: &str) -> Option<&'static DesktopSetting> {
    SETTINGS.iter().find(|s| s.key == key)
}

/// Reads every offered key. A key the schema doesn't have is skipped
/// rather than reported: schemas vary by GNOME version, and a missing one
/// means "this desktop doesn't have that setting", not an error the user
/// can act on.
pub fn read_all() -> Result<Vec<(&'static str, String)>, DesktopError> {
    let mut out = Vec::new();
    for setting in SETTINGS {
        if let Some(value) = read(setting.key)? {
            out.push((setting.key, value));
        }
    }
    Ok(out)
}

/// The desktop's accessibility text scaling, as a multiplier.
///
/// Not in [`SETTINGS`] on purpose: that list is the controls the
/// Appearance screen offers, and this is read-only input. Hyprforge
/// follows the system's accessibility setting rather than adding a
/// second, per-app text-size control the user would have to find.
///
/// Anything unreadable — no gsettings, no schema, a value that isn't a
/// number — is 1.0. A wrong scale is a UI nobody asked for; no scale is
/// simply the normal one.
pub fn text_scaling_factor() -> f32 {
    read(TEXT_SCALING)
        .ok()
        .flatten()
        .and_then(|v| v.trim().parse::<f32>().ok())
        .filter(|v| v.is_finite() && *v > 0.0)
        .unwrap_or(1.0)
}

/// Read, never written, so it stays out of [`SETTINGS`].
const TEXT_SCALING: &str = "text-scaling-factor";

/// One key's current value, or `None` if this desktop's schema lacks it.
pub fn read(key: &str) -> Result<Option<String>, DesktopError> {
    let out = Command::new("gsettings")
        .args(["get", SCHEMA, key])
        .output()
        .map_err(DesktopError::Spawn)?;
    if !out.status.success() {
        return Ok(None);
    }
    Ok(Some(unquote(
        String::from_utf8_lossy(&out.stdout).trim(),
    )))
}

/// Writes `key`, returning what it held before so the caller can offer to
/// put it back.
///
/// The read happens first and its result is returned even if the write
/// then fails — a caller that has already told the user "was X, now Y"
/// needs X to be true regardless.
pub fn set(key: &str, value: &str) -> Result<Option<String>, DesktopError> {
    let previous = read(key)?;
    let out = Command::new("gsettings")
        .args(["set", SCHEMA, key, value])
        .output()
        .map_err(DesktopError::Spawn)?;
    if !out.status.success() {
        return Err(DesktopError::Refused {
            key: key.to_string(),
            value: value.to_string(),
            message: String::from_utf8_lossy(&out.stderr).trim().to_string(),
        });
    }
    Ok(previous)
}

/// `gsettings get` prints strings with single quotes around them and
/// integers bare. Stripping them is not cosmetic: the quoted form is what
/// `gsettings set` would store *including* the quotes, so a value read and
/// written back unchanged would gain a pair every round trip.
fn unquote(raw: &str) -> String {
    match raw.strip_prefix('\'').and_then(|r| r.strip_suffix('\'')) {
        Some(inner) => inner.to_string(),
        None => raw.to_string(),
    }
}

/// Splits a font string into its family and point size.
///
/// gsettings stores `"Noto Sans 10"` — and on this machine
/// `"Noto Sans  10"`, with two spaces — so the size is the trailing
/// numeric token and the family is everything before it. A string with no
/// trailing number has no size, which is valid: the family alone is a
/// legal font description.
///
/// The family keeps any style words (`"Noto Sans Bold"`), because
/// dropping them would silently change the font when the user was only
/// editing the size.
pub fn split_font(value: &str) -> (String, Option<u32>) {
    let trimmed = value.trim();
    match trimmed.rsplit_once(char::is_whitespace) {
        Some((family, last)) => match last.parse::<u32>() {
            Ok(size) if !family.trim().is_empty() => (family.trim().to_string(), Some(size)),
            _ => (trimmed.to_string(), None),
        },
        None => (trimmed.to_string(), None),
    }
}

/// Rebuilds a font string from a family and a size.
///
/// One space, whatever the original had — gsettings accepts it and it is
/// what every other tool writes.
pub fn join_font(family: &str, size: Option<u32>) -> String {
    let family = family.trim();
    match size {
        Some(size) => format!("{family} {size}"),
        None => family.to_string(),
    }
}

/// Whether `value` is something this key will accept, checked before
/// writing so a bad value is a message rather than a desktop-wide change
/// that half-applies.
pub fn check(setting: &DesktopSetting, value: &str) -> Result<(), String> {
    match setting.kind {
        // An installed name is checked for emptiness only, not against
        // the installed list: a theme can live outside the search paths,
        // and refusing a value the system accepts would be a dead end.
        DesktopKind::Text | DesktopKind::Installed(_) | DesktopKind::Font => {
            if value.trim().is_empty() {
                Err("can't be empty".to_string())
            } else {
                Ok(())
            }
        }
        DesktopKind::Enum(choices) => {
            if choices.contains(&value) {
                Ok(())
            } else {
                Err(format!("expected one of {}", choices.join(", ")))
            }
        }
        DesktopKind::Int { min, max } => match value.trim().parse::<i64>() {
            Ok(v) if v >= min && v <= max => Ok(()),
            Ok(_) => Err(format!("must be between {min} and {max}")),
            Err(_) => Err("expected a whole number".to_string()),
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A quoted value written back unchanged would gain a pair of quotes
    /// on every save.
    #[test]
    fn quotes_are_stripped_from_what_gsettings_prints() {
        assert_eq!(unquote("'Dracula'"), "Dracula");
        assert_eq!(unquote("24"), "24");
        assert_eq!(unquote("'Noto Sans  10'"), "Noto Sans  10");
    }

    /// An apostrophe inside a theme name must not be mistaken for the
    /// wrapping quotes.
    #[test]
    fn an_unbalanced_quote_is_left_alone() {
        assert_eq!(unquote("'unterminated"), "'unterminated");
        assert_eq!(unquote("trailing'"), "trailing'");
    }

    #[test]
    fn an_enum_only_accepts_its_declared_values() {
        let scheme = get_setting("color-scheme").unwrap();
        assert!(check(scheme, "prefer-dark").is_ok());
        let err = check(scheme, "dark").unwrap_err();
        assert!(err.contains("prefer-dark"), "{err}");
    }

    #[test]
    fn a_size_outside_its_range_is_refused_with_the_range() {
        let size = get_setting("cursor-size").unwrap();
        assert!(check(size, "24").is_ok());
        assert!(check(size, "999").unwrap_err().contains("between 8 and 128"));
        assert!(check(size, "big").unwrap_err().contains("whole number"));
    }

    /// An empty theme name is a real mistake — gsettings would take it and
    /// apps would fall back to something unpredictable.
    #[test]
    fn an_empty_theme_name_is_refused() {
        let theme = get_setting("gtk-theme").unwrap();
        assert!(check(theme, "Dracula").is_ok());
        assert!(check(theme, "   ").is_err());
    }

    /// The real value from this machine, two spaces and all.
    #[test]
    fn a_font_splits_into_family_and_size() {
        assert_eq!(split_font("Noto Sans  10"), ("Noto Sans".into(), Some(10)));
        assert_eq!(split_font("Cantarell 11"), ("Cantarell".into(), Some(11)));
    }

    /// Style words belong to the family. Dropping them would silently
    /// change the font when the user was only editing the size.
    #[test]
    fn a_style_stays_with_the_family() {
        assert_eq!(
            split_font("Noto Sans Bold 12"),
            ("Noto Sans Bold".into(), Some(12))
        );
    }

    /// A family alone is a legal font description, and a family that ends
    /// in a number must not have it eaten.
    #[test]
    fn a_font_without_a_size_keeps_its_whole_name() {
        assert_eq!(split_font("Cantarell"), ("Cantarell".into(), None));
        assert_eq!(split_font(""), (String::new(), None));
        assert_eq!(split_font("12"), ("12".into(), None), "no family to split off");
    }

    #[test]
    fn a_font_round_trips_through_split_and_join() {
        for original in ["Noto Sans 10", "Noto Sans Bold 12", "Cantarell"] {
            let (family, size) = split_font(original);
            assert_eq!(join_font(&family, size), original);
        }
    }

    /// The odd double space this machine actually stores normalises to
    /// one, which gsettings accepts and every other tool writes.
    #[test]
    fn joining_normalises_spacing() {
        let (family, size) = split_font("Noto Sans  10");
        assert_eq!(join_font(&family, size), "Noto Sans 10");
    }

    #[test]
    fn every_setting_is_reachable_by_key() {
        for s in SETTINGS {
            assert!(get_setting(s.key).is_some(), "{}", s.key);
        }
    }
}
