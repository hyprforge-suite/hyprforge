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
use crate::glyph;
use crate::icon::{self, entry_icon};
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
    /// Count what is inside each of these directories, off the UI
    /// thread, and feed the answers back as [`Message::CountsLoaded`].
    ///
    /// A second pass rather than part of the listing itself, and that is
    /// the whole design: counting means one `read_dir` per subdirectory,
    /// so folding it into the first read would make a directory of two
    /// hundred folders take two hundred times as long *before anything
    /// appeared*. The names arrive immediately and the counts fill in
    /// behind them, which is what every file manager that shows this
    /// column does.
    CountFolders(Vec<PathBuf>),
}

#[derive(Debug, Clone, PartialEq)]
pub enum Message {
    Navigate(PathBuf),
    DirLoaded(PathBuf, Result<Vec<Entry>, DirError>),
    /// Answers to [`Outcome::CountFolders`]. `None` for a directory that
    /// could not be read — never `Some(0)`, which would claim it is
    /// empty.
    CountsLoaded(Vec<(PathBuf, Option<usize>)>),
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
    counts: &'a std::collections::HashMap<PathBuf, Option<usize>>,
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
    /// How many entries each listed directory holds, once the second
    /// pass has answered — see [`Outcome::CountFolders`].
    ///
    /// Keyed by path rather than by row index, so a count that arrives
    /// after the listing was re-sorted still lands on the right folder.
    /// Cleared on every navigation: a stale count is worse than none,
    /// because a number is read as current.
    counts: std::collections::HashMap<PathBuf, Option<usize>>,
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
            counts: std::collections::HashMap::new(),
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
            Message::CountsLoaded(counts) => {
                // No staleness guard needed: `apply_dir_loaded` clears
                // the map on every navigation, and these are keyed by
                // path — a count for a folder that is no longer listed
                // simply sits in the map unread rather than showing up
                // against the wrong row.
                self.counts.extend(counts);
                Outcome::None
            }
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
        // Counts from wherever we were before are about other folders.
        // Dropping them is not an optimisation — a number left over from
        // the previous directory would be read as this one's.
        self.counts.clear();

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

        let folders: Vec<PathBuf> =
            self.entries.iter().filter(|e| e.is_dir).map(|e| e.path.clone()).collect();
        if folders.is_empty() {
            Outcome::None
        } else {
            Outcome::CountFolders(folders)
        }
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
            counts: &self.counts,
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
    // Header and status bar span the whole window; the sidebar sits
    // between them.
    //
    // This was the other way round — `row![sidebar, column![toolbar,
    // body]]` — and it is the difference between a window and a panel
    // with a strip above it. The header is the only horizontal chrome
    // here and everything else is vertical, which is what lets it read
    // as a titlebar on a compositor that draws no titlebar. Cutting it
    // short at the sidebar's edge destroys that: the eye reads two
    // columns with their own headers instead of one window.
    column![
        header_bar(&vm, scale),
        plane_edge(),
        row![sidebar_view(&vm, scale), file_area(&vm, scale)].height(Length::Fill),
        plane_edge(),
        status_bar(&vm, scale),
    ]
    .width(Length::Fill)
    .height(Length::Fill)
    .into()
}

/// The listing and its column headers — everything right of the sidebar
/// and between the two bars.
fn file_area<'a>(vm: &ViewModel<'a>, scale: FontScale) -> Element<'a, Message> {
    container(body_view(vm, scale))
        .padding(spacing::SM)
        .width(Length::Fill)
        .height(Length::Fill)
        .style(|_t: &iced::Theme| container::Style {
            background: Some(iced::Background::Color(hyprforge_ui::theme::surface::card())),
            ..container::Style::default()
        })
        .into()
}

