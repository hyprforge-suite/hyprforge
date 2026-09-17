//! Everything a person can ask the file manager to do, as data.
//!
//! One vocabulary for every way of asking. A key press, a menu item and a
//! toolbar button all end in an [`Action`], and [`crate::browser::Browser::perform`]
//! is the one place an action is carried out. Before this, a key's
//! meaning was decided in three places — the window checked Delete
//! itself, then its own Ctrl shortcuts, then `keymap::resolve` — and the
//! precedence between them was a fact you could only learn by reading
//! the function top to bottom. That cannot be made configurable, because
//! there is no single table to configure.
//!
//! Each action has a stable id (`"trash"`, `"tab-3"`). That id is what
//! `files-config.toml` names, so it is part of the file format: renaming one
//! breaks somebody's config. [`Action::from_id`] and [`Action::id`] are
//! tested to round-trip over [`Action::all`] so a new variant cannot be
//! added without one.
//!
//! Actions come in two scopes. Most are about the listing and belong to
//! [`crate::browser::Browser`], which the open/save dialog also renders.
//! A few — tabs — are about the *window*, which the dialog does not
//! have. [`Scope`] says which, so a host can ignore what it has no
//! notion of rather than every host re-deriving the split.

/// One thing a person can ask for.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Action {
    /// Open the focused entry: a folder navigates into itself, a file is
    /// handed to the host.
    Open,
    /// Open the focused folder in a new tab.
    OpenInNewTab,
    GoUp,
    GoBack,
    GoForward,
    /// Move the keyboard focus one row, collapsing the selection onto it.
    FocusUp,
    FocusDown,
    FocusLeft,
    FocusRight,
    /// Move the keyboard focus one row and extend the selection to it,
    /// from the same anchor a shift-click uses.
    ExtendUp,
    ExtendDown,
    SelectAll,
    ClearSearch,
    /// Show or hide dotfiles. Remembered.
    ToggleHidden,
    /// Move the selection to the trash.
    Trash,
    /// Delete the selection for good, bypassing the trash.
    DeletePermanently,
    /// Put the selected trashed items back where they came from.
    Restore,
    /// Delete everything in the Trash for good.
    EmptyTrash,
    /// Put the selection on the clipboard, to be copied when pasted.
    Copy,
    /// Put the selection on the clipboard, to be moved when pasted.
    Cut,
    /// Put what the clipboard holds into this folder.
    Paste,
    /// Put the selected paths on the clipboard as text.
    CopyPath,
    /// Read this folder again.
    Refresh,
    /// Edit the selected name in place.
    Rename,
    /// Make a folder here, and start naming it.
    NewFolder,
    /// Open the right-click menu from the keyboard, for the focused row.
    ContextMenu,
    // Window scope: the tab strip.
    NewTab,
    CloseTab,
    NextTab,
    PreviousTab,
    /// Jump to tab `n`, counted from 1 the way the keys are labelled.
    /// Only 1 to 9 exist, because those are the keys there are.
    Tab(u8),
}

/// Whether an action is about the listing or about the window around it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Scope {
    /// Carried out by [`crate::browser::Browser::perform`] — available in
    /// the app and in the dialog alike.
    Browser,
    /// Carried out by the host window. A host with no tabs ignores these.
    Window,
}

impl Action {
    /// Every action, in the order a settings page or a test would list
    /// them.
    pub fn all() -> Vec<Action> {
        let mut all = vec![
            Action::Open,
            Action::OpenInNewTab,
            Action::GoUp,
            Action::GoBack,
            Action::GoForward,
            Action::FocusUp,
            Action::FocusDown,
            Action::FocusLeft,
            Action::FocusRight,
            Action::ExtendUp,
            Action::ExtendDown,
            Action::SelectAll,
            Action::ClearSearch,
            Action::ToggleHidden,
            Action::Trash,
            Action::DeletePermanently,
            Action::Restore,
            Action::EmptyTrash,
            Action::Copy,
            Action::Cut,
            Action::Paste,
            Action::CopyPath,
            Action::Refresh,
            Action::Rename,
            Action::NewFolder,
            Action::ContextMenu,
            Action::NewTab,
            Action::CloseTab,
            Action::NextTab,
            Action::PreviousTab,
        ];
        all.extend((1..=9).map(Action::Tab));
        all
    }

