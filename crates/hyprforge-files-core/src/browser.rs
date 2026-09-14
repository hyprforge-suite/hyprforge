//! The browser view: the one thing the app window and the portal's
//! open/save dialog both render.
//!
//! [`Browser`] owns browsing state and knows how to update itself and
//! draw itself, in iced's `update`/`view` idiom — but it never touches a
//! filesystem. Every directory read goes out through [`Outcome::ReadDir`]
//! for a host to perform off the UI thread (through [`crate::backend::FsBackend`],
//! synchronously, on whatever thread the host chooses) and comes back in
//! through [`Message::DirLoaded`]. That split is what makes `Browser`
//! itself generic over nothing and dependent on nothing but `iced`,
//! `hyprforge-ui` and `hyprforge-look` — no `FsBackend` type parameter,
//! because it never calls one.
//!
//! # The `Mode` seam
//!
//! [`Mode`] exists so a host can tell `Browser` which role it's playing,
//! and so that a click that lands on a file becomes [`Outcome::Activated`]
//! for the host to interpret — the app opens the file, the dialog treats
//! it as the chosen one. `view` never reads `Mode` to decide what to
//! draw: see [`ViewModel`], the struct `render` actually takes, which has
//! no `Mode` field at all — not "doesn't currently use one", structurally
//! cannot have one without every call site changing. [`tests::view_never_branches_on_mode`]
//! pins the property that matters: two browsers differing only in `Mode`
//! feed `render` the exact same data.

use crate::filter::{filter_hidden, filter_query};
use crate::format::human_readable_size;
use crate::icon::entry_icon;
use crate::keymap::{self, Direction, Key, KeyAction, Modifiers};
use crate::prefs::Prefs;
use crate::sidebar::SidebarItem;
use crate::sort::{sort, SortColumn, SortDirection};
use crate::types::{Entry, EntryKind, FilesError};
use hyprforge_ui::theme::{spacing, FontScale, BASE_TEXT_SIZE};
use hyprforge_ui::widgets::{divider, meta_text, primary_button, scaled_text, secondary_button};
use iced::widget::{column, container, row, scrollable, text_input};
use iced::{Element, Length};
use std::collections::HashSet;
use std::path::{Path, PathBuf};

/// Which host is rendering this [`Browser`], and — for a dialog — what
/// it's collecting a path for.
///
/// See the module doc: this changes what an [`Outcome::Activated`] means
/// to the *host*, never what `view` draws.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    App,
    Dialog(DialogKind),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DialogKind {
    Open,
    Save,
}

/// A directory-read failure, reduced to what the view needs: an
/// actionable sentence (pillar 3 — never a blank list standing in for a
/// reason) and which kind it was, so "this folder is empty" and "you
/// don't have permission to open this folder" render visibly
/// differently. Cloneable and comparable, unlike [`FilesError`] itself
/// (which carries a `std::io::Error`), so it can ride in [`Message`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DirError {
    pub message: String,
    pub kind: DirErrorKind,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DirErrorKind {
    PermissionDenied,
    NotFound,
    NotADirectory,
    Vanished,
    Other,
}

impl From<&FilesError> for DirError {
    fn from(e: &FilesError) -> Self {
        let kind = match e {
            FilesError::PermissionDenied { .. } => DirErrorKind::PermissionDenied,
            FilesError::NotFound { .. } => DirErrorKind::NotFound,
            FilesError::NotADirectory { .. } => DirErrorKind::NotADirectory,
            FilesError::VanishedMidRead { .. } => DirErrorKind::Vanished,
            FilesError::Io { .. } => DirErrorKind::Other,
        };
        DirError { message: e.to_string(), kind }
    }
}

impl From<FilesError> for DirError {
    fn from(e: FilesError) -> Self {
        DirError::from(&e)
    }
}

/// Where a directory read currently stands.
///
/// `Loading` and `Loaded` with zero entries are deliberately different
/// states from `Error` — the rule CLAUDE.md draws from `hlconfig::storage`,
/// applied here: a permission failure must never render like an empty,
/// readable directory.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LoadState {
    Loading,
    Loaded,
    Error(DirError),
}

/// Multi-selection over the currently shown (sorted/filtered) entry
/// list, indexed by position in that list.
///
/// `focused` is the keyboard cursor — what Enter activates, what an
/// arrow key moves — and is not always the same as "the only selected
/// item": a shift-range click leaves every row in the range selected but
/// `focused` pinned to the row that was actually clicked.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Selection {
    anchor: Option<usize>,
    focused: Option<usize>,
    selected: HashSet<usize>,
}

impl Selection {
    pub fn is_selected(&self, index: usize) -> bool {
        self.selected.contains(&index)
    }

    pub fn is_focused(&self, index: usize) -> bool {
        self.focused == Some(index)
    }

