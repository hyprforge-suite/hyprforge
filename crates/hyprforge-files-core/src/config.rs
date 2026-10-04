//! The file browser's hand-written configuration: `files-config.toml`.
//!
//! Not `files.toml`. That file is state the app rewrites whole on every
//! save — sort order, window size, pinned folders — and a whole-struct
//! rewrite drops comments and reorders tables. A person writing key
//! bindings by hand needs a file the app never *rewrites*, or their
//! notes vanish the next time they resize a window. See
//! `hyprforge_paths::files_config_toml_path`.
//!
//! The Preferences sheet does write to it, but never the way `files.toml`
//! is written: one entry at a time, through [`crate::config_edit`], which
//! keeps every comment and line it was not asked to change and refuses a
//! file it cannot parse. This module still only reads.
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
//!
//! [sidebar]
//! places = ["home", "downloads", "documents"]
//! show-trash = true
//! show-recent = true    # Recent and Starred, at the top of Places
//! show-starred = true
//! collapse-below = 760   # window width; 0 never collapses on its own
//!
//! [sidebar.icons]
//! # A place, "trash", or a folder's path = an icon theme name, or an
//! # image file (anything with a slash in it).
//! downloads = "folder-cloud"
//! trash = "user-trash-full"
//! "~/Projects" = "~/.local/share/icons/projects.svg"
//! ```

use crate::keymap::Keymap;
use crate::menu::{MenuConfig, MenuEntry, MenuKind};
use crate::sidebar::Place;
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
    pub menus: MenuConfig,
    pub behaviour: Behaviour,
    pub sidebar: SidebarConfig,
}

/// `[sidebar]`: what the sidebar offers, and when it folds to a rail.
#[derive(Debug, Clone, PartialEq)]
pub struct SidebarConfig {
    /// The Places section, in order. A place whose folder does not exist
    /// is still left out.
    pub places: Vec<Place>,
    pub show_trash: bool,
    /// `show-recent` and `show-starred`: the Recent and Starred rows at
    /// the top of Places. Hiding a row hides the way in, not the data —
    /// Files still records what it opens, and stars stay starred.
    pub show_recent: bool,
    pub show_starred: bool,
    /// The window width below which the sidebar folds to its rail on its
    /// own. `0` never folds it; the toggle still does.
    pub collapse_below: f32,
    /// `[sidebar.icons]`: the icons the user chose over the theme's, in
    /// the file's order.
    pub icons: Vec<(IconFor, IconChoice)>,
}

/// What a `[sidebar.icons]` line is about.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum IconFor {
    Place(Place),
    Trash,
    /// Any folder, by absolute path — a pinned one, or one that only
    /// ever shows up in a listing.
    Folder(std::path::PathBuf),
}

/// What a `[sidebar.icons]` line chose.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum IconChoice {
    /// A name the icon theme is asked for, like `folder-cloud`.
    Themed(String),
    /// An image of the user's own — SVG, or any picture this build
    /// decodes.
    File(std::path::PathBuf),
}

impl SidebarConfig {
    /// The icon the user chose for `target`, if they chose one.
    pub fn icon_for(&self, target: &IconFor) -> Option<&IconChoice> {
        self.icons.iter().find(|(t, _)| t == target).map(|(_, c)| c)
    }
}

impl Default for SidebarConfig {
    fn default() -> Self {
        SidebarConfig {
            places: Place::ALL.to_vec(),
            show_trash: true,
            show_recent: true,
            show_starred: true,
            collapse_below: crate::density::SIDEBAR_COLLAPSE_BELOW,
            icons: Vec::new(),
        }
    }
}

// `f32` has no `Eq`; the width is a whole number of pixels from a TOML
// integer, never NaN.
impl Eq for SidebarConfig {}

