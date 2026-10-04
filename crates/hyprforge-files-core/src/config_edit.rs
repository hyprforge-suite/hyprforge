//! Changing `files-config.toml` from the Preferences sheet without
//! rewriting it.
//!
//! The file is hand-written: [`crate::config`]'s doc explains why it is
//! not `files.toml`, which the app rewrites whole. So the sheet never
//! *saves the configuration*. It edits one entry of the document as the
//! person left it — through `toml_edit`, which keeps every comment, every
//! blank line, the order of the tables and the spelling of every value it
//! was not asked to touch — and writes the result back atomically.
//!
//! Three rules, each from the 37 binds this suite once lost:
//!
//! - **A file that will not parse is never written.** It is reported, and
//!   the edit is refused until the person fixes it. Writing a fresh file
//!   over it would destroy everything they had, typo included.
//! - **A missing file is first run.** The edit creates it, with a line
//!   saying where it came from, so a person who opens it later finds out
//!   what wrote it.
//! - **Read at the moment of the edit**, not when the sheet opened, so a
//!   change made in an editor meanwhile is edited too rather than
//!   reverted.

use crate::action::Action;
use crate::keymap::Combo;
use std::path::Path;
use toml_edit::{Array, DocumentMut, Item, Table, Value};

/// The first line of a file this sheet had to create.
pub const HEADER: &str = "# Files' own settings: [keys], [menu], [behaviour] and [sidebar].\n\
     # Files' Preferences writes here one line at a time, and the file is\n\
     # yours to edit too: comments and anything you add are kept.\n";

/// Why an edit was not made.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum EditError {
    /// The file exists and is not TOML. Nothing was written.
    #[error("{path} isn't valid TOML, so Preferences won't write to it until it is fixed: {why}")]
    Unparseable { path: String, why: String },
    /// A section the edit needs is something other than a table — say
    /// `keys = 3`. Overwriting it would destroy what the person meant.
    #[error("{path} has `{section}` as something other than a table, so it was left alone")]
    NotATable { path: String, section: String },
    #[error("{path} couldn't be read: {why}")]
    Read { path: String, why: String },
    #[error("{path} couldn't be written: {why}")]
    Write { path: String, why: String },
}

/// One change to the file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Edit {
    /// `[keys] <action> = …`. `None` removes the line, which puts the
    /// action back on its shipped keys; `Some(&[])` leaves it unbound.
    Keys(Action, Option<Vec<Combo>>),
    /// `[behaviour] <key> = <value>`.
    Behaviour(&'static str, BehaviourValue),
    /// `[sidebar] <key> = <value>` — a value of the same kinds
    /// `[behaviour]` takes.
    Sidebar(&'static str, BehaviourValue),
}

/// A `[behaviour]` value, as the sheet sets it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BehaviourValue {
    Switch(bool),
    Word(&'static str),
}

impl BehaviourValue {
    /// The value as a TOML value.
    fn to_toml(&self) -> Value {
        match self {
            BehaviourValue::Switch(on) => Value::from(*on),
            BehaviourValue::Word(word) => Value::from(*word),
        }
    }
}

/// `text` with `edits` applied, or why not. `path` is only for messages.
pub fn apply(text: &str, edits: &[Edit], path: &Path) -> Result<String, EditError> {
    let shown = path.display().to_string();
    let mut doc: DocumentMut = text
        .parse()
        .map_err(|e: toml_edit::TomlError| EditError::Unparseable { path: shown.clone(), why: e.to_string() })?;
    for edit in edits {
        match edit {
            Edit::Keys(action, keys) => {
                let table = section(&mut doc, "keys", &shown)?;
                match keys {
                    None => {
                        table.remove(action.id());
                    }
                    Some(keys) => set(table, action.id(), keys_value(keys)),
                }
            }
            Edit::Behaviour(key, value) => {
                let table = section(&mut doc, "behaviour", &shown)?;
                let value = match value {
                    BehaviourValue::Switch(on) => Value::from(*on),
                    BehaviourValue::Word(word) => Value::from(*word),
                };
                set(table, key, value);
            }
            Edit::Sidebar(key, value) => {
                let table = section(&mut doc, "sidebar", &shown)?;
                set(table, key, value.to_toml());
            }
        }
    }
    Ok(doc.to_string())
}

/// `key = value` in `table`, changing only the value when the key is
/// already there.
///
/// `Table::insert` would replace the whole entry, and with it the
/// comment above the line — measured: "# the one I keep hitting" went
/// with it. Swapping the value in place keeps the key, its comment and
/// the value's own surrounding spaces and trailing comment.
fn set(table: &mut Table, key: &str, value: Value) {
    match table.get_mut(key) {
        Some(Item::Value(old)) => {
            let decor = old.decor().clone();
            *old = value;
            *old.decor_mut() = decor;
        }
        _ => {
            table.insert(key, Item::Value(value));
        }
    }
}

/// `[name]` as a table to edit, made if it is not there.
fn section<'d>(doc: &'d mut DocumentMut, name: &str, path: &str) -> Result<&'d mut Table, EditError> {
    let item = doc.entry(name).or_insert_with(|| Item::Table(Table::new()));
    // An inline `keys = { trash = "Delete" }` is a table too, and turning
    // it into a `[keys]` section is a smaller change to someone's file
    // than refusing; but it is *their* layout, so it is kept as written
    // by editing it in place where possible.
    if let Item::Value(Value::InlineTable(inline)) = item {
        let table = std::mem::take(inline).into_table();
        *item = Item::Table(table);
    }
    item.as_table_mut().ok_or_else(|| EditError::NotATable { path: path.to_string(), section: name.to_string() })
}

