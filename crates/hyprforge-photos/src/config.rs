//! `photos-config.toml` — the file a person writes by hand.
//!
//! Not `photos.toml`. That one is state this app rewrites whole on every
//! save, and a whole-struct rewrite drops comments and reorders tables.
//! Somebody writing key bindings needs a file the app only ever *reads*,
//! or their notes vanish the next time they resize the window.
//!
//! Three states, kept distinct:
//!
//! - **Missing** is first run: the defaults, nothing to report.
//! - **Present but unparseable** is reported, the defaults are used, and
//!   nothing here ever writes over it.
//! - **Parseable with some bad entries** keeps every good entry and
//!   reports each bad one by name. One typo must not cost a person every
//!   other binding they wrote — the lesson of the 37 binds.
//!
//! ```toml
//! [keys]
//! # action id = one binding, or a list. An empty list unbinds.
//! next = ["Right", "Space"]
//! trash = "Delete"
//! info = []
//! ```

use crate::keys::{Action, Keymap, BARE_KEYS};
use serde::Deserialize;
use std::collections::BTreeMap;
use std::fmt;
use std::path::Path;

/// Everything `photos-config.toml` configures, resolved against the
/// defaults.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Config {
    pub keymap: Keymap,
}

impl Default for Config {
    fn default() -> Self {
        Config { keymap: crate::keys::defaults() }
    }
}

/// One thing in the file that could not be used.
///
/// Worded to be shown to the person who wrote it: it names the section,
/// the entry and what is wrong, never "check the logs".
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConfigProblem {
    pub message: String,
}

impl fmt::Display for ConfigProblem {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.message)
    }
}

impl From<hyprforge_keys::Problem> for ConfigProblem {
    fn from(p: hyprforge_keys::Problem) -> ConfigProblem {
        ConfigProblem { message: p.message }
    }
}

/// The file as written, before anything is checked.
#[derive(Debug, Default, Deserialize)]
#[serde(default)]
struct RawConfig {
    keys: BTreeMap<String, OneOrMany>,
}

/// A binding is one string or a list of them, and an empty list unbinds.
#[derive(Debug, Deserialize)]
#[serde(untagged)]
enum OneOrMany {
    One(String),
    Many(Vec<String>),
}

impl OneOrMany {
    fn into_vec(self) -> Vec<String> {
        match self {
            // An empty string unbinds too — the same intent as an empty
            // list, written the way somebody would reach for first.
            OneOrMany::One(s) if s.trim().is_empty() => Vec::new(),
            OneOrMany::One(s) => vec![s],
            OneOrMany::Many(v) => v,
        }
    }
}

pub fn path() -> std::path::PathBuf {
    hyprforge_paths::photos_config_toml_path()
}

pub fn load() -> (Config, Vec<ConfigProblem>) {
    load_from(&path())
}

pub fn load_from(path: &Path) -> (Config, Vec<ConfigProblem>) {
    match std::fs::read_to_string(path) {
        Ok(text) => parse(&text, path),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => (Config::default(), Vec::new()),
        Err(e) => (
            Config::default(),
            vec![ConfigProblem {
                message: format!("{} could not be read: {e} — using the defaults", path.display()),
            }],
        ),
    }
}

/// Parses the file's text. A file that will not parse at all is one
/// problem and the defaults; a file that parses with bad entries keeps
/// every good one.
pub fn parse(text: &str, path: &Path) -> (Config, Vec<ConfigProblem>) {
    let raw: RawConfig = match toml::from_str(text) {
        Ok(raw) => raw,
        Err(e) => {
            return (
                Config::default(),
                vec![ConfigProblem {
                    message: format!(
                        "{} could not be read: {e} — using the defaults",
                        path.display()
                    ),
                }],
            )
        }
    };

    let overrides: BTreeMap<String, Vec<String>> =
        raw.keys.into_iter().map(|(id, v)| (id, v.into_vec())).collect();

    // The merge rules are `hyprforge-keys`'s, because they are decisions
    // about a file format and the file manager reads the same one. What
    // is this app's to say is `BARE_KEYS` — see `crate::keys`.
    let (keymap, problems) = hyprforge_keys::merge::keymap_with::<Action>(&overrides, BARE_KEYS);
    (Config { keymap }, problems.into_iter().map(ConfigProblem::from).collect())
}

