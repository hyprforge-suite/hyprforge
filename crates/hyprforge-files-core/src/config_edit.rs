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
use toml_edit::{Array, ArrayOfTables, DocumentMut, Item, Table, Value};

/// The first line of a file this sheet had to create.
pub const HEADER: &str = "# Files' own settings: [keys], [menu], [behaviour], [sidebar] and [[action]].\n\
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
    /// The `[[action]]` block an edit was for is not in the file any
    /// more — changed in an editor since the sheet read it. Nothing was
    /// written: guessing which block was meant could change the wrong one.
    #[error("{path} has no action called \u{201c}{id}\u{201d} any more, so nothing was changed")]
    NoSuchAction { path: String, id: String },
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
    /// `[behaviour] terminal = [...]`. `None` removes the line, which is
    /// Automatic.
    Terminal(Option<Vec<String>>),
    /// One `[[action]]` block added, changed or removed — see
    /// [`crate::custom::Change`].
    Action(crate::custom::Change),
}

/// A `[behaviour]` value, as the sheet sets it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BehaviourValue {
    Switch(bool),
    Word(&'static str),
    /// A whole number, written bare — `watch-network-every = 3`, not
    /// `"3"`, which the loader would refuse as not a number.
    Number(i64),
}

impl BehaviourValue {
    /// The value as a TOML value.
    fn to_toml(&self) -> Value {
        match self {
            BehaviourValue::Switch(on) => Value::from(*on),
            BehaviourValue::Word(word) => Value::from(*word),
            BehaviourValue::Number(n) => Value::from(*n),
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
                set(table, key, value.to_toml());
            }
            Edit::Sidebar(key, value) => {
                let table = section(&mut doc, "sidebar", &shown)?;
                set(table, key, value.to_toml());
            }
            Edit::Terminal(words) => {
                let table = section(&mut doc, "behaviour", &shown)?;
                match words {
                    None => {
                        table.remove("terminal");
                    }
                    Some(words) => set(table, "terminal", Value::Array(words.iter().map(String::as_str).collect())),
                }
            }
            Edit::Action(change) => change_action(&mut doc, change, &shown)?,
        }
    }
    Ok(doc.to_string())
}

