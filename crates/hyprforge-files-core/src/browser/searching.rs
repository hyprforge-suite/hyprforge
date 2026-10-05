//! The browser's half of searching: what the box holds, which rows a
//! search below the folder shows, the scope rail under the header, and
//! the saved searches in the sidebar.
//!
//! A child of `browser` so it can reach the browser's own state without
//! that state growing public accessors for one feature — and a file of
//! its own so the feature reads in one place. The pure parts live
//! elsewhere: the query language in [`crate::query`], the walk and the
//! saved-search list in [`crate::search`].
//!
//! # Two kinds of search, one box
//!
//! **This folder** filters the listing already in memory, on every
//! keystroke, exactly as the box always has. **Subfolders** and **Home**
//! walk, which the browser cannot do itself — it does no I/O — so it
//! asks the host with [`Ask::Run`] and shows what comes back as it
//! arrives. While a walk's results are on screen they *are* the
//! listing: selection, the menus, Copy, Trash and drag all act on them,
//! because every one of those is keyed by path, not by which folder a
//! row came from. The folder's own listing is kept underneath, and comes
//! back the moment the search ends.

use super::{many, quiet_link_style, Browser, Message, Outcome, ViewModel};
use crate::action::Action;
use crate::menu::MenuItem;
use crate::query::{self, Filter, Query};
use crate::content::ContentReport;
use crate::search::{Ask, Budget, End, Request, Scope, SearchMessage, SmartChange, SmartFolder, Summary};
use crate::types::Entry;
use hyprforge_ui::theme::{spacing, FontScale};
use hyprforge_ui::widgets::{meta_text, removable_chip, scaled_text, segment_style, segmented, SegmentLook};
use iced::widget::{container, row, text_input, Id};
use iced::{Element, Length};
use std::path::{Path, PathBuf};

/// What a browser holds about searching, beside the field's text (which
/// stays `Browser::search_query`, the one thing a host asks about).
#[derive(Debug, Clone, Default)]
pub(super) struct SearchState {
    scope: Scope,
    /// Finished filters, taken out of the field as they were typed — see
    /// [`query::take_chips`].
    chips: Vec<Filter>,
    /// The walk on screen, when the scope walks and something is typed.
    run: Option<Run>,
    /// Numbers handed out so far — see [`Request::run`].
    runs: u64,
    /// The name being typed for "Save search".
    naming: Option<Naming>,
    /// Recent or Starred, while one is on screen — see
    /// `browser/collections.rs`. Here rather than beside the search
    /// because it *is* a set of results: the rows come from here, and
    /// everything that already works on a walk's results works on it.
    pub(super) collection: Option<super::collections::Listed>,
}

impl SearchState {
    /// The rows when they are not the folder's own: a walk's results
    /// while one is on screen, else a collection's.
    pub(super) fn results(&self) -> Option<&[Entry]> {
        self.run
            .as_ref()
            .map(|r| r.results.as_slice())
            .or_else(|| self.collection.as_ref().map(|c| c.results.as_slice()))
    }

    /// Whether a walk's results are on screen.
    pub(super) fn walking(&self) -> bool {
        self.run.is_some()
    }
}

#[derive(Debug, Clone)]
struct Run {
    id: u64,
    root: PathBuf,
    results: Vec<Entry>,
    /// `None` while the walk is going.
    summary: Option<Summary>,
}

#[derive(Debug, Clone)]
struct Naming {
    text: String,
    /// Fresh each time, like the rename field's.
    id: Id,
}

/// What the view needs to draw the search — see [`ViewModel::search`].
#[derive(Debug, Clone, PartialEq)]
pub(super) struct SearchModel<'a> {
    /// Something is typed: the rail shows.
    pub(super) active: bool,
    pub(super) scope: Scope,
    /// Searching below is not offered in the Trash — see
    /// [`Browser::walk_offered`].
    walks: bool,
    /// Home is not offered when the folder in view *is* home.
    home: bool,
    pub(super) chips: &'a [Filter],
    problems: Vec<String>,
    /// A walk's progress or how it ended. `None` for "This folder".
    status: Option<String>,
    running: bool,
    pub(super) results: bool,
    /// Where the results on screen were searched from.
    pub(super) root: Option<&'a Path>,
    naming: Option<(&'a str, Id)>,
    saved_as: Option<&'a str>,
    /// Which saved search is on screen, for its sidebar row.
    pub(super) current: Option<usize>,
}

impl SearchModel<'static> {
    /// No search in force — for a test building a view by hand.
    #[cfg(test)]
    pub(super) fn idle() -> Self {
        SearchModel {
            active: false,
            scope: Scope::Folder,
            walks: true,
            home: true,
            chips: &[],
            problems: Vec::new(),
            status: None,
            running: false,
            results: false,
            root: None,
            naming: None,
            saved_as: None,
            current: None,
        }
    }
}

impl Browser {
    /// The query as the box holds it: the chips, then what is still in
    /// the field.
    pub(super) fn query(&self) -> Query {
        let mut query = query::parse(&self.search_query);
        let typed = std::mem::take(&mut query.filters);
        query.filters = self.search.chips.iter().cloned().chain(typed).collect();
        query
    }

    /// Whether a search is in force at all — text, or a chip with none.
    pub(super) fn searching(&self) -> bool {
        !self.search_query.is_empty() || !self.search.chips.is_empty()
    }

    /// The entries the rows are drawn from: a walk's results while one is
    /// on screen, the folder's own listing otherwise.
    pub(super) fn listed(&self) -> &[Entry] {
        self.search.results().unwrap_or(&self.entries)
    }

    /// Whether the rows are a walk's results, or Recent or Starred,
    /// rather than this folder.
    pub(super) fn in_results(&self) -> bool {
        self.search.run.is_some() || self.search.collection.is_some()
    }

    /// The saved searches, as the window's list changes — the same
    /// arrangement as [`Browser::set_pins`]: the window owns the list and
    /// tells every tab.
    pub fn set_smart_folders(&mut self, searches: Vec<SmartFolder>) {
        self.prefs.searches = searches;
    }

    /// Searching below a folder means something everywhere but the
    /// Trash, whose folders hold trashed things under stored names that
    /// nothing can be restored from one at a time.
    fn walk_offered(&self) -> bool {
        !self.in_trash()
    }