    /// The id `files-config.toml` uses for this action. Part of the file format.
    pub fn id(self) -> &'static str {
        match self {
            Action::Open => "open",
            Action::OpenInNewTab => "open-in-new-tab",
            Action::GoUp => "go-up",
            Action::GoBack => "go-back",
            Action::GoForward => "go-forward",
            Action::FocusUp => "focus-up",
            Action::FocusDown => "focus-down",
            Action::FocusLeft => "focus-left",
            Action::FocusRight => "focus-right",
            Action::ExtendUp => "extend-up",
            Action::ExtendDown => "extend-down",
            Action::SelectAll => "select-all",
            Action::ClearSearch => "clear-search",
            Action::ToggleHidden => "show-hidden",
            Action::Trash => "trash",
            Action::DeletePermanently => "delete-permanently",
            Action::Restore => "restore",
            Action::EmptyTrash => "empty-trash",
            Action::Copy => "copy",
            Action::Cut => "cut",
            Action::Paste => "paste",
            Action::CopyPath => "copy-path",
            Action::Refresh => "refresh",
            Action::Rename => "rename",
            Action::NewFolder => "new-folder",
            Action::ContextMenu => "context-menu",
            Action::NewTab => "new-tab",
            Action::CloseTab => "close-tab",
            Action::NextTab => "next-tab",
            Action::PreviousTab => "previous-tab",
            Action::Tab(n) => match n {
                1 => "tab-1",
                2 => "tab-2",
                3 => "tab-3",
                4 => "tab-4",
                5 => "tab-5",
                6 => "tab-6",
                7 => "tab-7",
                8 => "tab-8",
                // `Tab` is only ever built from 1..=9 — by `all()` and
                // by `from_id` — so anything else is a bug in this file,
                // and naming it after the last real tab is safer than a
                // panic in a key handler.
                _ => "tab-9",
            },
        }
    }

    /// The action a `files-config.toml` id names, or `None` for an id this
    /// version does not know — which the caller reports, never ignores.
    pub fn from_id(id: &str) -> Option<Action> {
        Action::all().into_iter().find(|a| a.id() == id)
    }

    /// What a menu or a settings page calls this action.
    pub fn label(self) -> &'static str {
        match self {
            Action::Open => "Open",
            Action::OpenInNewTab => "Open in New Tab",
            Action::GoUp => "Go Up",
            Action::GoBack => "Back",
            Action::GoForward => "Forward",
            Action::FocusUp => "Previous Item",
            Action::FocusDown => "Next Item",
            Action::FocusLeft => "Item to the Left",
            Action::FocusRight => "Item to the Right",
            Action::ExtendUp => "Extend Selection Up",
            Action::ExtendDown => "Extend Selection Down",
            Action::SelectAll => "Select All",
            Action::ClearSearch => "Clear Search",
            Action::ToggleHidden => "Show Hidden Files",
            Action::Trash => "Move to Trash",
            Action::DeletePermanently => "Delete Permanently",
            Action::Restore => "Restore",
            Action::EmptyTrash => "Empty Trash",
            Action::Copy => "Copy",
            Action::Cut => "Cut",
            Action::Paste => "Paste",
            Action::CopyPath => "Copy Path",
            Action::Refresh => "Refresh",
            Action::Rename => "Rename",
            Action::NewFolder => "New Folder",
            Action::ContextMenu => "Show Menu",
            Action::NewTab => "New Tab",
            Action::CloseTab => "Close Tab",
            Action::NextTab => "Next Tab",
            Action::PreviousTab => "Previous Tab",
            Action::Tab(n) => match n {
                1 => "Tab 1",
                2 => "Tab 2",
                3 => "Tab 3",
                4 => "Tab 4",
                5 => "Tab 5",
                6 => "Tab 6",
                7 => "Tab 7",
                8 => "Tab 8",
                _ => "Tab 9",
            },
        }
    }

    pub fn scope(self) -> Scope {
        match self {
            Action::NewTab
            | Action::CloseTab
            | Action::NextTab
            | Action::PreviousTab
            | Action::Tab(_) => Scope::Window,
            _ => Scope::Browser,
        }
    }

    /// The keys this action is bound to out of the box, in the syntax
    /// `files-config.toml` uses.
    ///
    /// Written as strings and parsed by the same function that reads the
    /// config file, so the shipped defaults *are* a config — there is no
    /// second, hand-built table for the file to drift away from.
    pub fn default_keys(self) -> &'static [&'static str] {
        match self {
            Action::Open => &["Enter"],
            Action::OpenInNewTab => &["Ctrl+Enter"],
            // Backspace goes up a level and Alt+Left goes back in
            // history: two different navigations, kept distinct the way
            // Explorer, Nautilus and Dolphin all keep them.
            Action::GoUp => &["Backspace", "Alt+Up"],
            Action::GoBack => &["Alt+Left"],
            Action::GoForward => &["Alt+Right"],
            Action::FocusUp => &["Up"],
            Action::FocusDown => &["Down"],
            Action::FocusLeft => &["Left"],
            Action::FocusRight => &["Right"],
            Action::ExtendUp => &["Shift+Up"],
            Action::ExtendDown => &["Shift+Down"],
            Action::SelectAll => &["Ctrl+A"],
            Action::ClearSearch => &["Escape"],
            Action::ToggleHidden => &["Ctrl+H"],
            Action::Trash => &["Delete"],
            // Unbound on purpose. A single key that skips the trash is one
            // slip away from losing a file for good; anyone who wants
            // Shift+Delete can bind it in `files-config.toml`.
            Action::DeletePermanently => &[],
            // Reached from the Trash's menu. Neither is common enough to
            // spend a key on by default.
            Action::Restore => &[],
            Action::EmptyTrash => &[],
            Action::Copy => &["Ctrl+C"],
            Action::Cut => &["Ctrl+X"],
            Action::Paste => &["Ctrl+V"],
            Action::CopyPath => &["Ctrl+Shift+C"],
            Action::Refresh => &["F5", "Ctrl+R"],
            Action::Rename => &["F2"],
            Action::NewFolder => &["Ctrl+Shift+N"],
            // The two keys every desktop uses for "the menu a right click
            // would open".
            Action::ContextMenu => &["Menu", "Shift+F10"],
            Action::NewTab => &["Ctrl+T"],
            Action::CloseTab => &["Ctrl+W"],
            Action::NextTab => &["Ctrl+Tab"],
            Action::PreviousTab => &["Ctrl+Shift+Tab"],
            Action::Tab(n) => match n {
                1 => &["Ctrl+1"],
                2 => &["Ctrl+2"],
                3 => &["Ctrl+3"],
                4 => &["Ctrl+4"],
                5 => &["Ctrl+5"],
                6 => &["Ctrl+6"],
                7 => &["Ctrl+7"],
                8 => &["Ctrl+8"],
                _ => &["Ctrl+9"],
            },
        }
    }
}

