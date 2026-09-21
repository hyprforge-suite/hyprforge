//! A binding: which key, and which modifiers held with it.
//!
//! Bindings are written the way the config files spell them —
//! `Ctrl+Shift+N`, `Alt+Left`, `F2` — and every shipped default goes
//! through this same parser, so there is no second hand-built table for a
//! config file to drift from.
//!
//! [`Key`] deliberately names only keys an application binds, rather than
//! mirroring a toolkit's key enum: that is what lets this module's tests
//! run without a window.

use std::fmt;

/// The keys a binding can name.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Key {
    Enter,
    Backspace,
    Escape,
    Delete,
    Tab,
    Space,
    Up,
    Down,
    Left,
    Right,
    Home,
    End,
    PageUp,
    PageDown,
    Insert,
    /// The Menu (application) key.
    Menu,
    /// F1 to F24.
    F(u8),
    /// A character key, always stored lowercase — `Ctrl+N` and
    /// `Ctrl+Shift+N` differ by their modifiers, not by the letter's
    /// case, which is also how iced reports the unmodified key.
    Char(char),
}

impl Key {
    /// Whether pressing this key produces a character someone might mean
    /// to type.
    ///
    /// The question [`crate::BareKeys`] exists to answer: in an app with
    /// somewhere to type, binding one of these without a modifier would
    /// swallow the character.
    pub fn types_text(self) -> bool {
        matches!(self, Key::Char(_) | Key::Space)
    }
}

/// Which modifier keys were held.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, PartialOrd, Ord)]
pub struct Modifiers {
    pub ctrl: bool,
    pub alt: bool,
    pub shift: bool,
    /// The Super/Logo key. Parsed so a binding naming it is not an
    /// error, though the compositor will usually take such combos first.
    pub logo: bool,
}

impl Modifiers {
    /// Whether any modifier that makes a key "somebody's shortcut" is
    /// held. Shift is deliberately not one of them: `Shift+A` is how a
    /// capital A gets typed.
    pub fn any_shortcut_modifier(self) -> bool {
        self.ctrl || self.alt || self.logo
    }
}

/// A key and the modifiers held with it — one binding.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct Combo {
    pub key: Key,
    pub mods: Modifiers,
}

/// Why a binding string would not parse.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ComboError {
    #[error("the binding is empty")]
    Empty,
    #[error("`{0}` is not a modifier this app knows (use Ctrl, Alt, Shift or Super)")]
    UnknownModifier(String),
    #[error("`{0}` is not a key this app knows")]
    UnknownKey(String),
    #[error("the binding names no key after its modifiers")]
    NoKey,
}

impl Combo {
    /// Parses `Ctrl+Shift+N`, `Alt+Left`, `F2`, `Delete`.
    ///
    /// Case-insensitive throughout, `+` between parts, the key last.
    /// A literal plus is spelled `Plus`, since `Ctrl++` has no
    /// unambiguous reading.
    pub fn parse(text: &str) -> Result<Combo, ComboError> {
        let text = text.trim();
        if text.is_empty() {
            return Err(ComboError::Empty);
        }
        let parts: Vec<&str> = text.split('+').map(str::trim).collect();
        let (key_part, modifier_parts) = parts.split_last().ok_or(ComboError::Empty)?;
        if key_part.is_empty() {
            return Err(ComboError::NoKey);
        }
        let mut mods = Modifiers::default();
        for part in modifier_parts {
            match part.to_ascii_lowercase().as_str() {
                "ctrl" | "control" => mods.ctrl = true,
                "alt" => mods.alt = true,
                "shift" => mods.shift = true,
                "super" | "logo" | "meta" | "mod4" => mods.logo = true,
                _ => return Err(ComboError::UnknownModifier((*part).to_string())),
            }
        }
        Ok(Combo { key: parse_key(key_part)?, mods })
    }

    /// Whether binding this would swallow a character a person meant to
    /// type — a text-producing key with no Ctrl, Alt or Super held.
    ///
    /// Only meaningful for an app that *has* somewhere to type; see
    /// [`crate::BareKeys`].
    pub fn would_swallow_typing(&self) -> bool {
        self.key.types_text() && !self.mods.any_shortcut_modifier()
    }
}

