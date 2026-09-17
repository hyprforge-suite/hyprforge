//! What a key press means, as one table.
//!
//! The host turns an `iced::keyboard::Event` into a [`KeyPress`]; a
//! [`Keymap`] turns that into either an [`Action`] or a character to
//! type into the search box. The app window and the portal's dialog
//! share both halves, so the two can never disagree about what Ctrl+A
//! does.
//!
//! Bindings are written in the syntax `files-config.toml` uses — `Ctrl+Shift+N`,
//! `Alt+Left`, `F2` — and parsed by [`Combo::parse`]. The shipped
//! defaults go through the same parser ([`Action::default_keys`]), so
//! there is no second hand-built table for the config to drift from.
//!
//! Deliberately free of `iced::keyboard` types: [`Key`] names only the
//! keys a file manager binds, so this module's tests need no window.
//!
//! **Nothing here may log what was typed.** A [`KeyPress`] carries the
//! character a key produced, and CLAUDE.md's rule — never write anything
//! derived from a keystroke to a log, a panic message or a debug dump —
//! is kept by a hand-written `Debug` that renders that character as
//! `<char>`. A binding *string from the config file* is fine to print;
//! a key the user pressed is not.

use crate::action::Action;
use std::collections::HashMap;
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
    /// F1 to F24.
    F(u8),
    /// A character key, always stored lowercase — `Ctrl+N` and
    /// `Ctrl+Shift+N` differ by their modifiers, not by the letter's
    /// case, which is also how iced reports the unmodified key.
    Char(char),
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
            Key::F(n) => write!(f, "F{n}"),
            Key::Char('+') => f.write_str("Plus"),
            Key::Char('-') => f.write_str("Minus"),
            Key::Char(c) => write!(f, "{}", c.to_uppercase()),
        }
    }
}

/// One key press, as the host saw it.
#[derive(Clone, Copy, PartialEq, Eq)]
pub struct KeyPress {
    /// The key, *without* modifiers applied — iced's `key`, never its
    /// `modified_key`. A binding matches on this.
    pub key: Key,
    pub mods: Modifiers,
    /// What the press *typed*, if anything — iced's `text`. Only ever
    /// used for typing into search, because `key` is unmodified and
    /// reading it for text would turn `Shift+/` into `/`.
    pub text: Option<char>,
}

/// Hand-written so a keystroke never reaches a debug dump — see the
/// module doc. The typed character, and a character key, render as a
/// placeholder; named keys and modifiers are not text and are shown.
impl fmt::Debug for KeyPress {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let key: &dyn fmt::Debug = match self.key {
            Key::Char(_) => &"<char>",
            ref named => named,
        };
        f.debug_struct("KeyPress")
            .field("key", key)
            .field("mods", &self.mods)
            .field("text", &self.text.map(|_| "<char>"))
            .finish()
    }
}

/// What a key press turned out to mean.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Resolved {
    Action(Action),
    /// Type this into the search box.
    Type(char),
}

/// Every binding, in one table.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Keymap {
    bindings: HashMap<Combo, Action>,
}

impl Default for Keymap {
    fn default() -> Self {
        Keymap::defaults()
    }
}

impl Keymap {
    /// The shipped bindings, parsed from [`Action::default_keys`].
    pub fn defaults() -> Keymap {
        let mut bindings = HashMap::new();
        for action in Action::all() {
            for text in action.default_keys() {
                // The defaults are compiled in, so a parse failure here is
                // a bug in this crate — and `every_default_binding_parses`
                // fails on it long before anyone runs the app. Skipping
                // rather than panicking keeps a typo from taking the
                // keyboard down with it.
                if let Ok(combo) = Combo::parse(text) {
                    bindings.insert(combo, action);
                }
            }
        }
        Keymap { bindings }
    }

    /// A keymap holding exactly these bindings — how
    /// [`crate::config`] builds one after merging a file over the
    /// defaults.
    pub fn from_bindings(bindings: impl IntoIterator<Item = (Combo, Action)>) -> Keymap {
        Keymap { bindings: bindings.into_iter().collect() }
    }

    /// What `press` means, or `None` if nothing.
    ///
    /// A binding wins over typing, and an exact match is required — the
    /// modifiers held must be exactly the binding's, so Ctrl+Enter is
    /// never also Enter.
    ///
    /// Otherwise a press that typed a printable character with neither
    /// Ctrl nor Alt held types into search. Ctrl- and Alt-held
    /// characters are always somebody's shortcut and must never leak into
    /// the search box, bound or not.
    pub fn resolve(&self, press: &KeyPress) -> Option<Resolved> {
        let combo = Combo { key: press.key, mods: press.mods };
        if let Some(action) = self.bindings.get(&combo) {
            return Some(Resolved::Action(*action));
        }
        if press.mods.ctrl || press.mods.alt || press.mods.logo {
            return None;
        }
        press
            .text
            .filter(|c| !c.is_control())
            .map(Resolved::Type)
    }

