//! The file browser's hand-written configuration: `files-config.toml`.
//!
//! Not `files.toml`. That file is state the app rewrites whole on every
//! save — sort order, window size, pinned folders — and a whole-struct
//! rewrite drops comments and reorders tables. A person writing key
//! bindings by hand needs a file the app only ever *reads*, or their
//! notes vanish the next time they resize a window. See
//! `hyprforge_paths::files_config_toml_path`.
//!
//! Three states, and they stay distinct (CLAUDE.md's `hlconfig::storage`
//! rule):
//!
//! - **Missing** is first run: the defaults, and nothing to report.
//! - **Present but unparseable** is reported, and the defaults are used
//!   until it is fixed. Nothing here ever writes over it.
//! - **Parseable with some bad entries** keeps every good entry and
//!   reports each bad one by name. One typo must not cost a person every
//!   other binding they wrote — the lesson of the 37 binds.
//!
//! Every section *merges* over the defaults. A `[keys]` table with one
//! line changes one action's keys and nothing else.
//!
//! ```toml
//! [keys]
//! # action id = one binding, or a list. An empty list unbinds.
//! trash  = "Delete"
//! rename = ["F2", "Ctrl+R"]
//! go-up  = []
//! ```

use crate::action::Action;
use crate::keymap::{Combo, Key, Keymap};
use serde::Deserialize;
use std::collections::BTreeMap;
use std::fmt;
use std::path::Path;

/// Everything `files-config.toml` configures, resolved against the
/// defaults. `Default` is the shipped configuration, because
/// `Keymap::default` is the shipped bindings.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Config {
    pub keymap: Keymap,
}

/// One thing in the file that could not be used.
///
/// Worded to be shown to the person who wrote the file: it names the
/// section, the entry and what is wrong, never "check the logs".
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConfigProblem {
    pub message: String,
}

impl fmt::Display for ConfigProblem {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.message)
    }
}

fn problem(message: impl Into<String>) -> ConfigProblem {
    ConfigProblem { message: message.into() }
}

/// The file as written, before anything is checked.
#[derive(Debug, Default, Deserialize)]
#[serde(default)]
struct RawConfig {
    keys: BTreeMap<String, OneOrMany>,
}

/// `trash = "Delete"` and `rename = ["F2", "Ctrl+R"]` are both allowed —
/// one binding is the common case and should not need brackets.
#[derive(Debug, Deserialize)]
#[serde(untagged)]
enum OneOrMany {
    One(String),
    Many(Vec<String>),
}

impl OneOrMany {
    fn into_vec(self) -> Vec<String> {
        match self {
            OneOrMany::One(s) if s.trim().is_empty() => Vec::new(),
            OneOrMany::One(s) => vec![s],
            OneOrMany::Many(v) => v,
        }
    }
}

/// Loads `files-config.toml` from its usual place.
pub fn load() -> (Config, Vec<ConfigProblem>) {
    load_from(&hyprforge_paths::files_config_toml_path())
}

/// [`load`], from a given path — the seam tests use.
///
/// Never fails: every problem becomes a [`ConfigProblem`] and the
/// affected setting keeps its default, because a configuration mistake
/// must never stop the window opening.
pub fn load_from(path: &Path) -> (Config, Vec<ConfigProblem>) {
    let text = match std::fs::read_to_string(path) {
        Ok(text) => text,
        // First run — the file simply has not been written yet.
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            return (Config::default(), Vec::new());
        }
        Err(e) => {
            return (
                Config::default(),
                vec![problem(format!(
                    "{} could not be read, so the default settings are in use: {e}",
                    path.display()
                ))],
            );
        }
    };
    parse(&text, path)
}