/// The 44px bar across the top.
///
/// Three depth levels, used consistently everywhere in this window: the
/// bar's own plane (`sidebar`), fields recessed into it (`card`), and
/// raised live controls (`row`). A control that is *on* right now is
/// filled; one that is off or disabled is not. That single idiom covers
/// the back button, the active view segment, and nothing else needs
/// explaining.
///
/// Laid out fixed · fill · fixed · fixed, so dragging the window edge
/// only ever stretches the path bar and the two right-hand clusters stay
/// anchored where the hand expects them.
fn header_bar<'a>(vm: &ViewModel<'a>, scale: FontScale) -> Element<'a, Message> {
    let nav = row![
        nav_button(glyph::Nav::Back, vm.can_go_back.then_some(Message::GoBack), scale),
        nav_button(glyph::Nav::Forward, vm.can_go_forward.then_some(Message::GoForward), scale),
        nav_button(glyph::Nav::Up, Some(Message::GoUp), scale),
    ]
    .spacing(spacing::XS)
    .align_y(iced::Alignment::Center);

    container(
        row![
            nav,
            path_bar(vm.current_dir, scale),
            search_field(vm.search_query, vm.current_dir, scale),
            view_mode_toggle(vm.prefs, scale),
        ]
        .spacing(spacing::SM)
        .align_y(iced::Alignment::Center),
    )
    .height(Length::Fixed(density::bar_height(scale)))
    .center_y(Length::Fixed(density::bar_height(scale)))
    .padding([0, spacing::SM as u16])
    .width(Length::Fill)
    .style(|_t: &iced::Theme| container::Style {
        background: Some(iced::Background::Color(hyprforge_ui::theme::surface::sidebar())),
        // Closed off below by `plane_edge`, not by a border here — see
        // that function's own doc.
        ..container::Style::default()
    })
    .into()
}

/// The 1px line under the header and over the status bar.
///
/// Its own element rather than a `Border` on the bar, because iced's
/// border draws on all four sides — a bar with a border would be a box,
/// and the design closes the bar off on one edge only.
fn plane_edge<'a>() -> Element<'a, Message> {
    container(iced::widget::Space::new())
        .width(Length::Fill)
        .height(Length::Fixed(1.0))
        .style(|_t: &iced::Theme| container::Style {
            background: Some(iced::Background::Color(hyprforge_ui::theme::surface::root())),
            ..container::Style::default()
        })
        .into()
}

/// Back, forward and up: three fixed squares, 4px apart.
///
/// Fixed 26px squares rather than padded glyphs, so the cluster never
/// resizes with the glyph or the locale and the path bar beside it never
/// moves.
///
/// The enabled signal is a *fill*, matching the active view-mode segment
/// exactly — one idiom for "this control is live" across the whole bar.
/// A disabled direction keeps its footprint and dims its glyph rather
/// than disappearing: a button that vanishes reflows the row and moves
/// its neighbours under the pointer.
fn nav_button<'a>(
    kind: glyph::Nav,
    message: Option<Message>,
    scale: FontScale,
) -> Element<'a, Message> {
    let enabled = message.is_some();
    // Back is the one that gets a fill when live. Up is always
    // available — there is always a parent — so filling it too would
    // make the fill mean "exists" rather than "does something", and the
    // row would read as two lit buttons and one dark one for no reason
    // the user could act on.
    let filled = enabled && matches!(message, Some(Message::GoBack));
    let side = density::glyph_button(scale);
    // The mark's colour is decided here rather than inherited, because a
    // canvas draws with what it is given — `text_color` on the button
    // below styles text, and there is no text.
    let color = if enabled {
        hyprforge_ui::theme::text()
    } else {
        hyprforge_ui::theme::text_dim()
    };
    iced::widget::button(
        container(glyph::nav(kind, side, color)).center_x(Length::Fill).center_y(Length::Fill),
    )
    .width(Length::Fixed(side))
    .height(Length::Fixed(side))
    // No padding. An iced button pads by default, which left the canvas
    // inside about 16px of this 26px square — and because the mark is
    // sized as a fraction of what it is given, it drew a chevron scaled
    // to the *padded* box. That is why the first drawn version came out
    // visibly smaller than the typed glyphs it replaced.
    .padding(0)
    .on_press_maybe(message)
    .style(move |_t: &iced::Theme, status| {
        let hovered = matches!(status, iced::widget::button::Status::Hovered);
        iced::widget::button::Style {
            background: (filled || hovered)
                .then(|| iced::Background::Color(hyprforge_ui::theme::surface::row())),
            text_color: if enabled {
                hyprforge_ui::theme::text()
            } else {
                hyprforge_ui::theme::text_dim()
            },
            border: iced::Border {
                radius: density::inner_radius().into(),
                ..iced::Border::default()
            },
            ..iced::widget::button::Style::default()
        }
    })
    .into()
}