    fn search_root(&self) -> PathBuf {
        match self.search.scope {
            Scope::Home => super::home_dir().unwrap_or_else(|| self.current_dir.clone()),
            Scope::Folder | Scope::Below => self.current_dir.clone(),
        }
    }

    /// The field's text changed — typed into, or a character from
    /// type-to-search. Takes out any filter just finished as a chip, then
    /// searches again.
    pub(super) fn search_typed(&mut self) -> Outcome {
        let (chips, rest) = query::take_chips(&self.search_query);
        if !chips.is_empty() {
            self.search.chips.extend(chips);
            self.search_query = rest;
        }
        self.search_again()
    }

    /// Re-applies the search after anything about it changed: the
    /// listing filters at once, and a walking scope starts a new walk —
    /// the old one is the host's to cancel when the new one arrives.
    ///
    /// One request per change, never per result: each keystroke is "look
    /// again", and the host keeps one walk per tab running.
    ///
    /// A `content:` filter walks even in "This folder", one level deep:
    /// the listing in memory holds names, not what the files say, and
    /// reading them is I/O the browser never does.
    pub(super) fn search_again(&mut self) -> Outcome {
        let reads = self.query().reads_contents();
        let walk = (self.search.scope.walks() || reads) && self.walk_offered() && self.searching();
        if !walk {
            let stop = self.search.run.take().is_some();
            self.refresh_view();
            return if stop { Outcome::Search(Ask::Stop) } else { Outcome::None };
        }
        self.search.runs += 1;
        let id = self.search.runs;
        let root = self.search_root();
        self.search.run = Some(Run { id, root: root.clone(), results: Vec::new(), summary: None });
        self.refresh_view();
        let skip = crate::sidebar::trash_path().parent().map(Path::to_path_buf).into_iter().collect();
        Outcome::Search(Ask::Run(Request {
            run: id,
            root,
            matcher: self.query().matcher(chrono::Local::now()),
            show_hidden: self.prefs.show_hidden,
            skip,
            budget: Budget { depth: if self.search.scope.walks() { Budget::default().depth } else { 1 }, ..Budget::default() },
        }))
    }

    /// The folder was read again — after a paste, a rename, a trash or
    /// F5. Results on screen may now name files that moved, so a walk on
    /// screen is run again rather than left showing them.
    pub(super) fn search_after_refresh(&mut self) -> Outcome {
        if self.search.run.is_some() {
            self.search_again()
        } else {
            // A collection's rows may name a file that just moved too.
            self.collection_after_refresh()
        }
    }

    /// Everything about the search, back to nothing — Escape, and
    /// arriving somewhere new. The scope goes back to "This folder" too:
    /// the next thing typed anywhere should be the instant search, not a
    /// walk somebody started earlier for a different folder.
    pub(super) fn end_search(&mut self) -> Outcome {
        self.search_query.clear();
        self.search.chips.clear();
        self.search.naming = None;
        self.search.scope = Scope::Folder;
        let stop = self.search.run.take().is_some();
        if stop {
            Outcome::Search(Ask::Stop)
        } else {
            Outcome::None
        }
    }

    pub(super) fn update_search(&mut self, message: SearchMessage) -> Outcome {
        match message {
            SearchMessage::Scope(scope) => {
                self.search.scope = scope;
                self.search_again()
            }
            SearchMessage::RemoveChip(index) => {
                if index < self.search.chips.len() {
                    self.search.chips.remove(index);
                }
                self.search_again()
            }
            SearchMessage::Found { run, entries } => {
                let Some(current) = self.search.run.as_mut().filter(|r| r.id == run) else {
                    // A walk since replaced: what it found answers a
                    // question nobody is asking any more.
                    return Outcome::None;
                };
                current.results.extend(entries);
                self.refresh_view();
                Outcome::None
            }
            SearchMessage::Finished { run, summary } => {
                if let Some(current) = self.search.run.as_mut().filter(|r| r.id == run) {
                    current.summary = Some(summary);
                }
                Outcome::None
            }
            SearchMessage::Stop => {
                let Some(current) = self.search.run.as_mut().filter(|r| r.summary.is_none()) else {
                    return Outcome::None;
                };
                // Said at once, with what is known; the host's own report
                // for this walk replaces it with the real counts.
                current.summary = Some(Summary { found: current.results.len(), ..Summary::stopped() });
                Outcome::Search(Ask::Stop)
            }
            SearchMessage::SaveStart => {
                let id = Id::unique();
                self.search.naming = Some(Naming { text: self.suggested_name(), id: id.clone() });
                Outcome::FocusPath { id, select_all: true }
            }
            SearchMessage::SaveName(text) => {
                if let Some(naming) = &mut self.search.naming {
                    naming.text = text;
                }
                Outcome::None
            }
            SearchMessage::SaveCommit => {
                let Some(naming) = self.search.naming.take() else { return Outcome::None };
                let name = naming.text.trim().to_string();
                if name.is_empty() || !self.searching() {
                    return Outcome::None;
                }
                let saved = SmartFolder {
                    name,
                    query: self.query_text(),
                    folder: self.search_root(),
                    subfolders: self.search.scope.walks(),
                };
                // Kept here at once, so the rail says "Saved" without
                // waiting for the window to hand the list back.
                self.prefs.searches = crate::search::apply_smart_change(&self.prefs.searches, &SmartChange::Save(saved.clone()));
                Outcome::Search(Ask::Smart(SmartChange::Save(saved)))
            }
            SearchMessage::SaveCancel => {
                self.search.naming = None;
                Outcome::None
            }
            SearchMessage::Forget => {
                let Some(index) = self.current_smart() else { return Outcome::None };
                let name = self.prefs.searches[index].name.clone();
                let change = SmartChange::Forget(name);
                self.prefs.searches = crate::search::apply_smart_change(&self.prefs.searches, &change);
                Outcome::Search(Ask::Smart(change))
            }
            SearchMessage::Open(index) => {
                let Some(saved) = self.prefs.searches.get(index).cloned() else { return Outcome::None };
                // A saved search is a question about a folder, not about
                // Recent or Starred: whichever was open is left.
                self.search.collection = None;
                let arrive = if saved.folder == self.current_dir {
                    let stop = self.end_search();
                    many(vec![stop])
                } else {
                    self.go_to(saved.folder.clone())
                };
                let (chips, text) = crate::search::load_query(&saved.query);
                self.search.chips = chips;
                self.search_query = text;
                self.search.scope = if saved.subfolders { Scope::Below } else { Scope::Folder };
                let run = self.search_again();
                many(vec![arrive, run])
            }
        }
    }

