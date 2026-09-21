//! One key press, as the host saw it — and the rule that it never leaks.
//!
//! This is a file of its own for the `Debug` impl below, not for the
//! struct. That impl *is* CLAUDE.md's keystroke rule made executable, and
//! buried two hundred lines into a module about parsing it was easy to
//! miss. Here it is the first thing anyone adding a field sees.

use crate::combo::{Key, Modifiers};
use std::fmt;

/// One key press, as the host saw it.
///
/// The three fields are the three things a toolkit reports about a press,
/// and only one of them is what was typed — the distinction CLAUDE.md
/// records after a lock screen rejected passwords it had been given
/// correctly. `key` is the *unmodified* logical key and is what a binding
/// matches on; `text` is what the press produced and is the only thing
/// that may be treated as input.
#[derive(Clone, Copy, PartialEq, Eq)]
pub struct KeyPress {
    /// The key, *without* modifiers applied — iced's `key`, never its
    /// `modified_key`. A binding matches on this.
    pub key: Key,
    pub mods: Modifiers,
    /// What the press *typed*, if anything — iced's `text`. Only ever
    /// used for typing into a field, because `key` is unmodified and
    /// reading it for text would turn `Shift+/` into `/`.
    pub text: Option<char>,
}

/// Hand-written so a keystroke never reaches a debug dump — see the
/// crate doc. The typed character, and a character key, render as a
/// placeholder; named keys and modifiers are not text and are shown.
///
/// A keysym name *is* the character: rendering `Key::Char('a')` as
/// `Char('a')` writes the letter to the log just as surely as printing
/// `text` would. Both are replaced.
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

#[cfg(test)]
mod tests {
    use super::*;

    /// The keystroke rule, pinned: nothing a user typed survives `Debug`.
    #[test]
    fn debugging_a_key_press_never_shows_what_was_typed() {
        // A character that cannot appear in the struct's own field names,
        // so finding it anywhere in the output means it leaked.
        let p = KeyPress { key: Key::Char('ž'), mods: Modifiers::default(), text: Some('ž') };
        let shown = format!("{p:?}");
        assert!(!shown.contains('ž'), "{shown}");
        assert!(!shown.contains("Char"), "{shown}");
        assert!(shown.contains("<char>"));
    }

    /// A named key is not text and is worth seeing in a log — the rule
    /// redacts what was typed, not everything.
    #[test]
    fn debugging_a_named_key_still_says_which_key_it_was() {
        let p = KeyPress { key: Key::Escape, mods: Modifiers::default(), text: None };
        assert!(format!("{p:?}").contains("Escape"));
    }
}