fn parse_key(part: &str) -> Result<Key, ComboError> {
    let lower = part.to_ascii_lowercase();
    let key = match lower.as_str() {
        "enter" | "return" => Key::Enter,
        "backspace" => Key::Backspace,
        "escape" | "esc" => Key::Escape,
        "delete" | "del" => Key::Delete,
        "tab" => Key::Tab,
        "space" => Key::Space,
        "up" => Key::Up,
        "down" => Key::Down,
        "left" => Key::Left,
        "right" => Key::Right,
        "home" => Key::Home,
        "end" => Key::End,
        "pageup" => Key::PageUp,
        "pagedown" => Key::PageDown,
        "insert" | "ins" => Key::Insert,
        "menu" | "apps" => Key::Menu,
        "plus" => Key::Char('+'),
        "minus" => Key::Char('-'),
        _ => {
            if let Some(n) = lower.strip_prefix('f').and_then(|n| n.parse::<u8>().ok()) {
                if (1..=24).contains(&n) {
                    return Ok(Key::F(n));
                }
                return Err(ComboError::UnknownKey(part.to_string()));
            }
            let mut chars = part.chars();
            match (chars.next(), chars.next()) {
                (Some(c), None) if !c.is_control() && !c.is_whitespace() => {
                    Key::Char(c.to_lowercase().next().unwrap_or(c))
                }
                _ => return Err(ComboError::UnknownKey(part.to_string())),
            }
        }
    };
    Ok(key)
}

/// Renders a binding the way [`Combo::parse`] reads it, so a menu's
/// shortcut hint and the config file spell a binding identically.
impl fmt::Display for Combo {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.mods.ctrl {
            f.write_str("Ctrl+")?;
        }
        if self.mods.alt {
            f.write_str("Alt+")?;
        }
        if self.mods.shift {
            f.write_str("Shift+")?;
        }
        if self.mods.logo {
            f.write_str("Super+")?;
        }
        match self.key {
            Key::Enter => f.write_str("Enter"),
            Key::Backspace => f.write_str("Backspace"),
            Key::Escape => f.write_str("Escape"),
            Key::Delete => f.write_str("Delete"),
            Key::Tab => f.write_str("Tab"),
            Key::Space => f.write_str("Space"),
            Key::Up => f.write_str("Up"),
            Key::Down => f.write_str("Down"),
            Key::Left => f.write_str("Left"),
            Key::Right => f.write_str("Right"),
            Key::Home => f.write_str("Home"),
            Key::End => f.write_str("End"),
            Key::PageUp => f.write_str("PageUp"),
            Key::PageDown => f.write_str("PageDown"),
            Key::Insert => f.write_str("Insert"),
            Key::Menu => f.write_str("Menu"),
            Key::F(n) => write!(f, "F{n}"),
            Key::Char('+') => f.write_str("Plus"),
            Key::Char('-') => f.write_str("Minus"),
            Key::Char(c) => write!(f, "{}", c.to_uppercase()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parsing_is_case_insensitive_and_round_trips() {
        for text in
            ["Ctrl+Shift+N", "Alt+Left", "F2", "Delete", "Ctrl+Plus", "Super+E", "Ctrl+Shift+Tab"]
        {
            let combo = Combo::parse(text).unwrap();
            assert_eq!(combo.to_string(), text);
            assert_eq!(Combo::parse(&text.to_lowercase()).unwrap(), combo);
        }
    }

    #[test]
    fn a_bad_binding_says_which_part_is_wrong() {
        assert_eq!(Combo::parse(""), Err(ComboError::Empty));
        assert_eq!(Combo::parse("Hyper+X"), Err(ComboError::UnknownModifier("Hyper".into())));
        assert_eq!(Combo::parse("Ctrl+Banana"), Err(ComboError::UnknownKey("Banana".into())));
        assert_eq!(Combo::parse("F25"), Err(ComboError::UnknownKey("F25".into())));
        assert_eq!(Combo::parse("Ctrl+"), Err(ComboError::NoKey));
    }

    /// Shift is not a shortcut modifier: `Shift+A` is how a capital A is
    /// typed, so it does not rescue a bare letter from swallowing text.
    #[test]
    fn only_ctrl_alt_and_super_make_a_text_key_safe_to_bind() {
        assert!(Combo::parse("N").unwrap().would_swallow_typing());
        assert!(Combo::parse("Shift+N").unwrap().would_swallow_typing());
        assert!(Combo::parse("Space").unwrap().would_swallow_typing());
        assert!(!Combo::parse("Ctrl+N").unwrap().would_swallow_typing());
        assert!(!Combo::parse("Alt+N").unwrap().would_swallow_typing());
        assert!(!Combo::parse("Super+N").unwrap().would_swallow_typing());
        // A named key produces no text of its own to swallow.
        assert!(!Combo::parse("Delete").unwrap().would_swallow_typing());
        assert!(!Combo::parse("F2").unwrap().would_swallow_typing());
    }
}