/// `[behaviour]`: how the file manager acts when there is a choice to
/// make.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Behaviour {
    pub on_conflict: OnConflict,
    /// Ask before moving things to the trash. Off by default: the trash
    /// is the undo, and a question every time teaches people to click
    /// through questions.
    pub confirm_trash: bool,
    /// Ask before deleting for good. On by default, because there is no
    /// undo; turning it off is a choice somebody has to make.
    pub confirm_delete: bool,
    /// How long a copy or move runs before its progress line appears. A
    /// short job finishes before a bar could be read, and one that
    /// flashes up and vanishes reads as something going wrong.
    pub progress_after_ms: u64,
    /// How many steps Ctrl+Z can take back. `0` turns undo off.
    pub undo_depth: u64,
    /// How long "Moved 3 items to the Trash · Undo" stays in the status
    /// bar. `0` turns the notice off; Ctrl+Z still works.
    pub undo_notice_seconds: u64,
    /// Whether a folder on screen re-reads itself when something else
    /// changes it — a download finishing, a file saved from an editor.
    /// On by default: a listing that is quietly out of date is the thing
    /// a file manager must not be. Off is for someone who would rather
    /// press F5 than have rows move under the pointer.
    pub watch: bool,
    /// How often, in seconds, a folder on a network share is looked at
    /// again while it is shown. A share sends no change notices — the
    /// kernel only hears about changes made through this machine — so
    /// the only way to notice someone else's is to ask. Also what a
    /// local folder falls back to when the kernel's watch limit is
    /// spent.
    pub watch_network_every: u64,
}

impl Default for Behaviour {
    fn default() -> Self {
        Behaviour {
            on_conflict: OnConflict::Ask,
            confirm_trash: false,
            confirm_delete: true,
            progress_after_ms: 500,
            undo_depth: 20,
            undo_notice_seconds: 6,
            watch: true,
            watch_network_every: 3,
        }
    }
}

/// What a paste does when something is already at a destination.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum OnConflict {
    /// Stop and ask, with "apply to the rest" on offer. The default:
    /// the other three all decide something about a file the user has
    /// not been shown.
    #[default]
    Ask,
    /// Paste alongside, under a numbered name.
    KeepBoth,
    /// Leave what is there, and the pasted one where it was.
    Skip,
    /// Overwrite what is there. Configurable because some people want
    /// it; never the default, because it is the one that loses data.
    Replace,
}

impl OnConflict {
    fn parse(text: &str) -> Option<OnConflict> {
        match text.trim() {
            "ask" => Some(OnConflict::Ask),
            "keep-both" => Some(OnConflict::KeepBoth),
            "skip" => Some(OnConflict::Skip),
            "replace" => Some(OnConflict::Replace),
            _ => None,
        }
    }
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

/// A complaint from the shared keymap merge, in this file's own terms.
///
/// The two types are the same shape on purpose — both are a sentence for
/// the person who wrote the file, not an error to branch on — so every
/// problem from either half ends up in one list, in one place, and the
/// caller never has to know which half found it.
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
    menu: BTreeMap<String, Vec<String>>,
    behaviour: RawBehaviour,
    sidebar: RawSidebar,
}

/// `[sidebar]` as written — checked by hand, like `[behaviour]`.
#[derive(Debug, Default, Deserialize)]
#[serde(default, rename_all = "kebab-case")]
struct RawSidebar {
    places: Option<toml::Value>,
    show_trash: Option<toml::Value>,
    show_recent: Option<toml::Value>,
    show_starred: Option<toml::Value>,
    collapse_below: Option<toml::Value>,
    icons: Option<toml::Value>,
}

/// `[behaviour]` as written. Values are strings checked by hand rather
/// than enums serde would check, because a serde error fails the whole
/// file — and one misspelled value must cost one setting, not every
/// binding written above it.
#[derive(Debug, Default, Deserialize)]
#[serde(default, rename_all = "kebab-case")]
struct RawBehaviour {
    on_conflict: Option<String>,
    confirm_trash: Option<toml::Value>,
    confirm_delete: Option<toml::Value>,
    progress_after_ms: Option<toml::Value>,
    undo_depth: Option<toml::Value>,
    undo_notice_seconds: Option<toml::Value>,
    watch: Option<toml::Value>,
    watch_network_every: Option<toml::Value>,
}