/// Applies one change to the `[[action]]` blocks, in place.
///
/// A block is found by its id ([`crate::custom::block_id`]) rather than
/// by where it sits, so a block someone added above it in an editor
/// since the sheet read the file does not shift the edit onto the wrong
/// one. Changing a block sets each field with [`set`], which keeps the
/// comment above a line and the one after it; a key the draft leaves
/// empty (`types`) is removed, because an empty list would read as a
/// restriction someone wrote.
fn change_action(doc: &mut DocumentMut, change: &crate::custom::Change, path: &str) -> Result<(), EditError> {
    use crate::custom::{block_id, slug, Change, Draft};
    let item = doc.entry("action").or_insert_with(|| Item::ArrayOfTables(ArrayOfTables::new()));
    let Some(blocks) = item.as_array_of_tables_mut() else {
        return Err(EditError::NotATable { path: path.to_string(), section: "action".to_string() });
    };
    let id_of = |table: &Table| {
        block_id(table.get("id").and_then(Item::as_str), table.get("label").and_then(Item::as_str))
    };
    let position = |blocks: &ArrayOfTables, id: &str| {
        blocks
            .iter()
            .position(|t| id_of(t).as_deref() == Some(id))
            .ok_or_else(|| EditError::NoSuchAction { path: path.to_string(), id: id.to_string() })
    };
    let fill = |table: &mut Table, draft: &Draft| {
        set(table, "label", Value::from(draft.label.trim()));
        set(table, "command", Value::Array(draft.command.iter().map(String::as_str).collect()));
        if draft.types.is_empty() {
            table.remove("types");
        } else {
            set(table, "types", Value::Array(draft.types.iter().map(String::as_str).collect()));
        }
        set(table, "selection", Value::from(draft.selection.id()));
        set(table, "terminal", Value::from(draft.terminal));
    };
    match change {
        Change::Add(draft) => {
            // An id of its own, written out, so renaming it later in the
            // sheet does not change what the block is called.
            let taken: Vec<String> = blocks.iter().filter_map(id_of).collect();
            let base = slug(&draft.label);
            let id = std::iter::once(base.clone())
                .chain((2..).map(|n| format!("{base}-{n}")))
                .find(|id| !taken.contains(id))
                .expect("an unbounded sequence finds a free name");
            let mut table = Table::new();
            table.insert("id", Item::Value(Value::from(id)));
            fill(&mut table, draft);
            blocks.push(table);
        }
        Change::Replace { id, draft } => {
            let at = position(blocks, id)?;
            fill(blocks.get_mut(at).expect("found just now"), draft);
        }
        Change::Remove { id } => {
            let at = position(blocks, id)?;
            blocks.remove(at);
        }
    }
    Ok(())
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

    fn draft(label: &str, command: &[&str]) -> crate::custom::Draft {
        crate::custom::Draft {
            label: label.to_string(),
            command: command.iter().map(|w| w.to_string()).collect(),
            ..Default::default()
        }
    }

    /// Automatic is the line's absence; a chosen terminal is one line of
    /// words — and the loader reads back what was written.
    #[test]
    fn the_terminal_control_writes_exactly_its_line() {
        let text = "[behaviour]\n# mine\nconfirm-trash = true\n";
        let out = edited(text, &[Edit::Terminal(Some(vec!["kitty".into(), "--single-instance".into()]))]);
        assert_eq!(out, "[behaviour]\n# mine\nconfirm-trash = true\nterminal = [\"kitty\", \"--single-instance\"]\n");
        let (config, problems) = crate::config::parse(&out, Path::new("x"));
        assert!(problems.is_empty(), "{problems:?}");
        assert_eq!(config.terminal, Some(vec!["kitty".to_string(), "--single-instance".to_string()]));
        assert_eq!(edited(&out, &[Edit::Terminal(None)]), text, "Automatic takes the line away again");
    }

    #[test]
    fn adding_an_action_writes_one_block_the_loader_reads_back() {
        use crate::custom::{Change, Selection};
        let mut new = draft("Resize to 50%", &["magick", "mogrify", "-resize", "50%"]);
        new.types = vec!["image/*".into()];
        new.selection = Selection::Many;
        let out = edited("[keys]\ntrash = \"Delete\"\n", &[Edit::Action(Change::Add(new))]);
        assert!(out.starts_with("[keys]\ntrash = \"Delete\"\n"), "{out}");
        assert!(
            out.contains(
                "[[action]]\nid = \"resize-to-50\"\nlabel = \"Resize to 50%\"\n\
                 command = [\"magick\", \"mogrify\", \"-resize\", \"50%\"]\ntypes = [\"image/*\"]\n\
                 selection = \"many\"\nterminal = false\n"
            ),
            "{out}"
        );
        let (config, problems) = crate::config::parse(&out, Path::new("x"));
        assert!(problems.is_empty(), "{problems:?}");
        assert_eq!(config.actions[0].selection, Selection::Many);
        assert_eq!(config.actions[0].types, ["image/*"]);
    }

    #[test]
    fn a_second_action_with_the_same_label_gets_an_id_of_its_own() {
        use crate::custom::Change;
        let out = edited(
            "",
            &[Edit::Action(Change::Add(draft("Shrink", &["a"]))), Edit::Action(Change::Add(draft("Shrink", &["b"])))],
        );
        let (config, problems) = crate::config::parse(&out, Path::new("x"));
        assert!(problems.is_empty(), "{problems:?}");
        assert_eq!(config.actions.iter().map(|a| a.id.as_str()).collect::<Vec<_>>(), ["shrink", "shrink-2"]);
    }

    /// Editing a block changes its fields and nothing else: the comments
    /// in it and around it, and the blocks either side, are as written.
    #[test]
    fn editing_an_action_keeps_its_comments_and_the_blocks_around_it() {
        use crate::custom::{Change, Selection};
        let text = "# my actions\n\
                    [[action]]\nlabel = \"First\"\ncommand = [\"one\"]\n\
                    \n# the one I use most\n[[action]]\n\
                    label = \"Shrink\"  # for the blog\n\
                    # the argv, not a shell line\n\
                    command = [\"magick\"]\ntypes = [\"image/*\"]\n\
                    \n[[action]]\nlabel = \"Last\"\ncommand = [\"three\"]\n";
        let mut changed = draft("Shrink more", &["magick", "mogrify", "-resize", "25%"]);
        changed.selection = Selection::One;
        let out = edited(text, &[Edit::Action(Change::Replace { id: "shrink".into(), draft: changed })]);
        for kept in [
            "# my actions",
            "[[action]]\nlabel = \"First\"\ncommand = [\"one\"]",
            "# the one I use most",
            "label = \"Shrink more\"  # for the blog",
            "# the argv, not a shell line\ncommand = [\"magick\", \"mogrify\", \"-resize\", \"25%\"]",
            "[[action]]\nlabel = \"Last\"\ncommand = [\"three\"]",
        ] {
            assert!(out.contains(kept), "lost {kept:?} from:\n{out}");
        }
        assert!(!out.contains("types"), "an empty list of types is no line at all:\n{out}");
        let (config, problems) = crate::config::parse(&out, Path::new("x"));
        assert!(problems.is_empty(), "{problems:?}");
        assert_eq!(config.actions[1].label, "Shrink more");
        assert_eq!(config.actions[1].selection, Selection::One);
    }

    #[test]
    fn removing_an_action_takes_only_its_block() {
        use crate::custom::Change;
        let text = "[[action]]\nlabel = \"Keep\"\ncommand = [\"a\"]\n\
                    [[action]]\nid = \"bye\"\nlabel = \"Go\"\ncommand = [\"b\"]\n";
        let out = edited(text, &[Edit::Action(Change::Remove { id: "bye".into() })]);
        assert_eq!(out, "[[action]]\nlabel = \"Keep\"\ncommand = [\"a\"]\n");
    }

    /// The block was renamed in an editor since the sheet read the file:
    /// nothing is written, rather than the edit landing on a neighbour.
    #[test]
    fn an_action_that_is_no_longer_in_the_file_is_not_guessed_at() {
        use crate::custom::Change;
        let result = apply(
            "[[action]]\nlabel = \"Other\"\ncommand = [\"a\"]\n",
            &[Edit::Action(Change::Remove { id: "gone".into() })],
            Path::new("x.toml"),
        );
        assert!(matches!(result, Err(EditError::NoSuchAction { .. })), "{result:?}");
    }
}