    /// Every combo bound to `action`, sorted so a menu's hint is stable.
    pub fn combos_for(&self, action: Action) -> Vec<Combo> {
        let mut combos: Vec<Combo> =
            self.bindings.iter().filter(|(_, a)| **a == action).map(|(c, _)| *c).collect();
        combos.sort_by_key(|c| c.to_string());
        combos
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn press(text: &str) -> KeyPress {
        let combo = Combo::parse(text).unwrap();
        KeyPress { key: combo.key, mods: combo.mods, text: None }
    }

    fn typed(c: char) -> KeyPress {
        KeyPress {
            key: Key::Char(c.to_ascii_lowercase()),
            mods: Modifiers { shift: c.is_uppercase(), ..Modifiers::default() },
            text: Some(c),
        }
    }

    #[test]
    fn every_default_binding_parses() {
        for action in Action::all() {
            for text in action.default_keys() {
                assert!(Combo::parse(text).is_ok(), "{action:?}: {text}");
            }
        }
    }

    /// Two actions on one key would make one of them unreachable, and
    /// which one would depend on `HashMap` iteration order.
    #[test]
    fn no_two_actions_share_a_default_key() {
        let mut seen: HashMap<Combo, Action> = HashMap::new();
        for action in Action::all() {
            for text in action.default_keys() {
                let combo = Combo::parse(text).unwrap();
                if let Some(other) = seen.insert(combo, action) {
                    panic!("{text} is bound to both {other:?} and {action:?}");
                }
            }
        }
    }

    /// The table this replaced, key for key, so moving to it changed no
    /// behaviour anyone relied on.
    #[test]
    fn the_defaults_keep_every_binding_that_existed_before() {
        let keys = Keymap::defaults();
        let expect = [
            ("Enter", Action::Open),
            ("Backspace", Action::GoUp),
            ("Alt+Left", Action::GoBack),
            ("Escape", Action::ClearSearch),
            ("Up", Action::FocusUp),
            ("Down", Action::FocusDown),
            ("Left", Action::FocusLeft),
            ("Right", Action::FocusRight),
            ("Delete", Action::Trash),
            ("Ctrl+T", Action::NewTab),
            ("Ctrl+W", Action::CloseTab),
            ("Ctrl+Tab", Action::NextTab),
            ("Ctrl+Shift+Tab", Action::PreviousTab),
            ("Ctrl+1", Action::Tab(1)),
            ("Ctrl+9", Action::Tab(9)),
        ];
        for (text, action) in expect {
            assert_eq!(keys.resolve(&press(text)), Some(Resolved::Action(action)), "{text}");
        }
    }

    #[test]
    fn a_binding_needs_its_exact_modifiers() {
        let keys = Keymap::defaults();
        assert_eq!(keys.resolve(&press("Ctrl+Enter")), Some(Resolved::Action(Action::OpenInNewTab)));
        assert_eq!(keys.resolve(&press("Shift+Enter")), None, "Shift+Enter is not Enter");
        assert_eq!(keys.resolve(&press("Ctrl+0")), None, "there is no tab 0");
    }

    #[test]
    fn a_plain_character_types_into_search_with_its_case() {
        let keys = Keymap::defaults();
        assert_eq!(keys.resolve(&typed('r')), Some(Resolved::Type('r')));
        assert_eq!(keys.resolve(&typed('R')), Some(Resolved::Type('R')));
    }

    /// Reading the unmodified key for text would turn Shift+/ into `/`.
    /// The typed character is what goes into search.
    #[test]
    fn typing_uses_what_the_press_produced_not_the_key() {
        let keys = Keymap::defaults();
        let question = KeyPress {
            key: Key::Char('/'),
            mods: Modifiers { shift: true, ..Modifiers::default() },
            text: Some('?'),
        };
        assert_eq!(keys.resolve(&question), Some(Resolved::Type('?')));
    }

    #[test]
    fn a_ctrl_held_character_never_reaches_search() {
        let keys = Keymap::defaults();
        let ctrl_q = KeyPress {
            key: Key::Char('q'),
            mods: Modifiers { ctrl: true, ..Modifiers::default() },
            text: Some('q'),
        };
        assert_eq!(keys.resolve(&ctrl_q), None);
    }

    /// Enter, Backspace and Escape produce text of their own (`\r`,
    /// `\u{8}`, `\u{1b}`). That text must never reach the search box —
    /// the CLAUDE.md rule about iced's three key fields.
    #[test]
    fn a_control_character_is_never_typed() {
        let mut keys = Keymap::defaults();
        keys.bindings.clear();
        for c in ['\r', '\u{8}', '\u{1b}', '\t'] {
            let p = KeyPress { key: Key::Enter, mods: Modifiers::default(), text: Some(c) };
            assert_eq!(keys.resolve(&p), None);
        }
    }

    #[test]
    fn parsing_is_case_insensitive_and_round_trips() {
        for text in ["Ctrl+Shift+N", "Alt+Left", "F2", "Delete", "Ctrl+Plus", "Super+E", "Ctrl+Shift+Tab"] {
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

    /// The keystroke rule, pinned: nothing a user typed survives `Debug`.
    #[test]
    fn debugging_a_key_press_never_shows_what_was_typed() {
        // A character that cannot appear in the struct's own field names,
        // so finding it anywhere in the output means it leaked.
        let p = KeyPress {
            key: Key::Char('ž'),
            mods: Modifiers::default(),
            text: Some('ž'),
        };
        let shown = format!("{p:?}");
        assert!(!shown.contains('ž'), "{shown}");
        assert!(!shown.contains("Char"), "{shown}");
        assert!(shown.contains("<char>"));
    }

    #[test]
    fn a_menu_hint_lists_every_key_an_action_has() {
        let keys = Keymap::defaults();
        let hints: Vec<String> = keys.combos_for(Action::GoUp).iter().map(|c| c.to_string()).collect();
        assert_eq!(hints, ["Alt+Up", "Backspace"]);
    }
}