    pub fn focused(&self) -> Option<usize> {
        self.focused
    }

    pub fn selected_indices(&self) -> &HashSet<usize> {
        &self.selected
    }

    /// A plain click: replaces the selection with just `index` and moves
    /// the shift-range anchor there.
    fn click(&mut self, index: usize) {
        self.selected.clear();
        self.selected.insert(index);
        self.anchor = Some(index);
        self.focused = Some(index);
    }

    /// A ctrl-click: toggles `index` in the selection without disturbing
    /// the rest, and becomes the new anchor for a following shift-click —
    /// the behaviour every mainstream file manager uses.
    fn ctrl_click(&mut self, index: usize) {
        if !self.selected.insert(index) {
            self.selected.remove(&index);
        }
        self.anchor = Some(index);
        self.focused = Some(index);
    }

    /// A shift-click: selects the closed range between the anchor and
    /// `index`, in either direction — `index` before the anchor selects
    /// backwards just as well as after it, since the range is built from
    /// `min..=max`, not from counting forward off the anchor.
    fn shift_click(&mut self, index: usize) {
        let anchor = self.anchor.unwrap_or(index);
        let (lo, hi) = if anchor <= index { (anchor, index) } else { (index, anchor) };
        self.selected = (lo..=hi).collect();
        self.focused = Some(index);
        // Anchor is deliberately left where it was: a second shift-click
        // extends/contracts from the same starting point, not from the
        // row the previous shift-click landed on.
    }

    /// Applies a click with the given modifiers. Ctrl wins over shift if
    /// somehow both are held, since a ctrl-click's "toggle just this one"
    /// is the more specific request.
    pub fn click_with(&mut self, index: usize, ctrl: bool, shift: bool) {
        if ctrl {
            self.ctrl_click(index);
        } else if shift {
            self.shift_click(index);
        } else {
            self.click(index);
        }
    }

    /// Moves the keyboard focus by one row and collapses the selection
    /// to just the new focus — arrow-key navigation, which replaces a
    /// multi-select rather than extending it (shift+arrow range-select is
    /// not in this pass's brief).
    fn move_focus(&mut self, delta: i32, len: usize) {
        if len == 0 {
            self.focused = None;
            self.selected.clear();
            return;
        }
        let current = self.focused.unwrap_or(0) as i32;
        let next = (current + delta).clamp(0, len as i32 - 1) as usize;
        self.click(next);
    }

    fn clear(&mut self) {
        *self = Selection::default();
    }
}

/// Everything a host must do in response to [`Browser::update`] or
/// [`Browser::handle_key`] — the outcomes `Browser` cannot carry out
/// itself because each one is either blocking I/O or a decision that
/// belongs to the host (what "activating" a file means; where
/// preferences get persisted).
#[derive(Debug, Clone, PartialEq)]
pub enum Outcome {
    /// Nothing for the host to do.
    None,
    /// Read this directory (through [`crate::backend::FsBackend::read_dir`],
    /// off the UI thread) and feed the result back as
    /// [`Message::DirLoaded`].
    ReadDir(PathBuf),
    /// A file (never a directory — see [`Browser::activate`]) was
    /// activated. The app opens it; the dialog treats it as the chosen
    /// path. `Browser` does not know or care which.
    Activated(PathBuf),
    /// A preference changed and should be persisted — via
    /// `crate::prefs::update`, off the UI thread, the same as any other
    /// file write.
    PrefsChanged(Prefs),
}

#[derive(Debug, Clone, PartialEq)]
pub enum Message {
    Navigate(PathBuf),
    DirLoaded(PathBuf, Result<Vec<Entry>, DirError>),
    GoBack,
    GoForward,
    GoUp,
    EntryClicked { index: usize, ctrl: bool, shift: bool },
    EntryActivated(usize),
    SearchChanged(String),
    SearchCleared,
    SortBy(SortColumn),
    ToggleSortDirection,
    ToggleDirectoriesFirst,
    ToggleShowHidden,
    SetViewMode(crate::prefs::ViewMode),
    Key(Key, Modifiers),
}

/// Exactly what `render` needs, and nothing `Browser` has that it
/// doesn't — see the module doc's note on the `Mode` seam. This has no
/// `Mode` field; that omission is the enforcement mechanism, not a
/// convention someone has to remember.
#[derive(Debug, Clone, PartialEq)]
struct ViewModel<'a> {
    current_dir: &'a Path,
    view_entries: &'a [Entry],
    selection: &'a Selection,
    search_query: &'a str,
    load_state: &'a LoadState,
    prefs: &'a Prefs,
    sidebar: &'a [SidebarItem],
    can_go_back: bool,
    can_go_forward: bool,
}

