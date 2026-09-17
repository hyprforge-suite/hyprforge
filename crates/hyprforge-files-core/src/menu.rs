//! The right-click menu, as data.
//!
//! What a menu contains is configuration (`[menu]` in
//! `files-config.toml`), whether each item works is
//! [`crate::action::enabled`], and the shortcut shown beside it comes
//! from the live [`Keymap`] — so a rebinding shows up in the menu without
//! anyone remembering to update a label. This module turns those three
//! into a list of [`MenuItem`]s and decides where the menu goes on
//! screen. Both are pure, so both are tested without a window.
//!
//! Drawing it is [`crate::browser`]'s job, and needs no custom overlay:
//! iced 0.14's `stack`, `pin` and `opaque` are enough.

use crate::action::{self, Action, ActionContext};
use crate::keymap::Keymap;

/// What was right-clicked, which decides which menu opens.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum MenuKind {
    /// A file.
    Entry,
    /// A folder.
    Folder,
    /// The empty space around the rows — the current folder itself.
    Empty,
    /// Anything inside the Trash, where the everyday actions mostly do
    /// not apply.
    Trash,
}

impl MenuKind {
    /// The key under `[menu]` in `files-config.toml`.
    pub fn id(self) -> &'static str {
        match self {
            MenuKind::Entry => "entry",
            MenuKind::Folder => "folder",
            MenuKind::Empty => "empty",
            MenuKind::Trash => "trash",
        }
    }

    pub fn all() -> [MenuKind; 4] {
        [MenuKind::Entry, MenuKind::Folder, MenuKind::Empty, MenuKind::Trash]
    }
}

/// One line of a menu as configured.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MenuEntry {
    Action(Action),
    /// A dividing line, written `"-"` in the file.
    Separator,
}

impl MenuEntry {
    /// Reads one line of a `[menu]` list. `None` for an id this version
    /// does not know — which the config loader reports.
    pub fn parse(id: &str) -> Option<MenuEntry> {
        if id.trim() == "-" {
            return Some(MenuEntry::Separator);
        }
        Action::from_id(id.trim()).map(MenuEntry::Action)
    }
}

/// Which entries each menu has.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MenuConfig {
    pub entry: Vec<MenuEntry>,
    pub folder: Vec<MenuEntry>,
    pub empty: Vec<MenuEntry>,
    pub trash: Vec<MenuEntry>,
}

impl Default for MenuConfig {
    /// The shipped menus.
    ///
    /// Only actions that exist today. Copy, paste, rename and the rest
    /// join these defaults in the change that makes them work, so the
    /// menu never offers something that does nothing.
    fn default() -> Self {
        use Action::*;
        use MenuEntry::{Action as A, Separator as Sep};
        MenuConfig {
            entry: vec![
                A(Open),
                Sep,
                A(Cut),
                A(Copy),
                A(CopyPath),
                A(Rename),
                Sep,
                A(SelectAll),
                Sep,
                A(Trash),
            ],
            folder: vec![
                A(Open),
                A(OpenInNewTab),
                Sep,
                A(Cut),
                A(Copy),
                A(CopyPath),
                A(Rename),
                Sep,
                A(SelectAll),
                Sep,
                A(Trash),
            ],
            empty: vec![
                A(Paste),
                A(NewFolder),
                Sep,
                A(SelectAll),
                A(ToggleHidden),
                A(Refresh),
                Sep,
                A(GoUp),
                A(NewTab),
            ],
            trash: vec![A(Open), Sep, A(Copy), A(CopyPath), Sep, A(SelectAll), Sep, A(DeletePermanently)],
        }
    }
}

impl MenuConfig {
    pub fn get(&self, kind: MenuKind) -> &[MenuEntry] {
        match kind {
            MenuKind::Entry => &self.entry,
            MenuKind::Folder => &self.folder,
            MenuKind::Empty => &self.empty,
            MenuKind::Trash => &self.trash,
        }
    }

    pub fn set(&mut self, kind: MenuKind, entries: Vec<MenuEntry>) {
        match kind {
            MenuKind::Entry => self.entry = entries,
            MenuKind::Folder => self.folder = entries,
            MenuKind::Empty => self.empty = entries,
            MenuKind::Trash => self.trash = entries,
        }
    }
}