/// One key as a string, several as a list — the same choice a person
/// writing the file makes, so the line reads like theirs.
fn keys_value(keys: &[Combo]) -> Value {
    match keys {
        [one] => Value::from(one.to_string()),
        many => {
            let mut array = Array::new();
            for key in many {
                array.push(key.to_string());
            }
            Value::Array(array)
        }
    }
}

/// Applies `edits` to the file at `path`: read now, edited in place,
/// written atomically. Blocking — a host runs it off the UI thread.
pub fn apply_to_file(path: &Path, edits: &[Edit]) -> Result<(), EditError> {
    let shown = path.display().to_string();
    let edited = match std::fs::read_to_string(path) {
        Ok(text) => apply(&text, edits, path)?,
        // The header is put on afterwards rather than edited around: a
        // document of nothing but comments keeps them *after* any table
        // added to it, which would leave the file's first line a
        // `[keys]` and its explanation at the bottom.
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => format!("{HEADER}\n{}", apply("", edits, path)?),
        Err(e) => return Err(EditError::Read { path: shown, why: e.to_string() }),
    };
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir).map_err(|e| EditError::Write { path: shown.clone(), why: e.to_string() })?;
    }
    hyprforge_paths::write_atomic(path, &edited).map_err(|e| EditError::Write { path: shown, why: e.to_string() })
}

/// What the Preferences sheet needs to know about the file before it
/// offers to change anything.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct FileState {
    /// The action ids its `[keys]` names — see [`customised`].
    pub customised: std::collections::BTreeSet<String>,
    /// Why nothing can be written to it, when something can't — it will
    /// not parse, or it could not be read. Missing is not a reason: that
    /// is first run, and the first edit creates it.
    pub refused: Option<EditError>,
}

/// Reads the file at `path` for the sheet. Blocking.
pub fn read_state(path: &Path) -> FileState {
    let shown = path.display().to_string();
    match std::fs::read_to_string(path) {
        Ok(text) => FileState {
            customised: customised(&text),
            refused: text
                .parse::<DocumentMut>()
                .err()
                .map(|e| EditError::Unparseable { path: shown, why: e.to_string() }),
        },
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => FileState::default(),
        Err(e) => FileState {
            customised: Default::default(),
            refused: Some(EditError::Read { path: shown, why: e.to_string() }),
        },
    }
}