/// A number within `range`, or a problem naming it. `name` includes
/// its section, as in `[behaviour] undo-depth`.
fn number(
    name: &str,
    value: &Option<toml::Value>,
    range: std::ops::RangeInclusive<i64>,
    current: u64,
    unit: &str,
    problems: &mut Vec<ConfigProblem>,
) -> u64 {
    match value {
        None => current,
        Some(toml::Value::Integer(n)) if range.contains(n) => *n as u64,
        Some(other) => {
            problems.push(problem(format!(
                "{name} = {other}: use {unit} from {} to {} — {current} until this is fixed",
                range.start(),
                range.end()
            )));
            current
        }
    }
}

/// A switch: `true` or `false`, or a problem naming it — checked by
/// hand for the same reason as the rest of `[behaviour]`. `name`
/// includes its section.
fn switch(
    name: &str,
    value: &Option<toml::Value>,
    default: bool,
    problems: &mut Vec<ConfigProblem>,
) -> bool {
    match value {
        None => default,
        Some(toml::Value::Boolean(on)) => *on,
        Some(other) => {
            problems.push(problem(format!(
                "{name} = {other}: use true or false — {} until this is fixed",
                if default { "on" } else { "off" }
            )));
            default
        }
    }
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
    let (keymap, mut problems) = keymap_with(&overrides);
    let (menus, menu_problems) = menus_with(&raw.menu);
    problems.extend(menu_problems);
    let mut behaviour = Behaviour::default();
    if let Some(text) = &raw.behaviour.on_conflict {
        match OnConflict::parse(text) {
            Some(choice) => behaviour.on_conflict = choice,
            None => problems.push(problem(format!(
                "[behaviour] on-conflict = \"{text}\": use ask, keep-both, skip or replace — \
                 asking until this is fixed"
            ))),
        }
    }
    behaviour.confirm_trash =
        switch("[behaviour] confirm-trash", &raw.behaviour.confirm_trash, behaviour.confirm_trash, &mut problems);
    behaviour.confirm_delete =
        switch("[behaviour] confirm-delete", &raw.behaviour.confirm_delete, behaviour.confirm_delete, &mut problems);
    behaviour.progress_after_ms = number(
        "[behaviour] progress-after-ms",
        &raw.behaviour.progress_after_ms,
        0..=60_000,
        behaviour.progress_after_ms,
        "milliseconds",
        &mut problems,
    );
    behaviour.undo_depth =
        number("[behaviour] undo-depth", &raw.behaviour.undo_depth, 0..=1_000, behaviour.undo_depth, "a count", &mut problems);
    behaviour.undo_notice_seconds = number(
        "[behaviour] undo-notice-seconds",
        &raw.behaviour.undo_notice_seconds,
        0..=600,
        behaviour.undo_notice_seconds,
        "seconds",
        &mut problems,
    );
    behaviour.watch = switch("[behaviour] watch", &raw.behaviour.watch, behaviour.watch, &mut problems);
    // Not below a second: every look at a share is a whole listing over
    // the network, and a faster one would be the polling costing more
    // than the folder is worth. Not above ten minutes, which is no
    // longer "live" in any sense.
    behaviour.watch_network_every = number(
        "[behaviour] watch-network-every",
        &raw.behaviour.watch_network_every,
        1..=600,
        behaviour.watch_network_every,
        "seconds",
        &mut problems,
    );
    let sidebar = sidebar_with(&raw.sidebar, &mut problems);
    (Config { keymap, menus, behaviour, sidebar }, problems)
}