/// Browsing state for one directory tree view. Owns no filesystem
/// handle — see the module doc.
#[derive(Debug, Clone)]
pub struct Browser {
    mode: Mode,
    current_dir: PathBuf,
    /// The raw listing for `current_dir`, as last reported by
    /// [`Message::DirLoaded`] — unsorted, unfiltered.
    entries: Vec<Entry>,
    /// `entries`, sorted per `prefs` and filtered by `search_query` and
    /// `prefs.show_hidden`. Recomputed by [`Self::refresh_view`]
    /// whenever any input to it changes, so `view` never has to.
    view_entries: Vec<Entry>,
    selection: Selection,
    search_query: String,
    back_stack: Vec<PathBuf>,
    forward_stack: Vec<PathBuf>,
    load_state: LoadState,
    prefs: Prefs,
    sidebar: Vec<SidebarItem>,
}

impl Browser {
    /// Builds a browser starting at `start_dir`, with a sidebar the host
    /// already built (via [`crate::sidebar::build`], off the UI thread —
    /// see that function's own doc). Returns the [`Outcome::ReadDir`]
    /// the host must fulfil to show anything at all.
    pub fn new(mode: Mode, prefs: Prefs, start_dir: PathBuf, sidebar: Vec<SidebarItem>) -> (Browser, Outcome) {
        let browser = Browser {
            mode,
            current_dir: start_dir.clone(),
            entries: Vec::new(),
            view_entries: Vec::new(),
            selection: Selection::default(),
            search_query: String::new(),
            back_stack: Vec::new(),
            forward_stack: Vec::new(),
            load_state: LoadState::Loading,
            prefs,
            sidebar,
        };
        (browser, Outcome::ReadDir(start_dir))
    }

    pub fn mode(&self) -> Mode {
        self.mode
    }

    pub fn current_dir(&self) -> &Path {
        &self.current_dir
    }

    pub fn prefs(&self) -> &Prefs {
        &self.prefs
    }

    pub fn selection(&self) -> &Selection {
        &self.selection
    }

    /// The currently shown entries, sorted and filtered — what `view`
    /// draws and what [`Selection`]'s indices refer to.
    pub fn view_entries(&self) -> &[Entry] {
        &self.view_entries
    }

    pub fn update(&mut self, message: Message) -> Outcome {
        match message {
            Message::Navigate(path) => self.go_to(path, true),
            Message::DirLoaded(path, result) => self.apply_dir_loaded(path, result),
            Message::GoBack => self.go_back(),
            Message::GoForward => self.go_forward(),
            Message::GoUp => self.go_up(),
            Message::EntryClicked { index, ctrl, shift } => {
                if index < self.view_entries.len() {
                    self.selection.click_with(index, ctrl, shift);
                }
                Outcome::None
            }
            Message::EntryActivated(index) => self.activate(index),
            Message::SearchChanged(query) => {
                self.search_query = query;
                self.refresh_view();
                Outcome::None
            }
            Message::SearchCleared => {
                self.search_query.clear();
                self.refresh_view();
                Outcome::None
            }
            Message::SortBy(column) => {
                if self.prefs.sort_column() == column {
                    self.prefs.set_sort_direction(match self.prefs.sort_direction() {
                        SortDirection::Ascending => SortDirection::Descending,
                        SortDirection::Descending => SortDirection::Ascending,
                    });
                } else {
                    self.prefs.set_sort_column(column);
                }
                self.refresh_view();
                Outcome::PrefsChanged(self.prefs.clone())
            }
            Message::ToggleSortDirection => {
                self.prefs.set_sort_direction(match self.prefs.sort_direction() {
                    SortDirection::Ascending => SortDirection::Descending,
                    SortDirection::Descending => SortDirection::Ascending,
                });
                self.refresh_view();
                Outcome::PrefsChanged(self.prefs.clone())
            }
            Message::ToggleDirectoriesFirst => {
                self.prefs.directories_first = !self.prefs.directories_first;
                self.refresh_view();
                Outcome::PrefsChanged(self.prefs.clone())
            }
            Message::ToggleShowHidden => {
                self.prefs.show_hidden = !self.prefs.show_hidden;
                self.refresh_view();
                Outcome::PrefsChanged(self.prefs.clone())
            }
            Message::SetViewMode(mode) => {
                self.prefs.view_mode = mode;
                Outcome::PrefsChanged(self.prefs.clone())
            }
            Message::Key(key, mods) => self.handle_key(key, mods),
        }
    }

