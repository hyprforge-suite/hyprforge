//! What a key press means, as data — so the app window and the portal's
//! dialog wire the *same* grammar to iced's keyboard subscription instead
//! of each inventing its own. This module owns the meaning; the host
//! owns turning an `iced::keyboard::Event` into the [`Key`]/[`Modifiers`]
//! this module reads, and calling [`crate::browser::Browser::handle_key`]
//! with the result.
//!
//! Deliberately free of `iced::keyboard` types in its own vocabulary:
//! [`Key`] only names the handful of keys this browser gives meaning to,
//! not iced's full key enumeration, so this module's own tests don't
//! need iced's keyboard module at all.

/// The keys this browser assigns a meaning to. Anything else a host sees
/// is not this browser's concern — text-entry widgets like the search
/// field and the dialog's filename field handle their own input the
/// normal iced way.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Key {
    Enter,
    Backspace,
    Escape,
    ArrowUp,
    ArrowDown,
    ArrowLeft,
    ArrowRight,
    /// A printable character, for typing straight into the search box —
    /// "typing to search" in the brief, not a keyboard shortcut.
    Character(char),
}

/// Which modifier keys were held down alongside [`Key`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Modifiers {
    pub ctrl: bool,
    pub alt: bool,
    pub shift: bool,
}

/// What a key press asks the browser to do, independent of how it gets
/// carried out — [`crate::browser::Browser::handle_key`] is what actually
/// applies one of these.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum KeyAction {
    /// Enter: open the focused entry, or (for a folder) navigate into it.
    Activate,
    /// Backspace: up one directory level.
    GoUp,
    /// Alt+Left: back in navigation history.
    GoBack,
    /// An arrow key with no modifier: move the focused/selected entry by
    /// one in the given direction.
    Move(Direction),
    /// A plain, unmodified character: append it to the search query.
    TypeToSearch(char),
    /// Escape: clear the search query.
    ClearSearch,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Direction {
    Up,
    Down,
    Left,
    Right,
}

/// Resolves one key press (with its modifiers) to the [`KeyAction`] it
/// means, or `None` if this browser gives it no meaning — in which case
/// a host should let the key fall through to whatever iced widget has
/// focus (a text field, most likely).
pub fn resolve(key: Key, mods: Modifiers) -> Option<KeyAction> {
    match key {
        Key::Enter => Some(KeyAction::Activate),
        // Backspace goes up a level; Alt+Left goes back in history — two
        // different navigations the brief pairs together but that this
        // browser keeps distinct, the way every mainstream file manager
        // does (Explorer/Nautilus/Dolphin all separate "up" from "back").
        Key::Backspace => Some(KeyAction::GoUp),
        Key::ArrowLeft if mods.alt => Some(KeyAction::GoBack),
        Key::Escape => Some(KeyAction::ClearSearch),
        Key::ArrowUp => Some(KeyAction::Move(Direction::Up)),
        Key::ArrowDown => Some(KeyAction::Move(Direction::Down)),
        Key::ArrowLeft => Some(KeyAction::Move(Direction::Left)),
        Key::ArrowRight => Some(KeyAction::Move(Direction::Right)),
        // Ctrl/Alt-held characters are somebody else's shortcut (copy,
        // paste, select-all) and must never leak into the search box as
        // typed text.
        Key::Character(c) if !mods.ctrl && !mods.alt => Some(KeyAction::TypeToSearch(c)),
        Key::Character(_) => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn enter_activates() {
        assert_eq!(resolve(Key::Enter, Modifiers::default()), Some(KeyAction::Activate));
    }

    #[test]
    fn backspace_goes_up_not_back() {
        assert_eq!(resolve(Key::Backspace, Modifiers::default()), Some(KeyAction::GoUp));
    }

    #[test]
    fn alt_left_goes_back_but_plain_left_moves_the_selection() {
        let alt = Modifiers { alt: true, ..Modifiers::default() };
        assert_eq!(resolve(Key::ArrowLeft, alt), Some(KeyAction::GoBack));
        assert_eq!(
            resolve(Key::ArrowLeft, Modifiers::default()),
            Some(KeyAction::Move(Direction::Left))
        );
    }

    #[test]
    fn escape_clears_search() {
        assert_eq!(resolve(Key::Escape, Modifiers::default()), Some(KeyAction::ClearSearch));
    }

    #[test]
    fn a_plain_character_types_into_search() {
        assert_eq!(
            resolve(Key::Character('r'), Modifiers::default()),
            Some(KeyAction::TypeToSearch('r'))
        );
    }

    #[test]
    fn a_ctrl_held_character_is_left_for_a_shortcut_not_search() {
        let ctrl = Modifiers { ctrl: true, ..Modifiers::default() };
        assert_eq!(resolve(Key::Character('c'), ctrl), None);
    }

    #[test]
    fn all_four_arrows_move_without_a_modifier() {
        assert_eq!(resolve(Key::ArrowUp, Modifiers::default()), Some(KeyAction::Move(Direction::Up)));
        assert_eq!(resolve(Key::ArrowDown, Modifiers::default()), Some(KeyAction::Move(Direction::Down)));
        assert_eq!(resolve(Key::ArrowRight, Modifiers::default()), Some(KeyAction::Move(Direction::Right)));
    }
}