/// The search field: a fixed-width sibling of the path bar.
///
/// Deliberately the same recipe as the path bar — same fill, same
/// border, same radius, same height — because both are text inputs and
/// they should look like siblings. Fixed width, so it never competes
/// with the path for space: the path is the only thing on this bar that
/// flexes.
///
/// The placeholder names the scope — "Search crates", not "Search" —
/// so what it will search is stated before anything is typed, rather
/// than discovered afterwards.
fn search_field<'a>(query: &str, current_dir: &Path, scale: FontScale) -> Element<'a, Message> {
    let here = current_dir
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| current_dir.display().to_string());
    let placeholder = format!("Search {here}");

    // The magnifier is a sibling widget rather than the input's own
    // `icon`, because iced's `text_input::Icon` wants a `Font` to take
    // the glyph from and this suite ships none — the theme's font is
    // whatever the desktop chose, and it may have no magnifier at all. A
    // drawn ring always renders.
    let ring_side = scale.apply(SEARCH_RING_BASE);
    let ring = container(iced::widget::Space::new())
        .width(Length::Fixed(ring_side))
        .height(Length::Fixed(ring_side))
        .style(move |_t: &iced::Theme| container::Style {
            border: iced::Border {
                // Half the side: a circle, not a rounded square.
                radius: (ring_side / 2.0).into(),
                width: 1.5,
                color: hyprforge_ui::theme::text_dim(),
            },
            ..container::Style::default()
        });

    container(
        row![
            ring,
            text_input(&placeholder, query)
                .on_input(Message::SearchChanged)
                .size(scale.apply(density::META_TEXT_BASE))
                .style(|t: &iced::Theme, _status| iced::widget::text_input::Style {
                    // Transparent: the bordered container around this row
                    // is the field. An input drawing its own background
                    // inside it would show as a box within a box.
                    background: iced::Background::Color(iced::Color::TRANSPARENT),
                    border: iced::Border::default(),
                    icon: hyprforge_ui::theme::text_dim(),
                    placeholder: hyprforge_ui::theme::text_dim(),
                    value: hyprforge_ui::theme::text(),
                    selection: t.extended_palette().primary.weak.color,
                })
                .width(Length::Fill),
        ]
        .spacing(spacing::XS)
        .align_y(iced::Alignment::Center),
    )
    .width(Length::Fixed(scale.apply(density::SEARCH_FIELD_WIDTH)))
    .height(Length::Fixed(density::field_height(scale)))
    .center_y(Length::Fixed(density::field_height(scale)))
    .padding([0, spacing::SM as u16])
    .style(inset_field_style)
    .into()
}

/// The magnifier ring's diameter at 100% scale.
const SEARCH_RING_BASE: f32 = 9.0;

