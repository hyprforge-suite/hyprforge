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
    /// Free text — a theme or font name.
    Text,
    /// A closed set the schema itself declares.
    Enum(&'static [&'static str]),
    Int { min: i64, max: i64 },
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
        help: "Widget theme for GTK apps, by name — one of the folders in ~/.themes or /usr/share/themes.",
        kind: DesktopKind::Text,
    contested_by: None,
    },
    DesktopSetting {
        key: "icon-theme",
        label: "Icon theme",
        help: "Icon set for GTK apps, by name.",
        kind: DesktopKind::Text,
    contested_by: None,
    },
    DesktopSetting {
        key: "cursor-theme",
        label: "Cursor theme",
        help: "Pointer theme by name.",
        kind: DesktopKind::Text,
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
        help: "Font and size for app interfaces, e.g. \"Noto Sans 10\".",
        kind: DesktopKind::Text,
    contested_by: None,
    },
    DesktopSetting {
        key: "document-font-name",
        label: "Document font",
        help: "Font and size for document text.",
        kind: DesktopKind::Text,
    contested_by: None,
    },
    DesktopSetting {
        key: "monospace-font-name",
        label: "Monospace font",
        help: "Font and size for terminals and code.",
        kind: DesktopKind::Text,
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

/// Whether `value` is something this key will accept, checked before
/// writing so a bad value is a message rather than a desktop-wide change
/// that half-applies.
pub fn check(setting: &DesktopSetting, value: &str) -> Result<(), String> {
    match setting.kind {
        DesktopKind::Text => {
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

    #[test]
    fn every_setting_is_reachable_by_key() {
        for s in SETTINGS {
            assert!(get_setting(s.key).is_some(), "{}", s.key);
        }
    }
}
