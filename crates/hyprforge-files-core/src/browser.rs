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

use crate::density;
use crate::filter::{filter_hidden, filter_query};
use crate::format::{format_modified, human_readable_size};
use crate::icon::entry_icon;
use crate::keymap::{self, Direction, Key, KeyAction, Modifiers};
use crate::prefs::{Prefs, ViewMode};
use crate::sidebar::{self, PinnedItem, SidebarItem};
use crate::sort::{sort, SortColumn, SortDirection};
use crate::types::{Entry, FilesError};
// `EntryKind` is used only by this module's tests now that the Kind
// column is gone — imported there rather than here so the lib build
// does not carry an unused import.
use hyprforge_ui::theme::{spacing, FontScale, BASE_TEXT_SIZE};
use hyprforge_ui::widgets::{divider, meta_text, scaled_text};
use iced::widget::{column, container, row, scrollable, text_input, Id};
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
    /// The Pinned sidebar section's content, computed off the UI thread
    /// via [`crate::sidebar::build_pinned`] — see that function's own doc
    /// for why this is a message rather than a `Browser::new` argument
    /// the way Places is.
    PinnedLoaded(Vec<PinnedItem>),
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
    pinned: &'a [PinnedItem],
    /// The entry list's `scrollable` identity — see
    /// [`Browser::list_scrollable_id`]'s own doc for why this exists.
    list_scrollable_id: Id,
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
    /// The Pinned section's content. Empty until a host delivers
    /// [`Message::PinnedLoaded`] — see that variant's doc — which is also
    /// the correct "nobody has pinned anything" state, not a placeholder
    /// waiting to be filled: [`sidebar_sections`] renders no Pinned
    /// heading at all for an empty list, exactly like a fresh install.
    pinned: Vec<PinnedItem>,
    /// The entry list's `scrollable` identity, generated once and held
    /// for the life of this `Browser` — see
    /// [`Self::list_scrollable_id`]'s own doc.
    list_scrollable_id: Id,
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
            pinned: Vec::new(),
            // `Id::unique()` — not a per-widget-tree literal — is what
            // makes this stable across `view()` calls: iced can only
            // preserve a `scrollable`'s offset across frames if the same
            // `Id` names it every time, and generating one here rather
            // than inside `render` (which runs on every `view()` call)
            // is what guarantees that.
            list_scrollable_id: Id::unique(),
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

    /// The Pinned sidebar section's content, as last delivered by
    /// [`Message::PinnedLoaded`]. Empty until a host sends that message —
    /// see the field's own doc.
    pub fn pinned(&self) -> &[PinnedItem] {
        &self.pinned
    }

    /// The entry list's `scrollable` identity.
    ///
    /// Per-`Browser`, not a shared constant: each `Browser` here is one
    /// tab (a host holds one per open tab, per its own titlebar-tabs
    /// design), and a shared `Id` across tabs would make every tab's
    /// list resolve to the *same* scroll position rather than each
    /// tab keeping its own — iced keys scroll state by `Id`, so two
    /// widgets sharing one `Id` are, as far as that state is concerned,
    /// one widget. Generated once in [`Self::new`] and never regenerated
    /// — see that field's own doc for why regenerating it per `view()`
    /// call would defeat the point. A host tracks a scroll offset per
    /// tab keyed by this `Id` (it is already `Clone + PartialEq + Eq +
    /// Hash`) and issues `scrollable::scroll_to` with it on tab switch;
    /// restoring the offset itself is the host's job, not this crate's —
    /// `Browser` only has to keep the identity stable for iced to have
    /// something to restore *to*.
    pub fn list_scrollable_id(&self) -> Id {
        self.list_scrollable_id.clone()
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
            Message::PinnedLoaded(items) => {
                self.pinned = items;
                Outcome::None
            }
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
            pinned: &self.pinned,
            list_scrollable_id: self.list_scrollable_id.clone(),
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
    breadcrumb_from(path, home_dir().as_deref())
}

/// The user's home, or `None` when `$HOME` is unset or relative.
///
/// A relative `$HOME` is rejected rather than joined against the current
/// directory: a path is abbreviated by *prefix comparison* below, and a
/// relative prefix would either match nothing or match something the
/// user did not mean.
fn home_dir() -> Option<PathBuf> {
    std::env::var_os("HOME").map(PathBuf::from).filter(|p| p.is_absolute())
}