    /// The scopes the rail offers here, in its order — see
    /// [`Self::walk_offered`] and the rail's own Home rule.
    fn offered_scopes(&self) -> Vec<Scope> {
        let mut scopes = vec![Scope::Folder];
        if self.walk_offered() {
            scopes.push(Scope::Below);
            if super::home_dir().is_some_and(|home| home != self.current_dir) {
                scopes.push(Scope::Home);
            }
        }
        scopes
    }

    /// The rail's next scope, from the keyboard: This folder, Subfolders,
    /// Home, and round again — the buttons in the order they are drawn,
    /// skipping any the rail is not showing.
    pub(super) fn next_scope(&mut self) -> Outcome {
        let offered = self.offered_scopes();
        let at = offered.iter().position(|s| *s == self.search.scope);
        let next = offered[at.map_or(0, |i| (i + 1) % offered.len())];
        if next == self.search.scope {
            return Outcome::None;
        }
        self.update_search(SearchMessage::Scope(next))
    }

    /// The query as a saved search stores it.
    fn query_text(&self) -> String {
        self.query().to_text(&self.search_query)
    }

    /// "ext:rs in crates" — what "Save search" offers before anything is
    /// typed over it.
    fn suggested_name(&self) -> String {
        let place = crate::sidebar::place_name(&self.search_root());
        let what = self.query_text();
        let what = if what.chars().count() > 32 {
            format!("{}\u{2026}", what.chars().take(31).collect::<String>())
        } else {
            what
        };
        format!("{what} in {place}")
    }

    /// The saved search that is what is on screen, if one is: same
    /// folder, same reach, same query.
    fn current_smart(&self) -> Option<usize> {
        if !self.searching() {
            return None;
        }
        let (folder, subfolders, query) = (self.search_root(), self.search.scope.walks(), self.query_text());
        self.prefs
            .searches
            .iter()
            .position(|s| s.folder == folder && s.subfolders == subfolders && s.query == query)
    }

    /// Show in Folder: go to where the focused result lives, with it
    /// selected — the arrangement the path bar uses for a file.
    pub(super) fn show_in_folder(&mut self) -> Outcome {
        let Some(path) = self.selection.focused().map(Path::to_path_buf) else { return Outcome::None };
        let Some(folder) = path.parent().map(Path::to_path_buf) else { return Outcome::None };
        if folder == self.current_dir {
            // Already here: ending the search — and leaving Recent or
            // Starred — puts this folder's own listing back, which is
            // already loaded.
            self.search.collection = None;
            let stop = self.end_search();
            self.refresh_view();
            if let Some(index) = self.rows().iter().position(|e| e.path == path) {
                self.with_rows(|selection, rows| selection.click_with(rows, index, false, false));
            }
            return stop;
        }
        let outcome = self.go_to(folder);
        self.after_listing = Some((path, false));
        outcome
    }

    /// A row's menu among results starts with Show in Folder — the one
    /// thing a result needs that a folder's own row does not. Added here
    /// rather than to every configured menu, where it would sit greyed
    /// out everywhere else.
    pub(super) fn with_result_items(&self, mut items: Vec<MenuItem>) -> Vec<MenuItem> {
        if !self.in_results() {
            return items;
        }
        let show = MenuItem::Action {
            action: Action::ShowInFolder,
            label: Action::ShowInFolder.label(),
            hint: self.config.keymap.combos_for(Action::ShowInFolder).first().map(|c| c.to_string()),
            enabled: crate::action::enabled(Action::ShowInFolder, &self.action_context()),
        };
        if !items.is_empty() {
            items.insert(0, MenuItem::Separator);
        }
        items.insert(0, show);
        items
    }

    pub(super) fn search_model(&self) -> SearchModel<'_> {
        let query = self.query();
        let mut problems: Vec<String> = query.problems.iter().map(|p| p.message()).collect();
        if query.reads_contents() && !self.walk_offered() {
            // The one place a content filter cannot run: the Trash's
            // files are stored under names that are not theirs. Said,
            // rather than quietly matching on the name alone.
            problems.push("Contents aren\u{2019}t searched in the Trash, so content: isn\u{2019}t applied.".to_string());
        }
        let status = self.search.run.as_ref().map(|run| status_line(run.results.len(), run.summary.as_ref()));
        SearchModel {
            active: self.searching(),
            scope: self.search.scope,
            walks: self.walk_offered(),
            home: super::home_dir().is_some_and(|home| home != self.current_dir),
            chips: &self.search.chips,
            problems,
            status,
            running: self.search.run.as_ref().is_some_and(|r| r.summary.is_none()),
            results: self.in_results(),
            root: self.search.run.as_ref().map(|r| r.root.as_path()),
            naming: self.search.naming.as_ref().map(|n| (n.text.as_str(), n.id.clone())),
            saved_as: self.current_smart().map(|i| self.prefs.searches[i].name.as_str()),
            current: self.current_smart(),
        }
    }
}

/// A count with thousands grouped, the way the status line writes one.
fn grouped(n: usize) -> String {
    let digits = n.to_string();
    let mut out = String::with_capacity(digits.len() + digits.len() / 3);
    for (i, c) in digits.chars().enumerate() {
        if i > 0 && (digits.len() - i).is_multiple_of(3) {
            out.push(',');
        }
        out.push(c);
    }
    out
}

fn plural(n: usize, one: &str, many: &str) -> String {
    format!("{} {}", grouped(n), if n == 1 { one } else { many })
}