/// One line of a menu as shown.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MenuItem {
    Action {
        action: Action,
        label: &'static str,
        /// The key that does the same thing, as the config file spells
        /// it, or `None` if the action is unbound.
        hint: Option<String>,
        enabled: bool,
    },
    Separator,
}

impl MenuItem {
    pub fn is_selectable(&self) -> bool {
        matches!(self, MenuItem::Action { enabled: true, .. })
    }
}

/// The menu to show for `entries` in `ctx`.
///
/// Separators are tidied: none at either end, never two in a row. A
/// configured menu that ends in `"-"`, or two separators around an item
/// the user removed, should not draw as stray lines.
///
/// Disabled items stay, greyed out, rather than vanishing. A menu whose
/// shape changes with the selection is one whose muscle memory never
/// sets.
pub fn build(entries: &[MenuEntry], ctx: &ActionContext, keymap: &Keymap) -> Vec<MenuItem> {
    let mut items: Vec<MenuItem> = Vec::with_capacity(entries.len());
    for entry in entries {
        match entry {
            MenuEntry::Separator => {
                if matches!(items.last(), Some(MenuItem::Action { .. })) {
                    items.push(MenuItem::Separator);
                }
            }
            MenuEntry::Action(action) => items.push(MenuItem::Action {
                action: *action,
                label: action.label(),
                hint: keymap.combos_for(*action).first().map(|c| c.to_string()),
                enabled: action::enabled(*action, ctx),
            }),
        }
    }
    while matches!(items.last(), Some(MenuItem::Separator)) {
        items.pop();
    }
    items
}

/// The next selectable item after `from` in `direction` (±1), wrapping —
/// arrow keys in an open menu. Skips separators and disabled items.
/// `None` when nothing in the menu can be chosen.
pub fn step(items: &[MenuItem], from: Option<usize>, direction: i32) -> Option<usize> {
    let len = items.len() as i32;
    if len == 0 {
        return None;
    }
    let start = match from {
        Some(i) => i as i32,
        // Nothing highlighted yet: Down starts at the top, Up at the
        // bottom.
        None if direction > 0 => -1,
        None => len,
    };
    (1..=len)
        .map(|k| (start + k * direction).rem_euclid(len) as usize)
        .find(|&i| items[i].is_selectable())
}