/// [`breadcrumb`] with the home directory passed in, so a test can set
/// one without touching the process's environment — which every other
/// test in this binary shares.
fn breadcrumb_from(path: &Path, home: Option<&Path>) -> Vec<(String, PathBuf)> {
    // A path inside the home directory starts at `~` rather than
    // spelling out `/home/<user>/`. That is what the design shows, and
    // the reason is width: the interesting end of a path is its tail,
    // and on this machine the head costs twelve characters of a bar
    // that has to fit a directory name, a search field and the view
    // toggles beside it.
    //
    // `~` is a real crumb, not decoration — clicking it navigates home,
    // exactly as clicking any other segment navigates there.
    if let Some(home) = home {
        if let Ok(rest) = path.strip_prefix(home) {
            let mut result = vec![("~".to_string(), home.to_path_buf())];
            let mut accum = home.to_path_buf();
            for component in rest.components() {
                accum.push(component);
                if let std::path::Component::Normal(s) = component {
                    result.push((s.to_string_lossy().into_owned(), accum.clone()));
                }
            }
            return result;
        }
    }

    // Not under home — an absolute path from `/`, as before. A file
    // manager reaches `/etc` and `/mnt` too, and abbreviating those
    // would be a lie.
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

/// Renders `vm`. A free function taking [`ViewModel`] rather than a
/// method on [`Browser`] — see the module doc for why that's the point.
fn render<'a>(vm: ViewModel<'a>, scale: FontScale) -> Element<'a, Message> {
    let sidebar = sidebar_view(&vm, scale);

    // One band, not three.
    //
    // Navigation, where you are, what you are looking for and how it is
    // shown all belong on the same row — that is what the design does,
    // and the alternative was found out the hard way: the window drew
    // its own strip of view toggles above this one while the search
    // field sat below it, and three stacked bands of chrome ate a
    // quarter of a small window before a single file was listed.
    //
    // The view toggles live *here*, in the shared view, rather than in
    // the window that hosts it. They switch `Prefs::view_mode`, which is
    // browser state with a `Message` variant already; putting their
    // controls in the window meant the portal's open/save dialog, which
    // renders this same view through a different host, would never have
    // had them at all.
    let toolbar = container(
        row![
            nav_button("\u{2039}", vm.can_go_back.then_some(Message::GoBack), scale),
            nav_button("\u{203A}", vm.can_go_forward.then_some(Message::GoForward), scale),
            nav_button("\u{2191}", Some(Message::GoUp), scale),
            path_bar(vm.current_dir, scale),
            search_field(vm.search_query, scale),
            view_mode_toggle(vm.prefs, scale),
        ]
        .spacing(spacing::SM)
        .align_y(iced::Alignment::Center),
    )
    // The design's 40px header band, derived rather than hardcoded —
    // see `density`'s own doc. `height` rather than `center_y` alone
    // still needs the fixed length; the vertical centering keeps the
    // toolbar's buttons/text mid-band at any scale.
    .height(Length::Fixed(density::header_height(scale)))
    .center_y(Length::Fixed(density::header_height(scale)));

    let body = body_view(&vm, scale);

    let content = column![toolbar, body]
        .spacing(spacing::SM)
        .padding(spacing::MD)
        .width(Length::Fill)
        .height(Length::Fill);

    row![sidebar, content].into()
}

/// Back, forward and up.
///
/// Deliberately quieter than `secondary_button`: three outlined buttons
/// at the head of the toolbar drew the eye before the path did, and the
/// path is the thing a person is actually reading. No border, no fill
/// until hovered, and a fixed square so the three read as one cluster
/// rather than three differently-sized pills.
///
/// A disabled direction (no history to go back to) is dimmed rather than
/// removed: a button that vanishes makes the row reflow and the other
/// two move under the pointer.
fn nav_button<'a>(
    glyph: &'a str,
    message: Option<Message>,
    scale: FontScale,
) -> Element<'a, Message> {
    let enabled = message.is_some();
    let side = density::header_height(scale) * 0.7;
    iced::widget::button(
        container(scaled_text(glyph, density::ROW_TEXT_BASE, scale))
            .center_x(Length::Fill)
            .center_y(Length::Fill),
    )
    .width(Length::Fixed(side))
    .height(Length::Fixed(side))
    .on_press_maybe(message)
    .style(move |_t: &iced::Theme, status| {
        let hovered = matches!(status, iced::widget::button::Status::Hovered);
        iced::widget::button::Style {
            background: hovered.then(|| iced::Background::Color(hyprforge_ui::theme::surface::row())),
            text_color: if enabled {
                hyprforge_ui::theme::text()
            } else {
                hyprforge_ui::theme::text_dim()
            },
            border: iced::Border { radius: density::inner_radius().into(), ..iced::Border::default() },
            ..iced::widget::button::Style::default()
        }
    })
    .into()
}

