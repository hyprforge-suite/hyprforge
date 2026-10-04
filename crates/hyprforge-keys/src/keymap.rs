//! What a key press means, as one table.

use crate::combo::Combo;
use crate::press::KeyPress;
use std::collections::HashMap;
use std::fmt::Debug;
use std::hash::Hash;

/// What an app can bind a key to.
///
/// Deliberately not named `Action`: every app implementing this has its
/// own enum by that name, and a trait sharing it could not be imported
/// alongside one.
///
/// Four methods, and an app's existing action enum almost certainly has
/// all four already — the ids and the default keys are things a config
/// file format needs regardless of who parses it.
pub trait Bindable: Copy + Eq + Ord + Hash + Debug + 'static {
    /// Every action, in the order a settings page or a test would list
    /// them. A `Vec` rather than a slice because an app may expand a
    /// parameterised variant at runtime — the file manager's `Tab(1..=9)`.
    fn all() -> Vec<Self>;

    /// The stable id the config file names (`"trash"`, `"tab-3"`).
    /// Part of the file format: renaming one breaks somebody's config.
    fn id(self) -> &'static str;

    /// The shipped bindings, in the syntax [`Combo::parse`] reads. The
    /// defaults go through the same parser as the config file, so there
    /// is no second table for one to drift from.
    fn default_keys(self) -> &'static [&'static str];

    /// [`Bindable::id`] in reverse. Provided as a scan over
    /// [`Bindable::all`]; an app with a large table is free to beat it.
    fn from_id(id: &str) -> Option<Self> {
        Self::all().into_iter().find(|a| a.id() == id)
    }
}

/// Whether keys that produce text are reserved for typing.
///
/// The one axis on which two perfectly reasonable apps disagree, and it
/// is a property of the app rather than a preference:
///
/// - A file manager has a search box. A bare `n` types the letter n, so
///   binding it would swallow the character and the binding is refused.
/// - An image viewer has nowhere to type at all. Bare `n`, `p`, `f`, `i`,
///   `+` and `-` are its entire grammar, as they are in every image
///   viewer anyone has used.
///
/// Forcing the first answer on the second is quiet and confusing: the
/// viewer's own defaults are refused at load, and the message tells the
/// user about a search box the app does not have.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum BareKeys {
    /// The app has somewhere to type. A bare printable key types into it
    /// and cannot be bound.
    ///
    /// The default, because it is the conservative answer: an app that
    /// gets this wrong refuses a binding, where the other way round it
    /// would silently eat a character the user meant to type.
    #[default]
    ReservedForTyping,
    /// The app has nowhere to type, so `n` is just a key.
    Bindable,
}

/// What a key press turned out to mean.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Resolved<A> {
    Action(A),
    /// A printable character that no binding claimed. Type it into
    /// whatever field has focus.
    ///
    /// Never produced under [`BareKeys::Bindable`] — an app with nowhere
    /// to type has nothing to do with this and should not have to match
    /// on it.
    Text(char),
}

/// Every binding, in one table.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Keymap<A> {
    bindings: HashMap<Combo, A>,
    bare: BareKeys,
}

impl<A: Bindable> Default for Keymap<A> {
    fn default() -> Self {
        Keymap::defaults(BareKeys::default())
    }
}

impl<A: Bindable> Keymap<A> {
    /// The shipped bindings, parsed from [`Bindable::default_keys`].
    pub fn defaults(bare: BareKeys) -> Keymap<A> {
        let mut bindings = HashMap::new();
        for action in A::all() {
            for text in action.default_keys() {
                // The defaults are compiled in, so a parse failure here is
                // a bug in the *app's* table — and a test like
                // `every_default_binding_parses` fails on it long before
                // anyone runs the app. Skipping rather than panicking
                // keeps a typo from taking the keyboard down with it.
                if let Ok(combo) = Combo::parse(text) {
                    bindings.insert(combo, action);
                }
            }
        }
        Keymap { bindings, bare }
    }