/// Where the menu's top-left corner goes, given where it was asked for,
/// its own size and the window's.
///
/// It opens down and to the right of the pointer, the way menus do,
/// unless that would run off the window — then it flips to open up, or
/// left, from the same point. A menu too large to fit either way is
/// pinned to the edge it overflows least, so its top-left is always on
/// screen and the first items are always reachable.
pub fn place(at: (f32, f32), menu: (f32, f32), window: (f32, f32)) -> (f32, f32) {
    fn axis(at: f32, size: f32, limit: f32) -> f32 {
        if at + size <= limit {
            at
        } else if at - size >= 0.0 {
            at - size
        } else {
            (limit - size).max(0.0)
        }
    }
    (axis(at.0, menu.0, window.0), axis(at.1, menu.1, window.1))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ctx_with_selection() -> ActionContext {
        ActionContext {
            selected: 1,
            focused_is_dir: Some(false),
            shown: 3,
            has_parent: true,
            ..ActionContext::default()
        }
    }

    fn labels(items: &[MenuItem]) -> Vec<&str> {
        items
            .iter()
            .map(|i| match i {
                MenuItem::Action { label, .. } => *label,
                MenuItem::Separator => "-",
            })
            .collect()
    }

    /// Every default menu names only actions that do something today.
    #[test]
    fn every_default_menu_item_is_a_real_action() {
        let config = MenuConfig::default();
        for kind in MenuKind::all() {
            assert!(!config.get(kind).is_empty(), "{kind:?} is empty");
            for entry in config.get(kind) {
                if let MenuEntry::Action(a) = entry {
                    assert!(Action::all().contains(a), "{a:?} in {kind:?}");
                }
            }
        }
    }

    /// Nobody should be offered "Move to Trash" on something already in
    /// the Trash.
    #[test]
    fn the_trash_menu_does_not_offer_to_trash_again() {
        assert!(!MenuConfig::default().trash.contains(&MenuEntry::Action(Action::Trash)));
    }

    #[test]
    fn stray_separators_are_tidied_away() {
        use MenuEntry::{Action as A, Separator as Sep};
        let entries = [Sep, A(Action::Open), Sep, Sep, A(Action::Trash), Sep];
        let items = build(&entries, &ctx_with_selection(), &Keymap::defaults());
        assert_eq!(labels(&items), ["Open", "-", "Move to Trash"]);
    }

    /// Disabled items stay where they are, greyed, so the menu keeps its
    /// shape.
    #[test]
    fn a_disabled_item_is_greyed_not_removed() {
        let entries = [MenuEntry::Action(Action::Trash)];
        let items = build(&entries, &ActionContext::default(), &Keymap::defaults());
        assert!(matches!(items[0], MenuItem::Action { enabled: false, .. }));
    }

    /// The hint comes from the keymap in use, so a rebinding shows up in
    /// the menu with no label to keep in step.
    #[test]
    fn the_shortcut_hint_follows_the_keymap() {
        let entries = [MenuEntry::Action(Action::Trash)];
        let items = build(&entries, &ctx_with_selection(), &Keymap::defaults());
        assert!(matches!(&items[0], MenuItem::Action { hint: Some(h), .. } if h == "Delete"));

        let rebound = crate::config::keymap_with(
            &[("trash".to_string(), vec!["Ctrl+D".to_string()])].into_iter().collect(),
        )
        .0;
        let items = build(&entries, &ctx_with_selection(), &rebound);
        assert!(matches!(&items[0], MenuItem::Action { hint: Some(h), .. } if h == "Ctrl+D"));

        let unbound = crate::config::keymap_with(
            &[("trash".to_string(), vec![])].into_iter().collect(),
        )
        .0;
        let items = build(&entries, &ctx_with_selection(), &unbound);
        assert!(matches!(&items[0], MenuItem::Action { hint: None, .. }));
    }

    #[test]
    fn arrows_skip_separators_and_disabled_items_and_wrap() {
        use MenuEntry::{Action as A, Separator as Sep};
        let ctx = ActionContext { focused_is_dir: Some(false), shown: 2, ..ActionContext::default() };
        // Open (on), sep, Trash (off: nothing selected), sep, Select All (on)
        let entries = [A(Action::Open), Sep, A(Action::Trash), Sep, A(Action::SelectAll)];
        let items = build(&entries, &ctx, &Keymap::defaults());
        assert_eq!(step(&items, None, 1), Some(0), "Down starts at the top");
        assert_eq!(step(&items, Some(0), 1), Some(4), "skips the separator and the disabled item");
        assert_eq!(step(&items, Some(4), 1), Some(0), "wraps");
        assert_eq!(step(&items, None, -1), Some(4), "Up starts at the bottom");
    }

    #[test]
    fn a_menu_with_nothing_to_choose_has_no_highlight() {
        let items = build(
            &[MenuEntry::Action(Action::Trash)],
            &ActionContext::default(),
            &Keymap::defaults(),
        );
        assert_eq!(step(&items, None, 1), None);
    }

    #[test]
    fn a_menu_opens_down_and_right_when_it_fits() {
        assert_eq!(place((100.0, 100.0), (200.0, 150.0), (800.0, 600.0)), (100.0, 100.0));
    }

    #[test]
    fn a_menu_near_an_edge_flips_to_open_the_other_way() {
        assert_eq!(place((700.0, 100.0), (200.0, 150.0), (800.0, 600.0)), (500.0, 100.0));
        assert_eq!(place((100.0, 550.0), (200.0, 150.0), (800.0, 600.0)), (100.0, 400.0));
    }

    /// Too big either way: pinned so its top-left is on screen and its
    /// first items are reachable.
    #[test]
    fn a_menu_too_big_for_the_window_keeps_its_corner_on_screen() {
        let (x, y) = place((50.0, 50.0), (200.0, 900.0), (800.0, 600.0));
        assert_eq!(x, 50.0);
        assert_eq!(y, 0.0);
    }

    #[test]
    fn a_separator_parses_and_an_unknown_id_does_not() {
        assert_eq!(MenuEntry::parse("-"), Some(MenuEntry::Separator));
        assert_eq!(MenuEntry::parse("trash"), Some(MenuEntry::Action(Action::Trash)));
        assert_eq!(MenuEntry::parse("defenestrate"), None);
    }
}