/// The look both text fields share: recessed into the bar, outlined.
///
/// One function rather than two identical closures, so the path bar and
/// the search field cannot drift apart — they are siblings by
/// construction, not by someone remembering to keep them matched.
fn inset_field_style(_t: &iced::Theme) -> container::Style {
    container::Style {
        background: Some(iced::Background::Color(hyprforge_ui::theme::surface::card())),
        border: iced::Border {
            radius: density::inner_radius().into(),
            width: 1.0,
            color: hyprforge_ui::theme::surface::card_border(),
        },
        ..container::Style::default()
    }
}



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
    let seg_w = density::glyph_button(scale) * 0.92;
    let seg_h = density::glyph_button(scale) * 0.77;
    let segment = |glyph_kind: glyph::View, mode: Option<ViewMode>| {
        let active = mode.is_some_and(|m| prefs.view_mode == m);
        let color = if active {
            hyprforge_ui::theme::text()
        } else {
            hyprforge_ui::theme::text_dim()
        };
        iced::widget::button(
            container(glyph::view(glyph_kind, seg_h, color))
                .center_x(Length::Fill)
                .center_y(Length::Fill),
        )
        .width(Length::Fixed(seg_w))
        .height(Length::Fixed(seg_h))
        // Same reason as `nav_button`: the default padding would shrink
        // the mark inside its own segment and push it off centre.
        .padding(0)
        .on_press_maybe(mode.map(Message::SetViewMode))
        .style(move |_t: &iced::Theme, status| {
            let hovered = matches!(status, iced::widget::button::Status::Hovered);
            iced::widget::button::Style {
                // `row`, not the accent. The active segment is the same
                // "this control is live" idiom as the back button's
                // fill, and reusing the accent here would put a second
                // purple on the bar competing with the one that means
                // selection.
                background: if active || hovered {
                    Some(iced::Background::Color(hyprforge_ui::theme::surface::row()))
                } else {
                    None
                },
                text_color: match (active, mode.is_some()) {
                    (true, _) => hyprforge_ui::theme::text(),
                    (false, true) => hyprforge_ui::theme::text_dim(),
                    (false, false) => hyprforge_ui::theme::text_dim(),
                },
                border: iced::Border { radius: density::nested_radius().into(), ..iced::Border::default() },
                ..iced::widget::button::Style::default()
            }
        })
    };

    container(
        row![
            segment(glyph::View::List, Some(ViewMode::List)),
            segment(glyph::View::Grid, Some(ViewMode::Grid)),
            segment(glyph::View::Columns, None),
        ]
        .spacing(3.0)
        .align_y(iced::Alignment::Center),
    )
    .padding(3.0)
    .style(|_t: &iced::Theme| container::Style {
        // The track's fill matches the bar it sits on rather than the
        // recessed fields beside it, so it reads as a groove cut into
        // the bar rather than as a raised control.
        background: Some(iced::Background::Color(hyprforge_ui::theme::surface::sidebar())),
        border: iced::Border {
            radius: density::inner_radius().into(),
            width: 1.0,
            color: hyprforge_ui::theme::surface::card_border(),
        },
        ..container::Style::default()
    })
    .into()
}

/// The line along the bottom: how much is here, and how much of it you
/// have picked.
///
/// Small, dim, and never in the way — but it answers a question the
/// listing itself cannot. "How many things are in this folder" requires
/// counting rows by eye, and "how many did I select" is genuinely
/// invisible once the selection scrolls off the top.
///
/// It says nothing at all when there is nothing to say: an empty folder
/// gets an empty bar rather than "0 items", because a row of zeroes is
/// noise where a blank is calm.
fn status_bar<'a>(vm: &ViewModel<'a>, scale: FontScale) -> Element<'a, Message> {
    let total = vm.view_entries.len();
    if total == 0 {
        return container(iced::widget::Space::new())
            .height(Length::Fixed(density::status_height(scale)))
            .width(Length::Fill)
            .style(|_t: &iced::Theme| container::Style {
                background: Some(iced::Background::Color(hyprforge_ui::theme::surface::sidebar())),
                ..container::Style::default()
            })
            .into();
    }

    let items = if total == 1 { "1 item".to_string() } else { format!("{total} items") };
    let selected = vm.selection.selected_indices().len();
    let text = if selected == 0 {
        items
    } else {
        // "· " rather than a comma: the two halves are separate facts,
        // not a list, and the design uses the same separator.
        format!("{items} \u{00B7} {selected} selected")
    };

    container(meta_text(text, density::META_TEXT_BASE, scale))
        .height(Length::Fixed(density::status_height(scale)))
        .center_y(Length::Fixed(density::status_height(scale)))
        .padding([0, spacing::MD as u16])
        .width(Length::Fill)
        .style(|_t: &iced::Theme| container::Style {
            // The same plane as the header: the chrome is one material,
            // top and bottom, with the listing recessed between them.
            background: Some(iced::Background::Color(hyprforge_ui::theme::surface::sidebar())),
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
    /// Which theme colour this row's folder mark takes — see
    /// [`sidebar::Tint`].
    tint: sidebar::Tint,
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
            .map(|item| SidebarRow {
                label: item.label.clone(),
                path: item.path.clone(),
                meta: None,
                tint: item.tint,
            })
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
                // A pin is a folder the user chose, so it takes the
                // accent — the same colour Home does, because both are
                // "somewhere you put yourself".
                tint: sidebar::Tint::Accent,
            })
            .collect(),
    };
    let trash = SidebarSection {
        title: "Trash",
        // Dim: the trash is a destination, not one of your places, and
        // giving it a hue of its own would put it in the same visual
        // class as Home.
        rows: vec![SidebarRow {
            label: "Trash".to_string(),
            path: sidebar::trash_path(),
            meta: None,
            tint: sidebar::Tint::Dim,
        }],
    };
    [places, pinned, trash].into_iter().filter(|s| !s.rows.is_empty()).collect()
}