/// `[sidebar]` over the defaults. An unknown place is reported and left
/// out; the others keep the file's order.
fn sidebar_with(raw: &RawSidebar, problems: &mut Vec<ConfigProblem>) -> SidebarConfig {
    let mut sidebar = SidebarConfig::default();
    match &raw.places {
        None => {}
        Some(toml::Value::Array(items)) => {
            let mut places = Vec::with_capacity(items.len());
            for item in items {
                match item.as_str().and_then(Place::from_id) {
                    Some(place) if places.contains(&place) => problems.push(problem(format!(
                        "[sidebar] places: \"{}\" is listed twice, so only the first is shown",
                        place.id()
                    ))),
                    Some(place) => places.push(place),
                    None => problems.push(problem(format!(
                        "[sidebar] places: {item} is not a place (use {}), so it was left out",
                        Place::ALL.map(Place::id).join(", ")
                    ))),
                }
            }
            sidebar.places = places;
        }
        Some(other) => problems.push(problem(format!(
            "[sidebar] places = {other}: use a list, like [\"home\", \"downloads\"] — \
             the usual places until this is fixed"
        ))),
    }
    sidebar.show_trash = switch("[sidebar] show-trash", &raw.show_trash, sidebar.show_trash, problems);
    sidebar.show_recent = switch("[sidebar] show-recent", &raw.show_recent, sidebar.show_recent, problems);
    sidebar.show_starred = switch("[sidebar] show-starred", &raw.show_starred, sidebar.show_starred, problems);
    sidebar.collapse_below = number(
        "[sidebar] collapse-below",
        &raw.collapse_below,
        0..=10_000,
        sidebar.collapse_below as u64,
        "a width in pixels",
        problems,
    ) as f32;
    sidebar.icons = icons_with(&raw.icons, problems);
    sidebar
}

/// `[sidebar.icons]`. Each bad line costs that line and is reported;
/// the rest still apply.
fn icons_with(raw: &Option<toml::Value>, problems: &mut Vec<ConfigProblem>) -> Vec<(IconFor, IconChoice)> {
    let table = match raw {
        None => return Vec::new(),
        Some(toml::Value::Table(table)) => table,
        Some(other) => {
            problems.push(problem(format!(
                "[sidebar] icons = {other}: use a table, like [sidebar.icons] with downloads = \"folder-cloud\""
            )));
            return Vec::new();
        }
    };
    let mut icons = Vec::new();
    for (key, value) in table {
        let target = if key == "trash" {
            Some(IconFor::Trash)
        } else if let Some(place) = Place::from_id(key) {
            Some(IconFor::Place(place))
        } else {
            expand_home(key).filter(|p| p.is_absolute()).map(IconFor::Folder)
        };
        let Some(target) = target else {
            problems.push(problem(format!(
                "[sidebar.icons] {key} is not a place ({}, trash) or a folder's path, so it was left out",
                Place::ALL.map(Place::id).join(", ")
            )));
            continue;
        };
        let choice = match value.as_str().map(str::trim) {
            Some(v) if v.contains('/') => expand_home(v).map(IconChoice::File),
            Some(v) if !v.is_empty() => Some(IconChoice::Themed(v.to_string())),
            _ => None,
        };
        match choice {
            Some(choice) => icons.push((target, choice)),
            None => problems.push(problem(format!(
                "[sidebar.icons] {key} = {value}: use an icon name like \"folder-cloud\", or an image's path"
            ))),
        }
    }
    icons
}

/// `~/x` to the home directory's `x`; anything else as written.
fn expand_home(text: &str) -> Option<std::path::PathBuf> {
    match text.strip_prefix("~/") {
        Some(rest) => std::env::var_os("HOME").map(|home| std::path::PathBuf::from(home).join(rest)),
        None => Some(std::path::PathBuf::from(text)),
    }
}

/// The default menus with `[menu]` applied.
///
/// A menu the file names is *replaced* by the file's list, in the
/// file's order — a menu is an ordered whole, and merging two orders has
/// no answer anyone would predict. A menu the file does not name keeps
/// its default. An unknown id is reported and left out; the rest of that
/// menu still shows.
pub fn menus_with(overrides: &BTreeMap<String, Vec<String>>) -> (MenuConfig, Vec<ConfigProblem>) {
    let mut menus = MenuConfig::default();
    let mut problems = Vec::new();
    for (name, ids) in overrides {
        let Some(kind) = MenuKind::all().into_iter().find(|k| k.id() == name) else {
            problems.push(problem(format!(
                "[menu] {name}: there is no menu with that name (use {})",
                MenuKind::all().map(|k| k.id()).join(", ")
            )));
            continue;
        };
        let mut entries = Vec::with_capacity(ids.len());
        for id in ids {
            match MenuEntry::parse(id) {
                Some(entry) => entries.push(entry),
                None => problems.push(problem(format!(
                    "[menu] {name}: \"{id}\" is not an action, so it was left out"
                ))),
            }
        }
        menus.set(kind, entries);
    }
    (menus, problems)
}