/// Parses the file's contents. `path` is only for messages.
pub fn parse(text: &str, path: &Path) -> (Config, Vec<ConfigProblem>) {
    let raw: RawConfig = match toml::from_str(text) {
        Ok(raw) => raw,
        Err(e) => {
            return (
                Config::default(),
                vec![problem(format!(
                    "{} is not valid TOML, so the default settings are in use until it is fixed: {e}",
                    path.display()
                ))],
            );
        }
    };
    let overrides: BTreeMap<String, Vec<String>> =
        raw.keys.into_iter().map(|(id, v)| (id, v.into_vec())).collect();
    let (keymap, problems) = keymap_with(&overrides);
    (Config { keymap }, problems)
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
pub fn keymap_with(overrides: &BTreeMap<String, Vec<String>>) -> (Keymap, Vec<ConfigProblem>) {
    let mut problems = Vec::new();
    let mut bindings: BTreeMap<Combo, Action> = BTreeMap::new();
    let mut from_file: BTreeMap<Combo, Action> = BTreeMap::new();

    // The actions the file names, resolved first so a default binding
    // for any of them is never installed.
    let mut overridden: BTreeMap<Action, &Vec<String>> = BTreeMap::new();
    for (id, combos) in overrides {
        match Action::from_id(id) {
            Some(action) => {
                overridden.insert(action, combos);
            }
            None => problems.push(problem(format!(
                "[keys] {id}: there is no action with that name"
            ))),
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
            if types_text(&combo) {
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
    for action in Action::all() {
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
    (Keymap::from_bindings(bindings), problems)
}

/// Whether a binding would swallow a character a person meant to type.
///
/// Any key that produces text — a letter, a digit, a symbol, Space —
/// is only safe to bind with Ctrl, Alt or Super held. Shift alone does
/// not count: Shift+A is how a capital A gets typed.
fn types_text(combo: &Combo) -> bool {
    matches!(combo.key, Key::Char(_) | Key::Space)
        && !combo.mods.ctrl
        && !combo.mods.alt
        && !combo.mods.logo
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::keymap::{KeyPress, Resolved};

    fn press(text: &str) -> KeyPress {
        let combo = Combo::parse(text).unwrap();
        KeyPress { key: combo.key, mods: combo.mods, text: None }
    }

    fn parsed(text: &str) -> (Config, Vec<ConfigProblem>) {
        parse(text, Path::new("files-config.toml"))
    }

    fn does(config: &Config, key: &str) -> Option<Action> {
        match config.keymap.resolve(&press(key)) {
            Some(Resolved::Action(a)) => Some(a),
            _ => None,
        }
    }

    #[test]
    fn a_missing_file_is_first_run_with_nothing_to_report() {
        let dir = tempfile::tempdir().unwrap();
        let (config, problems) = load_from(&dir.path().join("files-config.toml"));
        assert_eq!(config, Config::default());
        assert!(problems.is_empty());
    }

    /// The other half of the storage rule: a file that is there and
    /// will not parse is *said*, not silently treated as absent.
    #[test]
    fn a_file_that_will_not_parse_is_reported_and_the_defaults_used() {
        let (config, problems) = parsed("[keys\ntrash = ");
        assert_eq!(config, Config::default());
        assert_eq!(problems.len(), 1);
        assert!(problems[0].message.contains("not valid TOML"), "{}", problems[0]);
    }

    /// Merge, not replace. Rebinding one action must leave every other
    /// default exactly where it was — this is the 37-binds lesson, and
    /// the test checks the whole table rather than one neighbour.
    #[test]
    fn rebinding_one_action_leaves_every_other_default_bound() {
        let (config, problems) = parsed("[keys]\ntrash = \"Ctrl+D\"\n");
        assert!(problems.is_empty(), "{problems:?}");
        assert_eq!(does(&config, "Ctrl+D"), Some(Action::Trash));
        assert_eq!(does(&config, "Delete"), None, "the old key is released");
        for action in Action::all().into_iter().filter(|a| *a != Action::Trash) {
            for key in action.default_keys() {
                assert_eq!(does(&config, key), Some(action), "{key} lost {action:?}");
            }
        }
    }

    #[test]
    fn a_list_binds_several_keys_and_an_empty_list_unbinds() {
        let (config, problems) =
            parsed("[keys]\nselect-all = [\"Ctrl+A\", \"Ctrl+Shift+A\"]\ngo-up = []\n");
        assert!(problems.is_empty(), "{problems:?}");
        assert_eq!(does(&config, "Ctrl+Shift+A"), Some(Action::SelectAll));
        assert_eq!(does(&config, "Ctrl+A"), Some(Action::SelectAll));
        assert_eq!(does(&config, "Backspace"), None);
        assert_eq!(does(&config, "Alt+Up"), None);
    }

    #[test]
    fn an_empty_string_unbinds_too() {
        let (config, problems) = parsed("[keys]\ntrash = \"\"\n");
        assert!(problems.is_empty(), "{problems:?}");
        assert_eq!(does(&config, "Delete"), None);
    }

    /// One typo costs one binding, never the rest of the file.
    #[test]
    fn a_bad_entry_is_reported_and_the_good_ones_still_apply() {
        let (config, problems) = parsed(
            "[keys]\n\
             trash = \"Ctrl+Banana\"\n\
             select-all = \"Ctrl+E\"\n\
             copy-everything = \"Ctrl+K\"\n",
        );
        assert_eq!(does(&config, "Ctrl+E"), Some(Action::SelectAll));
        assert_eq!(problems.len(), 2, "{problems:?}");
        assert!(problems.iter().any(|p| p.message.contains("trash") && p.message.contains("Banana")));
        assert!(problems.iter().any(|p| p.message.contains("copy-everything")));
    }

    /// A bare letter bound to an action would make that letter
    /// untypeable in the search box.
    #[test]
    fn a_binding_that_would_eat_typing_is_refused() {
        let (config, problems) = parsed("[keys]\ntrash = \"d\"\nselect-all = \"Shift+A\"\n");
        assert_eq!(does(&config, "d"), None);
        assert_eq!(does(&config, "Shift+A"), None, "Shift alone still types a capital");
        assert_eq!(problems.len(), 2, "{problems:?}");
    }

    #[test]
    fn two_entries_on_one_key_keep_the_first_and_say_so() {
        let (config, problems) = parsed("[keys]\ntrash = \"Ctrl+D\"\nclear-search = \"Ctrl+D\"\n");
        // `clear-search` sorts before `trash`, so it keeps the key.
        assert_eq!(does(&config, "Ctrl+D"), Some(Action::ClearSearch));
        assert_eq!(problems.len(), 1, "{problems:?}");
        assert!(problems[0].message.contains("trash"));
    }

    /// Taking a default's key is a normal rebind and says nothing —
    /// unless it leaves that action with no key at all.
    #[test]
    fn taking_the_only_key_of_another_action_is_reported() {
        let (config, problems) = parsed("[keys]\nselect-all = \"Ctrl+T\"\n");
        assert_eq!(does(&config, "Ctrl+T"), Some(Action::SelectAll));
        assert_eq!(problems.len(), 1, "{problems:?}");
        assert!(problems[0].message.contains("new-tab"), "{}", problems[0]);

        // GoUp has two defaults; taking one leaves it reachable, quietly.
        let (config, problems) = parsed("[keys]\nselect-all = \"Alt+Up\"\n");
        assert!(problems.is_empty(), "{problems:?}");
        assert_eq!(does(&config, "Backspace"), Some(Action::GoUp));
    }

    /// Sections this version does not know yet — `[menu]`, which a
    /// later version adds — must not make the whole file unreadable.
    #[test]
    fn an_unknown_section_is_ignored_rather_than_fatal() {
        let (config, problems) = parsed("[menu]\nentry = [\"open\"]\n[keys]\ntrash = \"Ctrl+D\"\n");
        assert!(problems.is_empty(), "{problems:?}");
        assert_eq!(does(&config, "Ctrl+D"), Some(Action::Trash));
    }

    /// The config is read, never written — this module has no function
    /// that could, and a file with comments comes back from `load`
    /// unchanged on disk.
    #[test]
    fn loading_leaves_the_file_exactly_as_written() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("files-config.toml");
        let text = "# my bindings\n[keys]\ntrash = \"Ctrl+D\"  # muscle memory\n";
        std::fs::write(&path, text).unwrap();
        let _ = load_from(&path);
        assert_eq!(std::fs::read_to_string(&path).unwrap(), text);
    }
}