    /// A keymap holding exactly these bindings — how [`crate::merge`]
    /// builds one after applying a config file over the defaults.
    pub fn from_bindings(
        bindings: impl IntoIterator<Item = (Combo, A)>,
        bare: BareKeys,
    ) -> Keymap<A> {
        Keymap { bindings: bindings.into_iter().collect(), bare }
    }

    /// Whether this app reserves text-producing keys for typing.
    pub fn bare_keys(&self) -> BareKeys {
        self.bare
    }

    /// What `press` means, or `None` if nothing.
    ///
    /// A binding wins over typing, and an exact match is required — the
    /// modifiers held must be exactly the binding's, so Ctrl+Enter is
    /// never also Enter.
    ///
    /// Otherwise, and only under [`BareKeys::ReservedForTyping`], a press
    /// that produced a printable character with neither Ctrl nor Alt nor
    /// Super held is text. Ctrl- and Alt-held characters are always
    /// somebody's shortcut and must never leak into a text field, bound
    /// or not.
    pub fn resolve(&self, press: &KeyPress) -> Option<Resolved<A>> {
        let combo = Combo { key: press.key, mods: press.mods };
        if let Some(action) = self.bindings.get(&combo) {
            return Some(Resolved::Action(*action));
        }
        if self.bare == BareKeys::Bindable {
            return None;
        }
        if press.mods.any_shortcut_modifier() {
            return None;
        }
        // Enter, Backspace, Escape and Tab produce text of their own
        // (`\r`, `\u{8}`, `\u{1b}`, `\t`). That text must never reach a
        // field — CLAUDE.md's rule about iced's three key fields, and the
        // reason a password once lost every capital.
        press.text.filter(|c| !c.is_control()).map(Resolved::Text)
    }

    /// [`Keymap::resolve`], for an app that knows whether something is
    /// being typed right now — a type-to-search query under way, say.
    ///
    /// While `typing`, a Space with neither Ctrl, Alt nor Super held is
    /// text whatever it is bound to: it is the gap between two words of
    /// the query, not a request. Otherwise exactly [`Keymap::resolve`].
    /// The other half of the rule that lets a bare Space be bound at
    /// all — see [`Combo::would_swallow_typing`].
    pub fn resolve_typing(&self, press: &KeyPress, typing: bool) -> Option<Resolved<A>> {
        if typing
            && self.bare == BareKeys::ReservedForTyping
            && press.key == crate::combo::Key::Space
            && !press.mods.any_shortcut_modifier()
        {
            if let Some(c) = press.text.filter(|c| !c.is_control()) {
                return Some(Resolved::Text(c));
            }
        }
        self.resolve(press)
    }