/// The search field, sized to the space left over rather than to the
/// whole row.
///
/// `Length::FillPortion` rather than `Fill`: the path bar and this share
/// the middle of the toolbar, and an equal split would give a long path
/// nowhere to go. Two-to-one matches the design, where the path is the
/// wider of the two.
fn search_field<'a>(query: &str, scale: FontScale) -> Element<'a, Message> {
    // The magnifier is a sibling widget rather than the input's own
    // `icon`, because iced's `text_input::Icon` wants a `Font` to take
    // the glyph from and this suite does not ship one — the theme's font
    // is whatever the desktop chose, and it may have no magnifier at
    // all. A container beside the input always draws.
    let icon = meta_text("\u{25CB}", density::META_TEXT_BASE, scale);

    container(
        row![
            icon,
            text_input("Search", query)
                .on_input(Message::SearchChanged)
                .size(scale.apply(density::ROW_TEXT_BASE))
                .style(|_t: &iced::Theme, _status| iced::widget::text_input::Style {
                    // Transparent: the bordered container around this row
                    // is the field. An input drawing its own background
                    // inside it would show as a box within a box.
                    background: iced::Background::Color(iced::Color::TRANSPARENT),
                    border: iced::Border::default(),
                    icon: hyprforge_ui::theme::text_dim(),
                    placeholder: hyprforge_ui::theme::text_dim(),
                    value: hyprforge_ui::theme::text(),
                    selection: _t.extended_palette().primary.weak.color,
                })
                .width(Length::Fill),
        ]
        .spacing(spacing::XS)
        .align_y(iced::Alignment::Center),
    )
    // A floor, not a share.
    //
    // As a `FillPortion` beside the path this clipped its own
    // placeholder to "Searc" on a narrow window: three-to-one of not
    // much is not much. The path is what should absorb a wide window,
    // and the search field only needs to be wide enough to read — so
    // the path fills and this takes a fixed width derived from the text
    // size, which keeps it legible at any scale.
    .width(Length::Fixed(scale.apply(SEARCH_WIDTH_BASE)))
    .padding([4, spacing::SM as u16])
    .style(|_t: &iced::Theme| container::Style {
        background: Some(iced::Background::Color(hyprforge_ui::theme::surface::card())),
        border: iced::Border {
            radius: density::inner_radius().into(),
            width: 1.0,
            color: hyprforge_ui::theme::surface::card_border(),
        },
        ..container::Style::default()
    })
    .into()
}

/// The search field's width at 100% scale: enough for its placeholder
/// and a short query, and no more. Scaled with the font so it does not
/// clip at 125%.
const SEARCH_WIDTH_BASE: f32 = 150.0;

/// List / Grid / Columns, at the right end of the toolbar.
///
/// Icons in one grouped strip, not three text buttons. The design shows
/// a compact segmented control, and the reason is width: "List Grid
/// Columns" spelled out is most of what a narrow window has left after
/// the path and the search field, and it is the least-read control on
/// the row — you set a view and then stop thinking about it.
///
/// Monochrome symbols, not emoji: these take the text colour like any
/// other glyph, which is exactly what the file-type badges could not do
/// and why those had to be drawn instead.
///
/// Columns has no `on_press`, which iced renders as disabled — column
/// view is a later phase, and a control that appears from nowhere later
/// is a worse surprise than one that was visibly waiting.
fn view_mode_toggle<'a>(prefs: &Prefs, scale: FontScale) -> Element<'a, Message> {
    let side = density::header_height(scale) * 0.7;
    let segment = |glyph: &'a str, mode: Option<ViewMode>| {
        let active = mode.is_some_and(|m| prefs.view_mode == m);
        iced::widget::button(
            container(scaled_text(glyph, density::ROW_TEXT_BASE, scale))
                .center_x(Length::Fill)
                .center_y(Length::Fill),
        )
        .width(Length::Fixed(side))
        .height(Length::Fixed(side))
        .on_press_maybe(mode.map(Message::SetViewMode))
        .style(move |t: &iced::Theme, status| {
            let hovered = matches!(status, iced::widget::button::Status::Hovered);
            // Through the palette, the same route the selected row takes
            // — one source for "this is the accent", so the active view
            // segment and the selected row can never drift apart.
            let accent = t.extended_palette().primary.weak.color;
            iced::widget::button::Style {
                background: if active {
                    Some(iced::Background::Color(accent))
                } else if hovered {
                    Some(iced::Background::Color(hyprforge_ui::theme::surface::row()))
                } else {
                    None
                },
                text_color: match (active, mode.is_some()) {
                    // On the accent fill, text has to read against the
                    // accent rather than against the surface behind it.
                    (true, _) => hyprforge_ui::theme::surface::root(),
                    (false, true) => hyprforge_ui::theme::text(),
                    (false, false) => hyprforge_ui::theme::text_dim(),
                },
                border: iced::Border { radius: density::inner_radius().into(), ..iced::Border::default() },
                ..iced::widget::button::Style::default()
            }
        })
    };

    container(
        row![
            segment("\u{2630}", Some(ViewMode::List)),
            segment("\u{229E}", Some(ViewMode::Grid)),
            segment("\u{2016}", None),
        ]
        .spacing(2.0)
        .align_y(iced::Alignment::Center),
    )
    .padding(2.0)
    .style(|_t: &iced::Theme| container::Style {
        background: Some(iced::Background::Color(hyprforge_ui::theme::surface::card())),
        border: iced::Border {
            radius: density::inner_radius().into(),
            width: 1.0,
            color: hyprforge_ui::theme::surface::card_border(),
        },
        ..container::Style::default()
    })
    .into()
}

