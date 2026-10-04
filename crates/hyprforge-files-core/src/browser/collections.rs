//! The browser's half of Recent and Starred: the sidebar rows, what the
//! listing shows while one is open, the star on a starred row, and the
//! strip under the header that says what is on screen.
//!
//! A collection is shown the way a search's results are, and on purpose
//! by the same road: its entries live in many folders, and the results
//! overlay (`browser/searching.rs`) already lists entries from many
//! folders with a Folder column, Show in Folder at the top of a row's
//! menu, and every action keyed by path. So a collection's entries ride
//! in the search state as the rows, and Copy, Trash, drag, Rename and
//! Open all work on them unchanged. The folder in view stays loaded
//! underneath, exactly as it does behind a search, and comes back when
//! the person goes anywhere.
//!
//! The browser does no I/O, so it only *asks*: [`Outcome::ReadRecent`]
//! and [`Outcome::ReadStarred`] go to the host, and the answers come back
//! as [`Message::RecentRead`] and [`Message::StarredRead`]. The pure
//! parts live elsewhere: the bookmark file in [`crate::recent`], the
//! starred list in [`crate::starred`].

use super::{many, quiet_link_style, Browser, Message, Outcome, ViewModel};
use crate::starred::{Collection, StarChange};
use crate::types::Entry;
use hyprforge_ui::theme::{spacing, FontScale};
use hyprforge_ui::widgets::{meta_text, scaled_text};
use iced::widget::{container, row};
use iced::{Element, Length};
use std::collections::HashSet;
use std::path::PathBuf;

/// A collection on screen.
#[derive(Debug, Clone)]
pub(super) struct Listed {
    pub(super) kind: Collection,
    pub(super) results: Vec<Entry>,
    /// Asked for and not yet answered.
    reading: bool,
    /// Why Recent could not be read. Said in the strip, and the listing
    /// says it could not be read — never shown as an empty Recent.
    problem: Option<String>,
    /// Stars whose file could not be found.
    missing: Vec<PathBuf>,
}

/// What the view needs to draw the strip — see [`ViewModel::collection`].
#[derive(Debug, Clone, PartialEq)]
pub(super) struct CollectionModel<'a> {
    kind: Collection,
    shown: usize,
    reading: bool,
    problem: Option<&'a str>,
    missing: usize,
}

impl CollectionModel<'_> {
    pub(super) fn kind(&self) -> Collection {
        self.kind
    }
}

impl Browser {
    /// Which collection is on screen, if one is.
    pub fn collection(&self) -> Option<Collection> {
        self.search.collection.as_ref().map(|l| l.kind)
    }

    /// The window's starred list changed; this tab's copy follows it. A
    /// Starred view on screen is read again, so a star made in another
    /// tab shows here too — the [`Outcome`] is that read, for the host to
    /// carry out like any other.
    pub fn set_stars(&mut self, starred: Vec<PathBuf>) -> Outcome {
        let changed = starred != self.prefs.starred;
        self.stars = starred.iter().cloned().collect();
        self.prefs.starred = starred;
        match self.collection() {
            Some(Collection::Starred) if changed => self.read_collection(Collection::Starred),
            _ => Outcome::None,
        }
    }

    /// Opens a collection over the folder in view — a sidebar row's
    /// click. A search in force ends, as it does on going anywhere.
    pub(super) fn show_collection(&mut self, kind: Collection) -> Outcome {
        let stop = self.end_search();
        self.menu = None;
        self.renaming = None;
        self.path_edit = None;
        self.selection.clear();
        self.search.collection = Some(Listed { kind, results: Vec::new(), reading: true, problem: None, missing: Vec::new() });
        self.refresh_view();
        many(vec![stop, self.read_collection(kind)])
    }

    /// The question to ask the host for `kind`.
    pub(super) fn read_collection(&self, kind: Collection) -> Outcome {
        match kind {
            Collection::Recent => Outcome::ReadRecent,
            Collection::Starred => Outcome::ReadStarred(self.prefs.starred.clone()),
        }
    }

