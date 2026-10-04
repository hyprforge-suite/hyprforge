//! Custom actions: the person's own commands, run on what is selected.
//!
//! Written as `[[action]]` blocks in `files-config.toml`:
//!
//! ```toml
//! [[action]]
//! id = "resize-half"            # optional; made from the label when absent
//! label = "Resize to 50%"
//! command = ["magick", "mogrify", "-resize", "50%"]   # the selection is appended
//! types = ["image/*"]           # optional MIME globs
//! selection = "any"             # one | many | any
//! terminal = false              # run it inside the terminal
//! ```
//!
//! # A command is a list of words, never a line for a shell
//!
//! `command` is the program and its arguments, and every selected path is
//! appended as one more argument of its own. Nothing is run through
//! `sh -c`, nothing is expanded, and there are no `%f`-style field codes.
//! That is the suite's stated position on `Exec=`
//! (`hyprforge_mime::apps`'s module doc): reimplementing the desktop
//! entry's field codes is how a file manager ends up as a third opinion
//! about how a command line is built. It is also what makes a file called
//! `; rm -rf ~` an ordinary file name here: it reaches the program as one
//! argument, byte for byte, with no shell anywhere to read the semicolon.
//! A person who *wants* a shell writes `["sh", "-c", "…", "sh"]` and gets
//! exactly that, visibly, in their own file.
//!
//! # A bad block costs that block, and only that block
//!
//! Each `[[action]]` is checked on its own, the way every other section
//! of the file is (`crate::config`'s three states). A block with a
//! problem is reported by its label and left out; the rest still load.
//! A *restriction* that cannot be read — a misspelt `types`, an unknown
//! `selection`, an unknown key that may be one — leaves the whole block
//! out rather than dropping the restriction, because an action meant for
//! pictures that quietly starts offering itself on every file is worse
//! than one that is missing and says why.
//!
//! # What the browser does with them
//!
//! The menu for a file, a folder or the empty space (and the Ctrl+K
//! palette) offers the actions whose `types` suit everything the menu
//! would act on — see [`CustomAction::suits`]. From the empty space the
//! folder in view is what is acted on, as one folder: that is what makes
//! "Open in my editor" with `selection = "one"` and
//! `types = ["inode/directory"]` work from the background. Whether a type
//! *matches* needs the shared MIME database, which this crate does not
//! read — the window hands the browser a [`TypeOf`] that asks it.

use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::sync::Arc;

/// One `[[action]]`, as loaded.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CustomAction {
    /// Its identity in the file: what the Preferences editor finds the
    /// block by when it changes it. Made from the label when the file
    /// gives none — see [`slug`].
    pub id: String,
    /// What the menu and the palette call it.
    pub label: String,
    /// The program and its arguments. Never empty.
    pub command: Vec<String>,
    /// MIME globs (`image/*`, `text/plain`); empty means any file or
    /// folder.
    pub types: Vec<String>,
    pub selection: Selection,
    /// Run inside the terminal Files resolves for "Open Terminal Here".
    pub terminal: bool,
}

/// How many things an action takes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Selection {
    /// Exactly one.
    One,
    /// Two or more.
    Many,
    /// One or more. The default, because it is what a command that takes
    /// a list of files — nearly every command — already means.
    #[default]
    Any,
}

impl Selection {
    pub const ALL: [Selection; 3] = [Selection::One, Selection::Many, Selection::Any];

    /// The word the file uses.
    pub fn id(self) -> &'static str {
        match self {
            Selection::One => "one",
            Selection::Many => "many",
            Selection::Any => "any",
        }
    }

    pub fn parse(text: &str) -> Option<Selection> {
        Selection::ALL.into_iter().find(|s| s.id() == text.trim())
    }

    /// What the Preferences editor calls it.
    pub fn label(self) -> &'static str {
        match self {
            Selection::One => "One item",
            Selection::Many => "Several items",
            Selection::Any => "Any number",
        }
    }

    /// Whether `count` things are what this takes.
    pub fn accepts(self, count: usize) -> bool {
        match self {
            Selection::One => count == 1,
            Selection::Many => count >= 2,
            Selection::Any => count >= 1,
        }
    }
}

/// What a file is, for matching an action's `types`: its type by name,
/// followed by every type that one is a kind of (`text/x-rust`, then
/// `text/plain`, …) — so `types = ["text/plain"]` offers itself on source
/// code the way an application registered for plain text opens it.
/// Empty when the name says nothing.
///
/// Handed to the browser by the window, which holds the MIME database;
/// a browser without one (the open/save dialog) offers no custom actions
/// at all.
#[derive(Clone)]
pub struct TypeOf(Arc<TypeFn>);