fn sidebar_view<'a>(vm: &ViewModel<'a>, scale: FontScale) -> Element<'a, Message> {
    let mut list = column![].spacing(spacing::MD).width(Length::Fill);
    for section in sidebar_sections(vm) {
        // A section heading is a signpost, not content: small, dim, and
        // spaced out, so it separates without competing with the rows
        // under it. Padded on its own rather than by the column's
        // spacing so the heading sits nearer its own rows than the
        // section above — proximity is what makes the grouping read.
        let heading = container(meta_text(
            spaced_caps(section.title),
            density::SECTION_LABEL_BASE,
            scale,
        ))
        .padding([spacing::XS as u16, spacing::SM as u16]);
        let mut group = column![heading].spacing(1.0);
        for row_item in section.rows {
            let is_current = row_item.path == vm.current_dir;
            let mark = icon::folder_mark(
                hyprforge_ui::color::to_iced(row_item.tint.color()),
                density::SIDEBAR_MARK_BASE,
                scale,
            );
            let label = scaled_text(row_item.label.clone(), density::ROW_TEXT_BASE, scale);
            let content: Element<'a, Message> = match row_item.meta {
                Some(meta) => row![
                    mark,
                    label.width(Length::Fill),
                    meta_text(meta, density::META_TEXT_BASE, scale),
                ]
                .spacing(spacing::SM)
                .align_y(iced::Alignment::Center)
                .into(),
                None => row![mark, label]
                    .spacing(spacing::SM)
                    .align_y(iced::Alignment::Center)
                    .into(),
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
        .width(Length::Fixed(density::SIDEBAR_WIDTH))
        .height(Length::Fill)
        .style(|_t: &iced::Theme| container::Style {
            background: Some(iced::Background::Color(hyprforge_ui::theme::surface::sidebar())),
            ..container::Style::default()
        })
        .into()
}

/// `PLACES` from `Places` — uppercased, with a hair space between
/// letters.
///
/// iced has no letter-spacing, and the design's heading depends on it:
/// small uppercase text without it reads as a cramped word rather than
/// as a label. Inserting U+2009 THIN SPACE between characters is the
/// approximation available — coarser than real tracking, but it buys
/// most of the effect for a heading that is never more than two words.
fn spaced_caps(title: &str) -> String {
    title
        .to_uppercase()
        .chars()
        .map(|c| c.to_string())
        .collect::<Vec<_>>()
        .join("\u{2009}")
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
    let segments = elide(breadcrumb(current_dir));
    let last = segments.len().saturating_sub(1);
    for (i, (label, path)) in segments.into_iter().enumerate() {
        if i > 0 {
            crumbs = crumbs.push(
                meta_text("/", density::META_TEXT_BASE, scale).font(iced::Font::MONOSPACE),
            );
        }
        if i == last {
            // The folder you are actually in, as a filled chip.
            //
            // This is what makes the field a breadcrumb rather than a
            // text box: it says "you are here" without spending a label
            // on it. The fill is `card_border`, the same colour the
            // sidebar marks its current location with, so the header and
            // the sidebar agree about where you are rather than each
            // making its own claim.
            crumbs = crumbs.push(
                container(
                    scaled_text(label, density::META_TEXT_BASE, scale).font(iced::Font::MONOSPACE),
                )
                .padding([1, 5])
                .style(|_t: &iced::Theme| container::Style {
                    background: Some(iced::Background::Color(
                        hyprforge_ui::theme::surface::card_border(),
                    )),
                    // Nested inside a control, so the tighter radius —
                    // see `density::nested_radius`.
                    border: iced::Border {
                        radius: density::nested_radius().into(),
                        ..iced::Border::default()
                    },
                    ..container::Style::default()
                }),
            );
        } else if i == 0 && label == "~" {
            // Home in the accent, so you locate yourself by colour
            // before reading a single character of the path. One of only
            // two places colour does real work on this bar; everything
            // else is grey, which leaves the accent free to mean
            // selection the moment the eye drops into the list.
            crumbs = crumbs.push(crumb_button(label, path, scale, true));
        } else {
            crumbs = crumbs.push(crumb_button(label, path, scale, false));
        }
    }

    // One field, not a bare row of buttons.
    //
    // The design draws the whole path inside a single bordered box the
    // width of the toolbar's middle — it reads as the address field it
    // is, and it is the thing the eye should land on first. Loose
    // buttons on the toolbar's own background read as three more
    // controls beside the navigation ones.
    // The only thing on the bar that flexes. Everything else is fixed,
    // so dragging the window edge stretches this and nothing else moves.
    container(crumbs)
        .width(Length::Fill)
        .height(Length::Fixed(density::field_height(scale)))
        .center_y(Length::Fixed(density::field_height(scale)))
        .padding([0, spacing::SM as u16])
        .style(inset_field_style)
        .into()
}

/// Drops the middle of a path too deep to fit, keeping the ends.
///
/// Without this a deep path simply ran off the end of its field: four
/// levels into a project the bar read "~ / Documents" and the folder you
/// were actually in — the one thing it exists to tell you — was the part
/// clipped off. Silently, with nothing to say it had happened.
///
/// The head and the tail are what carry meaning. The head says which
/// tree you are in; the tail says where in it. The middle is the part a
/// person skips reading anyway, so it becomes one ellipsis — which is
/// deliberately *not* clickable, because there is no single directory it
/// could navigate to.
///
/// Elision by count rather than by measured width: iced cannot tell a
/// layout function how much room it got, so a width-aware version would
/// have to guess at glyph widths and would be wrong for any font but the
/// one it guessed for. A fixed depth is cruder and honest.
fn elide(segments: Vec<(String, PathBuf)>) -> Vec<(String, PathBuf)> {
    if segments.len() <= MAX_CRUMBS {
        return segments;
    }
    let mut out = Vec::with_capacity(MAX_CRUMBS + 1);
    out.push(segments[0].clone());
    // The ellipsis carries the *first hidden* path, so it is at least a
    // meaningful thing to hold rather than an empty one, even though
    // nothing clicks it today.
    out.push(("\u{2026}".to_string(), segments[1].1.clone()));
    // `MAX_CRUMBS - 2`, not `- 1`: the head and the ellipsis already
    // account for two of the budget. Getting this wrong returns one
    // crumb too many, which still *looks* right and quietly makes the
    // field overflow again — the exact bug elision was added to fix.
    out.extend(segments[segments.len() - (MAX_CRUMBS - 2)..].iter().cloned());
    out
}

/// How many crumbs are kept: the head, an ellipsis, and where you are.
///
/// Three, not four, and the difference was measured rather than
/// guessed. On this machine's window — 771px wide, 216 of it sidebar —
/// the toolbar leaves the path field about 200px after the navigation
/// buttons, the search field and the view toggles have taken theirs.
/// Four crumbs overflowed that, and iced clips a row from the right,
/// so the segment that got dropped was the *last* one: the folder you
/// are actually in, which is the single thing the bar exists to tell
/// you.
///
/// Losing the parent hurts less than losing where you are. A width-aware
/// version could keep more on a wide window, but iced gives a layout
/// function no way to ask how much room it got, so that version would be
/// guessing at glyph widths — and being wrong about it puts us straight
/// back to clipping the end.
const MAX_CRUMBS: usize = 3;

/// One ancestor in the path, clickable but drawn as text.
///
/// No button chrome: inside the path field, an outlined button per
/// segment would make a five-deep path look like a row of five
/// controls. It highlights on hover, which is where "this is clickable"
/// belongs.
fn crumb_button<'a>(
    label: String,
    path: PathBuf,
    scale: FontScale,
    accented: bool,
) -> Element<'a, Message> {
    iced::widget::button(
        meta_text(label, density::META_TEXT_BASE, scale).font(iced::Font::MONOSPACE),
    )
        .on_press(Message::Navigate(path))
        .padding([2, 4])
        .style(move |t: &iced::Theme, status| {
            let hovered = matches!(status, iced::widget::button::Status::Hovered);
            iced::widget::button::Style {
                background: hovered.then(|| iced::Background::Color(hyprforge_ui::theme::surface::row())),
                // An ancestor crumb is full-brightness text whether or
                // not the pointer is over it — hover is signalled by the
                // fill appearing, not by the text changing, so the path
                // does not shimmer as the pointer crosses it.
                text_color: if accented {
                    t.extended_palette().primary.weak.color
                } else {
                    hyprforge_ui::theme::text()
                },
                border: iced::Border { radius: density::nested_radius().into(), ..iced::Border::default() },
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

fn entry_row<'a>(
    index: usize,
    entry: &'a Entry,
    selected: bool,
    count: Option<Option<usize>>,
    scale: FontScale,
) -> Element<'a, Message> {
    // A directory's Size cell holds how many things are in it, not a
    // byte total — see `Entry::size`'s own doc for why this crate never
    // sums a tree. Three states, and they are deliberately different:
    // nothing yet (the count is still being read), a number, or an em
    // dash for a directory that could not be read. A folder you have no
    // permission to open must not read as "0 items".
    let size = if entry.is_dir {
        match count {
            None => String::new(),
            Some(Some(1)) => "1 item".to_string(),
            Some(Some(n)) => format!("{n} items"),
            Some(None) => "\u{2014}".to_string(),
        }
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
    let mut seen_file = false;
    for (index, entry) in vm.view_entries.iter().enumerate() {
        // One rule, where the directories end and the files begin.
        //
        // The only divider in the listing — there used to be one under
        // every row, which at this density was a stripe pattern that
        // fought the selected row for the eye. This one marks a boundary
        // that actually exists, and only when `directories_first` put it
        // there: without that setting the two are interleaved and a line
        // partway down would be marking nothing.
        if vm.prefs.directories_first && !entry.is_dir && !seen_file && index > 0 {
            list = list.push(container(divider()).padding([spacing::XS as u16, spacing::SM as u16]));
        }
        if !entry.is_dir {
            seen_file = true;
        }
        // `.copied()` and *not* `.flatten()`: the two levels mean
        // different things. Absent from the map is "not counted yet"
        // and renders blank; present-but-`None` is "could not be read"
        // and renders an em dash. Flattening collapses those into one,
        // which would show every folder as unreadable for the moment
        // before its count arrives.
        let count = vm.counts.get(&entry.path).copied();
        list = list.push(entry_row(index, entry, vm.selection.is_selected(index), count, scale));
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

    // --- folder counts ---------------------------------------------------------

    /// A listing asks for its folders to be counted, so the Size column
    /// has something to say about a directory — but only *after* the
    /// names are on screen, which is why it is a second pass.
    #[test]
    fn loading_a_directory_asks_for_its_folders_to_be_counted() {
        let (mut browser, _) =
            Browser::new(Mode::App, Prefs::default(), PathBuf::from("/dir"), vec![]);
        let entries = vec![entry("sub", true), entry("a.txt", false), entry("other", true)];
        let outcome =
            browser.update(Message::DirLoaded(PathBuf::from("/dir"), Ok(entries)));
        match outcome {
            Outcome::CountFolders(folders) => {
                assert_eq!(folders.len(), 2, "both directories, and neither file");
                assert!(folders.iter().all(|p| p.to_string_lossy().contains("sub")
                    || p.to_string_lossy().contains("other")));
            }
            other => panic!("expected a count request, got {other:?}"),
        }
    }

    /// A directory of nothing but files asks for nothing. Sending an
    /// empty request would spawn a background task to count zero things.
    #[test]
    fn a_listing_with_no_folders_asks_for_no_counts() {
        let (mut browser, _) =
            Browser::new(Mode::App, Prefs::default(), PathBuf::from("/dir"), vec![]);
        let entries = vec![entry("a.txt", false), entry("b.txt", false)];
        let outcome = browser.update(Message::DirLoaded(PathBuf::from("/dir"), Ok(entries)));
        assert!(matches!(outcome, Outcome::None));
    }

    /// Counts from the previous directory are dropped on navigation. A
    /// number left over from somewhere else is worse than no number,
    /// because a number is read as current.
    #[test]
    fn navigating_away_forgets_the_counts_it_had() {
        let (mut browser, _) =
            Browser::new(Mode::App, Prefs::default(), PathBuf::from("/dir"), vec![]);
        browser.update(Message::DirLoaded(PathBuf::from("/dir"), Ok(vec![entry("sub", true)])));
        browser.update(Message::CountsLoaded(vec![(PathBuf::from("/dir/sub"), Some(18))]));
        assert_eq!(browser.counts.get(Path::new("/dir/sub")), Some(&Some(18)));

        browser.update(Message::Navigate(PathBuf::from("/elsewhere")));
        browser.update(Message::DirLoaded(PathBuf::from("/elsewhere"), Ok(vec![])));
        assert!(browser.counts.is_empty(), "a count for a folder we have left must not survive");
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

    /// A path deeper than the bar can show keeps its ends. Without this
    /// the tail was what got clipped — so four levels in, the bar showed
    /// the tree you were in and not the folder you were actually looking
    /// at, which is the one thing it exists to say.
    #[test]
    fn a_path_too_deep_to_fit_keeps_its_head_and_its_tail() {
        let deep: Vec<(String, PathBuf)> = ["~", "Documents", "Projects", "hyprforge", "crates"]
            .iter()
            .map(|s| (s.to_string(), PathBuf::from(s)))
            .collect();
        let shown: Vec<String> = elide(deep).into_iter().map(|(l, _)| l).collect();
        assert_eq!(
            shown,
            vec!["~", "\u{2026}", "crates"],
            "the folder you are in must survive; the middle is what goes"
        );
    }

    /// A path that already fits is left exactly as it is — no ellipsis
    /// appears for a path with nothing hidden behind it.
    #[test]
    fn a_path_that_fits_is_not_elided_at_all() {
        let shallow: Vec<(String, PathBuf)> = ["~", "Documents"]
            .iter()
            .map(|s| (s.to_string(), PathBuf::from(s)))
            .collect();
        let shown: Vec<String> = elide(shallow).into_iter().map(|(l, _)| l).collect();
        assert_eq!(shown, vec!["~", "Documents"]);
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
        // A shared empty map so this helper can hand out a `&'a` to one
        // — a local would not outlive the call.
        static EMPTY_COUNTS: std::sync::OnceLock<std::collections::HashMap<PathBuf, Option<usize>>> =
            std::sync::OnceLock::new();
        ViewModel {
            current_dir,
            view_entries: &[],
            selection,
            search_query: "",
            counts: EMPTY_COUNTS.get_or_init(std::collections::HashMap::new),
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
        let places =
            vec![SidebarItem { label: "Home".to_string(), path: PathBuf::from("/home/alex"), tint: sidebar::Tint::Accent }];
        let (selection, prefs, load_state) = (Selection::default(), Prefs::default(), LoadState::Loaded);
        let vm = view_model_with(&places, &[], Path::new("/home/alex"), &selection, &prefs, &load_state);
        let sections = sidebar_sections(&vm);
        assert!(!sections.iter().any(|s| s.title == "Pinned"), "an empty section must render nothing at all");
        assert!(sections.iter().any(|s| s.title == "Places"));
        assert!(sections.iter().any(|s| s.title == "Trash"), "Trash is fixed and always present");
    }

    #[test]
    fn a_non_empty_pinned_list_appears_with_its_item_counts() {
        let places =
            vec![SidebarItem { label: "Home".to_string(), path: PathBuf::from("/home/alex"), tint: sidebar::Tint::Accent }];
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