/// One heading and its rows in the sidebar — Places, Pinned or Trash.
///
/// Plain data, not an `Element`, so [`sidebar_sections`] is testable
/// without building a window: the property the brief cares about
/// ("Tags with nothing under it reads as broken; no heading reads as
/// not-yet", generalised to every section) is "a section with no rows is
/// absent from this `Vec` entirely", which is a fact about data a test
/// can just look at, rather than a fact about a widget tree that would
/// need one built to inspect.
#[derive(Debug, Clone, PartialEq)]
struct SidebarSection {
    title: &'static str,
    rows: Vec<SidebarRow>,
}

#[derive(Debug, Clone, PartialEq)]
struct SidebarRow {
    label: String,
    path: PathBuf,
    /// Shown right-aligned after the label — Pinned's item count. `None`
    /// for Places and Trash, which have nothing to count.
    meta: Option<String>,
}

/// Builds every section the sidebar could show, **already filtered** to
/// the ones with content — see [`SidebarSection`]'s own doc. Places and
/// Trash always have at least one row (Home always exists; Trash is a
/// fixed path, not something that can fail to "have" rows), so in
/// practice only Pinned can ever be absent, but the filter is applied
/// uniformly rather than special-cased to Pinned: Tags and Remote will
/// land here the same way once they exist, and neither should need this
/// function taught a new special case to stay empty-safe.
fn sidebar_sections(vm: &ViewModel<'_>) -> Vec<SidebarSection> {
    let places = SidebarSection {
        title: "Places",
        rows: vm
            .sidebar
            .iter()
            .map(|item| SidebarRow { label: item.label.clone(), path: item.path.clone(), meta: None })
            .collect(),
    };
    let pinned = SidebarSection {
        title: "Pinned",
        rows: vm
            .pinned
            .iter()
            .map(|item| SidebarRow {
                label: item.label.clone(),
                path: item.path.clone(),
                meta: Some(match item.item_count {
                    Some(n) => n.to_string(),
                    // An unreadable pin still gets its own row — removing
                    // it silently would look like the user's own pin
                    // vanished — but the count reads as unknown, never 0.
                    None => "\u{2014}".to_string(),
                }),
            })
            .collect(),
    };
    let trash = SidebarSection {
        title: "Trash",
        rows: vec![SidebarRow { label: "Trash".to_string(), path: sidebar::trash_path(), meta: None }],
    };
    [places, pinned, trash].into_iter().filter(|s| !s.rows.is_empty()).collect()
}

fn sidebar_view<'a>(vm: &ViewModel<'a>, scale: FontScale) -> Element<'a, Message> {
    let mut list = column![].spacing(spacing::MD).width(Length::Fixed(180.0));
    for section in sidebar_sections(vm) {
        let mut group = column![meta_text(section.title.to_uppercase(), 12.0, scale)].spacing(spacing::XS);
        for row_item in section.rows {
            let is_current = row_item.path == vm.current_dir;
            let content: Element<'a, Message> = match row_item.meta {
                Some(meta) => row![
                    scaled_text(row_item.label.clone(), density::ROW_TEXT_BASE, scale).width(Length::Fill),
                    meta_text(meta, density::META_TEXT_BASE, scale),
                ]
                .spacing(spacing::XS)
                .into(),
                None => scaled_text(row_item.label.clone(), density::ROW_TEXT_BASE, scale).into(),
            };
            // Built with the raw `button` widget rather than
            // `primary_button`/`secondary_button` — those two only
            // accept a text fragment, and a Pinned row needs a label
            // plus a right-aligned count in the same clickable area, the
            // same reason `entry_row` below goes straight to `button`
            // instead of wrapping one of the two helpers.
            let button = iced::widget::button(content)
                .width(Length::Fill)
                .on_press(Message::Navigate(row_item.path))
                .style(move |t: &iced::Theme, status| sidebar_row_style(t, status, is_current));
            group = group.push(button);
        }
        list = list.push(group);
    }
    container(scrollable(list))
        .padding(spacing::SM)
        .width(Length::Fixed(200.0))
        .height(Length::Fill)
        .into()
}

