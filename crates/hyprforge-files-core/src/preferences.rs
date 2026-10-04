//! The Preferences sheet's decisions, as data: what each setting changes,
//! and what binding a key would actually do (mockup `1h`).
//!
//! Behaviour and key bindings only. **Appearance is not here**, and that
//! is a decision rather than an omission: colours, fonts, the accent and
//! the corner radius belong to the Settings app, which every Hyprforge
//! window reads them from — a Files-only colour setting would be the
//! drift the suite's single `Theme` exists to prevent. The sheet says so
//! where someone would look for it.
//!
//! The settings come from two files, and the split is the one the app
//! already had rather than a new one:
//!
//! - **What the listing remembers** (`files.toml`, rewritten whole):
//!   hidden files, folders first, the preview pane, the view, the
//!   sidebar. These are [`Setting`]s — the same fields the toolbar's own
//!   switches change, so the sheet and the toolbar cannot disagree.
//! - **What the person configured** (`files-config.toml`, hand-written):
//!   what a paste does on a clash, whether trashing and deleting ask, and
//!   every key binding. Edited one entry at a time by
//!   [`crate::config_edit`], never rewritten.
//!
//! A binding is the subtle one. `[keys]` *replaces* an action's keys, a
//! key can only mean one thing, and a bare letter belongs to the search
//! box — so "press a key for Rename" can mean taking a key from another
//! action, or be refused outright. [`plan_binding`] works that out before
//! anything is written, so the sheet can show the clash and ask, rather
//! than write a file the loader would then have to complain about.

use crate::action::{Action, Scope};
use crate::config::{Behaviour, OnConflict};
use crate::config_edit::{BehaviourValue, Edit};
use crate::keymap::{Combo, KeyPress, Keymap, Resolved};
use crate::prefs::{Prefs, SidebarPref, ViewMode};

/// One of the listing's remembered settings, as the sheet changes it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Setting {
    ShowHidden(bool),
    DirectoriesFirst(bool),
    PreviewPane(bool),
    View(ViewMode),
    Sidebar(SidebarPref),
}

impl Setting {
    /// Applies this to `prefs` — what every tab does on
    /// `Message::Adopt`, and what the host saves.
    pub fn apply(self, prefs: &mut Prefs) {
        match self {
            Setting::ShowHidden(on) => prefs.show_hidden = on,
            Setting::DirectoriesFirst(on) => prefs.directories_first = on,
            Setting::PreviewPane(on) => prefs.preview_pane = on,
            Setting::View(mode) => prefs.view_mode = mode,
            Setting::Sidebar(pref) => prefs.sidebar = pref,
        }
    }
}

/// One of `[behaviour]`'s settings the sheet offers.
///
/// Not all of them: the progress delay, the undo depth and how long the
/// undo notice stays are numbers a person tunes once by hand if ever, and
/// a slider for "milliseconds before a progress bar" is a control nobody
/// would know what to do with. Those stay in the file, which the sheet
/// names.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BehaviourSetting {
    OnConflict(OnConflict),
    ConfirmTrash(bool),
    ConfirmDelete(bool),
}

impl BehaviourSetting {
    /// The `files-config.toml` edit that makes this so.
    pub fn edit(self) -> Edit {
        match self {
            BehaviourSetting::OnConflict(policy) => Edit::Behaviour("on-conflict", BehaviourValue::Word(conflict_id(policy))),
            BehaviourSetting::ConfirmTrash(on) => Edit::Behaviour("confirm-trash", BehaviourValue::Switch(on)),
            BehaviourSetting::ConfirmDelete(on) => Edit::Behaviour("confirm-delete", BehaviourValue::Switch(on)),
        }
    }

    /// Whether `behaviour` already says this — a click on the value
    /// already chosen writes nothing.
    pub fn holds_in(self, behaviour: &Behaviour) -> bool {
        match self {
            BehaviourSetting::OnConflict(policy) => behaviour.on_conflict == policy,
            BehaviourSetting::ConfirmTrash(on) => behaviour.confirm_trash == on,
            BehaviourSetting::ConfirmDelete(on) => behaviour.confirm_delete == on,
        }
    }
}