/// The default keymap with `overrides` applied.
///
/// A thin wrapper now: the rules themselves — an entry replaces an
/// action's defaults rather than adding to them, an empty list unbinds,
/// two entries on one key are resolved by lowest id with the loser
/// reported, and an action left with no key at all is reported — live in
/// `hyprforge_keys::merge`, because they are decisions about a *file
/// format* and the image viewer reads the same one. Two apps re-deriving
/// them would produce two formats wearing one name.
///
/// What stays here is this app's answer to the one question that merge
/// cannot answer for itself: whether a bare printable key may be bound.
/// See [`crate::keymap::BARE_KEYS`].
pub fn keymap_with(overrides: &BTreeMap<String, Vec<String>>) -> (Keymap, Vec<ConfigProblem>) {
    let (keymap, problems) = hyprforge_keys::merge::keymap_with(overrides, crate::keymap::BARE_KEYS);
    (keymap, problems.into_iter().map(ConfigProblem::from).collect())
}

#[cfg(test)]
mod tests {
    use super::*;
    // `Action` and `Combo` are the test module's own now: with the merge
    // rules delegated to `hyprforge-keys`, nothing outside these tests
    // names either one.
    use crate::action::Action;
    use crate::keymap::{Combo, KeyPress, Resolved};

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
        let (config, problems) = parsed("[keys]\ntrash = \"Ctrl+J\"\n");
        assert!(problems.is_empty(), "{problems:?}");
        assert_eq!(does(&config, "Ctrl+J"), Some(Action::Trash));
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
        let (config, problems) = parsed("[keys]\ntrash = \"Ctrl+J\"\nclear-search = \"Ctrl+J\"\n");
        // `clear-search` sorts before `trash`, so it keeps the key.
        assert_eq!(does(&config, "Ctrl+J"), Some(Action::ClearSearch));
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

    /// Sections this version does not know yet — ones a later version
    /// adds — must not make the whole file unreadable.
    #[test]
    fn an_unknown_section_is_ignored_rather_than_fatal() {
        let (config, problems) =
            parsed("[from-the-future]\nwidth = 3\n[keys]\ntrash = \"Ctrl+J\"\n");
        assert!(problems.is_empty(), "{problems:?}");
        assert_eq!(does(&config, "Ctrl+J"), Some(Action::Trash));
    }

    #[test]
    fn a_menu_named_in_the_file_is_replaced_in_the_files_order() {
        let (config, problems) = parsed("[menu]\nentry = [\"trash\", \"-\", \"open\"]\n");
        assert!(problems.is_empty(), "{problems:?}");
        assert_eq!(
            config.menus.entry,
            [
                MenuEntry::Action(Action::Trash),
                MenuEntry::Separator,
                MenuEntry::Action(Action::Open)
            ]
        );
        assert_eq!(config.menus.folder, MenuConfig::default().folder, "untouched menus keep defaults");
    }

    #[test]
    fn an_unknown_menu_item_is_left_out_and_the_rest_still_shows() {
        let (config, problems) = parsed("[menu]\nentry = [\"open\", \"frobnicate\"]\nsidebar = []\n");
        assert_eq!(config.menus.entry, [MenuEntry::Action(Action::Open)]);
        assert_eq!(problems.len(), 2, "{problems:?}");
        assert!(problems.iter().any(|p| p.message.contains("frobnicate")));
        assert!(problems.iter().any(|p| p.message.contains("sidebar")));
    }

    #[test]
    fn asking_is_the_default_for_a_conflict() {
        assert_eq!(Config::default().behaviour.on_conflict, OnConflict::Ask);
    }

    #[test]
    fn the_conflict_policy_is_configurable() {
        let (config, problems) = parsed("[behaviour]\non-conflict = \"keep-both\"\n");
        assert!(problems.is_empty(), "{problems:?}");
        assert_eq!(config.behaviour.on_conflict, OnConflict::KeepBoth);
    }