    /// An answer from the host. One for a collection no longer on screen
    /// lands nowhere.
    pub(super) fn collection_read(&mut self, kind: Collection, answer: Result<(Vec<Entry>, Vec<PathBuf>), String>) -> Outcome {
        let Some(listed) = self.search.collection.as_mut().filter(|l| l.kind == kind) else {
            return Outcome::None;
        };
        listed.reading = false;
        match answer {
            Ok((entries, missing)) => {
                listed.results = entries;
                listed.missing = missing;
                listed.problem = None;
            }
            Err(why) => {
                listed.results.clear();
                listed.missing.clear();
                listed.problem = Some(why);
            }
        }
        let present: HashSet<PathBuf> = self.listed().iter().map(|e| e.path.clone()).collect();
        self.selection.retain(&present);
        self.refresh_view();
        Outcome::None
    }

    /// The folder was read again — after a rename, a paste, a trash or
    /// F5. A collection on screen may name a file that has just moved,
    /// so it is read again too.
    pub(super) fn collection_after_refresh(&self) -> Outcome {
        self.collection().map_or(Outcome::None, |kind| self.read_collection(kind))
    }

    /// Star or unstar the selection — see [`crate::starred::toggle`].
    /// Kept here at once, so the star shows without waiting for the
    /// window to hand the list back; the window's copy is the one saved.
    pub(super) fn toggle_star(&mut self) -> Outcome {
        let paths = self.selected_shown();
        if paths.is_empty() {
            return Outcome::None;
        }
        let change = crate::starred::toggle(&self.prefs.starred, paths);
        self.apply_stars(&change);
        Outcome::Stars(change)
    }

    /// "Unstar them", for the stars the Starred view could not find.
    pub(super) fn unstar_missing(&mut self) -> Outcome {
        let Some(listed) = self.search.collection.as_mut().filter(|l| l.kind == Collection::Starred) else {
            return Outcome::None;
        };
        let missing = std::mem::take(&mut listed.missing);
        if missing.is_empty() {
            return Outcome::None;
        }
        let change = StarChange::Unstar(missing);
        self.apply_stars(&change);
        Outcome::Stars(change)
    }

    fn apply_stars(&mut self, change: &StarChange) {
        self.prefs.starred = crate::starred::apply_star_change(&self.prefs.starred, change);
        self.stars = self.prefs.starred.iter().cloned().collect();
        // An unstarred row leaves the Starred view at once; a starred one
        // there already was.
        if let Some(listed) = self.search.collection.as_mut().filter(|l| l.kind == Collection::Starred) {
            let stars = &self.stars;
            listed.results.retain(|e| stars.contains(&e.path));
            self.refresh_view();
        }
    }

    /// Whether every selected row is starred — what turns the menu's
    /// "Star" into "Unstar".
    pub(super) fn selection_starred(&self) -> bool {
        let selected = self.selected_shown();
        !selected.is_empty() && selected.iter().all(|p| self.stars.contains(p))
    }

    pub(super) fn collection_model(&self) -> Option<CollectionModel<'_>> {
        let listed = self.search.collection.as_ref()?;
        Some(CollectionModel {
            kind: listed.kind,
            shown: listed.results.len(),
            reading: listed.reading,
            problem: listed.problem.as_deref(),
            missing: listed.missing.len(),
        })
    }
}

