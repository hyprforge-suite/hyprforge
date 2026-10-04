//! The command palette: every action, found by typing a few letters of
//! its name — the one idea from the design's command-first shell (`1l`)
//! worth keeping, on Ctrl+K inside the ordinary window rather than as the
//! whole window.
//!
//! Pure: which actions to offer and in what order is decided here, from
//! the action table that menus and keys already share, so the palette
//! cannot offer something no menu or key could do. What is offered is
//! what would do something *now* — an action that is off where you are is
//! left out, the same rule the menus follow for an empty row.

use crate::action::Action;

/// What the palette calls an action. The menu's own word, except where a
/// menu's context made a short one enough — "Move Up" means a pin only
/// beside the pin it is in a menu of.
pub fn label(action: Action) -> &'static str {
    match action {
        Action::PinUp => "Move Pin Up",
        Action::PinDown => "Move Pin Down",
        Action::Unpin => "Unpin from Sidebar",
        Action::Open => "Open Selected",
        Action::Restore => "Restore from Trash",
        // "Transfers" alone reads as a heading in a list of verbs.
        Action::Transfers => "Show Transfers",
        Action::TransferQueue => "Show Transfer Queue",
        other => other.label(),
    }
}

/// Whether an action belongs in the palette at all. Moving the keyboard
/// focus a row at a time, or opening the menu for the focused row, is a
/// thing a key does, not a command anyone looks for by name; jumping to
/// "Tab 7" by typing is slower than pressing Alt+7.
fn listed(action: Action) -> bool {
    !matches!(
        action,
        Action::FocusUp
            | Action::FocusDown
            | Action::FocusLeft
            | Action::FocusRight
            | Action::ExtendUp
            | Action::ExtendDown
            | Action::ContextMenu
            | Action::CommandPalette
            | Action::Tab(_)
    )
}

/// The actions to show for `query`, best first.
///
/// `enabled` says whether an action would do anything now; `allowed`, when
/// given, is every action the host carries out at all — the open/save
/// dialog's, which declines file operations. Ranked by the path bar's
/// matcher ([`crate::jump::score`]) against the whole name, so `nf` finds
/// New Folder and `trash` finds Move to Trash before Empty Trash; ties
/// keep the action table's own order.
pub fn matches(query: &str, enabled: impl Fn(Action) -> bool, allowed: Option<&[Action]>) -> Vec<Action> {
    let query = query.trim();
    let mut scored: Vec<(u32, usize, Action)> = Action::all()
        .into_iter()
        .filter(|a| listed(*a) && enabled(*a) && allowed.is_none_or(|allowed| allowed.contains(a)))
        .enumerate()
        .filter_map(|(order, action)| Some((score(query, label(action))?, order, action)))
        .collect();
    scored.sort_by_key(|(score, order, _)| (*score, *order));
    // Each action once: `Action::all` lists none twice, but a stray
    // duplicate there would show two rows that do one thing.
    let mut seen = std::collections::HashSet::new();
    scored.into_iter().map(|(_, _, a)| a).filter(|a| seen.insert(*a)).collect()
}

/// How well `name` answers `query` — lower is better, `None` is not at
/// all, and an empty query matches everything equally. Public so the
/// person's own actions (`crate::custom`), which are not [`Action`]s,
/// rank on the same scale as the built-in ones and merge into one list.
pub fn score(query: &str, name: &str) -> Option<u32> {
    let query = query.trim();
    if query.is_empty() {
        return Some(0);
    }
    // Initials as their own tier: `nf` for New Folder is how people
    // type a command they know by name.
    let initials: String = name.split_whitespace().filter_map(|w| w.chars().next()).collect();
    let by_initials = initials.to_lowercase().starts_with(&query.to_lowercase()).then_some(1);
    let by_name = crate::jump::score(name, query).map(|tier| tier as u32 * 2);
    match (by_name, by_initials) {
        (Some(a), Some(b)) => Some(a.min(b)),
        (a, b) => a.or(b),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn all_on(_: Action) -> bool {
        true
    }

    #[test]
    fn initials_find_a_command_by_its_name() {
        assert_eq!(matches("nf", all_on, None).first(), Some(&Action::NewFolder));
    }

    #[test]
    fn a_word_finds_the_commands_that_start_with_it_first() {
        let found = matches("trash", all_on, None);
        let pos = |a| found.iter().position(|x| *x == a).unwrap();
        assert!(pos(Action::Trash) < pos(Action::EmptyTrash), "{found:?}");
    }

    #[test]
    fn what_would_do_nothing_now_is_not_offered() {
        let found = matches("", |a| a != Action::Paste, None);
        assert!(!found.contains(&Action::Paste));
        assert!(found.contains(&Action::NewFolder));
    }

    #[test]
    fn keystroke_actions_are_not_commands() {
        let found = matches("", all_on, None);
        for a in [Action::FocusUp, Action::ExtendDown, Action::ContextMenu, Action::CommandPalette, Action::Tab(3)] {
            assert!(!found.contains(&a), "{a:?} offered");
        }
    }

    #[test]
    fn a_host_that_declines_an_action_is_never_offered_it() {
        let allowed = [Action::NewFolder, Action::Refresh];
        let found = matches("", all_on, Some(&allowed));
        assert_eq!(found, vec![Action::Refresh, Action::NewFolder].into_iter().filter(|a| found.contains(a)).collect::<Vec<_>>());
        assert!(found.iter().all(|a| allowed.contains(a)));
        assert!(!found.contains(&Action::Trash));
    }

    #[test]
    fn nothing_typed_lists_everything_in_the_tables_order() {
        let found = matches("  ", all_on, None);
        let order: Vec<Action> = Action::all().into_iter().filter(|a| found.contains(a)).collect();
        assert_eq!(found, order);
    }
}