    /// A misspelled value costs that one setting — and it falls back to
    /// asking, never to one of the choices that decides for the user.
    #[test]
    fn a_misspelled_conflict_policy_asks_and_keeps_the_rest_of_the_file() {
        let (config, problems) =
            parsed("[keys]\ntrash = \"Ctrl+J\"\n[behaviour]\non-conflict = \"overwrite\"\n");
        assert_eq!(config.behaviour.on_conflict, OnConflict::Ask);
        assert_eq!(problems.len(), 1, "{problems:?}");
        assert!(problems[0].message.contains("overwrite"));
        assert_eq!(does(&config, "Ctrl+J"), Some(Action::Trash), "the keys still applied");
    }

    #[test]
    fn deleting_asks_and_trashing_does_not_unless_configured() {
        let defaults = Config::default().behaviour;
        assert!(defaults.confirm_delete);
        assert!(!defaults.confirm_trash);
        let (config, problems) =
            parsed("[behaviour]\nconfirm-trash = true\nconfirm-delete = false\n");
        assert!(problems.is_empty(), "{problems:?}");
        assert!(config.behaviour.confirm_trash);
        assert!(!config.behaviour.confirm_delete);
    }

    /// A switch written as a string keeps its safe default and says so.
    #[test]
    fn a_switch_that_is_not_true_or_false_keeps_its_default() {
        let (config, problems) = parsed("[behaviour]\nconfirm-delete = \"no\"\n");
        assert!(config.behaviour.confirm_delete, "still asks");
        assert_eq!(problems.len(), 1, "{problems:?}");
        assert!(problems[0].message.contains("confirm-delete"));
    }

    #[test]
    fn the_sidebar_places_are_chosen_and_ordered_by_the_file() {
        let (config, problems) = parsed("[sidebar]\nplaces = [\"downloads\", \"home\"]\nshow-trash = false\n");
        assert!(problems.is_empty(), "{problems:?}");
        assert_eq!(config.sidebar.places, vec![Place::Downloads, Place::Home]);
        assert!(!config.sidebar.show_trash);
        assert_eq!(config.sidebar.collapse_below, SidebarConfig::default().collapse_below);
    }

    /// A place, the trash, or a folder's path, each given a theme icon
    /// name or an image — and a slash is what tells the two apart.
    #[test]
    fn sidebar_icons_are_chosen_by_place_trash_or_path() {
        let home = std::env::var("HOME").unwrap();
        let (config, problems) = parsed(
            "[sidebar.icons]\ndownloads = \"folder-cloud\"\ntrash = \"user-trash-full\"\n\
             \"~/Projects\" = \"~/icons/p.svg\"\n\"/mnt/data\" = \"drive-harddisk\"\n",
        );
        assert!(problems.is_empty(), "{problems:?}");
        let icons = &config.sidebar;
        assert_eq!(
            icons.icon_for(&IconFor::Place(Place::Downloads)),
            Some(&IconChoice::Themed("folder-cloud".into()))
        );
        assert_eq!(icons.icon_for(&IconFor::Trash), Some(&IconChoice::Themed("user-trash-full".into())));
        assert_eq!(
            icons.icon_for(&IconFor::Folder(format!("{home}/Projects").into())),
            Some(&IconChoice::File(format!("{home}/icons/p.svg").into())),
            "~ is the home directory, in the folder and in the file"
        );
        assert_eq!(
            icons.icon_for(&IconFor::Folder("/mnt/data".into())),
            Some(&IconChoice::Themed("drive-harddisk".into()))
        );
        assert_eq!(icons.icon_for(&IconFor::Place(Place::Home)), None, "unchosen keeps the theme's");
    }

    /// One bad line costs that line: a key that is neither a place nor a
    /// path, and a value that is not a name.
    #[test]
    fn a_bad_sidebar_icon_line_is_reported_and_the_rest_apply() {
        let (config, problems) =
            parsed("[sidebar.icons]\ndownlaods = \"x\"\nmusic = 3\npictures = \"folder-images\"\n");
        assert_eq!(problems.len(), 2, "{problems:?}");
        assert!(problems.iter().any(|p| p.message.contains("downlaods")));
        assert!(problems.iter().any(|p| p.message.contains("music")));
        assert_eq!(config.sidebar.icons.len(), 1);
    }

