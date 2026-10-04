//! The browser's half of launching things: "Open Terminal Here", and the
//! person's own `[[action]]`s in the menus and the palette.
//!
//! A child of `browser` for the reason `searching` and `drives` are: it
//! reaches the browser's own state without that state growing accessors
//! for one feature. Neither half runs anything — `Browser` does no I/O —
//! they decide *what* to run on *which* paths, from *which* folder, and
//! hand that to the host as [`Outcome::OpenTerminal`] and
//! [`Outcome::RunCustom`]. Which terminal, and the spawning, are the
//! window's (`hyprforge_files::terminal`).
//!
//! # Only in the window
//!
//! The open/save dialog renders this same browser and offers neither: a
//! file chooser that ran commands on what you were choosing would be a
//! file manager wearing a dialog's title (the reasoning
//! [`crate::action::Action::Properties`] gives). The terminal is kept out
//! by [`crate::menu::DIALOG_ACTIONS`]; custom actions by needing a
//! [`TypeOf`], which only the window hands over.

use super::{Browser, Mode, Outcome};
use crate::action::Action;
use crate::custom::{CustomAction, Target, TypeOf};
use crate::menu::MenuItem;
use std::path::{Path, PathBuf};

/// One row of the command palette.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum PaletteChoice {
    Action(Action),
    /// One of `Config::actions`, by index.
    Custom(usize),
}

impl Browser {
    /// Tells the browser what files are, for matching the `types` of the
    /// person's own actions — see [`TypeOf`]. Until it is told, it
    /// offers no custom actions at all: guessing that a type matches
    /// would offer "Resize" on a spreadsheet.
    pub fn set_type_of(&mut self, type_of: TypeOf) {
        self.type_of = Some(type_of);
    }

    /// What a custom action acts on right now: the selected rows on
    /// screen, or — with nothing selected, as from the empty space — the
    /// folder in view, as one folder.
    pub(super) fn custom_targets(&self) -> Vec<Target> {
        let rows = self.rows();
        let selected: Vec<Target> = rows
            .iter()
            .filter(|e| self.selection.is_selected(&e.path))
            .map(|e| Target { path: e.path.clone(), is_dir: e.is_dir })
            .collect();
        if selected.is_empty() {
            vec![Target { path: self.current_dir.clone(), is_dir: true }]
        } else {
            selected
        }
    }

    /// The type matcher, when custom actions can be offered here at all:
    /// in the window, and where the paths are real ones a program could
    /// open — not members of an archive, not the Trash's records.
    fn custom_matcher(&self) -> Option<&TypeOf> {
        if !matches!(self.mode, Mode::App) || self.in_trash() || self.in_archive() {
            return None;
        }
        self.type_of.as_ref()
    }

    /// The person's actions that suit what a menu would act on, as menu
    /// rows: hidden when the types do not suit, greyed when only the
    /// count does not (see [`CustomAction::suits`]).
    pub(super) fn custom_items(&self) -> Vec<MenuItem> {
        let Some(type_of) = self.custom_matcher() else { return Vec::new() };
        let targets = self.custom_targets();
        self.config
            .actions
            .iter()
            .enumerate()
            .filter(|(_, action)| action.suits(&targets, type_of))
            .map(|(index, action)| MenuItem::Custom {
                index,
                label: action.label.clone(),
                enabled: action.takes(targets.len()),
            })
            .collect()
    }

    /// The custom action at `index`, when it can run on what is selected
    /// now — asked again at the moment of choosing, because the menu was
    /// built a moment ago and the palette's list a keystroke ago.
    fn runnable(&self, index: usize) -> Option<(&CustomAction, Vec<Target>)> {
        let type_of = self.custom_matcher()?;
        let action = self.config.actions.get(index)?;
        let targets = self.custom_targets();
        (action.suits(&targets, type_of) && action.takes(targets.len())).then_some((action, targets))
    }

    /// Runs the custom action at `index` on what it acts on, from the
    /// folder in view.
    pub(super) fn run_custom(&mut self, index: usize) -> Outcome {
        let Some((action, targets)) = self.runnable(index) else { return Outcome::None };
        Outcome::RunCustom {
            action: Box::new(action.clone()),
            paths: targets.into_iter().map(|t| t.path).collect(),
            cwd: self.current_dir.clone(),
        }
    }