/// The strip under the header while a collection is on screen and no
/// search is: what it is, how many, and anything that went wrong. The
/// search's own rail takes its place while something is typed.
pub(super) fn rail<'a>(vm: &ViewModel<'a>, scale: FontScale) -> Option<Element<'a, Message>> {
    let model = vm.collection.as_ref()?;
    if vm.search.active {
        return None;
    }
    let text_size = super::density::META_TEXT_BASE;
    let mut strip = row![scaled_text(model.kind.label(), text_size, scale)]
        .spacing(spacing::SM)
        .align_y(iced::Alignment::Center);
    let status = match (model.reading, model.problem) {
        (true, _) => Some("Reading\u{2026}".to_string()),
        (false, Some(_)) => None,
        (false, None) => Some(match (model.kind, model.shown) {
            (Collection::Recent, 1) => "1 file, newest first".to_string(),
            (Collection::Recent, n) => format!("{n} files, newest first"),
            (Collection::Starred, 1) => "1 starred".to_string(),
            (Collection::Starred, n) => format!("{n} starred"),
        }),
    };
    if let Some(status) = status {
        strip = strip.push(meta_text(status, text_size, scale));
    }
    if let Some(problem) = model.problem {
        strip = strip.push(
            scaled_text(problem, text_size, scale)
                .color(hyprforge_ui::theme::warning())
                .wrapping(iced::widget::text::Wrapping::WordOrGlyph),
        );
    }
    if model.missing > 0 {
        // Missing, like a pin whose folder is gone: said, and left for
        // the person to clear, because "gone" may only be "unmounted".
        let said = match model.missing {
            1 => "1 starred item can\u{2019}t be found".to_string(),
            n => format!("{n} starred items can\u{2019}t be found"),
        };
        strip = strip.push(scaled_text(said, text_size, scale).color(hyprforge_ui::theme::warning())).push(
            iced::widget::button(scaled_text("Unstar them", text_size, scale))
                .padding([0, spacing::XS as u16])
                .on_press(Message::UnstarMissing)
                .style(quiet_link_style),
        );
    }
    Some(
        container(strip)
            .padding([spacing::XS as u16, spacing::SM as u16])
            .width(Length::Fill)
            .clip(true)
            .style(|_t: &iced::Theme| container::Style {
                // The search rail's plane: chrome over the listing.
                background: Some(iced::Background::Color(hyprforge_ui::theme::surface::sidebar())),
                ..container::Style::default()
            })
            .into(),
    )
}

/// What the listing says when a collection on screen has no rows —
/// `None` when no collection is, or a search inside one is the reason.
pub(super) fn empty_message(vm: &ViewModel<'_>) -> Option<&'static str> {
    let model = vm.collection.as_ref()?;
    if vm.search.active {
        return None;
    }
    Some(match (model.kind, model.reading, model.problem.is_some()) {
        (_, true, _) => "Reading\u{2026}",
        (Collection::Recent, false, true) => "Recent couldn\u{2019}t be read.",
        (Collection::Recent, false, false) => "Nothing has been opened lately.",
        (Collection::Starred, false, _) if model.missing > 0 => "None of the starred items can be found.",
        (Collection::Starred, false, _) => "Nothing is starred. Star a file from its menu.",
    })
}

/// The icon key a collection's sidebar row asks the theme for.
pub(super) fn icon(kind: Collection) -> String {
    crate::icon::themed_key(kind.icon_name())
}

/// `icon`, with the star on its corner when the row is starred — the
/// emblem arrangement every file manager uses for a file's state, so the
/// name and the columns beside it stay where every other row has them.
/// On a selected row the star takes the text colour: the accent would
/// vanish into the row's own accent fill.
pub(super) fn starred_icon<'a>(
    icon: Element<'a, Message>,
    starred: bool,
    side: f32,
    selected: bool,
) -> Element<'a, Message> {
    if !starred {
        return icon;
    }
    let colour = if selected { hyprforge_ui::theme::text() } else { hyprforge_ui::widgets::Tint::Accent.iced() };
    let mark = side * 0.55;
    iced::widget::stack![
        icon,
        container(hyprforge_ui::glyph::star(mark, colour))
            .width(Length::Fixed(side))
            .height(Length::Fixed(side))
            .align_x(iced::alignment::Horizontal::Right)
            .align_y(iced::alignment::Vertical::Bottom),
    ]
    .into()
}