    /// Resolves one key press through [`crate::keymap`] and applies it.
    /// A host wires this to iced's keyboard subscription; see the
    /// `keymap` module doc for why the grammar lives there instead of
    /// being reimplemented per host.
    pub fn handle_key(&mut self, key: Key, mods: Modifiers) -> Outcome {
        match keymap::resolve(key, mods) {
            Some(KeyAction::Activate) => match self.selection.focused() {
                Some(index) => self.activate(index),
                None => Outcome::None,
            },
            Some(KeyAction::GoUp) => self.go_up(),
            Some(KeyAction::GoBack) => self.go_back(),
            Some(KeyAction::Move(direction)) => {
                let delta = match direction {
                    Direction::Up | Direction::Left => -1,
                    Direction::Down | Direction::Right => 1,
                };
                self.selection.move_focus(delta, self.view_entries.len());
                Outcome::None
            }
            Some(KeyAction::TypeToSearch(c)) => {
                self.search_query.push(c);
                self.refresh_view();
                Outcome::None
            }
            Some(KeyAction::ClearSearch) => {
                self.search_query.clear();
                self.refresh_view();
                Outcome::None
            }
            None => Outcome::None,
        }
    }

    fn apply_dir_loaded(&mut self, path: PathBuf, result: Result<Vec<Entry>, DirError>) -> Outcome {
        // A slow read for a directory the user has since navigated away
        // from must not clobber what's on screen now — guard on the path
        // the result is actually for.
        if path != self.current_dir {
            return Outcome::None;
        }
        match result {
            Ok(entries) => {
                self.entries = entries;
                self.load_state = LoadState::Loaded;
            }
            Err(e) => {
                self.entries.clear();
                self.load_state = LoadState::Error(e);
            }
        }
        self.selection.clear();
        self.refresh_view();
        Outcome::None
    }

    fn refresh_view(&mut self) {
        let mut entries = filter_query(
            filter_hidden(self.entries.clone(), self.prefs.show_hidden),
            &self.search_query,
        );
        sort(
            &mut entries,
            self.prefs.sort_column(),
            self.prefs.sort_direction(),
            self.prefs.directories_first,
        );
        self.view_entries = entries;
    }

    fn go_to(&mut self, path: PathBuf, record_history: bool) -> Outcome {
        if record_history {
            self.back_stack.push(self.current_dir.clone());
            // A fresh navigation abandons whatever "forward" pointed to —
            // pinned by `tests::navigating_somewhere_new_truncates_the_forward_stack`.
            self.forward_stack.clear();
        }
        self.current_dir = path.clone();
        self.selection.clear();
        self.search_query.clear();
        self.view_entries.clear();
        self.load_state = LoadState::Loading;
        Outcome::ReadDir(path)
    }

    fn go_up(&mut self) -> Outcome {
        match self.current_dir.parent().map(|p| p.to_path_buf()) {
            Some(parent) => self.go_to(parent, true),
            None => Outcome::None,
        }
    }

    fn go_back(&mut self) -> Outcome {
        let Some(prev) = self.back_stack.pop() else {
            return Outcome::None;
        };
        self.forward_stack.push(self.current_dir.clone());
        self.current_dir = prev.clone();
        self.selection.clear();
        self.search_query.clear();
        self.view_entries.clear();
        self.load_state = LoadState::Loading;
        Outcome::ReadDir(prev)
    }

    fn go_forward(&mut self) -> Outcome {
        let Some(next) = self.forward_stack.pop() else {
            return Outcome::None;
        };
        self.back_stack.push(self.current_dir.clone());
        self.current_dir = next.clone();
        self.selection.clear();
        self.search_query.clear();
        self.view_entries.clear();
        self.load_state = LoadState::Loading;
        Outcome::ReadDir(next)
    }

    /// A folder navigates into itself; a file becomes [`Outcome::Activated`]
    /// for the host to interpret — the one place `Mode` matters, and it
    /// matters to the *host*, not to this function or to `view`.
    fn activate(&mut self, index: usize) -> Outcome {
        let Some(entry) = self.view_entries.get(index) else {
            return Outcome::None;
        };
        if entry.is_dir {
            self.go_to(entry.path.clone(), true)
        } else {
            Outcome::Activated(entry.path.clone())
        }
    }

    fn view_model(&self) -> ViewModel<'_> {
        ViewModel {
            current_dir: &self.current_dir,
            view_entries: &self.view_entries,
            selection: &self.selection,
            search_query: &self.search_query,
            load_state: &self.load_state,
            prefs: &self.prefs,
            sidebar: &self.sidebar,
            can_go_back: !self.back_stack.is_empty(),
            can_go_forward: !self.forward_stack.is_empty(),
        }
    }

    pub fn view(&self, scale: FontScale) -> Element<'_, Message> {
        render(self.view_model(), scale)
    }
}