    /// One misspelled place costs that place, not the list.
    #[test]
    fn an_unknown_or_repeated_place_is_left_out_and_reported() {
        let (config, problems) =
            parsed("[sidebar]\nplaces = [\"home\", \"downlaods\", \"home\", \"music\"]\n");
        assert_eq!(config.sidebar.places, vec![Place::Home, Place::Music]);
        assert_eq!(problems.len(), 2, "{problems:?}");
        assert!(problems[0].message.contains("downlaods"), "{problems:?}");
        assert!(problems[1].message.contains("twice"), "{problems:?}");
    }

    #[test]
    fn the_collapse_width_is_configurable_and_zero_turns_it_off() {
        let (config, problems) = parsed("[sidebar]\ncollapse-below = 0\n");
        assert!(problems.is_empty());
        assert_eq!(config.sidebar.collapse_below, 0.0);
        let (config, problems) = parsed("[sidebar]\ncollapse-below = \"wide\"\nplaces = \"home\"\n");
        assert_eq!(config.sidebar, SidebarConfig::default(), "both bad values keep their defaults");
        assert_eq!(problems.len(), 2, "{problems:?}");
    }

    #[test]
    fn the_progress_delay_is_configurable_within_reason() {
        assert_eq!(Config::default().behaviour.progress_after_ms, 500);
        let (config, problems) = parsed("[behaviour]\nprogress-after-ms = 0\n");
        assert!(problems.is_empty(), "{problems:?}");
        assert_eq!(config.behaviour.progress_after_ms, 0);
        let (config, problems) = parsed("[behaviour]\nprogress-after-ms = -5\n");
        assert_eq!(config.behaviour.progress_after_ms, 500);
        assert_eq!(problems.len(), 1);
    }

    #[test]
    fn undo_is_configurable_and_on_by_default() {
        let defaults = Config::default().behaviour;
        assert_eq!(defaults.undo_depth, 20);
        assert_eq!(defaults.undo_notice_seconds, 6);
        let (config, problems) = parsed("[behaviour]\nundo-depth = 0\nundo-notice-seconds = 0\n");
        assert!(problems.is_empty(), "{problems:?}");
        assert_eq!(config.behaviour.undo_depth, 0);
        assert_eq!(config.behaviour.undo_notice_seconds, 0);
    }

    #[test]
    fn live_updates_are_on_by_default_and_a_share_is_looked_at_every_three_seconds() {
        let defaults = Config::default().behaviour;
        assert!(defaults.watch);
        assert_eq!(defaults.watch_network_every, 3);
        let (config, problems) = parsed("[behaviour]\nwatch = false\nwatch-network-every = 10\n");
        assert!(problems.is_empty(), "{problems:?}");
        assert!(!config.behaviour.watch);
        assert_eq!(config.behaviour.watch_network_every, 10);
    }

    /// Zero would be a share listed as fast as the loop can go.
    #[test]
    fn a_network_poll_interval_of_zero_is_refused_and_the_default_kept() {
        let (config, problems) = parsed("[behaviour]\nwatch-network-every = 0\n");
        assert_eq!(config.behaviour.watch_network_every, 3);
        assert_eq!(problems.len(), 1, "{problems:?}");
        assert!(problems[0].message.contains("watch-network-every"), "{problems:?}");
    }

    /// The config is read, never written — this module has no function
    /// that could, and a file with comments comes back from `load`
    /// unchanged on disk.
    #[test]
    fn loading_leaves_the_file_exactly_as_written() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("files-config.toml");
        let text = "# my bindings\n[keys]\ntrash = \"Ctrl+J\"  # muscle memory\n";
        std::fs::write(&path, text).unwrap();
        let _ = load_from(&path);
        assert_eq!(std::fs::read_to_string(&path).unwrap(), text);
    }
}