/// The word `[behaviour] on-conflict` uses for a policy.
pub fn conflict_id(policy: OnConflict) -> &'static str {
    match policy {
        OnConflict::Ask => "ask",
        OnConflict::KeepBoth => "keep-both",
        OnConflict::Skip => "skip",
        OnConflict::Replace => "replace",
    }
}

/// What the sheet calls a policy.
pub fn conflict_label(policy: OnConflict) -> &'static str {
    match policy {
        OnConflict::Ask => "Ask",
        OnConflict::KeepBoth => "Keep both",
        OnConflict::Skip => "Skip",
        OnConflict::Replace => "Replace",
    }
}

pub const CONFLICT_POLICIES: [OnConflict; 4] =
    [OnConflict::Ask, OnConflict::KeepBoth, OnConflict::Skip, OnConflict::Replace];

/// The heading an action is listed under. A list of fifty actions with
/// no headings is a list nobody can find anything in; these follow the
/// order `Action::all` already keeps them in.
pub fn group(action: Action) -> &'static str {
    use Action::*;
    match action {
        Open | OpenInNewTab | ShowInFolder | GoUp | GoBack | GoForward | EditLocation | CommandPalette | Refresh => {
            "Going places"
        }
        FocusUp | FocusDown | FocusLeft | FocusRight | ExtendUp | ExtendDown | SelectAll | ClearSearch
        | ContextMenu => "Selecting",
        Trash | DeletePermanently | Restore | EmptyTrash | Copy | Cut | Paste | CopyPath | Rename | NewFolder
        | OpenWith | Extract | ExtractTo | Compress | Undo => "Files",
        ToggleHidden | TogglePreview | Properties | Preferences | Transfers | TransferQueue => "The window",
        Pin | Unpin | PinUp | PinDown => "Sidebar",
        NewTab | CloseTab | NextTab | PreviousTab | Tab(_) => "Tabs",
    }
}

/// The groups, in the order the sheet shows them.
pub const GROUPS: [&str; 6] = ["Going places", "Selecting", "Files", "The window", "Sidebar", "Tabs"];

/// One action's line in the key bindings list.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BindingRow {
    pub action: Action,
    pub keys: Vec<Combo>,
    /// Whether `files-config.toml` names this action — what "Reset"
    /// undoes.
    pub customised: bool,
    /// Whether it is bound to what it ships with, whatever the file
    /// says. A line in the file that happens to restate the default is
    /// still offered a reset, because the line is there.
    pub is_default: bool,
}

/// Every action's line, in `Action::all`'s order.
pub fn binding_rows(keymap: &Keymap, customised: &std::collections::BTreeSet<String>) -> Vec<BindingRow> {
    Action::all()
        .into_iter()
        .map(|action| {
            let keys = keymap.combos_for(action);
            let mut defaults: Vec<Combo> =
                action.default_keys().iter().filter_map(|text| Combo::parse(text).ok()).collect();
            defaults.sort_by_key(|c| c.to_string());
            BindingRow {
                action,
                is_default: keys == defaults,
                customised: customised.contains(action.id()),
                keys,
            }
        })
        .collect()
}

/// Whether a captured key replaces an action's keys or joins them.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Capture {
    Replace,
    Add,
}

/// What binding a captured key would do.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Plan {
    /// Write these, and nothing else changes.
    Bind(Vec<Edit>),
    /// The key already belongs to `holder`. Binding it anyway takes it
    /// from `holder` — the edits for that are carried, to be written if
    /// the person says yes.
    Taken { holder: Action, edits: Vec<Edit> },
    /// It cannot be bound; the sentence says why.
    Refused(String),
    /// It is already this action's key. Nothing to write.
    Already,
}