/// What a [`TypeOf`] calls.
type TypeFn = dyn Fn(&Path) -> Vec<String> + Send + Sync;

impl TypeOf {
    pub fn new(types: impl Fn(&Path) -> Vec<String> + Send + Sync + 'static) -> TypeOf {
        TypeOf(Arc::new(types))
    }

    /// The types `path` is, best first. A folder is `inode/directory` and
    /// nothing else, without asking: the database would only say so too.
    pub fn of(&self, path: &Path, is_dir: bool) -> Vec<String> {
        if is_dir {
            return vec![FOLDER_TYPE.to_string()];
        }
        (self.0)(path)
    }
}

impl std::fmt::Debug for TypeOf {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("TypeOf")
    }
}

/// The type the shared MIME database gives a folder.
pub const FOLDER_TYPE: &str = "inode/directory";

/// One thing an action would act on.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Target {
    pub path: PathBuf,
    pub is_dir: bool,
}

impl CustomAction {
    /// Whether this action belongs in a menu about `targets` at all —
    /// every one of them is a type it names. Not about how many there
    /// are: that greys the row ([`Self::takes`]) rather than hiding it,
    /// so the menu keeps its shape as the selection grows, which is the
    /// rule the built-in rows follow.
    pub fn suits(&self, targets: &[Target], type_of: &TypeOf) -> bool {
        if targets.is_empty() {
            return false;
        }
        if self.types.is_empty() {
            return true;
        }
        targets.iter().all(|target| {
            let kinds = type_of.of(&target.path, target.is_dir);
            kinds.iter().any(|kind| self.types.iter().any(|pattern| type_matches(pattern, kind)))
        })
    }

    /// Whether it can run on `count` things now.
    pub fn takes(&self, count: usize) -> bool {
        self.selection.accepts(count)
    }

    /// The whole command line for `paths`: the command, then each path
    /// as an argument of its own. `OsString`, because a file name need
    /// not be UTF-8 and must reach the program exactly as it is on disk.
    pub fn argv(&self, paths: &[PathBuf]) -> Vec<OsString> {
        self.command
            .iter()
            .map(OsString::from)
            .chain(paths.iter().map(|p| p.as_os_str().to_os_string()))
            .collect()
    }
}

/// Whether the MIME glob `pattern` covers `mime`. `*` matches any run of
/// characters, and case is ignored, as the database's own names are
/// compared.
pub fn type_matches(pattern: &str, mime: &str) -> bool {
    fn glob(p: &[u8], s: &[u8]) -> bool {
        match p.split_first() {
            None => s.is_empty(),
            Some((b'*', rest)) => (0..=s.len()).any(|i| glob(rest, &s[i..])),
            Some((c, rest)) => s.first().is_some_and(|h| h.eq_ignore_ascii_case(c)) && glob(rest, &s[1..]),
        }
    }
    glob(pattern.trim().as_bytes(), mime.trim().as_bytes())
}

/// Whether `text` reads as a MIME type or a glob of one: a media type and
/// a subtype, each of the characters MIME names use, with `*` allowed.
/// `image/*`, `text/plain`, `application/vnd.oasis.*`, `*/*`.
pub fn is_type_pattern(text: &str) -> bool {
    let ok = |part: &str| {
        !part.is_empty() && part.chars().all(|c| c.is_ascii_alphanumeric() || "*.+-_".contains(c))
    };
    match text.trim().split_once('/') {
        Some((major, minor)) => ok(major) && ok(minor),
        None => false,
    }
}

/// An id made from a label: lower case, words joined by `-`, nothing
/// but letters and digits. "Resize to 50%" is `resize-to-50`.
pub fn slug(label: &str) -> String {
    let mut out = String::new();
    for c in label.chars() {
        if c.is_alphanumeric() {
            out.extend(c.to_lowercase());
        } else if !out.is_empty() && !out.ends_with('-') {
            out.push('-');
        }
    }
    while out.ends_with('-') {
        out.pop();
    }
    if out.is_empty() {
        "action".to_string()
    } else {
        out
    }
}