    /// The custom actions the palette offers for `query`, each with its
    /// rank — on the same scale as the built-in ones', so the two lists
    /// merge into one order.
    pub(super) fn custom_palette_matches(&self, query: &str) -> Vec<(u32, PaletteChoice)> {
        self.custom_items()
            .into_iter()
            .filter_map(|item| match item {
                MenuItem::Custom { index, label, enabled: true } => {
                    crate::palette::score(query, &label).map(|score| (score, PaletteChoice::Custom(index)))
                }
                _ => None,
            })
            .collect()
    }

    /// What a palette row says: its label and the key that does the same.
    pub(super) fn palette_row(&self, choice: PaletteChoice) -> (String, Option<String>) {
        match choice {
            PaletteChoice::Action(action) => (
                crate::palette::label(action).to_string(),
                self.config.keymap.combos_for(action).first().map(|c| c.to_string()),
            ),
            PaletteChoice::Custom(index) => {
                (self.config.actions.get(index).map(|a| a.label.clone()).unwrap_or_default(), None)
            }
        }
    }

    /// "Open Terminal Here": in the folder a menu was opened on — a
    /// sidebar row (`target`) or a folder row (`row`) — and otherwise in
    /// the folder in view.
    ///
    /// Never the *selected* folder when the key is pressed: going back
    /// or up leaves the folder you came from selected, so F4 would open
    /// a terminal somewhere you had just left. A right click on a folder
    /// is a question about that folder; a key press is about where you
    /// are.
    pub(super) fn open_terminal(&self, target: Option<&Path>, row: Option<&Path>) -> Outcome {
        let folder: PathBuf = target.or(row).unwrap_or(&self.current_dir).to_path_buf();
        Outcome::OpenTerminal(folder)
    }
}

#[cfg(test)]
mod tests {
    use super::super::tests_support::loaded;
    use super::super::{DialogKind, Message, MenuSpot};
    use super::*;
    use std::sync::Arc;

    fn by_name() -> TypeOf {
        TypeOf::new(|path| match path.extension().and_then(|e| e.to_str()) {
            Some("png") => vec!["image/png".to_string()],
            Some("txt") => vec!["text/plain".to_string()],
            _ => Vec::new(),
        })
    }

    fn with_actions(mut browser: Browser, text: &str) -> Browser {
        let (config, problems) = crate::config::parse(text, Path::new("x"));
        assert!(problems.is_empty(), "{problems:?}");
        browser.set_config(Arc::new(config));
        browser.set_type_of(by_name());
        browser
    }

    const SHRINK: &str = "[[action]]\nlabel = \"Shrink\"\ncommand = [\"magick\", \"mogrify\"]\ntypes = [\"image/*\"]\n\
                          [[action]]\nlabel = \"Compare\"\ncommand = [\"meld\"]\nselection = \"many\"\n\
                          [[action]]\nlabel = \"Code here\"\ncommand = [\"code\"]\ntypes = [\"inode/directory\"]\nselection = \"one\"\n";

    fn custom_labels(browser: &Browser) -> Vec<(String, bool)> {
        browser
            .menu
            .as_ref()
            .map(|m| {
                m.items
                    .iter()
                    .filter_map(|i| match i {
                        MenuItem::Custom { label, enabled, .. } => Some((label.clone(), *enabled)),
                        _ => None,
                    })
                    .collect()
            })
            .unwrap_or_default()
    }

    fn right_click(browser: &mut Browser, spot: MenuSpot) {
        browser.update(Message::OpenContextMenu { spot, at: (0.0, 0.0) });
    }

    #[test]
    fn a_files_menu_offers_the_actions_that_suit_it_and_greys_a_count_that_does_not() {
        let mut browser = with_actions(loaded(&[("a.png", false), ("b.txt", false)]), SHRINK);
        right_click(&mut browser, MenuSpot::Row(0));
        assert_eq!(
            custom_labels(&browser),
            [("Shrink".to_string(), true), ("Compare".to_string(), false)],
            "Code here is for folders; Compare wants two"
        );
        right_click(&mut browser, MenuSpot::Row(1));
        assert_eq!(custom_labels(&browser), [("Compare".to_string(), false)], "a text file is not a picture");
    }

    /// From the empty space the folder in view is what an action acts
    /// on, so a folder action works from the background.
    #[test]
    fn the_empty_space_offers_actions_for_the_folder_in_view() {
        let mut browser = with_actions(loaded(&[("a.png", false)]), SHRINK);
        right_click(&mut browser, MenuSpot::Background);
        assert!(custom_labels(&browser).contains(&("Code here".to_string(), true)), "{:?}", custom_labels(&browser));
        let at = browser
            .menu
            .as_ref()
            .unwrap()
            .items
            .iter()
            .position(|i| i.label() == "Code here")
            .unwrap();
        let outcome = browser.update(Message::MenuChose(at));
        let Outcome::RunCustom { action, paths, cwd } = outcome else { panic!("{outcome:?}") };
        assert_eq!(action.command, ["code"]);
        assert_eq!(paths, [PathBuf::from("/dir")]);
        assert_eq!(cwd, PathBuf::from("/dir"));
    }