    /// Every combo bound to `action`, sorted so a menu's hint is stable.
    pub fn combos_for(&self, action: A) -> Vec<Combo> {
        let mut combos: Vec<Combo> =
            self.bindings.iter().filter(|(_, a)| **a == action).map(|(c, _)| *c).collect();
        combos.sort_by_key(|c| c.to_string());
        combos
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::combo::{Key, Modifiers};

    /// A stand-in for an app's action enum, with one of each interesting
    /// shape: a named-key binding, a modified one, two keys on one
    /// action, and a bare letter only a viewer could bind.
    #[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
    enum TestAction {
        Open,
        GoUp,
        NewTab,
        Next,
    }

    impl Bindable for TestAction {
        fn all() -> Vec<Self> {
            vec![TestAction::Open, TestAction::GoUp, TestAction::NewTab, TestAction::Next]
        }

        fn id(self) -> &'static str {
            match self {
                TestAction::Open => "open",
                TestAction::GoUp => "go-up",
                TestAction::NewTab => "new-tab",
                TestAction::Next => "next",
            }
        }

        fn default_keys(self) -> &'static [&'static str] {
            match self {
                TestAction::Open => &["Enter"],
                TestAction::GoUp => &["Backspace", "Alt+Up"],
                TestAction::NewTab => &["Ctrl+T"],
                TestAction::Next => &["N"],
            }
        }
    }

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

    fn typing_app() -> Keymap<TestAction> {
        Keymap::defaults(BareKeys::ReservedForTyping)
    }

    #[test]
    fn a_binding_needs_its_exact_modifiers() {
        let keys = typing_app();
        assert_eq!(keys.resolve(&press("Ctrl+T")), Some(Resolved::Action(TestAction::NewTab)));
        assert_eq!(keys.resolve(&press("Shift+Enter")), None, "Shift+Enter is not Enter");
        assert_eq!(keys.resolve(&press("Ctrl+Shift+T")), None);
    }

    #[test]
    fn a_plain_character_types_into_a_field_with_its_case() {
        let keys = typing_app();
        assert_eq!(keys.resolve(&typed('r')), Some(Resolved::Text('r')));
        assert_eq!(keys.resolve(&typed('R')), Some(Resolved::Text('R')));
    }

    /// Reading the unmodified key for text would turn Shift+/ into `/`.
    /// The typed character is what goes into the field.
    #[test]
    fn typing_uses_what_the_press_produced_not_the_key() {
        let keys = typing_app();
        let question = KeyPress {
            key: Key::Char('/'),
            mods: Modifiers { shift: true, ..Modifiers::default() },
            text: Some('?'),
        };
        assert_eq!(keys.resolve(&question), Some(Resolved::Text('?')));
    }

    #[test]
    fn a_ctrl_held_character_never_reaches_a_text_field() {
        let keys = typing_app();
        let ctrl_q = KeyPress {
            key: Key::Char('q'),
            mods: Modifiers { ctrl: true, ..Modifiers::default() },
            text: Some('q'),
        };
        assert_eq!(keys.resolve(&ctrl_q), None);
    }

    /// Enter, Backspace and Escape produce text of their own (`\r`,
    /// `\u{8}`, `\u{1b}`). That text must never reach a text field —
    /// the CLAUDE.md rule about iced's three key fields.
    #[test]
    fn a_control_character_is_never_typed() {
        let keys: Keymap<TestAction> =
            Keymap::from_bindings([], BareKeys::ReservedForTyping);
        for c in ['\r', '\u{8}', '\u{1b}', '\t'] {
            let p = KeyPress { key: Key::Enter, mods: Modifiers::default(), text: Some(c) };
            assert_eq!(keys.resolve(&p), None);
        }
    }

    /// The whole point of [`BareKeys`]: in an app with nowhere to type,
    /// a bare letter is a binding like any other.
    #[test]
    fn a_viewer_with_nowhere_to_type_can_bind_a_bare_letter() {
        let keys: Keymap<TestAction> = Keymap::defaults(BareKeys::Bindable);
        assert_eq!(keys.resolve(&typed('n')), Some(Resolved::Action(TestAction::Next)));
    }

    /// And it never reports text, even for a character nothing claimed —
    /// an app with no field has nothing to do with it.
    #[test]
    fn a_viewer_never_resolves_a_press_to_text() {
        let keys: Keymap<TestAction> = Keymap::defaults(BareKeys::Bindable);
        assert_eq!(keys.resolve(&typed('z')), None);
    }

    /// A bare Space may be bound — and while something is being typed it
    /// is still the space between two words, never the binding.
    #[test]
    fn a_bound_space_yields_to_a_query_being_typed() {
        let keys: Keymap<TestAction> = Keymap::from_bindings(
            [(Combo::parse("Space").unwrap(), TestAction::Open)],
            BareKeys::ReservedForTyping,
        );
        let space = KeyPress { key: Key::Space, mods: Modifiers::default(), text: Some(' ') };
        assert_eq!(keys.resolve_typing(&space, false), Some(Resolved::Action(TestAction::Open)));
        assert_eq!(keys.resolve_typing(&space, true), Some(Resolved::Text(' ')));
        let ctrl_space = KeyPress { mods: Modifiers { ctrl: true, ..Modifiers::default() }, ..space };
        assert_eq!(keys.resolve_typing(&ctrl_space, true), None, "Ctrl+Space is nobody's text");
    }

    #[test]
    fn a_menu_hint_lists_every_key_an_action_has() {
        let keys = typing_app();
        let hints: Vec<String> =
            keys.combos_for(TestAction::GoUp).iter().map(|c| c.to_string()).collect();
        assert_eq!(hints, ["Alt+Up", "Backspace"]);
    }
}