/// The id a block goes by: its own, or its label's [`slug`]. One function
/// for the loader and the editor, so the block the editor goes looking
/// for is the one the loader called by that name.
pub fn block_id(id: Option<&str>, label: Option<&str>) -> Option<String> {
    match (id.map(str::trim).filter(|s| !s.is_empty()), label.map(str::trim).filter(|s| !s.is_empty())) {
        (Some(id), _) => Some(id.to_string()),
        (None, Some(label)) => Some(slug(label)),
        (None, None) => None,
    }
}

/// The keys a block may have. Anything else is reported and costs the
/// block — see the module doc.
const KEYS: [&str; 6] = ["id", "label", "command", "types", "selection", "terminal"];

/// Reads the file's `action` value — `[[action]]` blocks — reporting each
/// bad block by name and keeping the rest, in the file's order.
pub fn parse_blocks(value: &toml::Value, problems: &mut Vec<String>) -> Vec<CustomAction> {
    let toml::Value::Array(blocks) = value else {
        problems.push(
            "action: write each custom action as its own [[action]] block, so none were loaded".to_string(),
        );
        return Vec::new();
    };
    let mut actions: Vec<CustomAction> = Vec::with_capacity(blocks.len());
    for (n, block) in blocks.iter().enumerate() {
        let name = || match block.get("label").and_then(toml::Value::as_str) {
            Some(label) if !label.trim().is_empty() => format!("[[action]] \u{201c}{}\u{201d}", label.trim()),
            _ => format!("[[action]] number {}", n + 1),
        };
        let left_out = |why: String| format!("{}: {why}, so this action was left out", name());
        let toml::Value::Table(table) = block else {
            problems.push(left_out("it is not a table".to_string()));
            continue;
        };
        match parse_block(table) {
            Ok(action) => {
                if actions.iter().any(|a| a.id == action.id) {
                    problems.push(left_out(format!(
                        "the id \u{201c}{}\u{201d} is already used by an action above it",
                        action.id
                    )));
                } else {
                    actions.push(action);
                }
            }
            Err(why) => problems.push(left_out(why)),
        }
    }
    actions
}

fn parse_block(table: &toml::Table) -> Result<CustomAction, String> {
    if let Some(unknown) = table.keys().find(|k| !KEYS.contains(&k.as_str())) {
        return Err(format!("\u{201c}{unknown}\u{201d} isn't a setting an action has (use {})", KEYS.join(", ")));
    }
    let label = match table.get("label") {
        Some(toml::Value::String(s)) if !s.trim().is_empty() => s.trim().to_string(),
        Some(other) => return Err(format!("label = {other}: use some text, like \"Resize to 50%\"")),
        None => return Err("it has no label, which is what the menu would call it".to_string()),
    };
    let id = match table.get("id") {
        None => slug(&label),
        Some(toml::Value::String(s)) if !s.trim().is_empty() => s.trim().to_string(),
        Some(other) => return Err(format!("id = {other}: use a word, like \"resize-half\"")),
    };
    let command = match table.get("command") {
        Some(toml::Value::Array(words)) => {
            let words: Option<Vec<String>> =
                words.iter().map(|w| w.as_str().filter(|s| !s.is_empty()).map(str::to_string)).collect();
            match words {
                Some(words) if !words.is_empty() => words,
                _ => return Err("command: use a list of words, like [\"magick\", \"mogrify\"]".to_string()),
            }
        }
        Some(toml::Value::String(s)) => {
            return Err(format!(
                "command = \"{s}\": use a list of words, like [\"magick\", \"mogrify\"]. \
                 A command is never run through a shell"
            ))
        }
        Some(other) => return Err(format!("command = {other}: use a list of words, like [\"magick\", \"mogrify\"]")),
        None => return Err("it has no command".to_string()),
    };
    let types = match table.get("types") {
        None => Vec::new(),
        Some(toml::Value::Array(items)) => {
            let mut types = Vec::with_capacity(items.len());
            for item in items {
                match item.as_str() {
                    Some(t) if is_type_pattern(t) => types.push(t.trim().to_string()),
                    _ => return Err(format!("types: {item} is not a file type, like \"image/*\" or \"text/plain\"")),
                }
            }
            types
        }
        Some(other) => return Err(format!("types = {other}: use a list, like [\"image/*\"]")),
    };
    let selection = match table.get("selection") {
        None => Selection::default(),
        Some(toml::Value::String(s)) => match Selection::parse(s) {
            Some(selection) => selection,
            None => return Err(format!("selection = \"{s}\": use one, many or any")),
        },
        Some(other) => return Err(format!("selection = {other}: use \"one\", \"many\" or \"any\"")),
    };
    let terminal = match table.get("terminal") {
        None => false,
        Some(toml::Value::Boolean(on)) => *on,
        Some(other) => return Err(format!("terminal = {other}: use true or false")),
    };
    Ok(CustomAction { id, label, command, types, selection, terminal })
}