/// A sidebar row's look: the current location uses `accent` and nothing
/// else does, matching [`entry_row_style`]'s own rule for the entry
/// list — "purple is the only selection colour" applies to *every*
/// selection-shaped state in this app, not just the file list's.
fn sidebar_row_style(theme: &iced::Theme, status: iced::widget::button::Status, is_current: bool) -> iced::widget::button::Style {
    use iced::widget::button;
    use iced::{Background, Border};
    let palette = theme.extended_palette();
    let background = if is_current {
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
        border: Border { radius: density::inner_radius().into(), ..Border::default() },
        ..button::Style::default()
    }
}

fn path_bar<'a>(current_dir: &Path, scale: FontScale) -> Element<'a, Message> {
    let mut crumbs = row![].spacing(spacing::XS).align_y(iced::Alignment::Center);
    let segments = breadcrumb(current_dir);
    let last = segments.len().saturating_sub(1);
    for (i, (label, path)) in segments.into_iter().enumerate() {
        if i > 0 {
            crumbs = crumbs.push(meta_text("/", density::META_TEXT_BASE, scale));
        }
        if i == last {
            // The folder you are actually in, as a filled chip rather
            // than plain text. In a path of five segments the last one
            // is the answer to "where am I", and without the chip it is
            // the least distinguishable word on the row.
            crumbs = crumbs.push(
                container(scaled_text(label, density::ROW_TEXT_BASE, scale))
                    .padding([2, 6])
                    .style(|_t: &iced::Theme| container::Style {
                        background: Some(iced::Background::Color(hyprforge_ui::theme::surface::row())),
                        border: iced::Border {
                            radius: density::inner_radius().into(),
                            ..iced::Border::default()
                        },
                        ..container::Style::default()
                    }),
            );
        } else {
            crumbs = crumbs.push(crumb_button(label, path, scale));
        }
    }

    // One field, not a bare row of buttons.
    //
    // The design draws the whole path inside a single bordered box the
    // width of the toolbar's middle — it reads as the address field it
    // is, and it is the thing the eye should land on first. Loose
    // buttons on the toolbar's own background read as three more
    // controls beside the navigation ones.
    container(crumbs)
        .width(Length::Fill)
        .padding([4, spacing::SM as u16])
        .style(|_t: &iced::Theme| container::Style {
            background: Some(iced::Background::Color(hyprforge_ui::theme::surface::card())),
            border: iced::Border {
                radius: density::inner_radius().into(),
                width: 1.0,
                color: hyprforge_ui::theme::surface::card_border(),
            },
            ..container::Style::default()
        })
        .into()
}

/// One ancestor in the path, clickable but drawn as text.
///
/// No button chrome: inside the path field, an outlined button per
/// segment would make a five-deep path look like a row of five
/// controls. It highlights on hover, which is where "this is clickable"
/// belongs.
fn crumb_button<'a>(label: String, path: PathBuf, scale: FontScale) -> Element<'a, Message> {
    iced::widget::button(meta_text(label, density::ROW_TEXT_BASE, scale))
        .on_press(Message::Navigate(path))
        .padding([2, 4])
        .style(|_t: &iced::Theme, status| {
            let hovered = matches!(status, iced::widget::button::Status::Hovered);
            iced::widget::button::Style {
                background: hovered.then(|| iced::Background::Color(hyprforge_ui::theme::surface::row())),
                text_color: if hovered {
                    hyprforge_ui::theme::text()
                } else {
                    hyprforge_ui::theme::text_dim()
                },
                border: iced::Border { radius: density::inner_radius().into(), ..iced::Border::default() },
                ..iced::widget::button::Style::default()
            }
        })
        .into()
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
    let size = if entry.is_dir {
        String::new()
    } else {
        human_readable_size(entry.size)
    };
    // Name, size, modified — the design's three columns, and no more.
    //
    // There was a fourth, Kind, and it had to go: in a directory of
    // folders it reads "Folder" forty times down the screen, which is
    // noise dressed as information. The icon already says what kind a
    // row is, in the same glance that reads the name.
    //
    // Git's `M`/`A` badges are deliberately not a column here either:
    // DESIGN.md defers git entirely, and a column with nothing to put in
    // it is exactly the "reads as broken, not as not-yet" trap the
    // sidebar section rule below is written to avoid.
    let row_content = row![
        entry_icon(entry.kind, 20.0, scale),
        scaled_text(entry.name.clone(), density::ROW_TEXT_BASE, scale).width(Length::FillPortion(NAME_PORTION)),
        meta_text(size, density::META_TEXT_BASE, scale).width(Length::FillPortion(SIZE_PORTION)),
        meta_text(format_modified(entry.modified), density::META_TEXT_BASE, scale)
            .width(Length::FillPortion(MODIFIED_PORTION)),
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
        // The design's 28px row, derived — see `density`'s own doc — so
        // it grows with `FontScale` instead of clipping `row_content`'s
        // text past 100%.
        .height(Length::Fixed(density::row_height(scale)))
        .style(move |t: &iced::Theme, status| entry_row_style(t, status, selected));
    styled.into()
}

/// A list/grid row's look. **Selection is the only place `accent`
/// appears in this file** — a hovered-but-not-selected row uses
/// `surface::row()`, a plain grey elevation distinct from both the
/// unselected default (no background at all) and the selected state, so
/// "this row is the selection" is never ambiguous with "the pointer
/// happens to be over it". [`tests::only_the_selected_row_ever_uses_the_accent_colour`]
/// pins this.
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
        // The design's 6px inner radius, taken from the Theme — see
        // `density::inner_radius`'s doc.
        border: Border { radius: density::inner_radius().into(), ..Border::default() },
        ..button::Style::default()
    }
}

