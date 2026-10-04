//! This app's keyboard grammar: the shared parser, bound to [`Action`].
//!
//! The grammar itself — what a binding looks like, how it parses, what a
//! press resolves to, and how a `[keys]` table merges over the defaults —
//! lives in `hyprforge-keys`, because the image viewer needs the same one
//! and two copies would drift. What is left here is the part that is
//! genuinely about *this* app: the actions it binds, and the answer to the
//! one question on which two reasonable apps differ.
//!
//! That question is [`hyprforge_keys::BareKeys`]. The listing has a
//! type-to-search box, so a bare `n` types the letter n and must not be
//! bindable — see [`BARE_KEYS`]. A viewer with nowhere to type answers the
//! other way.
//!
//! Everything below is a re-export or an alias, so call sites that said
//! `keymap::Keymap` or `keymap::Resolved` before the extraction still do.

use crate::action::Action;

pub use hyprforge_keys::{Combo, ComboError, Key, KeyPress, Modifiers};

/// Every binding, in one table — this app's actions in the shared
/// [`hyprforge_keys::Keymap`].
pub type Keymap = hyprforge_keys::Keymap<Action>;

/// What a key press turned out to mean.
pub type Resolved = hyprforge_keys::Resolved<Action>;

/// This app reserves text-producing keys for typing, because the listing
/// has a search box a bare letter goes into.
pub const BARE_KEYS: hyprforge_keys::BareKeys = hyprforge_keys::BareKeys::ReservedForTyping;

/// The shipped bindings.
///
/// A free function rather than `Keymap::defaults()`: the shared type takes
/// the [`BARE_KEYS`] answer as an argument, and there is one right answer
/// for this app, so it is given here once rather than at every call.
pub fn defaults() -> Keymap {
    Keymap::defaults(BARE_KEYS)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    fn press(text: &str) -> KeyPress {
        let combo = Combo::parse(text).unwrap();
        KeyPress { key: combo.key, mods: combo.mods, text: None }
    }

    // The tests here are the ones that are claims about *this app's*
    // table rather than about the parser. The parser's own tests moved
    // to `hyprforge-keys` along with it; these could not, because they
    // name `Action`.

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
    /// behaviour anyone relied on. Written before the grammar was
    /// extracted into `hyprforge-keys` and kept unchanged through it,
    /// which is what makes it the regression net for that move.
    #[test]
    fn the_defaults_keep_every_binding_that_existed_before() {
        let keys = defaults();
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
        let keys = defaults();
        assert_eq!(keys.resolve(&press("Ctrl+Enter")), Some(Resolved::Action(Action::OpenInNewTab)));
        assert_eq!(keys.resolve(&press("Shift+Enter")), None, "Shift+Enter is not Enter");
        assert_eq!(keys.resolve(&press("Ctrl+0")), None, "there is no tab 0");
    }

    #[test]
    fn a_menu_hint_lists_every_key_an_action_has() {
        let keys = defaults();
        let hints: Vec<String> =
            keys.combos_for(Action::GoUp).iter().map(|c| c.to_string()).collect();
        assert_eq!(hints, ["Alt+Up", "Backspace"]);
    }

    /// Space is Quick Look — except between two words of a search being
    /// typed at the listing, where it is the space it looks like.
    #[test]
    fn space_is_quick_look_unless_a_search_is_being_typed() {
        let keys = defaults();
        let space = KeyPress { key: Key::Space, mods: Modifiers::default(), text: Some(' ') };
        assert_eq!(keys.resolve_typing(&space, false), Some(Resolved::Action(Action::QuickLook)));
        assert_eq!(keys.resolve_typing(&space, true), Some(Resolved::Text(' ')));
    }

    /// Space survives a round trip through the file: Preferences writes
    /// an action's whole line when a key is added to it, and a loader that
    /// refused a bare Space would drop Quick Look's own key on the way.
    #[test]
    fn a_line_naming_space_is_loaded_not_refused() {
        let line = [("quick-look".to_string(), vec!["Space".to_string(), "Ctrl+Y".to_string()])];
        let (keys, problems) = crate::config::keymap_with(&line.into_iter().collect());
        assert!(problems.is_empty(), "{problems:?}");
        let space = KeyPress { key: Key::Space, mods: Modifiers::default(), text: Some(' ') };
        assert_eq!(keys.resolve(&space), Some(Resolved::Action(Action::QuickLook)));
    }

    /// This app's answer to the one question the shared grammar leaves to
    /// its host, pinned: a bare letter goes into search, not to an action.
    #[test]
    fn a_bare_letter_still_types_into_search_here() {
        let keys = defaults();
        let n = KeyPress { key: Key::Char('n'), mods: Modifiers::default(), text: Some('n') };
        assert_eq!(keys.resolve(&n), Some(Resolved::Text('n')));
    }
}