/// The ancestors of `path`, each paired with the full path clicking it
/// navigates to — root first. Pure and free of iced, so the path bar's
/// own logic is testable without building a window.
fn breadcrumb(path: &Path) -> Vec<(String, PathBuf)> {
    let mut result = Vec::new();
    let mut accum = PathBuf::new();
    for component in path.components() {
        accum.push(component);
        let label = match component {
            std::path::Component::RootDir => "/".to_string(),
            std::path::Component::Normal(s) => s.to_string_lossy().into_owned(),
            std::path::Component::CurDir
            | std::path::Component::ParentDir
            | std::path::Component::Prefix(_) => continue,
        };
        result.push((label, accum.clone()));
    }
    result
}

fn kind_label(kind: EntryKind) -> &'static str {
    match kind {
        EntryKind::Folder => "Folder",
        EntryKind::Image => "Image",
        EntryKind::Document => "Document",
        EntryKind::Archive => "Archive",
        EntryKind::Code => "Code",
        EntryKind::Audio => "Audio",
        EntryKind::Video => "Video",
        EntryKind::Other => "File",
    }
}

/// Renders `vm`. A free function taking [`ViewModel`] rather than a
/// method on [`Browser`] — see the module doc for why that's the point.
fn render<'a>(vm: ViewModel<'a>, scale: FontScale) -> Element<'a, Message> {
    let sidebar = sidebar_view(&vm, scale);
    let toolbar = row![
        secondary_button("\u{2190}").on_press_maybe(vm.can_go_back.then_some(Message::GoBack)),
        secondary_button("\u{2192}").on_press_maybe(vm.can_go_forward.then_some(Message::GoForward)),
        secondary_button("\u{2191}").on_press(Message::GoUp),
        path_bar(vm.current_dir, scale),
    ]
    .spacing(spacing::SM)
    .align_y(iced::Alignment::Center);

    let search = text_input("Search this folder", vm.search_query)
        .on_input(Message::SearchChanged)
        .width(Length::Fill);

    let body = body_view(&vm, scale);

    let content = column![toolbar, search, body]
        .spacing(spacing::MD)
        .padding(spacing::MD)
        .width(Length::Fill)
        .height(Length::Fill);

    row![sidebar, content].into()
}

fn sidebar_view<'a>(vm: &ViewModel<'a>, scale: FontScale) -> Element<'a, Message> {
    let mut list = column![].spacing(spacing::XS).width(Length::Fixed(180.0));
    for item in vm.sidebar {
        let is_current = item.path == vm.current_dir;
        let label = scaled_text(item.label.clone(), BASE_TEXT_SIZE, scale);
        let button = if is_current {
            primary_button(item.label.clone()).width(Length::Fill)
        } else {
            secondary_button(item.label.clone()).width(Length::Fill)
        };
        let _ = label; // the button already carries the label text
        list = list.push(button.on_press(Message::Navigate(item.path.clone())));
    }
    container(scrollable(list))
        .padding(spacing::SM)
        .width(Length::Fixed(200.0))
        .height(Length::Fill)
        .into()
}

fn path_bar<'a>(current_dir: &Path, scale: FontScale) -> Element<'a, Message> {
    let mut crumbs = row![].spacing(spacing::XS).align_y(iced::Alignment::Center);
    let segments = breadcrumb(current_dir);
    let last = segments.len().saturating_sub(1);
    for (i, (label, path)) in segments.into_iter().enumerate() {
        if i > 0 {
            crumbs = crumbs.push(meta_text("/", BASE_TEXT_SIZE, scale));
        }
        if i == last {
            crumbs = crumbs.push(scaled_text(label, BASE_TEXT_SIZE, scale));
        } else {
            crumbs = crumbs.push(secondary_button(label).on_press(Message::Navigate(path)));
        }
    }
    container(crumbs).width(Length::Fill).into()
}

fn body_view<'a>(vm: &ViewModel<'a>, scale: FontScale) -> Element<'a, Message> {
    match vm.load_state {
        LoadState::Loading => container(meta_text("Loading\u{2026}", BASE_TEXT_SIZE, scale))
            .center_x(Length::Fill)
            .padding(spacing::LG)
            .into(),
        LoadState::Error(err) => container(
            column![
                scaled_text("Couldn't open this folder", 16.0, scale).color(hyprforge_ui::theme::warning()),
                meta_text(err.message.clone(), BASE_TEXT_SIZE, scale),
            ]
            .spacing(spacing::SM),
        )
        .padding(spacing::LG)
        .into(),
        LoadState::Loaded if vm.view_entries.is_empty() => {
            let message = if vm.search_query.is_empty() {
                "This folder is empty."
            } else {
                "No entries match your search."
            };
            container(meta_text(message, BASE_TEXT_SIZE, scale))
                .center_x(Length::Fill)
                .padding(spacing::LG)
                .into()
        }
        LoadState::Loaded => match vm.prefs.view_mode {
            crate::prefs::ViewMode::List => list_view(vm, scale),
            crate::prefs::ViewMode::Grid => grid_view(vm, scale),
        },
    }
}