/// The list's column widths, shared by the header and every row.
///
/// Named constants rather than repeated literals because a header that
/// does not line up with the rows beneath it is the most obvious
/// possible bug and the easiest to introduce: change one `FillPortion`
/// and the columns silently drift apart.
const NAME_PORTION: u16 = 3;
const SIZE_PORTION: u16 = 1;
const MODIFIED_PORTION: u16 = 2;

/// The clickable column headers.
///
/// Sorting lives here rather than behind an overflow menu because this
/// is where a person already expects it: clicking a header sorts by it
/// and clicking again reverses, which is what every file manager anyone
/// has used does. That also removes the need for a separate sort
/// control entirely.
fn list_header<'a>(prefs: &Prefs, scale: FontScale) -> Element<'a, Message> {
    let heading = |label: &'static str, column: SortColumn, portion: u16| {
        let active = prefs.sort_column() == column;
        // The arrow marks the sorted column *and* its direction, so the
        // header is the whole answer to "how is this list ordered" — no
        // second indicator anywhere else to fall out of step with it.
        let text = if active {
            let arrow = match prefs.sort_direction() {
                SortDirection::Ascending => "\u{2191}",
                SortDirection::Descending => "\u{2193}",
            };
            format!("{label} {arrow}")
        } else {
            label.to_string()
        };
        let content = if active {
            scaled_text(text, density::META_TEXT_BASE, scale)
        } else {
            meta_text(text, density::META_TEXT_BASE, scale)
        };
        iced::widget::button(content)
            .on_press(Message::SortBy(column))
            .width(Length::FillPortion(portion))
            .style(header_button_style)
    };

    row![
        // An empty cell the width of a row's icon, so "Name" starts
        // above the names rather than above the icons.
        iced::widget::Space::new().width(Length::Fixed(scale.apply(20.0))),
        heading("Name", SortColumn::Name, NAME_PORTION),
        heading("Size", SortColumn::Size, SIZE_PORTION),
        heading("Modified", SortColumn::Modified, MODIFIED_PORTION),
    ]
    .spacing(spacing::SM)
    .align_y(iced::Alignment::Center)
    .into()
}

/// A header is a control, but it is not a button-shaped one: no fill, no
/// border, just text that responds to the pointer. Anything more would
/// put four button outlines across the top of every listing.
fn header_button_style(
    _theme: &iced::Theme,
    status: iced::widget::button::Status,
) -> iced::widget::button::Style {
    let hovered = matches!(status, iced::widget::button::Status::Hovered);
    iced::widget::button::Style {
        background: hovered.then(|| iced::Background::Color(hyprforge_ui::theme::surface::row())),
        text_color: hyprforge_ui::theme::text(),
        border: iced::Border {
            radius: density::inner_radius().into(),
            ..iced::Border::default()
        },
        ..iced::widget::button::Style::default()
    }
}