/// A grid cell with the star on its top corner, when it is starred.
pub(super) fn starred_cell<'a>(cell: Element<'a, Message>, starred: bool, selected: bool, scale: FontScale) -> Element<'a, Message> {
    if !starred {
        return cell;
    }
    let colour = if selected { hyprforge_ui::theme::text() } else { hyprforge_ui::widgets::Tint::Accent.iced() };
    iced::widget::stack![
        cell,
        container(hyprforge_ui::glyph::star(scale.apply(14.0), colour))
            .width(Length::Fill)
            .padding(scale.apply(4.0))
            .align_x(iced::alignment::Horizontal::Right),
    ]
    .into()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::action::Action;
    use crate::browser::Mode;
    use crate::prefs::Prefs;
    use std::path::Path;

    fn file(dir: &str, name: &str) -> Entry {
        let mut entry = crate::backend::mock::MockBackend::file(Path::new(dir), name, 1);
        entry.origin = Some(PathBuf::from(dir));
        entry
    }

    fn browser_at(dir: &str, entries: Vec<Entry>) -> Browser {
        let (mut browser, _) = Browser::new(Mode::App, Prefs::default(), PathBuf::from(dir), Vec::new());
        browser.update(Message::DirLoaded(PathBuf::from(dir), Ok(entries)));
        browser
    }

    fn names(browser: &Browser) -> Vec<String> {
        browser.rows().iter().map(|e| e.name.clone()).collect()
    }

    fn reads(outcome: &Outcome) -> Vec<Outcome> {
        match outcome {
            Outcome::ReadRecent | Outcome::ReadStarred(_) => vec![outcome.clone()],
            Outcome::Many(parts) => parts.iter().flat_map(reads).collect(),
            _ => Vec::new(),
        }
    }

    /// Recent is shown the way a search's results are: the rows are its
    /// entries, from wherever they live, in the order they were used —
    /// not re-sorted by name — and going anywhere puts the folder back.
    #[test]
    fn recent_lists_its_entries_newest_first_and_leaves_on_navigating() {
        let mut browser = browser_at("/d", vec![file("/d", "here.txt")]);
        let outcome = browser.update(Message::ShowCollection(Collection::Recent));
        assert_eq!(reads(&outcome), [Outcome::ReadRecent]);
        assert!(browser.rows().is_empty(), "nothing shown before it is read");
        browser.update(Message::RecentRead(Ok(vec![file("/x", "zebra.png"), file("/y", "apple.txt")])));
        assert_eq!(names(&browser), ["zebra.png", "apple.txt"], "newest first, not by name");
        assert!(browser.in_results(), "a Folder column and Show in Folder, as for results");
        let outcome = browser.update(Message::Navigate(PathBuf::from("/d")));
        assert!(outcome.navigates());
        browser.update(Message::DirLoaded(PathBuf::from("/d"), Ok(vec![file("/d", "here.txt")])));
        assert_eq!(browser.collection(), None);
        assert_eq!(names(&browser), ["here.txt"]);
    }

    /// The rule from the 37 binds, in the view: a Recent that could not
    /// be read says so, and is never an empty list that looks like
    /// nothing was opened.
    #[test]
    fn an_unreadable_recent_is_reported_not_shown_as_empty() {
        let mut browser = browser_at("/d", vec![]);
        browser.update(Message::ShowCollection(Collection::Recent));
        browser.update(Message::RecentRead(Err("recently-used.xbel isn't XML".into())));
        let vm = browser.view_model(1000.0);
        assert_eq!(vm.collection.as_ref().and_then(|c| c.problem), Some("recently-used.xbel isn't XML"));
        assert_eq!(empty_message(&vm), Some("Recent couldn\u{2019}t be read."));
    }

    #[test]
    fn an_answer_for_a_collection_no_longer_shown_lands_nowhere() {
        let mut browser = browser_at("/d", vec![file("/d", "a.txt")]);
        browser.update(Message::ShowCollection(Collection::Recent));
        browser.update(Message::ShowCollection(Collection::Starred));
        browser.update(Message::RecentRead(Ok(vec![file("/x", "late.txt")])));
        assert!(browser.rows().is_empty());
    }

    #[test]
    fn starring_from_the_keyboard_stars_the_selection_and_again_unstars_it() {
        let mut browser = browser_at("/d", vec![file("/d", "a.txt")]);
        browser.update(Message::EntryClicked { index: 0, ctrl: false, shift: false });
        let outcome = browser.perform(Action::ToggleStar);
        assert_eq!(outcome, Outcome::Stars(StarChange::Star(vec!["/d/a.txt".into()])));
        assert!(browser.selection_starred());
        assert_eq!(crate::action::label_in(Action::ToggleStar, &browser.action_context()), "Unstar");
        let outcome = browser.perform(Action::ToggleStar);
        assert_eq!(outcome, Outcome::Stars(StarChange::Unstar(vec!["/d/a.txt".into()])));
        assert!(browser.prefs().starred.is_empty());
    }

    /// The Starred view asks for the stars as the window knows them, and
    /// a star that cannot be found is said, with a way to clear it.
    #[test]
    fn the_starred_view_shows_what_is_found_and_counts_what_is_missing() {
        let mut browser = browser_at("/d", vec![]);
        browser.set_stars(vec!["/x/a.txt".into(), "/x/gone.txt".into()]);
        let outcome = browser.update(Message::ShowCollection(Collection::Starred));
        assert_eq!(reads(&outcome), [Outcome::ReadStarred(vec!["/x/a.txt".into(), "/x/gone.txt".into()])]);
        browser.update(Message::StarredRead { entries: vec![file("/x", "a.txt")], missing: vec!["/x/gone.txt".into()] });
        assert_eq!(names(&browser), ["a.txt"]);
        assert_eq!(browser.view_model(1000.0).collection.map(|c| c.missing), Some(1));
        let outcome = browser.update(Message::UnstarMissing);
        assert_eq!(outcome, Outcome::Stars(StarChange::Unstar(vec!["/x/gone.txt".into()])));
        assert_eq!(browser.prefs().starred, [PathBuf::from("/x/a.txt")]);
    }

    #[test]
    fn unstarring_in_the_starred_view_takes_the_row_away() {
        let mut browser = browser_at("/d", vec![]);
        browser.set_stars(vec!["/x/a.txt".into(), "/x/b.txt".into()]);
        browser.update(Message::ShowCollection(Collection::Starred));
        browser.update(Message::StarredRead { entries: vec![file("/x", "a.txt"), file("/x", "b.txt")], missing: vec![] });
        browser.update(Message::EntryClicked { index: 0, ctrl: false, shift: false });
        browser.perform(Action::ToggleStar);
        assert_eq!(names(&browser), ["b.txt"]);
    }

    /// The sidebar's two rows head Places, keyed as collections rather
    /// than as a path — and each switch in `[sidebar]` takes its row away.
    #[test]
    fn recent_and_starred_head_places_and_each_switch_hides_its_row() {
        let browser = browser_at("/d", vec![]);
        let vm = browser.view_model(1000.0);
        let sections = super::super::sidebar_sections(&vm);
        let places = sections.iter().find(|s| s.title == "Places").unwrap();
        let first: Vec<Option<Collection>> = places.rows.iter().take(2).map(|r| r.search.and_then(|q| q.collection())).collect();
        assert_eq!(first, [Some(Collection::Recent), Some(Collection::Starred)]);

        let mut browser = browser_at("/d", vec![]);
        let (config, problems) = crate::config::parse("[sidebar]\nshow-recent = false\n", Path::new("x"));
        assert!(problems.is_empty());
        browser.set_config(std::sync::Arc::new(config));
        let vm = browser.view_model(1000.0);
        let sections = super::super::sidebar_sections(&vm);
        let rows = &sections.iter().find(|s| s.title == "Places").unwrap().rows;
        assert!(rows.iter().all(|r| r.search.and_then(|q| q.collection()) != Some(Collection::Recent)));
        assert!(rows.iter().any(|r| r.search.and_then(|q| q.collection()) == Some(Collection::Starred)));
    }

    /// A refresh of the folder behind — after a rename, say — reads the
    /// collection again, since what it lists may just have moved.
    #[test]
    fn a_refresh_reads_the_collection_on_screen_again() {
        let mut browser = browser_at("/d", vec![]);
        browser.update(Message::ShowCollection(Collection::Recent));
        browser.update(Message::RecentRead(Ok(vec![])));
        let outcome = browser.update(Message::DirLoaded(PathBuf::from("/d"), Ok(vec![])));
        assert_eq!(reads(&outcome), [Outcome::ReadRecent]);
    }
}