/// What the rail says about a walk: going, finished, or stopped — and,
/// when stopped by a limit, which one and what to do about it. Every
/// folder not searched is counted aloud; see [`crate::search`]'s doc.
pub(super) fn status_line(found: usize, summary: Option<&Summary>) -> String {
    let Some(summary) = summary else {
        return format!("Searching\u{2026} {} found", grouped(found));
    };
    let mut line = match summary.end {
        End::Complete => format!("{} in {}", plural(found, "match", "matches"), plural(summary.folders, "folder", "folders")),
        End::Stopped => format!("Stopped \u{00b7} {} so far", plural(found, "match", "matches")),
        End::TimeLimit => format!(
            "Stopped after {}s \u{00b7} {} \u{2014} narrow the search to see the rest",
            summary.elapsed.as_secs(),
            plural(found, "match", "matches")
        ),
        End::FolderLimit => format!(
            "Stopped after {} \u{00b7} {} \u{2014} narrow the search or search a smaller folder",
            plural(summary.folders, "folder", "folders"),
            plural(found, "match", "matches")
        ),
        End::FoundLimit => format!("Showing the first {} \u{2014} narrow the search to see the rest", grouped(found)),
        End::ReadLimit => format!(
            "Stopped after reading {} \u{00b7} {} \u{2014} narrow the search to see the rest",
            crate::format::human_readable_size(summary.contents.bytes),
            plural(found, "match", "matches")
        ),
    };
    line.push_str(&contents_line(&summary.contents));
    if summary.unreadable > 0 {
        line.push_str(&format!(" \u{00b7} {} couldn't be read", plural(summary.unreadable, "folder", "folders")));
    }
    if summary.elsewhere > 0 {
        line.push_str(&format!(" \u{00b7} {} on other drives not searched", plural(summary.elsewhere, "folder", "folders")));
    }
    line
}

/// What a content search read and what it did not, for the rail — every
/// file it skipped is counted with its reason, never left out of the
/// total quietly. Empty when nothing was read or skipped.
fn contents_line(report: &ContentReport) -> String {
    if report.read == 0 && report.skipped() == 0 {
        return String::new();
    }
    // "0 files read" says nothing the reasons after it do not.
    let mut line = if report.read > 0 { format!(" \u{00b7} {} read", plural(report.read, "file", "files")) } else { String::new() };
    let reasons: Vec<String> = [
        (report.too_large, "too large", "too large"),
        (report.binary, "binary", "binary"),
        (report.unreadable, "unreadable", "unreadable"),
        (report.archives, "archive", "archives"),
        (report.packed, "inside an archive", "inside an archive"),
        (report.links, "link", "links"),
    ]
    .into_iter()
    .filter(|(n, _, _)| *n > 0)
    .map(|(n, one, many)| plural(n, one, many))
    .collect();
    if !reasons.is_empty() {
        line.push_str(&format!(" \u{00b7} not read: {}", reasons.join(", ")));
    }
    line
}

/// How wide the search field is: its usual width, plus room for its
/// chips, up to three times that. The chips are in mono at a fixed size,
/// so their width is predictable enough to reserve without measuring.
fn field_width(chips: &[Filter], scale: FontScale) -> f32 {
    let base = scale.apply(super::density::SEARCH_FIELD_WIDTH);
    let room: f32 = chips.iter().map(|c| scale.apply(c.source.chars().count() as f32 * 6.4 + 26.0)).sum();
    (base + room).min(base * 3.0)
}

/// The search field, with its chips inside it.
///
/// The placeholder names the scope — "Search crates", not "Search" — so
/// what it will search is stated before anything is typed.
pub(super) fn search_box<'a>(vm: &ViewModel<'a>, scale: FontScale) -> Element<'a, Message> {
    let placeholder = format!("Search {}", crate::sidebar::place_name(vm.current_dir));
    let chips = vm.search.chips;
    let tokens = chips
        .iter()
        .enumerate()
        .map(|(i, chip)| removable_chip(chip.source.clone(), Message::Search(SearchMessage::RemoveChip(i)), scale))
        .collect();
    hyprforge_ui::widgets::token_field(&placeholder, vm.search_query, tokens, Message::SearchChanged, None, Some(vm.search_field_id.clone()), scale)
        .width(Length::Fixed(field_width(chips, scale)))
        .into()
}

/// The scope rail: a strip under the header while a search is in force,
/// with where it looks, how it is going, what was not understood, and
/// saving it. Mockup `1e`'s rail, laid flat — a column down the side
/// would take a sidebar's width from the results for three buttons.
pub(super) fn search_rail<'a>(vm: &ViewModel<'a>, scale: FontScale) -> Option<Element<'a, Message>> {
    let model = &vm.search;
    if !model.active {
        return None;
    }
    let text_size = super::density::META_TEXT_BASE;
    let scope_button = |label: &'static str, scope: Scope| -> Element<'a, Message> {
        iced::widget::button(scaled_text(label, text_size, scale))
            .padding([1.0, scale.apply(8.0)])
            .on_press(Message::Search(SearchMessage::Scope(scope)))
            // Quiet: where the search looks is a mode, not a selection,
            // and the accent on this bar would compete with the rows.
            .style(segment_style(SegmentLook::Quiet, model.scope == scope))
            .into()
    };
    let mut scopes = vec![scope_button("This folder", Scope::Folder)];
    if model.walks {
        scopes.push(scope_button("Subfolders", Scope::Below));
        if model.home {
            scopes.push(scope_button("Home", Scope::Home));
        }
    }
    let link = |label: String, message: Message| -> Element<'a, Message> {
        iced::widget::button(scaled_text(label, text_size, scale))
            .padding([0, spacing::XS as u16])
            .on_press(message)
            .style(quiet_link_style)
            .into()
    };

    let mut strip = row![meta_text("Look in", text_size, scale), segmented(scopes)]
        .spacing(spacing::SM)
        .align_y(iced::Alignment::Center);
    if let Some(status) = &model.status {
        strip = strip.push(meta_text(status.clone(), text_size, scale).wrapping(iced::widget::text::Wrapping::None));
    }
    if model.running {
        strip = strip.push(link("Stop".to_string(), Message::Search(SearchMessage::Stop)));
    }
    for problem in &model.problems {
        strip = strip.push(
            scaled_text(problem.clone(), text_size, scale)
                .color(hyprforge_ui::theme::warning())
                .wrapping(iced::widget::text::Wrapping::None),
        );
    }
    strip = strip.push(iced::widget::Space::new().width(Length::Fill));
    strip = match (&model.naming, model.saved_as) {
        (Some((name, id)), _) => strip
            .push(
                text_input("Name this search", name)
                    .id(id.clone())
                    .on_input(|t| Message::Search(SearchMessage::SaveName(t)))
                    .on_submit(Message::Search(SearchMessage::SaveCommit))
                    .size(scale.apply(text_size))
                    .padding([2.0, spacing::SM])
                    .width(Length::Fixed(scale.apply(200.0)))
                    .style(hyprforge_ui::widgets::inset_input_style),
            )
            .push(link("Save".to_string(), Message::Search(SearchMessage::SaveCommit)))
            .push(link("Cancel".to_string(), Message::Search(SearchMessage::SaveCancel))),
        (None, Some(name)) => strip
            .push(meta_text(format!("Saved as \u{201c}{name}\u{201d}"), text_size, scale))
            .push(link("Remove".to_string(), Message::Search(SearchMessage::Forget))),
        (None, None) => strip.push(link("Save search\u{2026}".to_string(), Message::Search(SearchMessage::SaveStart))),
    };

    Some(
        container(strip)
            .padding([spacing::XS as u16, spacing::SM as u16])
            .width(Length::Fill)
            .clip(true)
            .style(|_t: &iced::Theme| container::Style {
                // The header's plane: the rail is part of the chrome the
                // listing is recessed under, not a row of the listing.
                background: Some(iced::Background::Color(hyprforge_ui::theme::surface::sidebar())),
                ..container::Style::default()
            })
            .into(),
    )
}