#[cfg(test)]
mod tests {
    use super::*;
    use hyprforge_keys::{Combo, KeyPress};

    fn press(text: &str) -> KeyPress {
        let combo = Combo::parse(text).unwrap();
        KeyPress { key: combo.key, mods: combo.mods, text: None }
    }

    fn parsed(text: &str) -> (Config, Vec<ConfigProblem>) {
        parse(text, Path::new("photos-config.toml"))
    }

    fn does(config: &Config, key: &str) -> Option<Action> {
        match config.keymap.resolve(&press(key)) {
            Some(hyprforge_keys::Resolved::Action(a)) => Some(a),
            _ => None,
        }
    }

    #[test]
    fn a_missing_file_is_first_run_with_nothing_to_report() {
        let dir = tempfile::tempdir().unwrap();
        let (config, problems) = load_from(&dir.path().join("photos-config.toml"));
        assert_eq!(config, Config::default());
        assert!(problems.is_empty());
    }

    #[test]
    fn a_file_that_will_not_parse_is_reported_and_the_defaults_used() {
        let (config, problems) = parsed("[keys\nnext = 3");
        assert_eq!(config, Config::default());
        assert_eq!(problems.len(), 1);
        assert!(problems[0].message.contains("photos-config.toml"));
    }

    #[test]
    fn rebinding_one_action_leaves_every_other_default_bound() {
        let (config, problems) = parsed("[keys]\nnext = \"J\"\n");
        assert!(problems.is_empty(), "{problems:?}");
        assert_eq!(does(&config, "J"), Some(Action::Next));
        assert_eq!(does(&config, "P"), Some(Action::Previous));
    }

    #[test]
    fn a_list_binds_several_keys_and_an_empty_list_unbinds() {
        let (config, problems) = parsed("[keys]\nnext = [\"J\", \"Ctrl+N\"]\ninfo = []\n");
        assert!(problems.is_empty(), "{problems:?}");
        assert_eq!(does(&config, "J"), Some(Action::Next));
        assert_eq!(does(&config, "Ctrl+N"), Some(Action::Next));
        assert_eq!(does(&config, "I"), None);
    }

    #[test]
    fn an_empty_string_unbinds_too() {
        let (config, problems) = parsed("[keys]\ninfo = \"\"\n");
        assert!(problems.is_empty(), "{problems:?}");
        assert_eq!(does(&config, "I"), None);
    }

    #[test]
    fn a_bad_entry_is_reported_and_the_good_ones_still_apply() {
        let (config, problems) = parsed("[keys]\nnext = \"Ctrl+Banana\"\ntrash = \"F8\"\n");
        assert_eq!(problems.len(), 1, "{problems:?}");
        assert!(problems[0].message.contains("Banana"));
        assert_eq!(does(&config, "F8"), Some(Action::Trash));
    }

    #[test]
    fn an_unknown_action_is_reported_by_name() {
        let (_, problems) = parsed("[keys]\nsharpen = \"S\"\n");
        assert_eq!(problems.len(), 1, "{problems:?}");
        assert!(problems[0].message.contains("sharpen"));
    }

    /// The axis, from the config file's side: a bare letter is a
    /// perfectly good binding here, where the same line in
    /// `files-config.toml` is refused.
    #[test]
    fn a_bare_letter_is_a_valid_binding_in_this_app() {
        let (config, problems) = parsed("[keys]\nzoom-in = \"Z\"\n");
        assert!(problems.is_empty(), "{problems:?}");
        assert_eq!(does(&config, "Z"), Some(Action::ZoomIn));
    }
}