/// One action as the Preferences editor holds it: the fields a person
/// fills in. The id is not one of them — a new action's is made from its
/// label, and an existing one's never changes, because it is what the
/// editor finds the block by.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Draft {
    pub label: String,
    pub command: Vec<String>,
    pub types: Vec<String>,
    pub selection: Selection,
    pub terminal: bool,
}

impl Draft {
    pub fn of(action: &CustomAction) -> Draft {
        Draft {
            label: action.label.clone(),
            command: action.command.clone(),
            types: action.types.clone(),
            selection: action.selection,
            terminal: action.terminal,
        }
    }

    /// What stops this being saved, as sentences for the editor — empty
    /// when it is fine. `on_path` answers whether a program can be run;
    /// the window's is `hyprforge_mime::apps::on_path`.
    pub fn problems(&self, on_path: &dyn Fn(&str) -> bool) -> Vec<String> {
        let mut out = Vec::new();
        if self.label.trim().is_empty() {
            out.push("Give it a name for the menu.".to_string());
        }
        match self.command.first() {
            None => out.push("Say what to run.".to_string()),
            Some(program) if !on_path(program) => {
                out.push(format!("{program} isn't installed, or isn't on your PATH."));
            }
            Some(_) => {}
        }
        for t in &self.types {
            if !is_type_pattern(t) {
                out.push(format!("\u{201c}{t}\u{201d} isn't a file type. Use names like image/* or text/plain."));
            }
        }
        out
    }
}

/// A change the Preferences editor makes to the `[[action]]` blocks —
/// written by `crate::config_edit`, one block at a time.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Change {
    /// A new block at the end of the file.
    Add(Draft),
    /// The block with this id gets these fields; anything else in it —
    /// comments, an `id` line — stays.
    Replace { id: String, draft: Draft },
    /// The block with this id goes.
    Remove { id: String },
}

/// Splits what was typed in the editor's command field into words: on
/// spaces, with `"…"` or `'…'` keeping a word that has spaces in it
/// together. Not a shell — nothing is expanded, and `|` or `;` are words
/// like any other — only the quoting a person would type to say "this is
/// one argument". `Err` for a quote left open.
pub fn split_words(text: &str) -> Result<Vec<String>, String> {
    let mut words = Vec::new();
    let mut word = String::new();
    let mut in_word = false;
    let mut quote: Option<char> = None;
    for c in text.chars() {
        match (quote, c) {
            (Some(q), c) if c == q => quote = None,
            (Some(_), c) => word.push(c),
            (None, '"' | '\'') => {
                quote = Some(c);
                in_word = true;
            }
            (None, c) if c.is_whitespace() => {
                if in_word {
                    words.push(std::mem::take(&mut word));
                    in_word = false;
                }
            }
            (None, c) => {
                word.push(c);
                in_word = true;
            }
        }
    }
    if let Some(q) = quote {
        return Err(format!("A {q} is opened and never closed."));
    }
    if in_word {
        words.push(word);
    }
    Ok(words)
}