/// What pressing `combo` while capturing for `action` would do, against
/// the keymap as it stands.
///
/// Every edit is a whole `[keys]` line for one action, because a line
/// *replaces* that action's keys: taking Ctrl+R from Refresh is writing
/// Refresh's line as "its keys without Ctrl+R" — `F5`, here — not
/// deleting something. The same reasoning makes clearing write `[]`
/// rather than delete the line, which would bring the defaults back.
pub fn plan_binding(keymap: &Keymap, action: Action, combo: Combo, capture: Capture) -> Plan {
    // A bare letter, or Shift and a letter, is how a character gets
    // typed into the search box; the loader refuses one, so the sheet
    // refuses it first rather than writing a line that will be ignored.
    if keymap.bare_keys() == hyprforge_keys::BareKeys::ReservedForTyping && combo.would_swallow_typing() {
        return Plan::Refused(format!(
            "{combo} types into the search box, so it can't be a shortcut. Hold Ctrl or Alt with it."
        ));
    }
    let current = keymap.combos_for(action);
    if current.contains(&combo) {
        return Plan::Already;
    }
    let mut keys = match capture {
        Capture::Replace => Vec::new(),
        Capture::Add => current,
    };
    keys.push(combo);
    let edit = Edit::Keys(action, Some(keys));
    match holder_of(keymap, combo) {
        Some(holder) if holder != action => {
            let theirs: Vec<Combo> = keymap.combos_for(holder).into_iter().filter(|c| *c != combo).collect();
            Plan::Taken { holder, edits: vec![Edit::Keys(holder, Some(theirs)), edit] }
        }
        _ => Plan::Bind(vec![edit]),
    }
}

/// The action `combo` is bound to now, if any.
pub fn holder_of(keymap: &Keymap, combo: Combo) -> Option<Action> {
    match keymap.resolve(&KeyPress { key: combo.key, mods: combo.mods, text: None }) {
        Some(Resolved::Action(action)) => Some(action),
        _ => None,
    }
}

/// The edit that leaves `action` with no key at all.
pub fn clear(action: Action) -> Edit {
    Edit::Keys(action, Some(Vec::new()))
}

/// The edit that puts `action` back on the keys it ships with.
pub fn reset(action: Action) -> Edit {
    Edit::Keys(action, None)
}