/// The action ids the file's `[keys]` names — the rows Preferences marks
/// as the person's own, and offers to reset. Empty for a file with no
/// `[keys]`, and for one that will not parse (whose problem is reported
/// by [`crate::config::load_from`]).
pub fn customised(text: &str) -> std::collections::BTreeSet<String> {
    let Ok(value) = text.parse::<toml::Table>() else { return Default::default() };
    match value.get("keys") {
        Some(toml::Value::Table(keys)) => keys.keys().cloned().collect(),
        _ => Default::default(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn edited(text: &str, edits: &[Edit]) -> String {
        apply(text, edits, Path::new("files-config.toml")).unwrap()
    }

    fn combo(text: &str) -> Combo {
        Combo::parse(text).unwrap()
    }

    /// The whole reason this module exists: the person's notes survive.
    #[test]
    fn rebinding_one_key_keeps_every_comment_and_every_other_line() {
        let text = "# my keys, tuned over a year\n\
                    [keys]\n\
                    # the one I keep hitting\n\
                    trash = \"Delete\"\n\
                    rename = [\"F2\", \"Ctrl+R\"]   # both, please\n\
                    \n\
                    [sidebar]\n\
                    show-trash = false\n";
        let out = edited(text, &[Edit::Keys(Action::Trash, Some(vec![combo("Ctrl+Delete")]))]);
        assert!(out.contains("# my keys, tuned over a year"), "{out}");
        assert!(out.contains("# the one I keep hitting"), "{out}");
        assert!(out.contains("trash = \"Ctrl+Delete\""), "{out}");
        assert!(out.contains("rename = [\"F2\", \"Ctrl+R\"]   # both, please"), "{out}");
        assert!(out.contains("[sidebar]\nshow-trash = false"), "{out}");
        assert!(!out.contains("\"Delete\""), "{out}");
    }

    #[test]
    fn a_file_that_will_not_parse_is_refused_rather_than_replaced() {
        let result = apply("[keys\ntrash = ", &[Edit::Keys(Action::Trash, None)], Path::new("x.toml"));
        assert!(matches!(result, Err(EditError::Unparseable { .. })), "{result:?}");
    }

    #[test]
    fn a_section_that_is_not_a_table_is_left_alone() {
        let result = apply("keys = 3\n", &[Edit::Keys(Action::Trash, None)], Path::new("x.toml"));
        assert!(matches!(result, Err(EditError::NotATable { .. })), "{result:?}");
    }

    /// Resetting is removing the line, not writing the defaults out: a
    /// written-out default would stop following the shipped one when a
    /// later version changes it.
    #[test]
    fn resetting_removes_the_line_and_leaves_the_rest_of_the_table() {
        let text = "[keys]\ntrash = \"Ctrl+Delete\"\nrename = \"F2\"\n";
        let out = edited(text, &[Edit::Keys(Action::Trash, None)]);
        assert_eq!(out, "[keys]\nrename = \"F2\"\n");
    }

    #[test]
    fn several_keys_are_a_list_one_is_a_string_and_none_is_an_empty_list() {
        let out = edited(
            "",
            &[
                Edit::Keys(Action::Rename, Some(vec![combo("F2"), combo("Ctrl+R")])),
                Edit::Keys(Action::Trash, Some(vec![combo("Delete")])),
                Edit::Keys(Action::GoUp, Some(Vec::new())),
            ],
        );
        let (config, problems) = crate::config::parse(&out, Path::new("x"));
        assert!(problems.is_empty(), "{problems:?}\n{out}");
        assert!(out.contains("rename = [\"F2\", \"Ctrl+R\"]"), "{out}");
        assert!(out.contains("trash = \"Delete\""), "{out}");
        assert!(out.contains("go-up = []"), "{out}");
        assert!(config.keymap.combos_for(Action::GoUp).is_empty(), "unbound");
    }

    /// What the sheet writes, the loader reads back as meant — the round
    /// trip that would catch a format the two disagree on.
    #[test]
    fn what_preferences_writes_is_what_the_config_loader_reads() {
        let out = edited(
            "",
            &[
                Edit::Keys(Action::Properties, Some(vec![combo("Ctrl+I")])),
                Edit::Behaviour("on-conflict", BehaviourValue::Word("keep-both")),
                Edit::Behaviour("confirm-trash", BehaviourValue::Switch(true)),
            ],
        );
        let (config, problems) = crate::config::parse(&out, Path::new("x"));
        assert!(problems.is_empty(), "{problems:?}");
        assert_eq!(config.keymap.combos_for(Action::Properties), vec![combo("Ctrl+I")]);
        assert_eq!(config.behaviour.on_conflict, crate::config::OnConflict::KeepBoth);
        assert!(config.behaviour.confirm_trash);
    }

    #[test]
    fn changing_a_behaviour_keeps_the_comment_beside_it() {
        let text = "[behaviour]\n# I like being asked\nconfirm-trash = true # really\n";
        let out = edited(text, &[Edit::Behaviour("confirm-trash", BehaviourValue::Switch(false))]);
        assert!(out.contains("# I like being asked\nconfirm-trash = false # really"), "{out}");
    }

    #[test]
    fn an_inline_keys_table_is_edited_too() {
        let out = edited("keys = { trash = \"Delete\" }\n", &[Edit::Keys(Action::Rename, Some(vec![combo("F2")]))]);
        let (config, problems) = crate::config::parse(&out, Path::new("x"));
        assert!(problems.is_empty(), "{problems:?}\n{out}");
        assert_eq!(config.keymap.combos_for(Action::Rename), vec![combo("F2")]);
        assert_eq!(config.keymap.combos_for(Action::Trash), vec![combo("Delete")]);
    }

    #[test]
    fn a_missing_file_is_created_with_a_line_saying_what_wrote_it() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("hyprforge").join("files-config.toml");
        apply_to_file(&path, &[Edit::Keys(Action::Trash, Some(vec![combo("Delete")]))]).unwrap();
        let text = std::fs::read_to_string(&path).unwrap();
        assert!(text.starts_with("# Files' own settings"), "{text}");
        assert!(text.contains("[keys]\ntrash = \"Delete\""), "{text}");
    }

    #[test]
    fn a_broken_file_on_disk_is_left_exactly_as_it_was() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("files-config.toml");
        let broken = "[keys]\ntrash = \"Delete\n# half a line, and my notes\n";
        std::fs::write(&path, broken).unwrap();
        let result = apply_to_file(&path, &[Edit::Keys(Action::Rename, Some(vec![combo("F2")]))]);
        assert!(matches!(result, Err(EditError::Unparseable { .. })), "{result:?}");
        assert_eq!(std::fs::read_to_string(&path).unwrap(), broken);
    }

    /// The sheet must know before the first click that a click would be
    /// refused, and missing must not be mistaken for broken.
    #[test]
    fn a_broken_file_is_known_to_be_unwritable_and_a_missing_one_is_not() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("files-config.toml");
        assert_eq!(read_state(&path), FileState::default(), "first run");
        std::fs::write(&path, "[keys]\ntrash = \"Delete\"\n").unwrap();
        let fine = read_state(&path);
        assert!(fine.refused.is_none());
        assert!(fine.customised.contains("trash"));
        std::fs::write(&path, "[keys\n").unwrap();
        assert!(matches!(read_state(&path).refused, Some(EditError::Unparseable { .. })));
    }

    #[test]
    fn the_customised_actions_are_the_ones_the_file_names() {
        let names = customised("[keys]\ntrash = \"Delete\"\ngo-up = []\n");
        assert_eq!(names.into_iter().collect::<Vec<_>>(), ["go-up", "trash"]);
        assert!(customised("[keys\n").is_empty());
        assert!(customised("").is_empty());
    }
}
