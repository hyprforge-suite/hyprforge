//! Which window the popup is about to paste into, and what that means
//! for the shortcut it should send.
//!
//! Identical to `hyprforge-clipmenu::target` — see this crate's
//! `chooser` module doc for why it is hand-copied rather than shared:
//! `hyprforge-clipmenu` is a binary, not part of `hyprforge-clipboard`'s
//! library, so this logic (small, and
//! Hyprland-flavoured rather than clipboard-flavoured) cannot be
//! imported, only re-derived the same way. Kept byte-for-byte in
//! behaviour rather than reinvented, so a terminal a person has already
//! taught the clipboard popup about behaves identically here.

use hyprforge_clipboard::Shortcut;
use hyprforge_process::{output, TIMEOUT};
use std::process::Command;

/// The window that had keyboard focus, read *before* this popup's own
/// layer surface takes it away — see `main.rs` for why this has to run
/// first.
pub struct FocusedWindow {
    pub class: String,
}

/// `hyprctl activewindow -j`, read once at startup. `None` covers every
/// way this can fail to answer — see `hyprforge-clipmenu::target::focused_window`'s
/// identical doc for the full reasoning.
pub fn focused_window() -> Option<FocusedWindow> {
    let result = output(Command::new("hyprctl").args(["activewindow", "-j"]), TIMEOUT).ok()?;
    let value: serde_json::Value = serde_json::from_slice(&result.stdout).ok()?;
    let class = value.get("class")?.as_str()?.to_string();
    Some(FocusedWindow { class })
}

const KNOWN_TERMINAL_NAMES: &[&str] =
    &["ghostty", "foot", "kitty", "alacritty", "konsole", "terminal", "urxvt", "rxvt", "rio", "contour", "st"];

/// See `hyprforge-clipmenu::target::paste_shortcut`'s doc comment for the
/// full reasoning behind the matching rule — identical here.
pub fn paste_shortcut(class: &str) -> Shortcut {
    let lower = class.to_lowercase();
    let last_segment = lower.rsplit('.').next().unwrap_or(&lower);

    let known = KNOWN_TERMINAL_NAMES.iter().any(|&name| {
        last_segment == name
            || last_segment.starts_with(&format!("{name}-"))
            || last_segment.starts_with(&format!("{name}_"))
    });
    let generic_term = lower.contains("term");

    if known || generic_term {
        Shortcut::CtrlShiftV
    } else {
        Shortcut::CtrlV
    }
}

/// Purely cosmetic — see `hyprforge-clipmenu::target::display_name`'s
/// identical doc.
pub fn display_name(class: &str) -> String {
    let segment = class.rsplit('.').next().unwrap_or(class);
    let mut chars = segment.chars();
    match chars.next() {
        Some(first) => first.to_uppercase().collect::<String>() + chars.as_str(),
        None => segment.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn assert_terminal(class: &str) {
        assert_eq!(paste_shortcut(class), Shortcut::CtrlShiftV, "{class:?} should be treated as a terminal");
    }

    fn assert_not_terminal(class: &str) {
        assert_eq!(paste_shortcut(class), Shortcut::CtrlV, "{class:?} should not be treated as a terminal");
    }

    #[test]
    fn every_named_terminal_class_gets_ctrl_shift_v() {
        for class in [
            "com.mitchellh.ghostty",
            "foot",
            "kitty",
            "Alacritty",
            "org.wezfurlong.wezterm",
            "konsole",
            "xterm",
            "st",
            "WezTerm",
            "org.gnome.Terminal",
            "terminator",
            "urxvt",
            "rio",
            "contour",
        ] {
            assert_terminal(class);
        }
    }

    #[test]
    fn ordinary_applications_get_plain_ctrl_v() {
        for class in ["google-chrome", "org.mozilla.firefox", "code"] {
            assert_not_terminal(class);
        }
    }

    #[test]
    fn an_empty_class_falls_back_to_plain_ctrl_v() {
        assert_not_terminal("");
    }

    #[test]
    fn matching_is_case_insensitive() {
        assert_terminal("GHOSTTY");
        assert_terminal("Kitty");
        assert_terminal("ALACRITTY");
        assert_not_terminal("GOOGLE-CHROME");
    }

    #[test]
    fn a_hyphenated_variant_of_a_known_terminal_is_still_caught() {
        assert_terminal("foot-extra");
        assert_terminal("kitty_wayland");
    }

    #[test]
    fn short_terminal_names_do_not_loosely_match_unrelated_classes() {
        assert_not_terminal("trio");
        assert_not_terminal("startpage");
        assert_not_terminal("mario");
    }

    #[test]
    fn a_non_terminal_class_containing_term_is_a_known_false_positive() {
        assert_terminal("com.example.TermsOfService");
    }

    #[test]
    fn a_reverse_dns_class_shows_only_its_last_segment_capitalized() {
        assert_eq!(display_name("com.mitchellh.ghostty"), "Ghostty");
    }

    #[test]
    fn a_plain_class_is_capitalized_as_is() {
        assert_eq!(display_name("foot"), "Foot");
        assert_eq!(display_name("Alacritty"), "Alacritty");
    }

    #[test]
    fn an_empty_class_produces_an_empty_name_rather_than_panicking() {
        assert_eq!(display_name(""), "");
    }
}