/// Keys whose meaning the open/save dialog never sees — said beside the
/// window-scope rows, so nobody rebinds New Tab and wonders why the
/// dialog ignores it.
pub fn only_in_the_window(action: Action) -> bool {
    action.scope() == Scope::Window
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::keymap_with;
    use std::collections::BTreeMap;

    fn combo(text: &str) -> Combo {
        Combo::parse(text).unwrap()
    }

    fn keymap() -> Keymap {
        crate::keymap::defaults()
    }

    /// Applies a plan's edits to an empty file and loads it, the way the
    /// sheet's write and the window's reload will.
    fn after(edits: &[Edit]) -> (Keymap, Vec<crate::config::ConfigProblem>) {
        let text = crate::config_edit::apply("", edits, std::path::Path::new("x")).unwrap();
        let (config, problems) = crate::config::parse(&text, std::path::Path::new("x"));
        (config.keymap, problems)
    }

    #[test]
    fn a_free_key_is_simply_bound() {
        let plan = plan_binding(&keymap(), Action::Properties, combo("Ctrl+I"), Capture::Replace);
        let Plan::Bind(edits) = plan else { panic!("{plan:?}") };
        let (keys, problems) = after(&edits);
        assert!(problems.is_empty(), "{problems:?}");
        assert_eq!(keys.combos_for(Action::Properties), vec![combo("Ctrl+I")]);
    }

    #[test]
    fn adding_keeps_the_keys_an_action_already_had() {
        let plan = plan_binding(&keymap(), Action::Rename, combo("Ctrl+E"), Capture::Add);
        let Plan::Bind(edits) = plan else { panic!("{plan:?}") };
        let (keys, _) = after(&edits);
        assert_eq!(keys.combos_for(Action::Rename), vec![combo("Ctrl+E"), combo("F2")]);
    }

    /// The conflict is named, and saying yes takes the key from its
    /// holder while leaving the holder its other keys — so the file
    /// that results loads with nothing to complain about.
    #[test]
    fn a_taken_key_names_its_holder_and_taking_it_leaves_no_clash_behind() {
        let plan = plan_binding(&keymap(), Action::Rename, combo("Ctrl+R"), Capture::Replace);
        let Plan::Taken { holder, edits } = plan else { panic!("{plan:?}") };
        assert_eq!(holder, Action::Refresh);
        let (keys, problems) = after(&edits);
        assert!(problems.is_empty(), "{problems:?}");
        assert_eq!(keys.combos_for(Action::Rename), vec![combo("Ctrl+R")]);
        assert_eq!(keys.combos_for(Action::Refresh), vec![combo("F5")], "Refresh keeps F5");
    }

    #[test]
    fn a_letter_on_its_own_belongs_to_the_search_box() {
        let plan = plan_binding(&keymap(), Action::Rename, combo("N"), Capture::Replace);
        assert!(matches!(plan, Plan::Refused(_)), "{plan:?}");
        let plan = plan_binding(&keymap(), Action::Rename, combo("Shift+N"), Capture::Replace);
        assert!(matches!(plan, Plan::Refused(_)), "Shift+N is a capital N: {plan:?}");
        let plan = plan_binding(&keymap(), Action::Rename, combo("Alt+N"), Capture::Replace);
        assert!(matches!(plan, Plan::Bind(_)), "{plan:?}");
    }

    #[test]
    fn pressing_a_key_the_action_already_has_writes_nothing() {
        assert_eq!(plan_binding(&keymap(), Action::Rename, combo("F2"), Capture::Add), Plan::Already);
    }

    #[test]
    fn clearing_unbinds_and_resetting_brings_the_shipped_keys_back() {
        let (keys, _) = after(&[clear(Action::Refresh)]);
        assert!(keys.combos_for(Action::Refresh).is_empty());
        let (keys, _) = after(&[clear(Action::Refresh), reset(Action::Refresh)]);
        assert_eq!(keys.combos_for(Action::Refresh), vec![combo("Ctrl+R"), combo("F5")]);
    }

    #[test]
    fn a_row_knows_whether_it_is_the_persons_own_and_whether_it_is_the_default() {
        let mut overrides = BTreeMap::new();
        overrides.insert("trash".to_string(), vec!["Ctrl+Delete".to_string()]);
        overrides.insert("rename".to_string(), vec!["F2".to_string()]);
        let (keymap, _) = keymap_with(&overrides);
        let customised = overrides.keys().cloned().collect();
        let rows = binding_rows(&keymap, &customised);
        let row = |action| rows.iter().find(|r| r.action == action).unwrap();
        assert!(row(Action::Trash).customised && !row(Action::Trash).is_default);
        assert!(row(Action::Rename).customised && row(Action::Rename).is_default, "restates the default");
        assert!(!row(Action::Copy).customised && row(Action::Copy).is_default);
        assert_eq!(rows.len(), Action::all().len(), "every action has a line");
    }

    #[test]
    fn every_action_is_listed_under_a_heading_the_sheet_shows() {
        for action in Action::all() {
            assert!(GROUPS.contains(&group(action)), "{action:?}");
        }
    }

    #[test]
    fn a_setting_changes_the_field_the_toolbar_changes() {
        let mut prefs = Prefs::default();
        Setting::ShowHidden(true).apply(&mut prefs);
        Setting::DirectoriesFirst(false).apply(&mut prefs);
        Setting::View(ViewMode::Grid).apply(&mut prefs);
        assert!(prefs.show_hidden && !prefs.directories_first);
        assert_eq!(prefs.view_mode, ViewMode::Grid);
    }

    #[test]
    fn a_behaviour_already_in_force_is_recognised() {
        let behaviour = Behaviour::default();
        assert!(BehaviourSetting::OnConflict(OnConflict::Ask).holds_in(&behaviour));
        assert!(!BehaviourSetting::ConfirmTrash(true).holds_in(&behaviour));
        for policy in CONFLICT_POLICIES {
            let Edit::Behaviour(_, BehaviourValue::Word(word)) = BehaviourSetting::OnConflict(policy).edit() else {
                panic!()
            };
            let (config, problems) =
                crate::config::parse(&format!("[behaviour]\non-conflict = \"{word}\"\n"), std::path::Path::new("x"));
            assert!(problems.is_empty());
            assert_eq!(config.behaviour.on_conflict, policy, "{word}");
        }
    }
}