/// The words back as one line, quoted where a word needs it, so
/// [`split_words`] reads the line back as the same words.
pub fn join_words(words: &[String]) -> String {
    words
        .iter()
        .map(|w| {
            if !w.is_empty() && !w.chars().any(|c| c.is_whitespace() || c == '"' || c == '\'') {
                w.clone()
            } else if !w.contains('"') {
                format!("\"{w}\"")
            } else {
                format!("'{w}'")
            }
        })
        .collect::<Vec<_>>()
        .join(" ")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn blocks(text: &str) -> (Vec<CustomAction>, Vec<String>) {
        let value: toml::Table = toml::from_str(text).unwrap();
        let mut problems = Vec::new();
        let actions = parse_blocks(&value["action"], &mut problems);
        (actions, problems)
    }

    /// By extension, the way a test can without a MIME database: `.png`
    /// is a picture, `.rs` is source code and therefore plain text too.
    fn by_name() -> TypeOf {
        TypeOf::new(|path| match path.extension().and_then(|e| e.to_str()) {
            Some("png") => vec!["image/png".to_string(), "application/octet-stream".to_string()],
            Some("rs") => vec!["text/rust".to_string(), "text/plain".to_string()],
            _ => Vec::new(),
        })
    }

    fn file(path: &str) -> Target {
        Target { path: path.into(), is_dir: false }
    }

    #[test]
    fn a_block_reads_with_every_field_and_the_defaults_fill_the_rest() {
        let (actions, problems) = blocks(
            "[[action]]\nid = \"half\"\nlabel = \"Resize to 50%\"\ncommand = [\"magick\", \"mogrify\", \"-resize\", \"50%\"]\n\
             types = [\"image/*\"]\nselection = \"many\"\nterminal = true\n\
             [[action]]\nlabel = \"Count lines\"\ncommand = [\"wc\", \"-l\"]\n",
        );
        assert!(problems.is_empty(), "{problems:?}");
        assert_eq!(actions[0].id, "half");
        assert_eq!(actions[0].selection, Selection::Many);
        assert!(actions[0].terminal);
        assert_eq!(actions[1].id, "count-lines", "made from the label");
        assert_eq!(actions[1].selection, Selection::Any);
        assert!(actions[1].types.is_empty() && !actions[1].terminal);
    }

    /// One bad block costs itself, by name, and the others still load.
    #[test]
    fn a_bad_action_block_is_reported_and_the_others_still_load() {
        let (actions, problems) = blocks(
            "[[action]]\nlabel = \"Good one\"\ncommand = [\"true\"]\n\
             [[action]]\nlabel = \"Shell line\"\ncommand = \"convert $1 out.png\"\n\
             [[action]]\nlabel = \"Typo\"\ncommand = [\"true\"]\ntype = [\"image/*\"]\n\
             [[action]]\nlabel = \"Not a type\"\ncommand = [\"true\"]\ntypes = [\"pictures\"]\n\
             [[action]]\nlabel = \"Last good\"\ncommand = [\"true\"]\n",
        );
        let labels: Vec<&str> = actions.iter().map(|a| a.label.as_str()).collect();
        assert_eq!(labels, ["Good one", "Last good"]);
        assert_eq!(problems.len(), 3, "{problems:?}");
        assert!(problems[0].contains("Shell line") && problems[0].contains("never run through a shell"));
        assert!(problems[1].contains("Typo") && problems[1].contains("\u{201c}type\u{201d}"), "{problems:?}");
        assert!(problems[2].contains("pictures"));
        assert!(problems.iter().all(|p| p.contains("left out")));
    }

    #[test]
    fn a_block_with_no_label_is_named_by_its_place() {
        let (actions, problems) = blocks("[[action]]\ncommand = [\"true\"]\n");
        assert!(actions.is_empty());
        assert!(problems[0].contains("number 1"), "{problems:?}");
    }

    #[test]
    fn a_repeated_id_keeps_the_first_and_says_so() {
        let (actions, problems) = blocks(
            "[[action]]\nlabel = \"Open\"\ncommand = [\"a\"]\n[[action]]\nlabel = \"open\"\ncommand = [\"b\"]\n",
        );
        assert_eq!(actions.len(), 1);
        assert_eq!(actions[0].command, ["a"]);
        assert!(problems[0].contains("already used"), "{problems:?}");
    }

    #[test]
    fn an_action_whose_type_does_not_match_is_not_offered() {
        let (actions, _) = blocks("[[action]]\nlabel = \"Shrink\"\ncommand = [\"x\"]\ntypes = [\"image/*\"]\n");
        let shrink = &actions[0];
        assert!(shrink.suits(&[file("/a/b.png")], &by_name()));
        assert!(!shrink.suits(&[file("/a/b.rs")], &by_name()));
        assert!(!shrink.suits(&[file("/a/b.png"), file("/a/c.rs")], &by_name()), "every one has to suit");
        assert!(!shrink.suits(&[file("/a/mystery")], &by_name()), "a type nobody knows is not a picture");
        assert!(!shrink.suits(&[Target { path: "/a".into(), is_dir: true }], &by_name()));
    }

    /// `text/plain` covers what is a kind of plain text, the way an
    /// application registered for it opens source code.
    #[test]
    fn a_parent_type_covers_the_types_that_are_kinds_of_it() {
        let (actions, _) = blocks("[[action]]\nlabel = \"Edit\"\ncommand = [\"x\"]\ntypes = [\"text/plain\"]\n");
        assert!(actions[0].suits(&[file("/a/main.rs")], &by_name()));
    }

    #[test]
    fn a_folder_is_inode_directory_without_asking() {
        let (actions, _) =
            blocks("[[action]]\nlabel = \"Code\"\ncommand = [\"code\"]\ntypes = [\"inode/directory\"]\n");
        let never = TypeOf::new(|_| panic!("a folder's type is not looked up"));
        assert!(actions[0].suits(&[Target { path: "/a".into(), is_dir: true }], &never));
    }

    #[test]
    fn how_many_are_selected_decides_whether_it_can_run() {
        assert!(Selection::One.accepts(1) && !Selection::One.accepts(2));
        assert!(!Selection::Many.accepts(1) && Selection::Many.accepts(2));
        assert!(Selection::Any.accepts(1) && Selection::Any.accepts(5) && !Selection::Any.accepts(0));
    }

    /// The paths are arguments of their own, after the command's, with
    /// no shell between: a name full of shell syntax arrives as written.
    #[test]
    fn a_selection_is_passed_as_arguments_and_never_through_a_shell() {
        let action = CustomAction {
            id: "x".into(),
            label: "X".into(),
            command: vec!["magick".into(), "mogrify".into()],
            types: Vec::new(),
            selection: Selection::Any,
            terminal: false,
        };
        let evil = PathBuf::from("/tmp/a; rm -rf ~ $(reboot).png");
        let argv = action.argv(&[evil.clone(), "/tmp/b c.png".into()]);
        assert_eq!(argv, [OsString::from("magick"), "mogrify".into(), evil.into_os_string(), "/tmp/b c.png".into()]);
        assert!(!argv.iter().any(|a| a == "sh" || a == "-c"));
    }

    #[test]
    fn a_name_that_is_not_utf8_reaches_the_program_byte_for_byte() {
        use std::os::unix::ffi::OsStrExt;
        let action = CustomAction {
            id: "x".into(),
            label: "X".into(),
            command: vec!["cat".into()],
            types: Vec::new(),
            selection: Selection::Any,
            terminal: false,
        };
        let raw = PathBuf::from(std::ffi::OsStr::from_bytes(b"/tmp/\xff.bin"));
        assert_eq!(action.argv(std::slice::from_ref(&raw))[1].as_bytes(), b"/tmp/\xff.bin");
    }

    #[test]
    fn type_globs_match_the_way_their_stars_say() {
        assert!(type_matches("image/*", "image/png"));
        assert!(type_matches("*/*", "text/plain"));
        assert!(type_matches("application/vnd.oasis.*", "application/vnd.oasis.opendocument.text"));
        assert!(type_matches("Image/PNG", "image/png"));
        assert!(!type_matches("image/*", "video/mp4"));
        assert!(!type_matches("image/png", "image/png+x"));
        assert!(is_type_pattern("image/*") && is_type_pattern("model/3mf"));
        assert!(!is_type_pattern("pictures") && !is_type_pattern("image/") && !is_type_pattern("a b/c"));
    }

    #[test]
    fn a_slug_is_the_labels_words_joined() {
        assert_eq!(slug("Resize to 50%"), "resize-to-50");
        assert_eq!(slug("  Open in VS Code!  "), "open-in-vs-code");
        assert_eq!(slug("%%%"), "action");
    }

    #[test]
    fn typed_words_split_on_spaces_and_quotes_keep_one_together() {
        assert_eq!(split_words("magick mogrify -resize 50%").unwrap(), ["magick", "mogrify", "-resize", "50%"]);
        assert_eq!(split_words("cp -t \"/my files\" ''").unwrap(), ["cp", "-t", "/my files", ""]);
        assert_eq!(split_words("echo a;b|c").unwrap(), ["echo", "a;b|c"], "not a shell");
        assert!(split_words("say \"hi").is_err());
        let words: Vec<String> = ["a b", "c", "it's", "", "x\"y"].map(String::from).to_vec();
        assert_eq!(split_words(&join_words(&words)).unwrap(), words, "joining reads back as the same words");
    }

    #[test]
    fn a_draft_names_what_stops_it_being_saved() {
        let found = |p: &str| p == "magick";
        let fine = Draft { label: "Shrink".into(), command: vec!["magick".into()], ..Draft::default() };
        assert!(fine.problems(&found).is_empty());
        let missing = Draft { command: vec!["nope".into()], ..fine.clone() };
        assert!(missing.problems(&found)[0].contains("nope"));
        let unnamed = Draft { label: " ".into(), ..fine.clone() };
        assert_eq!(unnamed.problems(&found).len(), 1);
        let bad_type = Draft { types: vec!["pictures".into()], ..fine };
        assert!(bad_type.problems(&found)[0].contains("pictures"));
    }
}