/// What the listing says when a search leaves it empty.
pub(super) fn empty_message(vm: &ViewModel<'_>) -> &'static str {
    match (&vm.search.status, vm.search.running) {
        (_, true) => "Searching\u{2026}",
        // A content search of this folder alone also walks, one level.
        (Some(_), false) if vm.search.scope == Scope::Folder => "No entries match your search.",
        (Some(_), false) => "Nothing below here matches your search.",
        (None, _) => "No entries match your search.",
    }
}

/// A result's Folder cell: where it was found, from the folder the
/// search started in — `crates/src`, not `/home/a/projects/crates/src`,
/// because the column is narrow and the part above the search's own
/// folder is the same on every row. Starts with that folder's name, so a
/// file found in the folder itself reads `crates` rather than nothing.
pub(super) fn found_in(entry: &Entry, root: &Path) -> String {
    let Some(origin) = &entry.origin else { return String::new() };
    let base = root.file_name().map_or_else(|| root.display().to_string(), |n| n.to_string_lossy().into_owned());
    match origin.strip_prefix(root) {
        Ok(rest) if rest.as_os_str().is_empty() => base,
        Ok(rest) => format!("{base}/{}", rest.display()),
        Err(_) => origin.display().to_string(),
    }
}

/// [`found_in`] cut to `max` characters for a grid cell, keeping the end:
/// the folder a result is *in* says more than the one the search started
/// from, which every result shares. Whole folder names are dropped from
/// the front behind an ellipsis — `…/files/src` — and only a last name
/// too long on its own is cut mid-name.
///
/// Counted in characters, not measured: iced 0.14 has no text ellipsis
/// and the grid has no layout pass to ask, so the cell's width in
/// average characters is the honest estimate ([`super::density`]), and
/// the cell clips whatever overruns it.
pub(super) fn elide_folder(folder: &str, max: usize) -> String {
    if folder.chars().count() <= max {
        return folder.to_string();
    }
    let parts: Vec<&str> = folder.split('/').collect();
    let mut kept = String::new();
    for part in parts.iter().rev() {
        let candidate = if kept.is_empty() { (*part).to_string() } else { format!("{part}/{kept}") };
        // Room for the "…/" in front.
        if candidate.chars().count() + 2 > max {
            break;
        }
        kept = candidate;
    }
    if kept.is_empty() {
        let last = parts.last().copied().unwrap_or(folder);
        let tail: String = last.chars().rev().take(max.saturating_sub(1)).collect::<Vec<_>>().into_iter().rev().collect();
        return format!("\u{2026}{tail}");
    }
    format!("\u{2026}/{kept}")
}

/// The icon key a saved search's sidebar row asks the theme for — the
/// freedesktop name for exactly this.
pub(super) fn saved_search_icon() -> String {
    crate::icon::themed_key("folder-saved-search")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::browser::Mode;
    use crate::prefs::Prefs;
    use std::time::Duration;

    fn file(dir: &str, name: &str) -> Entry {
        crate::backend::mock::MockBackend::file(Path::new(dir), name, 1)
    }

    fn browser_at(dir: &str, entries: Vec<Entry>) -> Browser {
        let (mut browser, _) = Browser::new(Mode::App, Prefs::default(), PathBuf::from(dir), Vec::new());
        browser.update(Message::DirLoaded(PathBuf::from(dir), Ok(entries)));
        browser
    }

    fn type_text(browser: &mut Browser, text: &str) -> Outcome {
        browser.update(Message::SearchChanged(text.to_string()))
    }

    fn requests(outcome: &Outcome) -> Vec<Request> {
        match outcome {
            Outcome::Search(Ask::Run(r)) => vec![r.clone()],
            Outcome::Many(parts) => parts.iter().flat_map(requests).collect(),
            _ => Vec::new(),
        }
    }

    fn smart(outcome: &Outcome) -> Option<SmartChange> {
        match outcome {
            Outcome::Search(Ask::Smart(change)) => Some(change.clone()),
            Outcome::Many(parts) => parts.iter().find_map(smart),
            _ => None,
        }
    }

    fn stops(outcome: &Outcome) -> bool {
        match outcome {
            Outcome::Search(Ask::Stop) => true,
            Outcome::Many(parts) => parts.iter().any(stops),
            _ => false,
        }
    }

    fn names(browser: &Browser) -> Vec<String> {
        browser.rows().iter().map(|e| e.name.clone()).collect()
    }

    fn summary(end: End) -> Summary {
        Summary { end, folders: 3, found: 0, elapsed: Duration::from_secs(1), ..Summary::stopped() }
    }

    /// The brief's first rule: a bare word in this folder is the search
    /// it always was — the listing filters at once and nothing walks.
    #[test]
    fn typing_in_this_folder_filters_at_once_and_asks_for_no_walk() {
        let mut browser = browser_at("/d", vec![file("/d", "readme.md"), file("/d", "main.rs")]);
        let outcome = type_text(&mut browser, "read");
        assert!(requests(&outcome).is_empty());
        assert_eq!(names(&browser), ["readme.md"]);
    }

    #[test]
    fn a_finished_filter_becomes_a_chip_and_still_filters() {
        let mut browser = browser_at("/d", vec![file("/d", "readme.md"), file("/d", "main.rs")]);
        type_text(&mut browser, "ext:rs ");
        assert_eq!(browser.search_query(), "", "the field keeps only what is still being written");
        assert_eq!(browser.search.chips.len(), 1);
        assert_eq!(names(&browser), ["main.rs"]);
        browser.update(Message::Search(SearchMessage::RemoveChip(0)));
        assert_eq!(names(&browser).len(), 2, "and removing the chip lifts it");
    }

    #[test]
    fn choosing_subfolders_asks_for_a_walk_from_the_folder_in_view() {
        let mut browser = browser_at("/d", vec![]);
        type_text(&mut browser, "notes");
        let outcome = browser.update(Message::Search(SearchMessage::Scope(Scope::Below)));
        let asked = requests(&outcome);
        assert_eq!(asked.len(), 1);
        assert_eq!(asked[0].root, PathBuf::from("/d"));
        assert!(asked[0].skip.iter().any(|p| p.ends_with("Trash")), "never the Trash's storage");
    }

    /// Each keystroke is "look again": a new run number, and a batch for
    /// the old one lands nowhere.
    #[test]
    fn results_for_a_replaced_walk_are_dropped() {
        let mut browser = browser_at("/d", vec![]);
        browser.update(Message::Search(SearchMessage::Scope(Scope::Below)));
        let first = requests(&type_text(&mut browser, "n"))[0].run;
        let second = requests(&type_text(&mut browser, "no"))[0].run;
        assert_ne!(first, second);
        browser.update(Message::Search(SearchMessage::Found { run: first, entries: vec![file("/d/a", "nope")] }));
        assert!(browser.rows().is_empty(), "the old walk's answer is not shown");
        browser.update(Message::Search(SearchMessage::Found { run: second, entries: vec![file("/d/a", "notes")] }));
        assert_eq!(names(&browser), ["notes"]);
    }

    #[test]
    fn results_replace_the_listing_and_the_listing_comes_back_after() {
        let mut browser = browser_at("/d", vec![file("/d", "top.txt")]);
        browser.update(Message::Search(SearchMessage::Scope(Scope::Below)));
        let run = requests(&type_text(&mut browser, "deep"))[0].run;
        browser.update(Message::Search(SearchMessage::Found { run, entries: vec![file("/d/x", "deep.txt")] }));
        assert_eq!(names(&browser), ["deep.txt"]);
        let outcome = browser.perform(Action::ClearSearch);
        assert!(stops(&outcome), "Escape stops the walk");
        assert_eq!(names(&browser), ["top.txt"], "the folder is what shows again");
    }

    #[test]
    fn going_somewhere_else_stops_the_walk_and_forgets_the_scope() {
        let mut browser = browser_at("/d", vec![]);
        browser.update(Message::Search(SearchMessage::Scope(Scope::Below)));
        type_text(&mut browser, "x");
        let outcome = browser.update(Message::Navigate(PathBuf::from("/e")));
        assert!(stops(&outcome));
        assert!(outcome.navigates());
        assert_eq!(browser.search.scope, Scope::Folder);
        assert!(!browser.in_results());
    }

    /// Navigating with no walk running must stay a bare `ReadDir`, as
    /// every host and test already expects.
    #[test]
    fn a_navigation_with_no_walk_is_still_a_plain_read() {
        let mut browser = browser_at("/d", vec![]);
        assert_eq!(browser.update(Message::Navigate(PathBuf::from("/e"))), Outcome::ReadDir(PathBuf::from("/e")));
    }

    #[test]
    fn the_trash_offers_no_walk() {
        let trash = crate::sidebar::trash_path();
        let (mut browser, _) = Browser::new(Mode::App, Prefs::default(), trash.clone(), Vec::new());
        browser.update(Message::DirLoaded(trash, Ok(vec![])));
        type_text(&mut browser, "x");
        let outcome = browser.update(Message::Search(SearchMessage::Scope(Scope::Below)));
        assert!(requests(&outcome).is_empty());
        assert!(!browser.search_model().walks);
    }

    /// The selection is keyed by path, so a result from three folders
    /// down is trashed as itself — not as a file of the same name here.
    #[test]
    fn acting_on_a_result_acts_on_the_result_s_own_path() {
        let mut browser = browser_at("/d", vec![file("/d", "same.txt")]);
        browser.update(Message::Search(SearchMessage::Scope(Scope::Below)));
        let run = requests(&type_text(&mut browser, "same"))[0].run;
        browser.update(Message::Search(SearchMessage::Found { run, entries: vec![file("/d/a/b", "same.txt")] }));
        browser.update(Message::EntryClicked { index: 0, ctrl: false, shift: false });
        assert_eq!(browser.perform(Action::Trash), Outcome::Trash(vec![PathBuf::from("/d/a/b/same.txt")]));
    }

    /// Found by review of the rename path: a result is renamed where it
    /// lives, never moved into the folder the search started from.
    #[test]
    fn renaming_a_result_keeps_it_in_its_own_folder() {
        let mut browser = browser_at("/d", vec![]);
        browser.update(Message::Search(SearchMessage::Scope(Scope::Below)));
        let run = requests(&type_text(&mut browser, "old"))[0].run;
        browser.update(Message::Search(SearchMessage::Found { run, entries: vec![file("/d/sub", "old.txt")] }));
        browser.update(Message::EntryClicked { index: 0, ctrl: false, shift: false });
        browser.perform(Action::Rename);
        browser.update(Message::RenameInput("new.txt".into()));
        assert_eq!(
            browser.update(Message::RenameCommit),
            Outcome::Rename { from: "/d/sub/old.txt".into(), to: "/d/sub/new.txt".into() }
        );
    }

    #[test]
    fn show_in_folder_goes_to_the_result_s_folder_and_selects_it() {
        let mut browser = browser_at("/d", vec![]);
        browser.update(Message::Search(SearchMessage::Scope(Scope::Below)));
        let run = requests(&type_text(&mut browser, "deep"))[0].run;
        browser.update(Message::Search(SearchMessage::Found { run, entries: vec![file("/d/x", "deep.txt")] }));
        browser.update(Message::EntryClicked { index: 0, ctrl: false, shift: false });
        let outcome = browser.perform(Action::ShowInFolder);
        assert!(matches!(&outcome, Outcome::Many(p) if p.contains(&Outcome::ReadDir("/d/x".into()))), "{outcome:?}");
        browser.update(Message::DirLoaded("/d/x".into(), Ok(vec![file("/d/x", "a.txt"), file("/d/x", "deep.txt")])));
        assert_eq!(browser.selection().focused(), Some(Path::new("/d/x/deep.txt")));
    }

    #[test]
    fn show_in_folder_is_offered_only_among_results() {
        let mut browser = browser_at("/d", vec![file("/d", "a.txt")]);
        browser.update(Message::EntryClicked { index: 0, ctrl: false, shift: false });
        assert_eq!(browser.perform(Action::ShowInFolder), Outcome::None);
        let items = browser.with_result_items(Vec::new());
        assert!(items.is_empty(), "not added to a folder's own menu");
    }

    #[test]
    fn a_refresh_runs_the_walk_on_screen_again() {
        let mut browser = browser_at("/d", vec![]);
        browser.update(Message::Search(SearchMessage::Scope(Scope::Below)));
        type_text(&mut browser, "x");
        let outcome = browser.update(Message::DirLoaded("/d".into(), Ok(vec![])));
        assert_eq!(requests(&outcome).len(), 1, "files may have moved; the results are asked for again");
    }

    #[test]
    fn stop_says_so_at_once_and_asks_the_host() {
        let mut browser = browser_at("/d", vec![]);
        browser.update(Message::Search(SearchMessage::Scope(Scope::Below)));
        type_text(&mut browser, "x");
        assert!(browser.search_model().running);
        assert!(stops(&browser.update(Message::Search(SearchMessage::Stop))));
        let model = browser.search_model();
        assert!(!model.running);
        assert!(model.status.unwrap().starts_with("Stopped"));
    }

    /// Home is home wherever you are — and, with no usable `$HOME`, the
    /// folder in view rather than nowhere.
    #[test]
    fn home_walks_from_home_wherever_you_are() {
        let home = super::super::home_dir().unwrap_or_else(|| PathBuf::from("/somewhere"));
        let mut browser = browser_at("/somewhere", vec![]);
        type_text(&mut browser, "x");
        let outcome = browser.update(Message::Search(SearchMessage::Scope(Scope::Home)));
        assert_eq!(requests(&outcome)[0].root, home);
    }

    #[test]
    fn an_unknown_key_is_said_in_the_rail() {
        let mut browser = browser_at("/d", vec![]);
        type_text(&mut browser, "colour:red");
        let model = browser.search_model();
        assert_eq!(model.problems.len(), 1);
        assert!(model.problems[0].contains("colour:"));
    }

    // --- saved searches --------------------------------------------------

    fn save(browser: &mut Browser, name: &str) -> Outcome {
        browser.update(Message::Search(SearchMessage::SaveStart));
        browser.update(Message::Search(SearchMessage::SaveName(name.into())));
        browser.update(Message::Search(SearchMessage::SaveCommit))
    }

    #[test]
    fn saving_records_the_query_the_folder_and_the_reach() {
        let mut browser = browser_at("/d", vec![]);
        type_text(&mut browser, "ext:rs main");
        browser.update(Message::Search(SearchMessage::Scope(Scope::Below)));
        let outcome = save(&mut browser, "Rust mains");
        let Some(SmartChange::Save(saved)) = smart(&outcome) else { panic!("{outcome:?}") };
        assert_eq!(saved.query, "ext:rs main");
        assert_eq!(saved.folder, PathBuf::from("/d"));
        assert!(saved.subfolders);
        assert_eq!(browser.search_model().saved_as, Some("Rust mains"), "the rail says it is saved");
    }

    #[test]
    fn a_blank_name_saves_nothing() {
        let mut browser = browser_at("/d", vec![]);
        type_text(&mut browser, "x");
        assert_eq!(save(&mut browser, "   "), Outcome::None);
    }

    #[test]
    fn the_suggested_name_says_what_and_where() {
        let mut browser = browser_at("/home/a/crates", vec![]);
        type_text(&mut browser, "ext:rs ");
        browser.update(Message::Search(SearchMessage::SaveStart));
        assert_eq!(browser.search.naming.as_ref().unwrap().text, "ext:rs in crates");
    }

    /// Opening a saved search goes to its folder, puts its filters back
    /// as chips, and starts its walk — all from one click.
    #[test]
    fn opening_a_saved_search_goes_there_and_runs_it() {
        let mut browser = browser_at("/d", vec![]);
        browser.set_smart_folders(vec![SmartFolder {
            name: "Big videos".into(),
            query: "kind:video size:>1G holiday".into(),
            folder: "/media".into(),
            subfolders: true,
        }]);
        let outcome = browser.update(Message::Search(SearchMessage::Open(0)));
        assert!(outcome.navigates());
        let asked = requests(&outcome);
        assert_eq!(asked.len(), 1);
        assert_eq!(asked[0].root, PathBuf::from("/media"));
        assert_eq!(browser.search.chips.len(), 2);
        assert_eq!(browser.search_query(), "holiday");
        assert_eq!(browser.search_model().current, Some(0), "its sidebar row is the current one");
        // The listing arriving for the folder does not start it twice.
        let outcome = browser.update(Message::DirLoaded("/media".into(), Ok(vec![])));
        assert!(requests(&outcome).is_empty());
    }

    #[test]
    fn forgetting_the_saved_search_on_screen_removes_it() {
        let mut browser = browser_at("/d", vec![]);
        type_text(&mut browser, "x");
        save(&mut browser, "X");
        let outcome = browser.update(Message::Search(SearchMessage::Forget));
        assert_eq!(smart(&outcome), Some(SmartChange::Forget("X".into())));
        assert!(browser.search_model().saved_as.is_none());
    }

    // --- what the rail says ------------------------------------------------

    #[test]
    fn the_status_line_names_each_ending() {
        assert_eq!(status_line(12, None), "Searching\u{2026} 12 found");
        assert_eq!(status_line(1, Some(&summary(End::Complete))), "1 match in 3 folders");
        assert!(status_line(2, Some(&summary(End::Stopped))).starts_with("Stopped"));
        assert!(status_line(2, Some(&summary(End::TimeLimit))).contains("after 1s"));
        assert!(status_line(2, Some(&summary(End::FolderLimit))).contains("smaller folder"));
        assert!(status_line(5000, Some(&summary(End::FoundLimit))).starts_with("Showing the first 5,000"));
    }

    /// Never quietly incomplete: what was not searched is said.
    #[test]
    fn the_status_line_counts_what_was_not_searched() {
        let mut ended = summary(End::Complete);
        ended.unreadable = 2;
        ended.elsewhere = 1;
        let line = status_line(4, Some(&ended));
        assert!(line.contains("2 folders couldn't be read"), "{line}");
        assert!(line.contains("1 folder on other drives not searched"), "{line}");
    }

    #[test]
    fn a_result_s_folder_is_written_from_where_the_search_started() {
        let mut deep = file("/home/a/crates/src", "x.rs");
        deep.origin = Some("/home/a/crates/src".into());
        assert_eq!(found_in(&deep, Path::new("/home/a/crates")), "crates/src");
        let mut here = file("/home/a/crates", "y.rs");
        here.origin = Some("/home/a/crates".into());
        assert_eq!(found_in(&here, Path::new("/home/a/crates")), "crates", "never an empty cell");
    }

    // --- content ----------------------------------------------------------

    /// The listing cannot answer `content:`, so even "This folder" asks
    /// the host — for the folder alone, one level deep.
    #[test]
    fn a_content_filter_in_this_folder_walks_one_level() {
        let mut browser = browser_at("/d", vec![file("/d", "a.txt")]);
        let outcome = type_text(&mut browser, "content:todo ");
        let asked = requests(&outcome);
        assert_eq!(asked.len(), 1);
        assert_eq!(asked[0].budget.depth, 1);
        assert_eq!(asked[0].root, PathBuf::from("/d"));
        assert!(browser.rows().is_empty(), "nothing is shown as a match before it is read");
        let outcome = browser.update(Message::Search(SearchMessage::RemoveChip(0)));
        assert!(stops(&outcome), "and taking the chip away goes back to the listing");
        assert_eq!(names(&browser), ["a.txt"]);
    }

    #[test]
    fn a_content_filter_below_walks_the_usual_depth() {
        let mut browser = browser_at("/d", vec![]);
        type_text(&mut browser, "content:todo ");
        let outcome = browser.update(Message::Search(SearchMessage::Scope(Scope::Below)));
        assert_eq!(requests(&outcome)[0].budget.depth, Budget::default().depth);
    }

    #[test]
    fn the_trash_says_it_cannot_search_contents() {
        let trash = crate::sidebar::trash_path();
        let (mut browser, _) = Browser::new(Mode::App, Prefs::default(), trash.clone(), Vec::new());
        browser.update(Message::DirLoaded(trash, Ok(vec![])));
        let outcome = type_text(&mut browser, "content:x ");
        assert!(requests(&outcome).is_empty());
        assert!(browser.search_model().problems.iter().any(|p| p.contains("Trash")));
    }

    /// Every file a content search did not read is counted with why.
    #[test]
    fn the_status_line_says_what_a_content_search_skipped_and_why() {
        let mut ended = summary(End::Complete);
        ended.contents = ContentReport { read: 1_200, bytes: 9_000, too_large: 12, binary: 340, ..ContentReport::default() };
        let line = status_line(3, Some(&ended));
        assert!(line.contains("1,200 files read"), "{line}");
        assert!(line.contains("not read: 12 too large, 340 binary"), "{line}");
        assert!(!line.contains("unreadable"), "{line}");
        ended.contents = ContentReport { archives: 1, links: 2, ..ContentReport::default() };
        let line = status_line(3, Some(&ended));
        assert!(line.contains("not read: 1 archive, 2 links"), "{line}");
        assert!(!status_line(3, Some(&summary(End::Complete))).contains("read"), "nothing about reading without content:");
        let mut spent = summary(End::ReadLimit);
        spent.contents.bytes = 1 << 30;
        assert!(status_line(3, Some(&spent)).starts_with("Stopped after reading 1.0 GiB"), "{}", status_line(3, Some(&spent)));
    }

    // --- scope from the keyboard ------------------------------------------

    #[test]
    fn the_scope_key_goes_round_the_rail_s_buttons_in_order() {
        let mut browser = browser_at("/somewhere/else", vec![]);
        type_text(&mut browser, "x");
        let outcome = browser.perform(Action::NextSearchScope);
        assert_eq!(browser.search.scope, Scope::Below);
        assert_eq!(requests(&outcome).len(), 1, "a new scope searches at once");
        browser.perform(Action::NextSearchScope);
        let expected = if super::super::home_dir().is_some() { Scope::Home } else { Scope::Folder };
        assert_eq!(browser.search.scope, expected);
        browser.perform(Action::NextSearchScope);
        if expected == Scope::Home {
            assert_eq!(browser.search.scope, Scope::Folder, "and round again");
        }
    }

    #[test]
    fn the_scope_key_does_nothing_without_a_search_or_in_the_trash() {
        let mut browser = browser_at("/d", vec![]);
        assert_eq!(browser.perform(Action::NextSearchScope), Outcome::None);
        assert_eq!(browser.search.scope, Scope::Folder, "no search, no hidden scope change");
        let trash = crate::sidebar::trash_path();
        let (mut browser, _) = Browser::new(Mode::App, Prefs::default(), trash.clone(), Vec::new());
        browser.update(Message::DirLoaded(trash, Ok(vec![])));
        type_text(&mut browser, "x");
        assert_eq!(browser.perform(Action::NextSearchScope), Outcome::None);
        assert_eq!(browser.search.scope, Scope::Folder);
    }

    // --- the grid's folder line --------------------------------------------

    #[test]
    fn a_short_folder_is_left_whole() {
        assert_eq!(elide_folder("crates/src", 15), "crates/src");
    }

    /// The end is kept: where a result is matters more than where the
    /// search began, which every result shares.
    #[test]
    fn a_long_folder_keeps_whole_names_from_the_end() {
        assert_eq!(elide_folder("hyprforge/crates/hyprforge-files/src", 15), "\u{2026}/src");
        assert_eq!(elide_folder("proj/crates/files/src", 15), "\u{2026}/files/src");
        let cut = elide_folder("proj/an-extremely-long-folder-name", 15);
        assert_eq!(cut.chars().count(), 15);
        assert!(cut.starts_with('\u{2026}') && cut.ends_with("folder-name"), "{cut}");
    }

    #[test]
    fn counts_are_grouped_by_thousands() {
        assert_eq!(grouped(0), "0");
        assert_eq!(grouped(999), "999");
        assert_eq!(grouped(17_865), "17,865");
        assert_eq!(grouped(1_175_818), "1,175,818");
    }
}