/// What the listing looks like right now, as far as deciding which
/// actions make sense is concerned.
///
/// Plain data a [`crate::browser::Browser`] fills in, so [`enabled`] is a
/// pure function a test can call without building one — and so a menu
/// greying an item out and a shortcut doing nothing are the same answer,
/// asked once.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct ActionContext {
    /// How many *shown* entries are selected. Entries a filter is hiding
    /// never count: an action must not reach what the user cannot see.
    pub selected: usize,
    /// Whether the keyboard focus is on a folder, a file, or nothing.
    pub focused_is_dir: Option<bool>,
    /// How many entries are shown.
    pub shown: usize,
    pub can_go_back: bool,
    pub can_go_forward: bool,
    pub has_parent: bool,
    pub searching: bool,
    /// Whether the listing is the Trash. Trashing something already in
    /// the trash would file it a second time under a new record, so the
    /// Trash action is off here.
    pub in_trash: bool,
    /// Whether the clipboard holds files to paste. The host knows; the
    /// browser is told.
    pub can_paste: bool,
}

/// Whether `action` would do anything in `ctx`.
///
/// Window-scope actions are always "enabled" here: whether there is a
/// ninth tab is the window's knowledge, not the listing's, and the host
/// already treats an out-of-range tab as a no-op.
pub fn enabled(action: Action, ctx: &ActionContext) -> bool {
    match action {
        Action::Open => ctx.focused_is_dir.is_some(),
        Action::OpenInNewTab => ctx.focused_is_dir == Some(true),
        Action::GoUp => ctx.has_parent,
        Action::GoBack => ctx.can_go_back,
        Action::GoForward => ctx.can_go_forward,
        Action::FocusUp
        | Action::FocusDown
        | Action::FocusLeft
        | Action::FocusRight
        | Action::ExtendUp
        | Action::ExtendDown
        | Action::SelectAll => ctx.shown > 0,
        Action::ClearSearch => ctx.searching,
        Action::ToggleHidden | Action::ContextMenu => true,
        Action::Trash => ctx.selected > 0 && !ctx.in_trash,
        Action::DeletePermanently => ctx.selected > 0,
        Action::Restore => ctx.in_trash && ctx.selected > 0,
        Action::EmptyTrash => ctx.in_trash && ctx.shown > 0,
        Action::Copy | Action::CopyPath => ctx.selected > 0,
        // Moving something out of the Trash by hand would leave its
        // record behind; restoring is the way out, and it comes later.
        Action::Cut => ctx.selected > 0 && !ctx.in_trash,
        Action::Paste => ctx.can_paste && !ctx.in_trash,
        Action::Refresh => true,
        Action::Rename => ctx.selected == 1 && !ctx.in_trash,
        Action::NewFolder => !ctx.in_trash,
        Action::NewTab
        | Action::CloseTab
        | Action::NextTab
        | Action::PreviousTab
        | Action::Tab(_) => true,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The ids are the config file's vocabulary. A variant that could not
    /// be named in `files-config.toml`, or two that shared a name, would be a
    /// setting nobody could write.
    #[test]
    fn every_action_round_trips_through_its_id() {
        let all = Action::all();
        for action in &all {
            assert_eq!(Action::from_id(action.id()), Some(*action), "{action:?}");
        }
        let mut ids: Vec<&str> = all.iter().map(|a| a.id()).collect();
        ids.sort_unstable();
        let before = ids.len();
        ids.dedup();
        assert_eq!(ids.len(), before, "two actions share an id");
    }

    #[test]
    fn an_unknown_id_is_none_rather_than_a_guess() {
        assert_eq!(Action::from_id("copy-files"), None);
        assert_eq!(Action::from_id("tab-0"), None);
        assert_eq!(Action::from_id("tab-10"), None);
    }

    /// Every action has a way in: a default key, or a place in a default
    /// menu. This is the test that stops an action going back to being
    /// built and unreachable.
    #[test]
    fn every_action_ships_with_a_key_or_a_menu_item() {
        let menus = crate::menu::MenuConfig::default();
        let in_a_menu = |action: Action| {
            crate::menu::MenuKind::all()
                .into_iter()
                .any(|kind| menus.get(kind).contains(&crate::menu::MenuEntry::Action(action)))
        };
        for action in Action::all() {
            assert!(
                !action.default_keys().is_empty() || in_a_menu(action),
                "{action:?} has no way to reach it"
            );
            assert!(!action.label().is_empty(), "{action:?} has no label");
        }
    }

    /// Deleting for good is never one unconfigured keypress away.
    #[test]
    fn deleting_permanently_has_no_key_out_of_the_box() {
        assert!(Action::DeletePermanently.default_keys().is_empty());
    }

    #[test]
    fn only_the_tab_actions_belong_to_the_window() {
        for action in Action::all() {
            let is_tab = matches!(
                action,
                Action::NewTab | Action::CloseTab | Action::NextTab | Action::PreviousTab | Action::Tab(_)
            );
            assert_eq!(action.scope() == Scope::Window, is_tab, "{action:?}");
        }
    }

    #[test]
    fn nothing_selected_means_nothing_to_trash() {
        let ctx = ActionContext { shown: 4, ..ActionContext::default() };
        assert!(!enabled(Action::Trash, &ctx));
        assert!(enabled(Action::Trash, &ActionContext { selected: 1, ..ctx }));
    }

    #[test]
    fn nothing_in_the_trash_can_be_trashed_again() {
        let ctx = ActionContext { selected: 2, shown: 2, in_trash: true, ..ActionContext::default() };
        assert!(!enabled(Action::Trash, &ctx));
    }

    #[test]
    fn restoring_and_emptying_belong_to_the_trash() {
        let elsewhere = ActionContext { selected: 1, shown: 1, ..ActionContext::default() };
        assert!(!enabled(Action::Restore, &elsewhere));
        assert!(!enabled(Action::EmptyTrash, &elsewhere));
        let trash = ActionContext { in_trash: true, ..elsewhere };
        assert!(enabled(Action::Restore, &trash));
        assert!(enabled(Action::EmptyTrash, &trash));
        let empty_trash = ActionContext { in_trash: true, ..ActionContext::default() };
        assert!(!enabled(Action::EmptyTrash, &empty_trash), "nothing to empty");
    }

    #[test]
    fn paste_needs_something_to_paste_and_somewhere_that_is_not_the_trash() {
        let ctx = ActionContext::default();
        assert!(!enabled(Action::Paste, &ctx));
        assert!(enabled(Action::Paste, &ActionContext { can_paste: true, ..ctx }));
        assert!(!enabled(Action::Paste, &ActionContext { can_paste: true, in_trash: true, ..ctx }));
    }

    #[test]
    fn copying_out_of_the_trash_is_fine_but_cutting_is_not() {
        let ctx = ActionContext { selected: 1, in_trash: true, ..ActionContext::default() };
        assert!(enabled(Action::Copy, &ctx));
        assert!(!enabled(Action::Cut, &ctx));
    }

    #[test]
    fn a_new_tab_opens_on_folders_only() {
        let file = ActionContext { focused_is_dir: Some(false), ..ActionContext::default() };
        let folder = ActionContext { focused_is_dir: Some(true), ..ActionContext::default() };
        assert!(!enabled(Action::OpenInNewTab, &file));
        assert!(enabled(Action::OpenInNewTab, &folder));
        assert!(enabled(Action::Open, &file), "a file still opens");
    }

    #[test]
    fn history_actions_follow_the_history() {
        let ctx = ActionContext::default();
        assert!(!enabled(Action::GoBack, &ctx));
        assert!(!enabled(Action::GoForward, &ctx));
        assert!(enabled(Action::GoBack, &ActionContext { can_go_back: true, ..ctx }));
    }

    #[test]
    fn clearing_a_search_needs_a_search() {
        assert!(!enabled(Action::ClearSearch, &ActionContext::default()));
        assert!(enabled(
            Action::ClearSearch,
            &ActionContext { searching: true, ..ActionContext::default() }
        ));
    }
}