fn list_view<'a>(vm: &ViewModel<'a>, scale: FontScale) -> Element<'a, Message> {
    // No rule between rows.
    //
    // The design draws none, and a line under every row is what made
    // ours read as a table of data rather than a list of things: at 28px
    // a separator every 28px is a stripe pattern, and it fights the
    // selected row for the eye. Rows are told apart by their own
    // spacing and by the hover elevation, which is enough.
    let mut list = column![].spacing(1.0);
    for (index, entry) in vm.view_entries.iter().enumerate() {
        list = list.push(entry_row(index, entry, vm.selection.is_selected(index), scale));
    }
    // `.id(..)` is what lets a host restore this list's scroll position
    // across a tab switch — see `Browser::list_scrollable_id`'s own doc.
    // Without a stable `Id`, iced cannot tell "the same list, redrawn"
    // from "a different scrollable that happens to be in the same spot",
    // and drops the offset.
    // The header sits outside the `scrollable`, so it stays put while
    // the rows move under it.
    column![
        list_header(vm.prefs, scale),
        divider(),
        scrollable(list).height(Length::Fill).id(vm.list_scrollable_id.clone()),
    ]
    .spacing(spacing::XS)
    .into()
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
    use crate::types::EntryKind;
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

    /// The design shows `~ / projects / hyprsuite / src`, not
    /// `/ home / apost / projects / ...`. The head of a path under home
    /// is twelve characters of a bar that also has to fit a search field
    /// and the view toggles.
    #[test]
    fn a_path_under_home_starts_at_a_tilde() {
        let home = Path::new("/home/apost");
        let segments = breadcrumb_from(Path::new("/home/apost/Documents/Projects"), Some(home));
        let labels: Vec<&str> = segments.iter().map(|(l, _)| l.as_str()).collect();
        assert_eq!(labels, vec!["~", "Documents", "Projects"]);
    }

    /// `~` is a crumb, not a label: clicking it goes home, the same as
    /// clicking any other segment goes there.
    #[test]
    fn the_tilde_crumb_navigates_to_the_home_directory() {
        let home = Path::new("/home/apost");
        let segments = breadcrumb_from(Path::new("/home/apost/Documents"), Some(home));
        assert_eq!(segments[0].1, home, "the ~ crumb's path is home itself");
    }

    #[test]
    fn the_home_directory_itself_is_just_a_tilde() {
        let home = Path::new("/home/apost");
        let segments = breadcrumb_from(home, Some(home));
        let labels: Vec<&str> = segments.iter().map(|(l, _)| l.as_str()).collect();
        assert_eq!(labels, vec!["~"]);
    }

    /// A file manager reaches `/etc` and `/mnt` too, and abbreviating
    /// those would be a lie about where you are.
    #[test]
    fn a_path_outside_home_still_spells_itself_out_from_the_root() {
        let segments = breadcrumb_from(Path::new("/etc/systemd"), Some(Path::new("/home/apost")));
        let labels: Vec<&str> = segments.iter().map(|(l, _)| l.as_str()).collect();
        assert_eq!(labels, vec!["/", "etc", "systemd"]);
    }

    /// A directory whose name merely *starts* with the home directory's
    /// path is not inside it. `strip_prefix` compares whole components,
    /// which is what makes this right — a string comparison would turn
    /// `/home/apostrophe` into `~trophe`.
    #[test]
    fn a_sibling_directory_sharing_a_name_prefix_is_not_abbreviated() {
        let segments = breadcrumb_from(Path::new("/home/apostrophe/notes"), Some(Path::new("/home/apost")));
        let labels: Vec<&str> = segments.iter().map(|(l, _)| l.as_str()).collect();
        assert_eq!(labels, vec!["/", "home", "apostrophe", "notes"]);
    }

    /// With no `$HOME` to compare against, every path spells itself out
    /// rather than the abbreviation silently applying to nothing.
    #[test]
    fn with_no_home_known_nothing_is_abbreviated() {
        let segments = breadcrumb_from(Path::new("/home/apost/Documents"), None);
        let labels: Vec<&str> = segments.iter().map(|(l, _)| l.as_str()).collect();
        assert_eq!(labels, vec!["/", "home", "apost", "Documents"]);
    }

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
        let pinned = vec![PinnedItem { label: "Projects".to_string(), path: PathBuf::from("/pin"), item_count: Some(3) }];
        for browser in [&mut app, &mut dialog] {
            browser.update(Message::DirLoaded(PathBuf::from("/dir"), Ok(entries.clone())));
            browser.update(Message::SearchChanged("a".to_string()));
            browser.update(Message::EntryClicked { index: 0, ctrl: false, shift: false });
            // Pinned is a later addition to `ViewModel`; feeding it
            // through here too keeps the Mode-seam test honest about
            // every field, not just the ones that existed when it was
            // first written.
            browser.update(Message::PinnedLoaded(pinned.clone()));
        }

        assert_ne!(app.mode(), dialog.mode(), "the test is vacuous unless the modes actually differ");

        // `list_scrollable_id` is a widget-identity concern, not model
        // data — it is deliberately unique *per `Browser`* (see that
        // field's own doc: two tabs must not share a scroll position),
        // so two independently-constructed browsers differ there by
        // design even though nothing about `Mode` caused it. Neutralised
        // before the equality check below, which is about `Mode`
        // specifically.
        dialog.list_scrollable_id = app.list_scrollable_id.clone();

        assert_eq!(
            app.view_model(),
            dialog.view_model(),
            "render()'s input must be identical regardless of Mode"
        );
    }

    // --- selection colour ----------------------------------------------------

    /// The design reserves purple for exactly one meaning: the current
    /// selection. A hovered-but-unselected row using the same colour
    /// would make "is this selected?" ambiguous the instant the pointer
    /// moved, so hover has to read as a plainly different colour — this
    /// pins that the accent (`palette.primary.weak.color`, what
    /// `entry_row_style` selects on) never appears for any non-selected
    /// status, hover included.
    #[test]
    fn only_the_selected_row_ever_uses_the_accent_colour() {
        use iced::widget::button::Status;
        let theme = hyprforge_ui::theme::app_theme();
        let accent_bg = theme.extended_palette().primary.weak.color;

        let selected = entry_row_style(&theme, Status::Active, true);
        assert_eq!(selected.background, Some(iced::Background::Color(accent_bg)));

        for status in [Status::Active, Status::Hovered, Status::Pressed, Status::Disabled] {
            let unselected = entry_row_style(&theme, status, false);
            assert_ne!(
                unselected.background,
                Some(iced::Background::Color(accent_bg)),
                "a non-selected row must never render the accent colour ({status:?})"
            );
        }
    }

    /// Hover has to be visibly different from *both* "plain" and
    /// "selected" — a hover that read the same as selection would make a
    /// row the user is merely pointing at look like one they picked.
    #[test]
    fn hover_is_a_distinct_elevation_from_both_plain_and_selected() {
        use iced::widget::button::Status;
        let theme = hyprforge_ui::theme::app_theme();
        let plain = entry_row_style(&theme, Status::Active, false);
        let hovered = entry_row_style(&theme, Status::Hovered, false);
        let selected = entry_row_style(&theme, Status::Active, true);
        assert_ne!(plain.background, hovered.background);
        assert_ne!(hovered.background, selected.background);
    }

    // --- sidebar sections ------------------------------------------------

    /// A hand-built `ViewModel` rather than a full `Browser`, so a test
    /// can vary just `sidebar`/`pinned` without a directory read to set
    /// up first — `sidebar_sections` is a pure function of these fields
    /// alone. Takes the `Selection`/`Prefs` it borrows as parameters
    /// rather than defaulting them internally, since a `ViewModel`
    /// borrows rather than owns and the caller has to keep those alive.
    fn view_model_with<'a>(
        sidebar: &'a [SidebarItem],
        pinned: &'a [PinnedItem],
        current_dir: &'a Path,
        selection: &'a Selection,
        prefs: &'a Prefs,
        load_state: &'a LoadState,
    ) -> ViewModel<'a> {
        ViewModel {
            current_dir,
            view_entries: &[],
            selection,
            search_query: "",
            load_state,
            prefs,
            sidebar,
            pinned,
            list_scrollable_id: Id::unique(),
            can_go_back: false,
            can_go_forward: false,
        }
    }

    /// Places always has Home, so this pins the general rule with the
    /// one section that is genuinely conditional on user data: an empty
    /// Pinned list must not appear in the sections at all — no "Pinned"
    /// heading with nothing under it, per DESIGN.md's own reasoning
    /// about why that reads as broken rather than as not-yet.
    #[test]
    fn an_empty_pinned_list_produces_no_pinned_section_at_all() {
        let places = vec![SidebarItem { label: "Home".to_string(), path: PathBuf::from("/home/alex") }];
        let (selection, prefs, load_state) = (Selection::default(), Prefs::default(), LoadState::Loaded);
        let vm = view_model_with(&places, &[], Path::new("/home/alex"), &selection, &prefs, &load_state);
        let sections = sidebar_sections(&vm);
        assert!(!sections.iter().any(|s| s.title == "Pinned"), "an empty section must render nothing at all");
        assert!(sections.iter().any(|s| s.title == "Places"));
        assert!(sections.iter().any(|s| s.title == "Trash"), "Trash is fixed and always present");
    }

    #[test]
    fn a_non_empty_pinned_list_appears_with_its_item_counts() {
        let places = vec![SidebarItem { label: "Home".to_string(), path: PathBuf::from("/home/alex") }];
        let pinned = vec![PinnedItem { label: "Projects".to_string(), path: PathBuf::from("/pin"), item_count: Some(7) }];
        let (selection, prefs, load_state) = (Selection::default(), Prefs::default(), LoadState::Loaded);
        let vm = view_model_with(&places, &pinned, Path::new("/home/alex"), &selection, &prefs, &load_state);
        let sections = sidebar_sections(&vm);
        let pinned_section = sections.iter().find(|s| s.title == "Pinned").expect("a non-empty Pinned list must appear");
        assert_eq!(pinned_section.rows[0].label, "Projects");
        assert_eq!(pinned_section.rows[0].meta.as_deref(), Some("7"));
    }

    #[test]
    fn trash_always_points_at_sidebar_trash_path() {
        let (selection, prefs, load_state) = (Selection::default(), Prefs::default(), LoadState::Loaded);
        let vm = view_model_with(&[], &[], Path::new("/x"), &selection, &prefs, &load_state);
        let sections = sidebar_sections(&vm);
        let trash = sections.iter().find(|s| s.title == "Trash").unwrap();
        assert_eq!(trash.rows[0].path, sidebar::trash_path());
    }
}