fn entry_row<'a>(index: usize, entry: &'a Entry, selected: bool, scale: FontScale) -> Element<'a, Message> {
    let meta = if entry.is_dir {
        String::new()
    } else {
        human_readable_size(entry.size)
    };
    let row_content = row![
        entry_icon(entry.kind, 20.0, scale),
        scaled_text(entry.name.clone(), BASE_TEXT_SIZE, scale).width(Length::FillPortion(3)),
        meta_text(meta, 13.0, scale).width(Length::FillPortion(1)),
        meta_text(kind_label(entry.kind), 13.0, scale).width(Length::FillPortion(1)),
    ]
    .spacing(spacing::SM)
    .align_y(iced::Alignment::Center);

    // `secondary_button`/`primary_button` wrap a `text` fragment; this
    // row needs several columns of content in one clickable area, so it
    // goes straight to `iced::widget::button` (which either wraps) with
    // its own style function instead.
    let styled = iced::widget::button(row_content)
        .on_press(Message::EntryClicked { index, ctrl: false, shift: false })
        .width(Length::Fill)
        .style(move |t: &iced::Theme, status| entry_row_style(t, status, selected));
    styled.into()
}

fn entry_row_style(theme: &iced::Theme, status: iced::widget::button::Status, selected: bool) -> iced::widget::button::Style {
    use iced::widget::button;
    use iced::{Background, Border};
    let palette = theme.extended_palette();
    let background = if selected {
        Some(Background::Color(palette.primary.weak.color))
    } else {
        match status {
            button::Status::Hovered => Some(Background::Color(hyprforge_ui::theme::surface::row())),
            _ => None,
        }
    };
    button::Style {
        background,
        text_color: hyprforge_ui::theme::text(),
        border: Border { radius: 6.0.into(), ..Border::default() },
        ..button::Style::default()
    }
}

fn list_view<'a>(vm: &ViewModel<'a>, scale: FontScale) -> Element<'a, Message> {
    let mut list = column![].spacing(2.0);
    for (index, entry) in vm.view_entries.iter().enumerate() {
        if index > 0 {
            list = list.push(divider());
        }
        list = list.push(entry_row(index, entry, vm.selection.is_selected(index), scale));
    }
    scrollable(list).height(Length::Fill).into()
}

