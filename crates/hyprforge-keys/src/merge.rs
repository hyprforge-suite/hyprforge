//! Applying a config file's `[keys]` table over the shipped defaults.
//!
//! Shared rather than per-app because these are decisions about a *file
//! format*, and two apps re-deriving them would produce two formats
//! wearing one name. Every rule here was a judgement call, and each is
//! written down where it is made.
//!
//! Nothing in this module is fatal. A bad entry is reported and skipped
//! and the rest still apply — one typo must not cost a person every other
//! binding they wrote, which is the lesson the file manager's config
//! carries from the 37 hand-written binds.

use crate::combo::Combo;
use crate::keymap::{BareKeys, Bindable, Keymap};
use std::collections::BTreeMap;
use std::fmt;

/// Something wrong with one entry, in words a person can act on.
///
/// A plain message rather than an error enum, because these are
/// advisory: the caller collects them and shows them, and carries on
/// with a working keymap either way.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Problem {
    pub message: String,
}

impl fmt::Display for Problem {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.message)
    }
}

fn problem(message: impl Into<String>) -> Problem {
    Problem { message: message.into() }
}

/// The default keymap with `overrides` applied.
///
/// For each action named: its default keys are dropped and the listed
/// ones used instead (an empty list leaves it unbound). An entry that
/// cannot be used is reported and skipped; the rest still apply.
///
/// When two entries in the file claim one key, the one whose id sorts
/// first keeps it and the other is reported. A TOML table carries no
/// order this parser can rely on, so "the last one wins" would mean
/// "whichever the map happened to list last" — the alphabetical rule is
/// at least one a person can predict.
///
/// When an entry takes a key that belonged to a *default* binding, that
/// is taken as meant — rebinding Ctrl+T is a normal thing to do. It is
/// reported only if the action that lost the key is left with no key at
/// all, since that action has then quietly become unreachable.
///
/// `bare` decides whether a text-producing key may be bound without a
/// modifier; see [`BareKeys`], and note that passing the wrong one makes
/// an app refuse its own defaults.
pub fn keymap_with<A: Bindable>(
    overrides: &BTreeMap<String, Vec<String>>,
    bare: BareKeys,
) -> (Keymap<A>, Vec<Problem>) {
    let mut problems = Vec::new();
    let mut bindings: BTreeMap<Combo, A> = BTreeMap::new();
    let mut from_file: BTreeMap<Combo, A> = BTreeMap::new();

    // The actions the file names, resolved first so a default binding
    // for any of them is never installed.
    let mut overridden: BTreeMap<A, &Vec<String>> = BTreeMap::new();
    for (id, combos) in overrides {
        match A::from_id(id) {
            Some(action) => {
                overridden.insert(action, combos);
            }
            None => {
                problems.push(problem(format!("[keys] {id}: there is no action with that name")))
            }
        }
    }

    for (action, combos) in &overridden {
        for text in combos.iter() {
            let combo = match Combo::parse(text) {
                Ok(combo) => combo,
                Err(e) => {
                    problems.push(problem(format!("[keys] {} = \"{text}\": {e}", action.id())));
                    continue;
                }
            };
            if bare == BareKeys::ReservedForTyping && combo.would_swallow_typing() {
                problems.push(problem(format!(
                    "[keys] {} = \"{text}\": a key with no Ctrl, Alt or Super would stop that \
                     character being typed into search, so it was not bound",
                    action.id()
                )));
                continue;
            }
            if let Some(other) = from_file.get(&combo) {
                problems.push(problem(format!(
                    "[keys] {} = \"{text}\": {combo} is already bound to {} in this file, \
                     which keeps it",
                    action.id(),
                    other.id()
                )));
                continue;
            }
            from_file.insert(combo, *action);
        }
    }

    // The defaults for every action the file did not mention, minus any
    // key the file has now given to something else.
    for action in A::all() {
        if overridden.contains_key(&action) {
            continue;
        }
        let mut kept = 0;
        for text in action.default_keys() {
            let Ok(combo) = Combo::parse(text) else { continue };
            if from_file.contains_key(&combo) {
                continue;
            }
            bindings.insert(combo, action);
            kept += 1;
        }
        if kept == 0 && !action.default_keys().is_empty() {
            problems.push(problem(format!(
                "[keys] {} has no key any more: its default ({}) was given to another action",
                action.id(),
                action.default_keys().join(", ")
            )));
        }
    }

    bindings.extend(from_file);
    (Keymap::from_bindings(bindings, bare), problems)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::keymap::Resolved;
    use crate::press::KeyPress;

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
                TestAction::Next => &["Ctrl+N"],
            }
        }
    }

    fn overrides(entries: &[(&str, &[&str])]) -> BTreeMap<String, Vec<String>> {
        entries
            .iter()
            .map(|(id, keys)| {
                ((*id).to_string(), keys.iter().map(|k| (*k).to_string()).collect())
            })
            .collect()
    }

    fn press(text: &str) -> KeyPress {
        let combo = Combo::parse(text).unwrap();
        KeyPress { key: combo.key, mods: combo.mods, text: None }
    }

    fn typing(
        entries: &[(&str, &[&str])],
    ) -> (Keymap<TestAction>, Vec<Problem>) {
        keymap_with(&overrides(entries), BareKeys::ReservedForTyping)
    }

    #[test]
    fn rebinding_one_action_leaves_every_other_default_bound() {
        let (keys, problems) = typing(&[("new-tab", &["Ctrl+Y"])]);
        assert!(problems.is_empty(), "{problems:?}");
        assert_eq!(keys.resolve(&press("Ctrl+Y")), Some(Resolved::Action(TestAction::NewTab)));
        assert_eq!(keys.resolve(&press("Ctrl+T")), None, "the default is dropped, not kept");
        assert_eq!(keys.resolve(&press("Enter")), Some(Resolved::Action(TestAction::Open)));
    }

    #[test]
    fn a_list_binds_several_keys_and_an_empty_list_unbinds() {
        let (keys, problems) = typing(&[("open", &["Ctrl+O", "F3"]), ("new-tab", &[])]);
        assert!(problems.is_empty(), "{problems:?}");
        assert_eq!(keys.resolve(&press("Ctrl+O")), Some(Resolved::Action(TestAction::Open)));
        assert_eq!(keys.resolve(&press("F3")), Some(Resolved::Action(TestAction::Open)));
        assert_eq!(keys.resolve(&press("Ctrl+T")), None);
    }

    #[test]
    fn a_bad_entry_is_reported_and_the_good_ones_still_apply() {
        let (keys, problems) = typing(&[("open", &["Ctrl+Banana"]), ("new-tab", &["Ctrl+Y"])]);
        assert_eq!(problems.len(), 1, "{problems:?}");
        assert!(problems[0].message.contains("Banana"), "{problems:?}");
        assert_eq!(keys.resolve(&press("Ctrl+Y")), Some(Resolved::Action(TestAction::NewTab)));
    }

    #[test]
    fn an_unknown_action_is_reported_by_name() {
        let (_, problems) = typing(&[("fly-to-the-moon", &["Ctrl+M"])]);
        assert_eq!(problems.len(), 1, "{problems:?}");
        assert!(problems[0].message.contains("fly-to-the-moon"), "{problems:?}");
    }

    #[test]
    fn a_binding_that_would_eat_typing_is_refused() {
        let (keys, problems) = typing(&[("next", &["N"])]);
        assert_eq!(problems.len(), 1, "{problems:?}");
        assert!(problems[0].message.contains("typed into search"), "{problems:?}");
        assert_eq!(keys.resolve(&press("N")), None);
    }

    /// The same file, in an app with nowhere to type, is simply valid.
    /// This is the pair that makes [`BareKeys`] worth having.
    #[test]
    fn the_same_bare_letter_binds_fine_in_a_viewer() {
        let (keys, problems): (Keymap<TestAction>, _) =
            keymap_with(&overrides(&[("next", &["N"])]), BareKeys::Bindable);
        assert!(problems.is_empty(), "{problems:?}");
        assert_eq!(keys.resolve(&press("N")), Some(Resolved::Action(TestAction::Next)));
    }

    #[test]
    fn two_entries_on_one_key_keep_the_first_and_say_so() {
        let (keys, problems) = typing(&[("new-tab", &["Ctrl+Y"]), ("go-up", &["Ctrl+Y"])]);
        assert_eq!(problems.len(), 1, "{problems:?}");
        // "go-up" sorts before "new-tab", so it keeps the key.
        assert_eq!(keys.resolve(&press("Ctrl+Y")), Some(Resolved::Action(TestAction::GoUp)));
        assert!(problems[0].message.contains("new-tab"), "{problems:?}");
    }

    #[test]
    fn taking_the_only_key_of_another_action_is_reported() {
        let (keys, problems) = typing(&[("go-up", &["Ctrl+T"])]);
        assert_eq!(keys.resolve(&press("Ctrl+T")), Some(Resolved::Action(TestAction::GoUp)));
        assert_eq!(problems.len(), 1, "{problems:?}");
        assert!(problems[0].message.contains("new-tab"), "{problems:?}");
        assert!(problems[0].message.contains("no key any more"), "{problems:?}");
    }

    /// Taking *one* of an action's two keys is a normal rebinding and
    /// says nothing, because the action is still reachable.
    #[test]
    fn taking_one_of_two_keys_is_taken_as_meant() {
        let (keys, problems) = typing(&[("open", &["Backspace"])]);
        assert!(problems.is_empty(), "{problems:?}");
        assert_eq!(keys.resolve(&press("Backspace")), Some(Resolved::Action(TestAction::Open)));
        assert_eq!(keys.resolve(&press("Alt+Up")), Some(Resolved::Action(TestAction::GoUp)));
    }

    #[test]
    fn no_overrides_at_all_is_exactly_the_defaults() {
        let (keys, problems) = typing(&[]);
        assert!(problems.is_empty(), "{problems:?}");
        assert_eq!(keys, Keymap::defaults(BareKeys::ReservedForTyping));
    }
}
