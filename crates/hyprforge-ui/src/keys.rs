//! From an iced key event to a [`hyprforge_keys::KeyPress`].
//!
//! The one adapter between the toolkit and the suite's keyboard grammar,
//! and the reason `hyprforge-keys` itself can stay free of iced: a crate
//! that only describes what a binding *is* should not need a GUI toolkit
//! to compile, and its tests should not need a window.
//!
//! It lives here rather than behind an optional feature on that crate
//! because a feature would not stay optional. Cargo unifies features
//! across the workspace, so any build containing one app would switch
//! iced on for every other consumer — the trap the root `Cargo.toml`
//! documents twice already. This crate *is* the iced layer, so nothing
//! is smuggled in by putting the adapter in it.
//!
//! Entirely action-agnostic: it knows which keys exist, not what any of
//! them do. That is why the file manager and the image viewer can share
//! it despite binding completely different things.

use hyprforge_keys::{Key, KeyPress, Modifiers};
use iced::keyboard::{self, key};

/// One iced key press, as the keymap reads it — or `None` for a key no
/// binding can name.
///
/// Two fields, deliberately from two places — the CLAUDE.md rule on the
/// three things iced reports for a key press:
///
/// - `key` comes from iced's *unmodified* `key`, which is what a binding
///   is matched against. Ctrl+Shift+N arrives as `n` with Ctrl and Shift
///   held, which is exactly how the binding is written.
/// - `text` comes from iced's `text`, what the press actually typed, and
///   is only ever used for typing into a field. Reading `key` for that
///   would turn `Shift+/` into `/` — and, in the case this rule was
///   written for, cost a password every capital it had.
pub fn key_press(event: &keyboard::Event) -> Option<KeyPress> {
    let keyboard::Event::KeyPressed { key: pressed, modifiers, text, .. } = event else {
        return None;
    };
    let mods = Modifiers {
        ctrl: modifiers.control(),
        alt: modifiers.alt(),
        shift: modifiers.shift(),
        logo: modifiers.logo(),
    };
    let key = match pressed.as_ref() {
        keyboard::Key::Named(named) => match named {
            key::Named::Enter => Key::Enter,
            key::Named::Backspace => Key::Backspace,
            key::Named::Escape => Key::Escape,
            key::Named::Delete => Key::Delete,
            key::Named::Tab => Key::Tab,
            key::Named::Space => Key::Space,
            key::Named::ArrowUp => Key::Up,
            key::Named::ArrowDown => Key::Down,
            key::Named::ArrowLeft => Key::Left,
            key::Named::ArrowRight => Key::Right,
            key::Named::Home => Key::Home,
            key::Named::End => Key::End,
            key::Named::PageUp => Key::PageUp,
            key::Named::PageDown => Key::PageDown,
            key::Named::Insert => Key::Insert,
            key::Named::ContextMenu => Key::Menu,
            other => Key::F(function_key_number(other)?),
        },
        keyboard::Key::Character(c) => {
            let mut chars = c.chars();
            match (chars.next(), chars.next()) {
                (Some(ch), None) => Key::Char(ch.to_lowercase().next().unwrap_or(ch)),
                _ => return None,
            }
        }
        _ => return None,
    };
    let text = text.as_ref().and_then(|t| {
        let mut chars = t.chars();
        match (chars.next(), chars.next()) {
            (Some(ch), None) => Some(ch),
            _ => None,
        }
    });
    Some(KeyPress { key, mods, text })
}

fn function_key_number(named: key::Named) -> Option<u8> {
    use key::Named as N;
    let n = match named {
        N::F1 => 1,
        N::F2 => 2,
        N::F3 => 3,
        N::F4 => 4,
        N::F5 => 5,
        N::F6 => 6,
        N::F7 => 7,
        N::F8 => 8,
        N::F9 => 9,
        N::F10 => 10,
        N::F11 => 11,
        N::F12 => 12,
        N::F13 => 13,
        N::F14 => 14,
        N::F15 => 15,
        N::F16 => 16,
        N::F17 => 17,
        N::F18 => 18,
        N::F19 => 19,
        N::F20 => 20,
        N::F21 => 21,
        N::F22 => 22,
        N::F23 => 23,
        N::F24 => 24,
        _ => return None,
    };
    Some(n)
}

#[cfg(test)]
mod tests {
    use super::*;
    use iced::keyboard::Modifiers as IcedModifiers;

    fn pressed(key: keyboard::Key, mods: IcedModifiers, text: Option<&str>) -> keyboard::Event {
        keyboard::Event::KeyPressed {
            key: key.clone(),
            modified_key: key.clone(),
            physical_key: keyboard::key::Physical::Unidentified(
                keyboard::key::NativeCode::Unidentified,
            ),
            location: keyboard::Location::Standard,
            modifiers: mods,
            text: text.map(Into::into),
            repeat: false,
        }
    }

    fn character(c: &str) -> keyboard::Key {
        keyboard::Key::Character(c.into())
    }

    /// The whole reason both fields are read: the binding matches on the
    /// unmodified key, and what gets typed is what the press produced.
    #[test]
    fn a_shifted_letter_keeps_its_lowercase_key_and_its_typed_capital() {
        let event = pressed(character("N"), IcedModifiers::SHIFT, Some("N"));
        let press = key_press(&event).unwrap();
        assert_eq!(press.key, Key::Char('n'));
        assert!(press.mods.shift);
        assert_eq!(press.text, Some('N'));
    }

    #[test]
    fn a_named_key_becomes_the_key_a_binding_can_name() {
        let event = pressed(keyboard::Key::Named(key::Named::ArrowLeft), IcedModifiers::ALT, None);
        let press = key_press(&event).unwrap();
        assert_eq!(press.key, Key::Left);
        assert!(press.mods.alt);
    }

    #[test]
    fn function_keys_arrive_by_number() {
        let event = pressed(keyboard::Key::Named(key::Named::F2), IcedModifiers::empty(), None);
        assert_eq!(key_press(&event).unwrap().key, Key::F(2));
    }

    /// A key with no name in this grammar is `None` rather than a guess —
    /// a binding that cannot be written cannot be matched.
    #[test]
    fn a_key_no_binding_can_name_is_not_invented() {
        let event =
            pressed(keyboard::Key::Named(key::Named::MediaPlay), IcedModifiers::empty(), None);
        assert_eq!(key_press(&event), None);
    }

    #[test]
    fn a_release_is_not_a_press() {
        let event = keyboard::Event::KeyReleased {
            key: character("n"),
            modified_key: character("n"),
            physical_key: keyboard::key::Physical::Unidentified(
                keyboard::key::NativeCode::Unidentified,
            ),
            location: keyboard::Location::Standard,
            modifiers: IcedModifiers::empty(),
        };
        assert_eq!(key_press(&event), None);
    }
}