fn grid_view<'a>(vm: &ViewModel<'a>, scale: FontScale) -> Element<'a, Message> {
    const COLUMNS: usize = 5;
    let mut grid = column![].spacing(spacing::SM);
    let mut current = row![].spacing(spacing::SM);
    for (index, entry) in vm.view_entries.iter().enumerate() {
        if index > 0 && index % COLUMNS == 0 {
            grid = grid.push(current);
            current = row![].spacing(spacing::SM);
        }
        let selected = vm.selection.is_selected(index);
        let cell = column![
            entry_icon(entry.kind, 48.0, scale),
            scaled_text(entry.name.clone(), 12.0, scale),
        ]
        .spacing(spacing::XS)
        .width(Length::Fixed(96.0))
        .align_x(iced::Alignment::Center);
        let button = iced::widget::button(cell)
            .on_press(Message::EntryClicked { index, ctrl: false, shift: false })
            .style(move |t: &iced::Theme, status| entry_row_style(t, status, selected));
        current = current.push(button);
    }
    grid = grid.push(current);
    scrollable(grid).height(Length::Fill).into()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::SystemTime;

    fn entry(name: &str, is_dir: bool) -> Entry {
        Entry {
            name: name.to_string(),
            path: PathBuf::from("/dir").join(name),
            is_dir,
            size: 10,
            modified: Some(SystemTime::UNIX_EPOCH),
            is_symlink: false,
            link_broken: false,
            hidden: name.starts_with('.'),
            kind: EntryKind::classify(is_dir, name),
        }
    }

    fn loaded_browser(names: &[&str]) -> Browser {
        let (mut browser, outcome) = Browser::new(Mode::App, Prefs::default(), PathBuf::from("/dir"), vec![]);
        assert_eq!(outcome, Outcome::ReadDir(PathBuf::from("/dir")));
        let entries: Vec<Entry> = names.iter().map(|n| entry(n, false)).collect();
        browser.update(Message::DirLoaded(PathBuf::from("/dir"), Ok(entries)));
        browser
    }

    // --- selection: ctrl-toggle and shift-range ---------------------------

    #[test]
    fn a_plain_click_replaces_the_selection() {
        let mut browser = loaded_browser(&["a", "b", "c"]);
        browser.update(Message::EntryClicked { index: 0, ctrl: false, shift: false });
        browser.update(Message::EntryClicked { index: 2, ctrl: false, shift: false });
        assert!(!browser.selection().is_selected(0));
        assert!(browser.selection().is_selected(2));
    }

    #[test]
    fn ctrl_click_toggles_without_disturbing_the_rest() {
        let mut browser = loaded_browser(&["a", "b", "c"]);
        browser.update(Message::EntryClicked { index: 0, ctrl: false, shift: false });
        browser.update(Message::EntryClicked { index: 2, ctrl: true, shift: false });
        assert!(browser.selection().is_selected(0));
        assert!(browser.selection().is_selected(2));
        assert!(!browser.selection().is_selected(1));

        // Toggling the same index again removes just that one.
        browser.update(Message::EntryClicked { index: 2, ctrl: true, shift: false });
        assert!(!browser.selection().is_selected(2));
        assert!(browser.selection().is_selected(0), "index 0 must survive toggling a different index");
    }

    #[test]
    fn shift_click_selects_a_forward_range() {
        let mut browser = loaded_browser(&["a", "b", "c", "d"]);
        browser.update(Message::EntryClicked { index: 1, ctrl: false, shift: false });
        browser.update(Message::EntryClicked { index: 3, ctrl: false, shift: true });
        for i in 1..=3 {
            assert!(browser.selection().is_selected(i), "index {i} should be in the range");
        }
        assert!(!browser.selection().is_selected(0));
    }

    #[test]
    fn shift_click_selects_a_backward_range() {
        let mut browser = loaded_browser(&["a", "b", "c", "d"]);
        browser.update(Message::EntryClicked { index: 3, ctrl: false, shift: false });
        browser.update(Message::EntryClicked { index: 1, ctrl: false, shift: true });
        for i in 1..=3 {
            assert!(browser.selection().is_selected(i), "index {i} should be in the backward range");
        }
        assert!(!browser.selection().is_selected(0));
    }

    // --- back/forward history ----------------------------------------------

    #[test]
    fn navigating_records_history_and_going_back_restores_the_previous_directory() {
        let (mut browser, _) = Browser::new(Mode::App, Prefs::default(), PathBuf::from("/a"), vec![]);
        let outcome = browser.update(Message::Navigate(PathBuf::from("/b")));
        assert_eq!(outcome, Outcome::ReadDir(PathBuf::from("/b")));
        assert_eq!(browser.current_dir(), Path::new("/b"));

        let outcome = browser.update(Message::GoBack);
        assert_eq!(outcome, Outcome::ReadDir(PathBuf::from("/a")));
        assert_eq!(browser.current_dir(), Path::new("/a"));
    }

    #[test]
    fn going_back_then_forward_returns_to_where_you_were() {
        let (mut browser, _) = Browser::new(Mode::App, Prefs::default(), PathBuf::from("/a"), vec![]);
        browser.update(Message::Navigate(PathBuf::from("/b")));
        browser.update(Message::GoBack);
        let outcome = browser.update(Message::GoForward);
        assert_eq!(outcome, Outcome::ReadDir(PathBuf::from("/b")));
        assert_eq!(browser.current_dir(), Path::new("/b"));
    }

    #[test]
    fn navigating_somewhere_new_truncates_the_forward_stack() {
        let (mut browser, _) = Browser::new(Mode::App, Prefs::default(), PathBuf::from("/a"), vec![]);
        browser.update(Message::Navigate(PathBuf::from("/b")));
        browser.update(Message::GoBack);
        // Forward now points at /b. Navigating somewhere new must drop it.
        browser.update(Message::Navigate(PathBuf::from("/c")));
        let outcome = browser.update(Message::GoForward);
        assert_eq!(outcome, Outcome::None, "there is nothing to go forward to any more");
    }

    #[test]
    fn going_back_with_empty_history_does_nothing() {
        let (mut browser, _) = Browser::new(Mode::App, Prefs::default(), PathBuf::from("/a"), vec![]);
        assert_eq!(browser.update(Message::GoBack), Outcome::None);
    }

    // --- error vs. empty ----------------------------------------------------

    #[test]
    fn an_unreadable_directory_produces_an_error_state_not_an_empty_list() {
        let (mut browser, _) = Browser::new(Mode::App, Prefs::default(), PathBuf::from("/secret"), vec![]);
        let err = DirError {
            message: "You don't have permission to open /secret.".to_string(),
            kind: DirErrorKind::PermissionDenied,
        };
        browser.update(Message::DirLoaded(PathBuf::from("/secret"), Err(err.clone())));

        // The state must be `Error`, never "loaded with zero entries" —
        // those two must stay visibly distinguishable (pinned directly
        // against the private field: this test lives inside `browser`'s
        // own module tree, the same access a unit test for `Prefs::update`
        // already relies on elsewhere in this crate).
        assert_eq!(browser.load_state, LoadState::Error(err));
        assert!(browser.view_entries().is_empty());
    }

    #[test]
    fn a_stale_dir_loaded_result_for_a_directory_left_behind_is_ignored() {
        let (mut browser, _) = Browser::new(Mode::App, Prefs::default(), PathBuf::from("/a"), vec![]);
        browser.update(Message::Navigate(PathBuf::from("/b")));
        // A slow result for /a arrives after the user already moved on.
        browser.update(Message::DirLoaded(PathBuf::from("/a"), Ok(vec![entry("late.txt", false)])));
        assert!(browser.view_entries().is_empty(), "a stale result must not appear in /b's listing");
    }

    // --- search filtering ----------------------------------------------------

    #[test]
    fn search_filters_then_clearing_restores_the_full_list() {
        let mut browser = loaded_browser(&["readme.txt", "license.txt", "notes.md"]);
        browser.update(Message::SearchChanged("readme".to_string()));
        assert_eq!(browser.view_entries().len(), 1);
        assert_eq!(browser.view_entries()[0].name, "readme.txt");

        browser.update(Message::SearchCleared);
        assert_eq!(browser.view_entries().len(), 3);
    }

    // --- activation: folder vs. file ----------------------------------------

    #[test]
    fn activating_a_folder_navigates_into_it() {
        let (mut browser, _) = Browser::new(Mode::App, Prefs::default(), PathBuf::from("/dir"), vec![]);
        let mut folder = entry("sub", true);
        folder.path = PathBuf::from("/dir/sub");
        browser.update(Message::DirLoaded(PathBuf::from("/dir"), Ok(vec![folder])));
        let outcome = browser.update(Message::EntryActivated(0));
        assert_eq!(outcome, Outcome::ReadDir(PathBuf::from("/dir/sub")));
    }

    #[test]
    fn activating_a_file_is_an_outcome_the_host_interprets() {
        let mut browser = loaded_browser(&["photo.png"]);
        let outcome = browser.update(Message::EntryActivated(0));
        assert_eq!(outcome, Outcome::Activated(PathBuf::from("/dir/photo.png")));
    }

    // --- keyboard grammar ----------------------------------------------------

    #[test]
    fn enter_activates_the_focused_entry_via_the_keymap() {
        let mut browser = loaded_browser(&["a.txt", "b.txt"]);
        browser.update(Message::EntryClicked { index: 1, ctrl: false, shift: false });
        let outcome = browser.handle_key(Key::Enter, Modifiers::default());
        assert_eq!(outcome, Outcome::Activated(PathBuf::from("/dir/b.txt")));
    }

    #[test]
    fn typing_a_character_searches_and_escape_clears_it() {
        let mut browser = loaded_browser(&["alpha.txt", "beta.txt"]);
        browser.handle_key(Key::Character('a'), Modifiers::default());
        browser.handle_key(Key::Character('l'), Modifiers::default());
        assert_eq!(browser.view_entries().len(), 1);
        browser.handle_key(Key::Escape, Modifiers::default());
        assert_eq!(browser.view_entries().len(), 2);
    }

    // --- breadcrumb ------------------------------------------------------------

    #[test]
    fn breadcrumb_lists_every_ancestor_with_its_own_full_path() {
        let segments = breadcrumb(Path::new("/home/alex/Documents"));
        let labels: Vec<&str> = segments.iter().map(|(l, _)| l.as_str()).collect();
        assert_eq!(labels, vec!["/", "home", "alex", "Documents"]);
        assert_eq!(segments[2].1, PathBuf::from("/home/alex"));
    }

    // --- the Mode seam ------------------------------------------------------

    /// The constraint the brief cares most about: `view` must render
    /// identically regardless of `Mode`. `render` takes a `ViewModel`
    /// with no `Mode` field at all, so this is structurally guaranteed —
    /// this test pins the property that makes that guarantee meaningful:
    /// two browsers differing *only* in `Mode`, after the same messages,
    /// feed `render` exactly the same data.
    #[test]
    fn view_never_branches_on_mode() {
        let (mut app, _) = Browser::new(Mode::App, Prefs::default(), PathBuf::from("/dir"), vec![]);
        let (mut dialog, _) =
            Browser::new(Mode::Dialog(DialogKind::Open), Prefs::default(), PathBuf::from("/dir"), vec![]);

        let entries = vec![entry("a.txt", false), entry("sub", true)];
        for browser in [&mut app, &mut dialog] {
            browser.update(Message::DirLoaded(PathBuf::from("/dir"), Ok(entries.clone())));
            browser.update(Message::SearchChanged("a".to_string()));
            browser.update(Message::EntryClicked { index: 0, ctrl: false, shift: false });
        }

        assert_ne!(app.mode(), dialog.mode(), "the test is vacuous unless the modes actually differ");
        assert_eq!(
            app.view_model(),
            dialog.view_model(),
            "render()'s input must be identical regardless of Mode"
        );
    }
}