    #[test]
    fn choosing_an_action_runs_it_on_the_selection_from_the_folder_in_view() {
        let mut browser = with_actions(loaded(&[("a.png", false), ("c.png", false)]), SHRINK);
        browser.perform(Action::SelectAll);
        assert_eq!(
            browser.run_custom(0),
            Outcome::RunCustom {
                action: Box::new(browser.config.actions[0].clone()),
                paths: vec!["/dir/a.png".into(), "/dir/c.png".into()],
                cwd: "/dir".into(),
            }
        );
    }

    #[test]
    fn an_action_that_no_longer_suits_the_selection_does_nothing_when_chosen() {
        let mut browser = with_actions(loaded(&[("b.txt", false)]), SHRINK);
        browser.update(Message::EntryClicked { index: 0, ctrl: false, shift: false });
        assert_eq!(browser.run_custom(0), Outcome::None, "Shrink is for pictures");
        assert_eq!(browser.run_custom(1), Outcome::None, "Compare wants two");
        assert_eq!(browser.run_custom(99), Outcome::None, "and there is no 100th action");
    }

    /// No matcher, no actions — and the dialog is never handed one.
    #[test]
    fn without_a_way_to_tell_types_apart_nothing_custom_is_offered() {
        let (config, _) = crate::config::parse(SHRINK, Path::new("x"));
        let mut browser = loaded(&[("a.png", false)]);
        browser.set_config(Arc::new(config.clone()));
        right_click(&mut browser, MenuSpot::Row(0));
        assert!(custom_labels(&browser).is_empty());

        let (mut dialog, _) = Browser::new(
            Mode::Dialog(DialogKind::Open),
            crate::prefs::Prefs::default(),
            PathBuf::from("/dir"),
            vec![],
        );
        dialog.set_config(Arc::new(config));
        dialog.set_type_of(by_name());
        assert!(dialog.custom_items().is_empty());
    }

    #[test]
    fn the_palette_lists_the_actions_that_suit_and_runs_one() {
        let mut browser = with_actions(loaded(&[("a.png", false)]), SHRINK);
        browser.update(Message::EntryClicked { index: 0, ctrl: false, shift: false });
        browser.perform(Action::CommandPalette);
        browser.update(Message::PaletteInput("shrink".to_string()));
        assert_eq!(browser.palette_matches().first(), Some(&PaletteChoice::Custom(0)));
        let outcome = browser.update(Message::PaletteSubmit);
        assert!(matches!(outcome, Outcome::RunCustom { .. }), "{outcome:?}");
        browser.perform(Action::CommandPalette);
        browser.update(Message::PaletteInput("compare".to_string()));
        assert!(!browser.palette_matches().contains(&PaletteChoice::Custom(1)), "greyed rows are not commands");
    }

    #[test]
    fn f4_opens_a_terminal_where_you_are_not_on_the_folder_you_came_from() {
        let mut browser = loaded(&[("sub", true)]);
        browser.update(Message::EntryClicked { index: 0, ctrl: false, shift: false });
        assert_eq!(browser.perform(Action::OpenTerminal), Outcome::OpenTerminal("/dir".into()));
    }

    #[test]
    fn a_folders_menu_opens_the_terminal_in_that_folder() {
        let mut browser = loaded(&[("sub", true)]);
        right_click(&mut browser, MenuSpot::Row(0));
        let at = browser
            .menu
            .as_ref()
            .unwrap()
            .items
            .iter()
            .position(|i| i.label() == "Open Terminal Here")
            .expect("the folder menu offers it");
        assert_eq!(browser.update(Message::MenuChose(at)), Outcome::OpenTerminal("/dir/sub".into()));
    }

    #[test]
    fn the_empty_space_opens_the_terminal_in_the_folder_in_view() {
        let mut browser = loaded(&[("sub", true)]);
        right_click(&mut browser, MenuSpot::Background);
        let at = browser.menu.as_ref().unwrap().items.iter().position(|i| i.label() == "Open Terminal Here").unwrap();
        assert_eq!(browser.update(Message::MenuChose(at)), Outcome::OpenTerminal("/dir".into()));
    }
}
