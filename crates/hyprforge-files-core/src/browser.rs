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
use crate::filter::{is_hidden, matches_query};
use crate::format::{format_kind, format_modified_at, format_origin, format_owner, format_permissions, format_size};
use crate::glyph;
use crate::icon::{self, entry_icon};
use crate::action::{self, Action, ActionContext, Scope};
use crate::config::Config;
use crate::menu::{self as menus, MenuItem, MenuKind};
use std::sync::Arc;
use crate::prefs::{Column, Prefs, ViewMode};
use crate::sidebar::{self, PinnedItem, SidebarItem};
use crate::sort::{sort_indices, SortColumn, SortDirection};
use crate::types::{Entry, EntrySize, FilesError, ItemCount};
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
    /// The listing is inside an encrypted archive. Never rendered
    /// either: the host opens a password prompt and reads again.
    PasswordRequired,
    /// The path led through a file that looked like an archive and is
    /// not one. Never rendered: [`Browser::apply_dir_loaded`] turns it
    /// into an ordinary activation instead — see there.
    NotAnArchive,
    Other,
}

impl From<&FilesError> for DirError {
    fn from(e: &FilesError) -> Self {
        let kind = match e {
            FilesError::PermissionDenied { .. } => DirErrorKind::PermissionDenied,
            FilesError::NotFound { .. } => DirErrorKind::NotFound,
            FilesError::NotADirectory { .. } => DirErrorKind::NotADirectory,
            FilesError::VanishedMidRead { .. } => DirErrorKind::Vanished,
            FilesError::NotAnArchive { .. } => DirErrorKind::NotAnArchive,
            FilesError::PasswordRequired { .. } => DirErrorKind::PasswordRequired,
            FilesError::Elsewhere { .. } | FilesError::Io { .. } => DirErrorKind::Other,
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

/// Multi-selection over a listing, **keyed by path**.
///
/// Not by row index. An index is a position in `view`, which is rebuilt
/// from scratch on every search keystroke and every sort change, and
/// nothing about rebuilding it moved the selection — so index 2 before
/// typing and index 2 after were different files, silently. That matters
/// because a host resolves these to paths and passes them to the trash:
/// the rebinding was destructive, not cosmetic.
///
/// `Browser::counts` had already made this argument for itself and won
/// it — "keyed by path rather than by row index, so a count that arrives
/// after the listing was re-sorted still lands on the right folder".
/// The selection is the same shape of problem with worse consequences.
///
/// `focused` is the keyboard cursor — what Enter activates, what an
/// arrow key moves — and is not always the same as "the only selected
/// item": a shift-range click leaves every row in the range selected but
/// `focused` pinned to the row that was actually clicked.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Selection {
    anchor: Option<PathBuf>,
    focused: Option<PathBuf>,
    selected: HashSet<PathBuf>,
}

impl Selection {
    pub fn is_selected(&self, path: &Path) -> bool {
        self.selected.contains(path)
    }

    pub fn focused(&self) -> Option<&Path> {
        self.focused.as_deref()
    }

    /// Every selected path. A host trashing the selection reads this
    /// directly — it no longer has to resolve indices against a list
    /// that may have been re-sorted since the click.
    pub fn selected_paths(&self) -> &HashSet<PathBuf> {
        &self.selected
    }

    /// A plain click: replaces the selection with just `path` and moves
    /// the shift-range anchor there.
    fn click(&mut self, path: &Path) {
        self.selected.clear();
        self.selected.insert(path.to_path_buf());
        self.anchor = Some(path.to_path_buf());
        self.focused = Some(path.to_path_buf());
    }

    /// A ctrl-click: toggles `path` in the selection without disturbing
    /// the rest, and becomes the new anchor for a following shift-click —
    /// the behaviour every mainstream file manager uses.
    fn ctrl_click(&mut self, path: &Path) {
        if !self.selected.insert(path.to_path_buf()) {
            self.selected.remove(path);
        }
        self.anchor = Some(path.to_path_buf());
        self.focused = Some(path.to_path_buf());
    }

    /// A shift-click: selects the closed range between the anchor and
    /// the clicked row, in either direction — clicking before the anchor
    /// selects backwards just as well as after it, since the range is
    /// built from `min..=max`, not by counting forward off the anchor.
    ///
    /// The range is resolved against `rows` *now*, at click time, which
    /// is the only moment "the rows between these two" means anything.
    /// What is stored is the resulting paths.
    fn shift_click(&mut self, rows: &[&Entry], index: usize) {
        let anchor = self
            .anchor
            .as_deref()
            .and_then(|a| rows.iter().position(|e| e.path == a))
            // An anchor that is no longer shown (it was filtered out by
            // a search, say) cannot anchor a range; the click becomes
            // its own start rather than silently ranging from row 0.
            .unwrap_or(index);
        let (lo, hi) = if anchor <= index { (anchor, index) } else { (index, anchor) };
        self.selected = rows[lo..=hi].iter().map(|e| e.path.clone()).collect();
        self.focused = Some(rows[index].path.clone());
        // Anchor is deliberately left where it was: a second shift-click
        // extends/contracts from the same starting point, not from the
        // row the previous shift-click landed on.
    }

    /// Applies a click with the given modifiers. Ctrl wins over shift if
    /// somehow both are held, since a ctrl-click's "toggle just this one"
    /// is the more specific request.
    pub fn click_with(&mut self, rows: &[&Entry], index: usize, ctrl: bool, shift: bool) {
        let Some(entry) = rows.get(index) else {
            return;
        };
        if ctrl {
            self.ctrl_click(&entry.path);
        } else if shift {
            self.shift_click(rows, index);
        } else {
            self.click(&entry.path);
        }
    }

    /// Moves the keyboard focus by one row and collapses the selection
    /// to just the new focus — arrow-key navigation, which replaces a
    /// multi-select rather than extending it (shift+arrow range-select is
    /// not in this pass's brief).
    fn move_focus(&mut self, rows: &[&Entry], delta: i32) {
        if rows.is_empty() {
            self.clear();
            return;
        }
        // A focus that is no longer shown restarts from the top, the
        // same reading as having no focus at all.
        let current = self
            .focused
            .as_deref()
            .and_then(|f| rows.iter().position(|e| e.path == f))
            .unwrap_or(0) as i32;
        let next = (current + delta).clamp(0, rows.len() as i32 - 1) as usize;
        self.click(&rows[next].path);
    }

    /// Forgets every selected, focused or anchor path not in `present`.
    fn retain(&mut self, present: &HashSet<PathBuf>) {
        self.selected.retain(|p| present.contains(p));
        if self.focused.as_ref().is_some_and(|p| !present.contains(p)) {
            self.focused = None;
        }
        if self.anchor.as_ref().is_some_and(|p| !present.contains(p)) {
            self.anchor = None;
        }
    }

    /// Moves the keyboard focus to `path` without touching what is
    /// selected — a right click inside an existing selection.
    fn focus(&mut self, path: &Path) {
        self.focused = Some(path.to_path_buf());
    }

    /// Selects every shown row, leaving the focus where it was — or on
    /// the first row if there was none, so a following arrow key has
    /// somewhere to start from.
    fn select_all(&mut self, rows: &[&Entry]) {
        self.selected = rows.iter().map(|e| e.path.clone()).collect();
        if self.focused.is_none() {
            self.focused = rows.first().map(|e| e.path.clone());
        }
        if self.anchor.is_none() {
            self.anchor = self.focused.clone();
        }
    }

    /// Moves the focus by one row and selects everything between it and
    /// the anchor — Shift+Arrow, the keyboard's shift-click.
    fn extend(&mut self, rows: &[&Entry], delta: i32) {
        if rows.is_empty() {
            return;
        }
        let current = self
            .focused
            .as_deref()
            .and_then(|f| rows.iter().position(|e| e.path == f));
        let next = match current {
            Some(i) => (i as i32 + delta).clamp(0, rows.len() as i32 - 1) as usize,
            None => 0,
        };
        if self.anchor.is_none() {
            // No anchor yet: the range starts where the focus was, or at
            // the row we are arriving on.
            self.anchor = Some(rows[current.unwrap_or(next)].path.clone());
        }
        self.shift_click(rows, next);
    }

    fn clear(&mut self) {
        *self = Selection::default();
    }
}

/// Where a context menu was asked for.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MenuSpot {
    /// On this row (by position in the current view).
    Row(usize),
    /// On the empty space around the rows.
    Background,
    /// Wherever the keyboard focus is — the Menu key and Shift+F10. The
    /// focused row if there is one, the background if not.
    Focused,
    /// On a sidebar row, for the place it names.
    Sidebar(PathBuf),
}

/// A name being edited in place.
#[derive(Debug, Clone, PartialEq)]
struct Renaming {
    path: PathBuf,
    text: String,
    /// The edit field's identity, so the host can focus it and select
    /// the name. Fresh per rename: a reused `Id` would let iced carry the
    /// last rename's cursor into this one.
    id: Id,
}

/// A context menu that is showing.
#[derive(Debug, Clone, PartialEq)]
struct OpenMenu {
    items: Vec<MenuItem>,
    /// Where it was asked for, in window coordinates. Placement against
    /// the window's edges happens at draw time, when the window's size is
    /// known.
    at: (f32, f32),
    /// The item the arrow keys are on, if any.
    highlighted: Option<usize>,
    /// The folder a sidebar menu is about. A listing's menus act on the
    /// selection; a sidebar row is not in the listing, so its menu
    /// carries its own target.
    target: Option<PathBuf>,
}

/// Everything a host must do in response to [`Browser::update`] or
/// [`Browser::perform`] — the outcomes `Browser` cannot carry out
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
    /// Move these to the trash. Only paths the user can currently see —
    /// see [`Browser::perform`].
    Trash(Vec<PathBuf>),
    /// Delete these for good. Only paths the user can currently see.
    DeletePermanently(Vec<PathBuf>),
    /// Put these trashed items back where they came from. Stored paths,
    /// as the Trash listing shows them.
    Restore(Vec<PathBuf>),
    /// Delete everything in the Trash, after asking.
    EmptyTrash,
    /// Open this folder in a new tab. A host without tabs ignores it.
    OpenInNewTab(PathBuf),
    /// Put these files on the clipboard.
    SetClipboard(crate::clipboard::FileClip),
    /// Put this text on the clipboard — the paths, for Copy Path.
    CopyText(String),
    /// Ask the window which application should open this file. The
    /// browser cannot answer: the mime database and the launching are
    /// both the host's, and the open/save dialog has no business
    /// launching anything at all.
    OpenWith(PathBuf),
    /// Change the pinned list. The window owns that list — every tab
    /// shows the same one — so the change goes to it rather than into
    /// this tab's own copy of the preferences.
    Pins(crate::sidebar::PinChange),
    /// Paste the clipboard's files into this folder.
    Paste(PathBuf),
    /// Focus the rename field and select its first `select` characters —
    /// the name without its extension.
    FocusRename { id: Id, select: usize },
    /// Rename `from` to `to` (same folder).
    Rename { from: PathBuf, to: PathBuf },
    /// Create this folder, then show it being renamed.
    CreateFolder(PathBuf),
    /// Tell the person something, in the status bar.
    Notice(String),
    /// Several things to do, in order.
    Many(Vec<Outcome>),
    /// Open a context menu at the pointer. The browser does not know
    /// where the pointer is — the window does, and fills it in, the same
    /// way it fills in modifier keys on a click.
    OpenContextMenuAtPointer(MenuSpot),
    /// Unpack these archives. `to` is where they go; `None` means
    /// "each into a new folder beside itself, named after it" — the
    /// host picks the name, because it is the one that can see which
    /// names are free.
    ///
    /// The unpacking itself is the host's for the same reason reading a
    /// directory is: it is long, blocking work that wants a progress
    /// bar and a cancel button, and `Browser` does no I/O.
    Extract {
        archives: Vec<PathBuf>,
        to: Option<PathBuf>,
    },
    /// Unpack these members *out of* the archive this listing is inside.
    /// The host asks where they should go.
    ExtractMembers {
        members: Vec<PathBuf>,
        from: PathBuf,
    },
    /// Make an archive out of these. The host asks for a name and a
    /// format.
    Compress {
        sources: Vec<PathBuf>,
        into: PathBuf,
    },
    /// A window-scope action — see [`crate::action::Scope`]. It reaches
    /// the host this way so a menu item for one travels the same road as
    /// every other action; a host without tabs ignores it.
    Window(Action),
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
    /// Show or hide one optional list column. Persisted, so this is an
    /// [`Outcome::PrefsChanged`] rather than pure view state.
    ToggleColumn(crate::prefs::Column),
    /// Show or hide the sidebar. Persisted — see
    /// [`crate::prefs::SidebarPref`].
    ///
    /// Carries the state to set rather than being a bare "toggle",
    /// because the view is the only thing that knows which way round
    /// that is: whether the sidebar is showing depends on the window
    /// width, which `update` has no business knowing. Same principle as
    /// the modifiers on `EntryClicked` — the side that observes the
    /// world reports it, and the model stays a function of its message.
    SetSidebar(crate::prefs::SidebarPref),
    /// Open or close the column picker. Deliberately *not* persisted —
    /// a panel left open is not a preference, and reopening the window
    /// to find it still down would read as a bug.
    ToggleColumnPicker,
    SetViewMode(crate::prefs::ViewMode),
    /// Carry out an action — from a menu, a button, or a key the host
    /// resolved through [`crate::keymap::Keymap`].
    Perform(Action),
    /// The rename field's text changed.
    RenameInput(String),
    /// Enter in the rename field.
    RenameCommit,
    /// Leave the name as it was.
    RenameCancel,
    /// When `path` next appears in a listing, select it — and start
    /// renaming it if `rename`. How a new folder arrives ready to name,
    /// and how a renamed file stays selected under its new name.
    AfterListing { path: PathBuf, rename: bool },
    /// Open the right-click menu for `spot` at `at` (window coordinates).
    /// The view sends this with `at` unset; the host fills it in from
    /// the pointer, because a right press carries no position.
    OpenContextMenu { spot: MenuSpot, at: (f32, f32) },
    /// A menu item was clicked, by position in the open menu.
    MenuChose(usize),
    /// Dismiss the menu — a click anywhere else.
    CloseMenu,
    /// Type a character into the search box. The host decides a key
    /// press means this; see [`crate::keymap::Resolved::Type`].
    TypeToSearch(char),
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
    /// Whether this listing is the Trash — which changes its columns.
    in_trash: bool,
    /// The name being edited, if any.
    renaming: Option<&'a Renaming>,
    column_picker_open: bool,
    /// Whether the sidebar is collapsed right now — the resolved answer,
    /// not the preference, so `render` never has to ask twice.
    sidebar_collapsed: bool,
    /// The host window's width in logical pixels — see [`Browser::view`].
    viewport_width: f32,
    /// How many entries this listing holds that the current filters are
    /// keeping off screen — see [`Browser::hidden_count`].
    hidden_count: usize,
    /// How many entries here are dotfiles, shown or not — whether the
    /// status bar offers its show/hide switch.
    dotfiles: usize,
    /// What key that switch names, as configured. `None` if the action
    /// has been left unbound.
    hidden_key: Option<String>,
    /// `[sidebar] show-trash`.
    show_trash: bool,
    /// The rows to draw, in order — borrowed out of the one owned
    /// listing rather than a second copy of it.
    rows: Vec<&'a Entry>,
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
    /// Which of `entries` to show, in what order — indices, not copies.
    ///
    /// Recomputed by [`Self::refresh_view`] whenever any input to it
    /// changes, so `view` never has to. Indices into `entries` and not a
    /// second `Vec<Entry>`: the listing was previously cloned whole,
    /// twice, on every search keystroke, and the browser then held two
    /// full copies of every directory at rest. Indices also mean a
    /// count arriving for one folder updates the single place that
    /// folder is stored, rather than one of two.
    ///
    /// Always valid: `entries` is assigned in exactly one place
    /// ([`Self::apply_dir_loaded`]), which refreshes this immediately
    /// after.
    view: Vec<usize>,
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
    /// Whether the column picker under the list header is open.
    ///
    /// Not in `Prefs`: a panel is a thing you are doing, not a thing you
    /// have configured, and finding it still open on next launch would
    /// read as the app having failed to close it.
    column_picker_open: bool,
    /// The entry list's `scrollable` identity, generated once and held
    /// for the life of this `Browser` — see
    /// [`Self::list_scrollable_id`]'s own doc.
    list_scrollable_id: Id,
    /// What `files-config.toml` configured — the menus, and the keymap
    /// their shortcut hints are read from. Shared, not copied per tab.
    config: Arc<Config>,
    /// The context menu, when one is showing.
    menu: Option<OpenMenu>,
    /// Whether the clipboard holds files — see [`Self::set_can_paste`].
    can_paste: bool,
    /// Whether this listing is inside an archive — see
    /// [`Self::set_in_archive`].
    in_archive: bool,
    /// The name being edited in place, if any.
    renaming: Option<Renaming>,
    /// See [`Message::AfterListing`].
    after_listing: Option<(PathBuf, bool)>,
    /// How many of `entries` are dotfiles, counted when the listing
    /// arrives rather than in `view` — iced runs `view` every frame,
    /// and a directory with 100,000 entries was being walked on each of
    /// them for a number that only changes when the listing does.
    dotfiles: usize,
    /// The key bound to "show hidden files", as text for the status
    /// bar. Resolved when the configuration is set: working it out
    /// meant allocating a `Vec`, sorting it with a `String` per
    /// comparison, and allocating the answer — per frame.
    hidden_key: Option<String>,
}

impl Browser {
    /// Builds a browser starting at `start_dir`, with a sidebar the host
    /// already built (via [`crate::sidebar::build`], off the UI thread —
    /// see that function's own doc). Returns the [`Outcome::ReadDir`]
    /// the host must fulfil to show anything at all.
    pub fn new(mode: Mode, prefs: Prefs, start_dir: PathBuf, sidebar: Vec<SidebarItem>) -> (Browser, Outcome) {
        let browser = Browser {
            column_picker_open: false,
            mode,
            current_dir: start_dir.clone(),
            entries: Vec::new(),
            view: Vec::new(),
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
            config: Arc::new(Config::default()),
            menu: None,
            can_paste: false,
            in_archive: false,
            renaming: None,
            after_listing: None,
            dotfiles: 0,
            // The shipped default, until `set_config` says otherwise.
            hidden_key: Config::default()
                .keymap
                .combos_for(Action::ToggleHidden)
                .first()
                .map(|combo| combo.to_string()),
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

    /// What has been typed into the search box, if anything. A host
    /// with a modal of its own open needs to be able to prove nothing
    /// leaked through to the listing behind it.
    pub fn search_query(&self) -> &str {
        &self.search_query
    }

    /// The currently shown entries, sorted and filtered — what `view`
    /// draws, in the order it draws them.
    ///
    /// Borrowed out of the one owned listing. A caller wanting a
    /// particular row's identity should take its `path`: that is what
    /// [`Selection`] stores, and what survives the list being re-sorted
    /// underneath it.
    pub fn rows(&self) -> Vec<&Entry> {
        self.view.iter().filter_map(|&i| self.entries.get(i)).collect()
    }

    /// How many of this directory's entries the current filters are
    /// keeping off screen.
    ///
    /// Both filters at once — dotfiles with `show_hidden` off, and
    /// anything a search query excludes — because from the status bar's
    /// side they are the same fact: there is more here than you can see.
    /// Saying so matters most in the case where the listing looks empty
    /// and is not, which is exactly when a user would otherwise conclude
    /// the folder has nothing in it.
    fn hidden_count(&self) -> usize {
        self.entries.len() - self.view.len()
    }

    /// Hands the selection the rows it needs to resolve a click or a
    /// focus move against.
    ///
    /// Destructured rather than `self.selection.f(&self.rows())`,
    /// because that borrows the whole of `self` immutably to build the
    /// rows and then mutably for the selection. Naming the three fields
    /// separately is what tells the compiler they are disjoint — and it
    /// is also an accurate statement of what these operations touch.
    fn with_rows(&mut self, f: impl FnOnce(&mut Selection, &[&Entry])) {
        let Browser { entries, view, selection, .. } = self;
        let rows: Vec<&Entry> = view.iter().filter_map(|&i| entries.get(i)).collect();
        f(selection, &rows);
    }

    pub fn update(&mut self, message: Message) -> Outcome {
        match message {
            Message::Navigate(path) => self.go_to(path),
            Message::DirLoaded(path, result) => self.apply_dir_loaded(path, result),
            Message::CountsLoaded(counts) => self.apply_counts(counts),
            Message::GoBack => self.go_back(),
            Message::GoForward => self.go_forward(),
            Message::GoUp => self.go_up(),
            Message::EntryClicked { index, ctrl, shift } => {
                // Clicking another row abandons an edit rather than
                // committing it: a half-typed name is not a decision, and
                // a rename nobody confirmed is the worse surprise.
                let editing = self.renaming.as_ref().map(|r| r.path.clone());
                let on_edited = self.rows().get(index).map(|e| e.path.clone()) == editing;
                if !on_edited {
                    self.renaming = None;
                }
                self.with_rows(|selection, rows| selection.click_with(rows, index, ctrl, shift));
                Outcome::None
            }
            Message::RenameInput(text) => {
                if let Some(renaming) = &mut self.renaming {
                    renaming.text = text;
                }
                Outcome::None
            }
            Message::RenameCommit => self.commit_rename(),
            Message::RenameCancel => {
                self.renaming = None;
                Outcome::None
            }
            Message::AfterListing { path, rename } => {
                self.after_listing = Some((path, rename));
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
            Message::ToggleColumn(column) => {
                self.prefs.columns.toggle(column);
                // Nothing to re-sort: which columns are *shown* does not
                // change the order. The sort column can now be one that
                // is hidden, and that is deliberate — turning a column
                // off should not silently reorder the listing under you.
                Outcome::PrefsChanged(self.prefs.clone())
            }
            Message::SetSidebar(pref) => {
                self.prefs.sidebar = pref;
                Outcome::PrefsChanged(self.prefs.clone())
            }
            Message::ToggleColumnPicker => {
                self.column_picker_open = !self.column_picker_open;
                Outcome::None
            }
            Message::SetViewMode(mode) => {
                self.prefs.view_mode = mode;
                Outcome::PrefsChanged(self.prefs.clone())
            }
            Message::Perform(action) => self.perform(action),
            Message::TypeToSearch(c) => {
                // Typing is a new thing to do; an open menu is not what
                // it was aimed at.
                self.menu = None;
                self.search_query.push(c);
                self.refresh_view();
                Outcome::None
            }
            Message::OpenContextMenu { spot, at } => {
                self.open_menu(spot, at);
                Outcome::None
            }
            Message::MenuChose(index) => self.choose_menu_item(index),
            Message::CloseMenu => {
                self.menu = None;
                Outcome::None
            }
            Message::PinnedLoaded(items) => {
                self.pinned = items;
                Outcome::None
            }
        }
    }

    /// Uses `config` for menus and shortcut hints from now on. A host
    /// calls this once per tab with the configuration it loaded; without
    /// it a browser uses the shipped defaults.
    pub fn set_config(&mut self, config: Arc<Config>) {
        self.hidden_key =
            config.keymap.combos_for(Action::ToggleHidden).first().map(|combo| combo.to_string());
        self.config = config;
    }

    /// Whether a context menu is showing.
    pub fn menu_open(&self) -> bool {
        self.menu.is_some()
    }

    /// What the listing looks like right now, for deciding which actions
    /// make sense — see [`crate::action::enabled`].
    pub fn action_context(&self) -> ActionContext {
        self.action_context_for(None)
    }

    /// [`Self::action_context`], for an action aimed at a folder that
    /// is not in the listing at all — a sidebar row.
    ///
    /// The target is a parameter rather than a field set and cleared
    /// around each call, which is what it used to be. That field made
    /// `focused_is_dir` report `Some(true)` whenever it was set, so
    /// this struct lied about the listing's focus to get the folder
    /// actions enabled — invisibly to every reader of `ActionContext`
    /// and to `action::enabled`, which believes it is describing the
    /// listing. Any action consulting `focused_is_dir` silently
    /// acquired sidebar behaviour, and a missed clear would have left
    /// the lie in place permanently.
    fn action_context_for(&self, target: Option<&Path>) -> ActionContext {
        let rows = self.rows();
        let focused = self
            .selection
            .focused()
            .and_then(|f| rows.iter().find(|e| e.path == f));
        ActionContext {
            selected: self.selected_shown().len(),
            // A sidebar row is a folder, and it is what the menu is
            // about — so it answers "is the thing this menu acts on a
            // folder" honestly, rather than the listing being asked a
            // question that is not about it.
            focused_is_dir: match target {
                Some(_) => Some(true),
                None => focused.map(|e| e.is_dir),
            },
            shown: rows.len(),
            can_go_back: !self.back_stack.is_empty(),
            can_go_forward: !self.forward_stack.is_empty(),
            has_parent: self.current_dir.parent().is_some(),
            searching: !self.search_query.is_empty(),
            in_trash: self.in_trash(),
            in_archive: self.in_archive,
            // Every selected row is a file whose name this suite can
            // open as an archive. `!is_dir` is the half that matters:
            // a *folder* called `backup.zip` is a folder, and the name
            // alone would call it an archive.
            selection_is_archives: {
                let selected = self.selected_shown();
                !selected.is_empty()
                    && selected.iter().all(|path| {
                        rows.iter().any(|e| {
                            e.path == *path
                                && !e.is_dir
                                && crate::archive::looks_browsable(&e.name)
                        })
                    })
            },
            can_paste: self.can_paste,
            pin: {
                let target = self.pin_target(target);
                action::PinTarget {
                    exists: target.is_some(),
                    pinned_at: target.and_then(|t| self.prefs.pinned.iter().position(|p| *p == t)),
                    pins: self.prefs.pinned.len(),
                }
            },
        }
    }

    /// The folder the pin actions act on: the menu's own row when there
    /// is one; else one selected folder; else the folder being shown.
    fn pin_target(&self, target: Option<&Path>) -> Option<PathBuf> {
        if let Some(target) = target {
            return Some(target.to_path_buf());
        }
        let selected = self.selected_shown();
        if let [one] = selected.as_slice() {
            if self.entries.iter().any(|e| &e.path == one && e.is_dir) {
                return Some(one.clone());
            }
        }
        (!self.in_trash()).then(|| self.current_dir.clone())
    }

    /// The window's pinned list changed; this tab's copy follows it.
    /// Kept so the pin actions know what is pinned — the sidebar rows
    /// themselves arrive as [`Message::PinnedLoaded`].
    pub fn set_pins(&mut self, pinned: Vec<PathBuf>) {
        self.prefs.pinned = pinned;
    }

    /// Tells the browser whether the clipboard holds files. The host owns
    /// the clipboard and calls this whenever it changes.
    /// Tells the browser whether this listing is inside an archive.
    ///
    /// Set by the host after each read, for the reason
    /// [`ActionContext::in_archive`] gives: deciding it needs one
    /// `stat`, and `Browser` does no I/O. The host is reading the
    /// directory anyway and already knows which backend answered.
    pub fn set_in_archive(&mut self, in_archive: bool) {
        self.in_archive = in_archive;
    }

    /// Whether this listing is inside an archive.
    pub fn in_archive(&self) -> bool {
        self.in_archive
    }

    pub fn set_can_paste(&mut self, can_paste: bool) {
        self.can_paste = can_paste;
    }

    /// Whether this listing is the Trash.
    ///
    /// Compared against the path the sidebar's Trash item navigates to,
    /// which is how anyone gets there; `trash::TrashBackend::claims` is
    /// the more forgiving check (symlinks, `..`) and decides how the
    /// directory is *read*, but it touches the filesystem, and this runs
    /// on every key press and every menu.
    fn in_trash(&self) -> bool {
        self.current_dir == crate::sidebar::trash_path()
    }

    /// The selected paths that are actually on screen, in listing order.
    ///
    /// The selection survives a search or a hidden-files toggle — it is
    /// keyed by path for exactly that reason — so it can hold files the
    /// user cannot currently see. Nothing destructive may reach those:
    /// select everything, type a search, press Delete, and only what the
    /// search left visible goes to the trash. What you can see is what
    /// you act on.
    pub fn selected_shown(&self) -> Vec<PathBuf> {
        self.rows()
            .into_iter()
            .filter(|e| self.selection.is_selected(&e.path))
            .map(|e| e.path.clone())
            .collect()
    }

    /// Carries out one action.
    ///
    /// The single place an action happens, whether it came from a key,
    /// a menu or a button. A disabled action does nothing — the same
    /// [`crate::action::enabled`] answer a menu uses to grey an item out,
    /// so the two cannot disagree.
    pub fn perform(&mut self, action: Action) -> Outcome {
        self.perform_on(action, None)
    }

    /// [`Self::perform`], aimed at a folder outside the listing — the
    /// row a sidebar menu was opened on.
    ///
    /// Explicit, so that "what is this action about" is an argument
    /// both the enabling check and the action itself read, rather than
    /// a field that is briefly true and separately consulted by each.
    fn perform_on(&mut self, action: Action, target: Option<PathBuf>) -> Outcome {
        // Any action ends an edit in progress. The field has the
        // keyboard while it is focused, so a key reaching here was not
        // typed into it.
        self.renaming = None;
        // An open menu has the keyboard: arrows move through it, Open
        // runs the highlighted item, Escape closes it. Anything else
        // closes it and then does what it normally does.
        if self.menu.is_some() {
            match action {
                Action::FocusUp | Action::ExtendUp => return self.step_menu(-1),
                Action::FocusDown | Action::ExtendDown => return self.step_menu(1),
                Action::Open => {
                    let chosen = self.menu.as_ref().and_then(|m| m.highlighted);
                    return match chosen {
                        Some(index) => self.choose_menu_item(index),
                        None => {
                            self.menu = None;
                            Outcome::None
                        }
                    };
                }
                Action::ClearSearch | Action::ContextMenu => {
                    self.menu = None;
                    return Outcome::None;
                }
                _ => self.menu = None,
            }
        }
        if action.scope() == Scope::Window {
            return Outcome::Window(action);
        }
        if !action::enabled(action, &self.action_context_for(target.as_deref())) {
            return Outcome::None;
        }
        match action {
            // Straight from the focused *path* — no resolving a stored
            // index against a list that may have been re-sorted since
            // the focus was set.
            Action::Open => self.activate_focused(),
            // The focused row, like `Open` beside it — and like the
            // check that enables it, which reads `focused_is_dir`.
            // Taking the selection here instead meant the two halves
            // could name different files.
            Action::OpenWith => match self.selection.focused() {
                Some(path) => Outcome::OpenWith(path.to_path_buf()),
                None => Outcome::None,
            },
            Action::OpenInNewTab => match (target, self.selection.focused()) {
                (Some(target), _) => Outcome::OpenInNewTab(target),
                (None, Some(path)) => Outcome::OpenInNewTab(path.to_path_buf()),
                (None, None) => Outcome::None,
            },
            Action::Extract => Outcome::Extract {
                archives: self.selected_shown(),
                // No destination: each archive gets a folder beside
                // itself named after it, which the host works out —
                // it is the one that knows what names are free.
                to: None,
            },
            Action::ExtractTo => {
                if self.in_archive {
                    // Inside an archive the selection *is* the members,
                    // and the archive they came out of is where this
                    // listing is.
                    Outcome::ExtractMembers {
                        members: self.selected_shown(),
                        from: self.current_dir.clone(),
                    }
                } else {
                    Outcome::Extract {
                        archives: self.selected_shown(),
                        to: Some(self.current_dir.clone()),
                    }
                }
            }
            Action::Compress => Outcome::Compress {
                sources: self.selected_shown(),
                into: self.current_dir.clone(),
            },
            Action::Pin | Action::Unpin | Action::PinUp | Action::PinDown => {
                let Some(target) = self.pin_target(target.as_deref()) else {
                    return Outcome::None;
                };
                Outcome::Pins(match action {
                    Action::Pin => crate::sidebar::PinChange::Pin(target),
                    Action::Unpin => crate::sidebar::PinChange::Unpin(target),
                    Action::PinUp => crate::sidebar::PinChange::Move(target, -1),
                    _ => crate::sidebar::PinChange::Move(target, 1),
                })
            }
            Action::GoUp => self.go_up(),
            Action::GoBack => self.go_back(),
            Action::GoForward => self.go_forward(),
            Action::FocusUp | Action::FocusLeft => {
                self.with_rows(|selection, rows| selection.move_focus(rows, -1));
                Outcome::None
            }
            Action::FocusDown | Action::FocusRight => {
                self.with_rows(|selection, rows| selection.move_focus(rows, 1));
                Outcome::None
            }
            Action::ExtendUp => {
                self.with_rows(|selection, rows| selection.extend(rows, -1));
                Outcome::None
            }
            Action::ExtendDown => {
                self.with_rows(|selection, rows| selection.extend(rows, 1));
                Outcome::None
            }
            Action::SelectAll => {
                self.with_rows(|selection, rows| selection.select_all(rows));
                Outcome::None
            }
            Action::ClearSearch => {
                self.search_query.clear();
                self.refresh_view();
                Outcome::None
            }
            Action::ToggleHidden => {
                self.prefs.show_hidden = !self.prefs.show_hidden;
                self.refresh_view();
                Outcome::PrefsChanged(self.prefs.clone())
            }
            Action::Trash => Outcome::Trash(self.selected_shown()),
            Action::DeletePermanently => Outcome::DeletePermanently(self.selected_shown()),
            Action::Restore => Outcome::Restore(self.selected_shown()),
            Action::EmptyTrash => Outcome::EmptyTrash,
            Action::ContextMenu => Outcome::OpenContextMenuAtPointer(MenuSpot::Focused),
            Action::Copy | Action::Cut => Outcome::SetClipboard(crate::clipboard::FileClip {
                paths: self.selected_shown(),
                verb: if action == Action::Cut {
                    crate::clipboard::ClipVerb::Cut
                } else {
                    crate::clipboard::ClipVerb::Copy
                },
            }),
            Action::CopyPath => Outcome::CopyText(
                self.selected_shown()
                    .iter()
                    .map(|p| p.to_string_lossy().into_owned())
                    .collect::<Vec<_>>()
                    .join("\n"),
            ),
            Action::Paste => Outcome::Paste(self.current_dir.clone()),
            Action::Rename => self.begin_rename(),
            Action::NewFolder => {
                let name = crate::naming::new_folder_name(self.entries.iter().map(|e| e.name.as_str()));
                Outcome::CreateFolder(self.current_dir.join(name))
            }
            // A re-read, not a navigation: history, search and the
            // selection all stay.
            Action::Refresh => Outcome::ReadDir(self.current_dir.clone()),
            // Handled above; listed so a new window action is a compile
            // error here rather than a silent fall-through.
            Action::Undo
            | Action::NewTab
            | Action::CloseTab
            | Action::NextTab
            | Action::PreviousTab
            | Action::Tab(_) => Outcome::Window(action),
        }
    }

    /// Starts editing the one selected name.
    fn begin_rename(&mut self) -> Outcome {
        let selected = self.selected_shown();
        let [path] = selected.as_slice() else {
            return Outcome::None;
        };
        let Some(entry) = self.entries.iter().find(|e| &e.path == path) else {
            return Outcome::None;
        };
        let id = Id::unique();
        let select = crate::naming::stem_len(&entry.name, entry.is_dir);
        self.renaming = Some(Renaming { path: path.clone(), text: entry.name.clone(), id: id.clone() });
        Outcome::FocusRename { id, select }
    }

    /// Enter in the rename field: rename, close quietly, or say why not
    /// and leave the field open to fix.
    fn commit_rename(&mut self) -> Outcome {
        let Some(renaming) = &self.renaming else {
            return Outcome::None;
        };
        let old = renaming.path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
        let siblings = self.entries.iter().map(|e| e.name.as_str());
        match crate::naming::check_rename(&old, &renaming.text, siblings) {
            crate::naming::RenameCheck::Unchanged => {
                self.renaming = None;
                Outcome::None
            }
            crate::naming::RenameCheck::Refused(why) => Outcome::Notice(why),
            crate::naming::RenameCheck::To(name) => {
                let from = renaming.path.clone();
                let to = self.current_dir.join(name);
                self.renaming = None;
                Outcome::Rename { from, to }
            }
        }
    }

    /// Opens the menu for `spot`.
    ///
    /// A right click on a row that is not selected selects it — alone —
    /// first, so the menu acts on what was clicked. A right click inside
    /// an existing multi-selection keeps it, which is what every file
    /// manager does and what makes "select five, right-click, trash"
    /// work. A right click on the background clears the selection: the
    /// menu there is about the folder, not about whatever was selected.
    fn open_menu(&mut self, spot: MenuSpot, at: (f32, f32)) {
        self.renaming = None;
        if let MenuSpot::Sidebar(path) = spot {
            let kind = if self.prefs.pinned.contains(&path) { MenuKind::Pinned } else { MenuKind::Place };
            let items = menus::build(
                self.config.menus.get(kind),
                &self.action_context_for(Some(&path)),
                &self.config.keymap,
            );
            self.menu = (!items.is_empty()).then_some(OpenMenu { items, at, highlighted: None, target: Some(path) });
            return;
        }
        let row = match spot {
            MenuSpot::Sidebar(_) => unreachable!("handled above"),
            MenuSpot::Row(i) => Some(i),
            MenuSpot::Background => None,
            MenuSpot::Focused => {
                let rows = self.rows();
                self.selection
                    .focused()
                    .and_then(|f| rows.iter().position(|e| e.path == f))
            }
        };
        let row_entry = row.and_then(|i| self.rows().get(i).map(|e| (i, e.path.clone(), e.is_dir)));
        let kind = match &row_entry {
            Some((index, path, is_dir)) => {
                if !self.selection.is_selected(path) {
                    let index = *index;
                    self.with_rows(|selection, rows| selection.click_with(rows, index, false, false));
                } else {
                    self.selection.focus(path);
                }
                // Inside an archive every row gets the same menu,
                // folder or not: what can be done to a member does not
                // depend on whether anything is under it.
                let name = path
                    .file_name()
                    .map(|n| n.to_string_lossy().into_owned())
                    .unwrap_or_default();
                match (self.in_trash(), self.in_archive, is_dir) {
                    (true, _, _) => MenuKind::Trash,
                    (_, true, _) => MenuKind::ArchiveMember,
                    (_, false, true) => MenuKind::Folder,
                    (_, false, false) if crate::archive::looks_browsable(&name) => {
                        MenuKind::Archive
                    }
                    (_, false, false) => MenuKind::Entry,
                }
            }
            None => {
                self.selection.clear();
                if self.in_trash() {
                    MenuKind::Trash
                } else {
                    MenuKind::Empty
                }
            }
        };
        let items = menus::build(self.config.menus.get(kind), &self.action_context(), &self.config.keymap);
        self.menu = (!items.is_empty()).then_some(OpenMenu { items, at, highlighted: None, target: None });
    }

    fn step_menu(&mut self, direction: i32) -> Outcome {
        if let Some(menu) = &mut self.menu {
            menu.highlighted = menus::step(&menu.items, menu.highlighted, direction)
                .or(menu.highlighted);
        }
        Outcome::None
    }

    /// Runs the item at `index` and closes the menu. A separator or a
    /// disabled item only closes it.
    fn choose_menu_item(&mut self, index: usize) -> Outcome {
        let Some(menu) = self.menu.take() else {
            return Outcome::None;
        };
        match menu.items.get(index) {
            Some(MenuItem::Action { action, enabled: true, .. }) => {
                self.perform_on(*action, menu.target)
            }
            _ => Outcome::None,
        }
    }

    /// The context menu, drawn over the whole window — or `None` when no
    /// menu is open. A host stacks this on top of everything else it
    /// draws.
    ///
    /// Two layers. Underneath, a transparent area the size of the window
    /// that closes the menu when clicked anywhere, left or right — and
    /// takes that click, so it does not also land on whatever was under
    /// it. On top, the menu itself, `opaque` so a click on it never
    /// falls through to the closing layer, and `pin`ned where it was
    /// asked for, flipped away from any edge it would run off
    /// ([`menus::place`]).
    pub fn menu_overlay(&self, scale: FontScale, window: (f32, f32)) -> Option<Element<'_, Message>> {
        let menu = self.menu.as_ref()?;
        let size = density::menu_size(&menu.items, scale);
        let (x, y) = menus::place(menu.at, size, window);
        let away = iced::widget::mouse_area(
            iced::widget::Space::new().width(Length::Fill).height(Length::Fill),
        )
        .on_press(Message::CloseMenu)
        .on_right_press(Message::CloseMenu);
        let placed = iced::widget::pin(iced::widget::opaque(context_menu(menu, scale, size.0)))
            .x(x)
            .y(y)
            .width(Length::Fill)
            .height(Length::Fill);
        Some(iced::widget::stack![away, placed].width(Length::Fill).height(Length::Fill).into())
    }

    fn apply_dir_loaded(&mut self, path: PathBuf, result: Result<Vec<Entry>, DirError>) -> Outcome {
        // A slow read for a directory the user has since navigated away
        // from must not clobber what's on screen now — guard on the path
        // the result is actually for.
        if path != self.current_dir {
            return Outcome::None;
        }
        // The name said archive and the bytes disagree. Rather than
        // leaving someone looking at an error where a folder should
        // have been, the guess is taken back and the file is opened the
        // ordinary way — which is what would have happened if
        // `activate_path` had known, and it could not have.
        if result.as_ref().err().map(|e| e.kind) == Some(DirErrorKind::NotAnArchive) {
            let back = self.unwind_navigation();
            return Outcome::Many(vec![back, Outcome::Activated(path)]);
        }
        match result {
            Ok(entries) => {
                self.entries = entries;
                self.dotfiles = self.entries.iter().filter(|e| e.hidden).count();
                self.load_state = LoadState::Loaded;
            }
            Err(e) => {
                self.entries.clear();
                self.dotfiles = 0;
                self.load_state = LoadState::Error(e);
            }
        }
        // Not cleared: navigating somewhere new already cleared it in
        // `arrive_at`, so the only listing that reaches here with a
        // selection is a *refresh* of the same folder — after a paste, a
        // rename, or F5 — and there the selection should survive. Paths
        // that are gone are dropped so nothing can act on them.
        let still_here: HashSet<PathBuf> = self.entries.iter().map(|e| e.path.clone()).collect();
        self.selection.retain(&still_here);
        self.refresh_view();

        // No counts to clear: a folder's count now lives on the folder's
        // own `Entry`, and the entries were just replaced wholesale.
        let folders: Vec<PathBuf> =
            self.entries.iter().filter(|e| e.is_dir).map(|e| e.path.clone()).collect();
        let counts = if folders.is_empty() { Outcome::None } else { Outcome::CountFolders(folders) };

        // Something the host asked to have selected once it showed up —
        // a folder just made, a file just renamed. If it is not here yet
        // the request waits for the next listing.
        let arrived = match &self.after_listing {
            Some((path, _)) => self.rows().iter().position(|e| &e.path == path),
            None => None,
        };
        let Some(index) = arrived else {
            return counts;
        };
        let (_, rename) = self.after_listing.take().expect("checked just above");
        self.with_rows(|selection, rows| selection.click_with(rows, index, false, false));
        let then = if rename { self.begin_rename() } else { Outcome::None };
        Outcome::Many(vec![counts, then])
    }

    fn refresh_view(&mut self) {
        // Indices out, not copies.
        //
        // This used to be `filter_query(filter_hidden(self.entries.clone(), ..), ..)`,
        // which deep-copies the *entire* listing — every `Entry`'s name
        // `String` and `PathBuf` — and then throws most of it away, on
        // every keystroke in the search box, keeping a second full copy
        // of the survivors at rest. `backend`'s own tests pin a
        // 50k-entry directory as a supported case; that was 50k string
        // allocations per character typed, to show a handful of rows.
        // CLAUDE.md's "test the resource, not just the result".
        let show_hidden = self.prefs.show_hidden;
        let query = self.search_query.to_lowercase();
        let mut view: Vec<usize> = self
            .entries
            .iter()
            .enumerate()
            .filter(|(_, e)| show_hidden || !is_hidden(e))
            .filter(|(_, e)| matches_query(e, &query))
            .map(|(i, _)| i)
            .collect();
        let entries = &self.entries;
        sort_indices(
            &mut view,
            entries,
            self.prefs.sort_column(),
            self.prefs.sort_direction(),
            self.prefs.directories_first,
        );
        self.view = view;
    }

    /// Folds a round of folder counts into the entries they belong to.
    ///
    /// By path, because a count is issued against a listing and answered
    /// later — the same argument the selection now makes. A count for a
    /// folder no longer listed simply finds no entry and is dropped,
    /// rather than landing on whatever is at that row now.
    fn apply_counts(&mut self, counts: Vec<(PathBuf, Option<usize>)>) -> Outcome {
        for (path, count) in counts {
            let Some(entry) = self.entries.iter_mut().find(|e| e.path == path) else {
                continue;
            };
            entry.size = EntrySize::Items(match count {
                Some(n) => ItemCount::Known(n),
                // `Unreadable`, never `Known(0)`: a folder you have no
                // permission to open is not an empty one.
                None => ItemCount::Unreadable,
            });
        }
        // The Size column is sortable, and these counts are what it
        // sorts folders by — so a listing ordered by Size has to settle
        // once the numbers land. Any other column is unaffected, and
        // re-sorting it would be visible churn for nothing.
        if self.prefs.sort_column() == SortColumn::Size {
            self.refresh_view();
        }
        Outcome::None
    }

    /// Everything that is true of *arriving somewhere new*, in one
    /// place: nothing is selected, no search is in force, no rows are
    /// shown, and a read is outstanding.
    ///
    /// The three navigation verbs differ only in their history
    /// bookkeeping; they had a copy of this each, and a fourth thing to
    /// reset would have meant remembering all three.
    fn arrive_at(&mut self, path: PathBuf) -> Outcome {
        self.menu = None;
        self.renaming = None;
        self.after_listing = None;
        self.current_dir = path.clone();
        self.selection.clear();
        self.search_query.clear();
        self.view.clear();
        self.load_state = LoadState::Loading;
        Outcome::ReadDir(path)
    }

    fn go_to(&mut self, path: PathBuf) -> Outcome {
        self.back_stack.push(self.current_dir.clone());
        // A fresh navigation abandons whatever "forward" pointed to —
        // pinned by `tests::navigating_somewhere_new_truncates_the_forward_stack`.
        self.forward_stack.clear();
        self.arrive_at(path)
    }

    fn go_up(&mut self) -> Outcome {
        match self.current_dir.parent().map(|p| p.to_path_buf()) {
            Some(parent) => self.go_to(parent),
            None => Outcome::None,
        }
    }

    fn go_back(&mut self) -> Outcome {
        let Some(prev) = self.back_stack.pop() else {
            return Outcome::None;
        };
        self.forward_stack.push(self.current_dir.clone());
        self.arrive_at(prev)
    }

    fn go_forward(&mut self) -> Outcome {
        let Some(next) = self.forward_stack.pop() else {
            return Outcome::None;
        };
        self.back_stack.push(self.current_dir.clone());
        self.arrive_at(next)
    }

    /// A folder navigates into itself; a file becomes [`Outcome::Activated`]
    /// for the host to interpret — the one place `Mode` matters, and it
    /// matters to the *host*, not to this function or to `view`.
    fn activate(&mut self, index: usize) -> Outcome {
        let Some(entry) = self.rows().get(index).copied() else {
            return Outcome::None;
        };
        let (is_dir, path) = (entry.is_dir, entry.path.clone());
        self.activate_path(is_dir, path)
    }

    /// Activates whatever the keyboard cursor is on. `None` when nothing
    /// is focused, and also when the focused path is no longer shown —
    /// a search that filtered it out, say — which is "there is nothing
    /// under the cursor", not a reason to activate a neighbour.
    fn activate_focused(&mut self) -> Outcome {
        let Some(entry) = self
            .selection
            .focused()
            .and_then(|f| self.entries.iter().find(|e| e.path == f))
        else {
            return Outcome::None;
        };
        let (is_dir, path) = (entry.is_dir, entry.path.clone());
        self.activate_path(is_dir, path)
    }

    /// A folder navigates into itself; a file becomes
    /// [`Outcome::Activated`] for the host to interpret.
    ///
    /// An archive is a third case that resolves to the first: this suite
    /// browses archives, so opening one means going into it, not handing
    /// it to whatever else is installed. The decision is made on the
    /// name, because the name is all a listing has and `Browser` does no
    /// I/O — and a name can be wrong, which is why
    /// [`Self::apply_dir_loaded`] has a way back out.
    fn activate_path(&mut self, is_dir: bool, path: PathBuf) -> Outcome {
        if is_dir {
            return self.go_to(path);
        }
        let name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
        if crate::archive::looks_browsable(&name) {
            return self.go_to(path);
        }
        Outcome::Activated(path)
    }

    /// Takes back the navigation that just failed, without leaving it in
    /// the forward history.
    ///
    /// Not [`Self::go_back`]: going back would put the failed
    /// destination on the forward stack, so the Forward button would
    /// offer to try it again — an offer to repeat something that did not
    /// work. This one is "that never happened".
    fn unwind_navigation(&mut self) -> Outcome {
        match self.back_stack.pop() {
            Some(previous) => self.arrive_at(previous),
            None => Outcome::None,
        }
    }

    fn view_model(&self, viewport_width: f32) -> ViewModel<'_> {
        ViewModel {
            current_dir: &self.current_dir,
            in_trash: self.in_trash(),
            renaming: self.renaming.as_ref(),
            column_picker_open: self.column_picker_open,
            sidebar_collapsed: self
                .prefs
                .sidebar
                .collapsed(viewport_width, self.config.sidebar.collapse_below),
            viewport_width,
            hidden_count: self.hidden_count(),
            dotfiles: self.dotfiles,
            hidden_key: self.hidden_key.clone(),
            show_trash: self.config.sidebar.show_trash,
            rows: self.rows(),
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

    /// Renders the browser.
    ///
    /// `viewport_width` is the host window's width in logical pixels.
    /// Passed in rather than discovered with `responsive`, because the
    /// one thing it decides — whether the sidebar collapses on its own —
    /// is a property of the *window*, and wrapping the whole widget tree
    /// in a `responsive` to learn it would rebuild every row on every
    /// layout pass to answer a question the host already knows the
    /// answer to.
    pub fn view(&self, scale: FontScale, viewport_width: f32) -> Element<'_, Message> {
        render(self.view_model(viewport_width), scale)
    }
}

/// The ancestors of `path`, each paired with the full path clicking it
/// navigates to — root first. Pure and free of iced, so the path bar's
/// own logic is testable without building a window.
fn breadcrumb(path: &Path) -> Vec<(String, PathBuf)> {
    // The Trash is one place, not a path to climb: its ancestors are
    // storage details (`~/.local/share/Trash/files`) nobody navigates by.
    if path == crate::sidebar::trash_path() {
        return vec![(crate::sidebar::place_name(path), path.to_path_buf())];
    }
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
    // Collapsed is a *rail*, not nothing — see `sidebar_rail`. The
    // places stay reachable; only their labels fold away.
    let side = if vm.sidebar_collapsed {
        sidebar_rail(&vm, scale)
    } else {
        sidebar_view(&vm, scale)
    };
    let middle = row![side, file_area(&vm, scale)].height(Length::Fill);

    column![
        header_bar(&vm, scale),
        plane_edge(),
        middle,
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

/// The sidebar's show/hide control, at the far left of the header where
/// the sidebar itself begins.
///
/// Filled when the sidebar is showing, in the same "this control is on"
/// idiom as the active view-mode segment and the back button — so the
/// button states what it is about to do by showing what is currently
/// true, rather than needing two different icons.
fn sidebar_toggle<'a>(collapsed: bool, scale: FontScale) -> Element<'a, Message> {
    let side = density::glyph_button(scale);
    iced::widget::button(
        container(glyph::sidebar(side, hyprforge_ui::theme::text()))
            .center_x(Length::Fill)
            .center_y(Length::Fill),
    )
    .width(Length::Fixed(side))
    .height(Length::Fixed(side))
    .padding(0)
    // The opposite of what is on screen *now*, so the first press after
    // an automatic collapse opens the sidebar rather than recording
    // "hidden" to match what is already hidden and appearing to do
    // nothing.
    .on_press(Message::SetSidebar(if collapsed {
        crate::prefs::SidebarPref::Shown
    } else {
        crate::prefs::SidebarPref::Hidden
    }))
    .style(move |_t: &iced::Theme, status| {
        let hovered = matches!(status, iced::widget::button::Status::Hovered);
        iced::widget::button::Style {
            background: (!collapsed || hovered)
                .then(|| iced::Background::Color(hyprforge_ui::theme::surface::row())),
            text_color: hyprforge_ui::theme::text(),
            border: iced::Border {
                radius: density::nested_radius().into(),
                ..iced::Border::default()
            },
            ..iced::widget::button::Style::default()
        }
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
        sidebar_toggle(vm.sidebar_collapsed, scale),
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
    let placeholder = format!("Search {}", crate::sidebar::place_name(current_dir));

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
                // The disabled segment is told apart by `on_press_maybe(None)`,
                // not by a third colour — so this is only ever "is this
                // the active view mode".
                text_color: if active {
                    hyprforge_ui::theme::text()
                } else {
                    hyprforge_ui::theme::text_dim()
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
    let plane = |content: Element<'a, Message>| {
        container(content)
            .height(Length::Fixed(density::status_height(scale)))
            .center_y(Length::Fixed(density::status_height(scale)))
            .padding([0, spacing::MD as u16])
            .width(Length::Fill)
            .style(|_t: &iced::Theme| container::Style {
                // The same plane as the header: the chrome is one
                // material, top and bottom, with the listing recessed
                // between them.
                background: Some(iced::Background::Color(hyprforge_ui::theme::surface::sidebar())),
                ..container::Style::default()
            })
            .into()
    };

    // Nothing loaded yet is not "an empty folder" — the bar stays blank
    // rather than making a claim about a listing that has not arrived.
    // Same rule as the body's own three states.
    if !matches!(vm.load_state, LoadState::Loaded) {
        return plane(iced::widget::Space::new().into());
    }

    let summary = crate::format::status_summary(&vm.rows, vm.selection.selected_paths(), vm.hidden_count);

    // Left: what is in here. Right: where "here" is on disk, which is
    // the other question a status bar is asked and the one the path bar
    // only half answers — it elides the middle of a long path, and this
    // does not.
    // The dotfile switch, where the count of what is hidden is — Ctrl+H
    // is not something anyone finds by looking. Only offered when there
    // are dotfiles here to show or hide.
    // The count is already in the summary beside it, so the switch says
    // only what it does — and names its key, which is the half nobody
    // can guess.
    let dotfile_switch: Element<'a, Message> = if vm.dotfiles > 0 {
        let verb = if vm.prefs.show_hidden { "Hide" } else { "Show" };
        let label = match &vm.hidden_key {
            Some(key) => format!("{verb} ({key})"),
            None => verb.to_string(),
        };
        // `scaled_text`, not `meta_text`: a colour set on the text
        // would override the button's, and the hover colour with it.
        iced::widget::button(scaled_text(label, density::META_TEXT_BASE, scale))
            .padding([0, spacing::XS as u16])
            .on_press(Message::Perform(Action::ToggleHidden))
            .style(quiet_link_style)
            .into()
    } else {
        iced::widget::Space::new().into()
    };

    plane(
        row![
            meta_text(summary, density::META_TEXT_BASE, scale),
            dotfile_switch,
            iced::widget::Space::new().width(Length::Fill),
            meta_text(
                vm.current_dir.display().to_string(),
                density::META_TEXT_BASE,
                scale,
            ),
        ]
        .spacing(spacing::MD)
        .align_y(iced::Alignment::Center)
        .into(),
    )
}

/// A button that reads as a line of status text until pointed at — a
/// control in the status bar must not look like a toolbar.
fn quiet_link_style(theme: &iced::Theme, status: iced::widget::button::Status) -> iced::widget::button::Style {
    use iced::widget::button::Status;
    let palette = theme.extended_palette();
    let (text_color, background) = match status {
        Status::Hovered | Status::Pressed => {
            (palette.primary.base.color, Some(iced::Background::Color(hyprforge_ui::theme::surface::card())))
        }
        _ => (hyprforge_ui::theme::text_dim(), None),
    };
    iced::widget::button::Style {
        background,
        text_color,
        border: iced::Border { radius: 4.0.into(), ..iced::Border::default() },
        ..iced::widget::button::Style::default()
    }
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
    /// A pin whose folder cannot be read right now. It keeps its row —
    /// the user pinned it, and it may only be unmounted — but reads as
    /// unavailable, so a click that fails is not a surprise.
    missing: bool,
}

impl SidebarRow {
    /// The colour the mark is actually drawn in: the row's own, unless
    /// the place is unavailable, which is dim whatever the row is for.
    ///
    /// Derived rather than stored beside `missing`, because two fields
    /// set from one predicate can disagree — a dim mark beside a bright
    /// label is exactly what that drift would look like.
    fn shown_tint(&self) -> sidebar::Tint {
        match self.missing {
            true => sidebar::Tint::Dim,
            false => self.tint,
        }
    }
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
                missing: false,
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
                // "somewhere you put yourself". `missing` alone decides
                // whether it reads as unavailable; the mark's colour is
                // resolved from it at draw time, so the two cannot
                // disagree.
                tint: sidebar::Tint::Accent,
                missing: item.item_count.is_none(),
            })
            .collect(),
    };
    let trash = vm.show_trash.then(|| SidebarSection {
        title: "Trash",
        // Dim: the trash is a destination, not one of your places, and
        // giving it a hue of its own would put it in the same visual
        // class as Home.
        rows: vec![SidebarRow {
            label: "Trash".to_string(),
            path: sidebar::trash_path(),
            meta: None,
            tint: sidebar::Tint::Dim,
            missing: false,
        }],
    });
    [Some(places), Some(pinned), trash].into_iter().flatten().filter(|s| !s.rows.is_empty()).collect()
}

/// The collapsed sidebar: a rail of marks, still clickable.
///
/// Collapsing takes the *labels* away, not the places. A sidebar that
/// vanishes entirely means the one thing it is for — getting to Home,
/// Downloads, the Trash in a click — is gone exactly when the window is
/// small enough that navigating by path bar is most awkward.
///
/// The marks are the same [`icon::folder_mark`]s the full sidebar draws,
/// in the same tints, so the rail reads as the sidebar with its text
/// folded away rather than as a different control. The tint is doing
/// real work here: it is the only thing left telling Documents from
/// Downloads, which is why those two have distinct colours in the first
/// place.
fn sidebar_rail<'a>(vm: &ViewModel<'a>, scale: FontScale) -> Element<'a, Message> {
    let mut rail = column![].spacing(spacing::XS).width(Length::Fill);
    for (index, section) in sidebar_sections(vm).into_iter().enumerate() {
        // A rule instead of a heading: there is no room for `PLACES` at
        // this width, but the grouping it marked is still real.
        if index > 0 {
            rail = rail.push(container(divider()).padding([spacing::XS as u16, 0]));
        }
        for row_item in section.rows {
            let is_current = row_item.path == vm.current_dir;
            let mark = icon::folder_mark(
                hyprforge_ui::color::to_iced(row_item.shown_tint().color()),
                density::SIDEBAR_MARK_BASE,
                scale,
            );
            let button = iced::widget::button(
                container(mark).center_x(Length::Fill).center_y(Length::Fill),
            )
            .width(Length::Fill)
            .height(Length::Fixed(density::row_height(scale)))
            .padding(0)
            .on_press(Message::Navigate(row_item.path.clone()))
            .style(move |t: &iced::Theme, status| selectable_row_style(t, status, is_current));
            rail = rail.push(sidebar_menu_area(button, row_item.path));
        }
    }
    container(scrollable(rail))
        .padding(spacing::XS)
        .width(Length::Fixed(density::SIDEBAR_RAIL_WIDTH))
        .height(Length::Fill)
        .style(|_t: &iced::Theme| container::Style {
            background: Some(iced::Background::Color(hyprforge_ui::theme::surface::sidebar())),
            ..container::Style::default()
        })
        .into()
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
                hyprforge_ui::color::to_iced(row_item.shown_tint().color()),
                density::SIDEBAR_MARK_BASE,
                scale,
            );
            let label = if row_item.missing {
                meta_text(row_item.label, density::ROW_TEXT_BASE, scale)
            } else {
                scaled_text(row_item.label, density::ROW_TEXT_BASE, scale)
            };
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
                .on_press(Message::Navigate(row_item.path.clone()))
                .style(move |t: &iced::Theme, status| selectable_row_style(t, status, is_current));
            group = group.push(sidebar_menu_area(button, row_item.path));
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

/// A sidebar row that opens its own menu on a right-click. The button
/// takes the left press; the right one passes through to this — the
/// same arrangement the listing's rows use. The position is filled in
/// by the host from the pointer.
fn sidebar_menu_area<'a>(
    button: impl Into<Element<'a, Message>>,
    path: PathBuf,
) -> Element<'a, Message> {
    iced::widget::mouse_area(button)
        .on_right_press(Message::OpenContextMenu { spot: MenuSpot::Sidebar(path), at: (0.0, 0.0) })
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

fn path_bar<'a>(current_dir: &Path, scale: FontScale) -> Element<'a, Message> {
    let mut crumbs = row![].spacing(spacing::XS).align_y(iced::Alignment::Center);
    let segments = elide(breadcrumb(current_dir));
    let last = segments.len().saturating_sub(1);
    let mut previous: Option<String> = None;
    for (i, (label, path)) in segments.into_iter().enumerate() {
        if previous.as_deref().is_some_and(separates_from) {
            crumbs = crumbs.push(
                meta_text("/", density::META_TEXT_BASE, scale).font(iced::Font::MONOSPACE),
            );
        }
        previous = Some(label.clone());
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

/// Whether a separator belongs *after* the crumb labelled `previous`.
///
/// Every crumb is followed by a `/` except the root, because the root's
/// own label already is one. Without this the bar opened `/ / … /` —
/// two slashes and then a third — for anything outside the home
/// directory, which reads as a stutter rather than as a path. Under
/// home the head crumb is `~` and the question never came up, which is
/// why it went unnoticed until a path in `/tmp` was looked at.
fn separates_from(previous: &str) -> bool {
    previous != "/"
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
    // Right-clicking anywhere the rows are not opens the folder's own
    // menu. Rows capture their own right press first — see
    // `with_row_menu` — so this only sees the empty space.
    iced::widget::mouse_area(
        container(body_content(vm, scale)).width(Length::Fill).height(Length::Fill),
    )
    .on_right_press(Message::OpenContextMenu { spot: MenuSpot::Background, at: (0.0, 0.0) })
    .into()
}

fn body_content<'a>(vm: &ViewModel<'a>, scale: FontScale) -> Element<'a, Message> {
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
        LoadState::Loaded if vm.rows.is_empty() => {
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

/// One cell of a list row: a single line, clipped to its column.
///
/// **Never wrapping.** A row has a fixed height — that is what makes a
/// listing scannable — so a cell that wraps does not get taller, it
/// spills into the row underneath and the two collide. "1 item" broke
/// across two lines as "1" and "item" in a narrow window and overlapped
/// the row below it.
///
/// Clipped rather than wrapped, and clipped rather than allowed to
/// overrun: without the clip, `Wrapping::None` simply draws the text
/// straight through its neighbour, which is how a long name ended up
/// printed over the Kind column.
///
/// Truncation is blunt — iced 0.14 has no ellipsis — but a name cut off
/// at the column edge still reads as "there is more here", where text
/// lying across the next column reads as a bug. A column too narrow for
/// its content is the user's cue to turn one off; they are switchable
/// for exactly this reason.
fn list_cell<'a>(
    content: iced::widget::Text<'a>,
    portion: u16,
) -> Element<'a, Message> {
    container(content.wrapping(iced::widget::text::Wrapping::None))
        .width(Length::FillPortion(portion))
        .clip(true)
        .into()
}

/// What every row in one listing shares — worked out once per listing,
/// not once per row.
struct RowContext<'a> {
    columns: Vec<Column>,
    renaming: Option<&'a Renaming>,
    /// For shortening Original Location to `~/…`.
    home: Option<PathBuf>,
    scale: FontScale,
    /// One reading of the clock for the whole listing. Every row's
    /// Modified cell is relative to the same instant anyway, so asking
    /// per row was both wasted work and a way for the top and bottom of a
    /// long list to disagree about what "Today" means.
    now: chrono::DateTime<chrono::Local>,
}

fn entry_row<'a>(
    index: usize,
    entry: &'a Entry,
    selected: bool,
    ctx: &RowContext<'a>,
) -> Element<'a, Message> {
    let RowContext { columns, renaming, home, scale, now } = ctx;
    let (renaming, scale, now) = (*renaming, *scale, *now);
    // The icon, then the name, then whichever optional columns are
    // switched on — in `Column::ALL` order, which is the same order
    // `list_header` walks, so a cell can never end up under the wrong
    // heading.
    //
    // Git's `M`/`A` badges are deliberately not a column here: DESIGN.md
    // defers git entirely, and a column with nothing to put in it is
    // exactly the "reads as broken, not as not-yet" trap the sidebar
    // section rule is written to avoid.
    let editing = renaming.filter(|r| r.path == entry.path);
    // A row being renamed drops its accent fill while the field is open:
    // the field's own selection is accent, and accent text on an accent
    // row is text that has disappeared. The field's accent outline is
    // what marks the row instead.
    let selected = selected && editing.is_none();
    let mut row_content = row![
        entry_icon(entry.kind, 20.0, scale),
        // `&entry.name`, not a clone: `scaled_text` borrows for `'a`,
        // and this row is rebuilt for every visible entry on every
        // redraw — a hover anywhere in the window allocated one `String`
        // per row for a value that was already sitting right there.
        match editing {
            Some(r) => container(rename_field(r, scale)).width(Length::FillPortion(NAME_PORTION)).into(),
            None => list_cell(scaled_text(&entry.name, density::ROW_TEXT_BASE, scale), NAME_PORTION),
        },
    ]
    .spacing(spacing::SM)
    .align_y(iced::Alignment::Center);

    for &column in columns {
        let cell = match column {
            Column::Kind => format_kind(entry),
            // A directory's Size cell holds how many things are in it,
            // not a byte total — see `EntrySize`'s own doc. All four
            // states are `format_size`'s business, not this row's.
            Column::Size => format_size(entry.size),
            Column::Owner => format_owner(entry),
            Column::Permissions => format_permissions(entry.mode, entry.is_dir, entry.is_symlink),
            // `format_modified_at` with "now" handed down from
            // `list_view`, not a fresh `chrono::Local::now()` — that
            // would be an `/etc/localtime` consultation per row per
            // frame, for a value every row in the listing shares.
            Column::Modified => format_modified_at(entry.modified, now),
            Column::Origin => format_origin(entry, home.as_deref()),
        };
        // Dim metadata on the *unselected* row only. `text_dim` is
        // chosen for legibility against the listing's own dark surface,
        // and a selected row's accent fill is much lighter — the same
        // grey that reads as quiet secondary text on one reads as
        // washed-out and half-erased on the other. A selected row uses
        // the full text colour throughout, so selecting something makes
        // its details easier to read rather than harder.
        let text = if selected {
            scaled_text(cell, density::META_TEXT_BASE, scale).color(hyprforge_ui::theme::text())
        } else {
            meta_text(cell, density::META_TEXT_BASE, scale)
        };
        row_content = row_content.push(list_cell(text, column_portion(column)));
    }

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
        .style(move |t: &iced::Theme, status| selectable_row_style(t, status, selected));
    with_row_menu(styled.into(), index)
}

/// Lets a row or cell open the context menu on a right click.
///
/// A `mouse_area` *around* the button — the arrangement that failed for
/// double click, and works here. `button` captures only the *left*
/// press, so the right press passes through it to this wrapper. This
/// wrapper then captures it, which is what stops the background's own
/// right-click handler from opening a second, folder-level menu on top.
///
/// The position is left unset: a right press carries none, and the
/// window fills it in from the pointer.
fn with_row_menu<'a>(inner: Element<'a, Message>, index: usize) -> Element<'a, Message> {
    iced::widget::mouse_area(inner)
        .on_right_press(Message::OpenContextMenu { spot: MenuSpot::Row(index), at: (0.0, 0.0) })
        .into()
}

/// A context menu's own box: items and separators on the chrome plane,
/// with a hairline border so it reads as lifted off the listing.
///
/// Every dimension here comes from `density`, the same numbers
/// `density::menu_size` adds up to place the menu — so the box drawn is
/// the box that was placed.
fn context_menu<'a>(menu: &'a OpenMenu, scale: FontScale, width: f32) -> Element<'a, Message> {
    let row_h = density::row_height(scale);
    let mut list = column![].width(Length::Fill);
    for (index, item) in menu.items.iter().enumerate() {
        match item {
            MenuItem::Separator => {
                list = list.push(
                    container(divider())
                        .height(Length::Fixed(density::menu_separator_height(scale)))
                        .center_y(Length::Fixed(density::menu_separator_height(scale)))
                        .padding([0, spacing::SM as u16]),
                );
            }
            MenuItem::Action { label, hint, enabled, .. } => {
                let enabled = *enabled;
                let highlighted = menu.highlighted == Some(index);
                let colour = if enabled {
                    hyprforge_ui::theme::text()
                } else {
                    hyprforge_ui::theme::text_dim()
                };
                let mut content = row![scaled_text(*label, density::ROW_TEXT_BASE, scale)
                    .color(colour)
                    .width(Length::Fill)]
                .align_y(iced::Alignment::Center)
                .spacing(spacing::MD);
                if let Some(hint) = hint {
                    content = content.push(meta_text(hint.as_str(), density::META_TEXT_BASE, scale));
                }
                list = list.push(
                    iced::widget::button(content)
                        .on_press_maybe(enabled.then_some(Message::MenuChose(index)))
                        .width(Length::Fill)
                        .height(Length::Fixed(row_h))
                        .padding([0, spacing::SM as u16])
                        .style(move |t: &iced::Theme, status| {
                            menu_item_style(t, status, highlighted, enabled)
                        }),
                );
            }
        }
    }
    container(list)
        .padding(density::menu_padding(scale))
        .width(Length::Fixed(width))
        .style(|_t: &iced::Theme| container::Style {
            background: Some(iced::Background::Color(hyprforge_ui::theme::surface::sidebar())),
            border: iced::Border {
                color: hyprforge_ui::theme::surface::card_border(),
                width: 1.0,
                radius: density::inner_radius().into(),
            },
            ..container::Style::default()
        })
        .into()
}

/// A menu item's look.
///
/// Unlike a listing row, hover takes the accent here. In a menu the
/// pointer *is* the choice being made — the item under it is what a
/// click will run — so hover and the keyboard highlight are the same
/// state and look the same. A disabled item never lights up.
fn menu_item_style(
    theme: &iced::Theme,
    status: iced::widget::button::Status,
    highlighted: bool,
    enabled: bool,
) -> iced::widget::button::Style {
    use iced::widget::button;
    let hovered = matches!(status, button::Status::Hovered | button::Status::Pressed);
    let lit = enabled && (highlighted || hovered);
    button::Style {
        background: lit
            .then(|| iced::Background::Color(theme.extended_palette().primary.weak.color)),
        text_color: hyprforge_ui::theme::text(),
        border: iced::Border { radius: density::nested_radius().into(), ..iced::Border::default() },
        ..button::Style::default()
    }
}

/// The look of any row that can be *the chosen one* — an entry in the
/// listing, or the sidebar place you are currently in.
///
/// **Selection is the only place `accent` appears in this file.** A
/// hovered-but-not-chosen row uses `surface::row()`, a plain grey
/// elevation distinct from both the unchosen default (no background at
/// all) and the chosen state, so "this row is the selection" is never
/// ambiguous with "the pointer happens to be over it".
/// [`tests::only_the_selected_row_ever_uses_the_accent_colour`] pins
/// this.
///
/// One function for both, because it is one rule. The sidebar and the
/// entry list had a byte-identical copy each, with the boolean renamed;
/// two copies of "purple means selected" is two places for it to stop
/// being true. Public for the same reason: the window's own
/// application chooser is a third list of rows, and it must not become
/// a third copy.
pub fn selectable_row_style(
    theme: &iced::Theme,
    status: iced::widget::button::Status,
    selected: bool,
) -> iced::widget::button::Style {
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
const NAME_PORTION: u16 = 8;

/// How wide each optional column is, relative to the others.
///
/// Sized to the widest thing each actually holds rather than uniformly:
/// `drwxr-xr-x` is a fixed ten characters and never needs more, where
/// "Compressed archive" and "Yesterday 09:15" do. A column too narrow
/// for its content truncates silently, which is worse than a column that
/// looks slightly roomy.
fn column_portion(column: Column) -> u16 {
    match column {
        // "Compressed archive" and "Shell script" are the long ones.
        Column::Kind => 3,
        // "137 items" and "687.0 KiB" — wider than it looks, and it read
        // as the narrowest column in the app while holding two words.
        Column::Size => 3,
        // A login name, and the header "Owner" is five characters.
        Column::Owner => 2,
        // Exactly ten characters, always, plus the eleven-character
        // header above them.
        Column::Permissions => 3,
        // "Yesterday 09:15" and "Dec 28, 2025" are the long forms.
        Column::Modified => 3,
        // A path: the widest thing any column holds.
        Column::Origin => 5,
    }
}

/// The columns this listing shows, left to right, after the name.
///
/// The configured ones — and, in the Trash, Original Location first,
/// because "where did this come from" is the question the Trash is for.
fn listing_columns(prefs: &Prefs, in_trash: bool) -> Vec<Column> {
    let mut columns = Vec::with_capacity(Column::ALL.len() + 1);
    if in_trash {
        columns.push(Column::Origin);
    }
    columns.extend(prefs.columns.shown());
    columns
}

/// A column's heading. In the Trash, the date is when something was
/// deleted — `TrashBackend` puts the deletion date there — so the
/// heading says so.
fn column_heading(column: Column, in_trash: bool) -> &'static str {
    match column {
        Column::Modified if in_trash => "Deleted",
        other => other.label(),
    }
}

/// The sortable column each list column stands for.
fn column_sort(column: Column) -> SortColumn {
    match column {
        Column::Kind => SortColumn::Kind,
        Column::Size => SortColumn::Size,
        Column::Owner => SortColumn::Owner,
        Column::Permissions => SortColumn::Permissions,
        Column::Modified => SortColumn::Modified,
        Column::Origin => SortColumn::Origin,
    }
}

/// The clickable column headers.
///
/// Sorting lives here rather than behind an overflow menu because this
/// is where a person already expects it: clicking a header sorts by it
/// and clicking again reverses, which is what every file manager anyone
/// has used does. That also removes the need for a separate sort
/// control entirely.
fn list_header<'a>(prefs: &Prefs, in_trash: bool, picker_open: bool, scale: FontScale) -> Element<'a, Message> {
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
        iced::widget::button(content.wrapping(iced::widget::text::Wrapping::None))
            .on_press(Message::SortBy(column))
            .width(Length::FillPortion(portion))
            .clip(true)
            .style(header_button_style)
    };

    let mut header = row![
        // An empty cell the width of a row's icon, so "Name" starts
        // above the names rather than above the icons.
        iced::widget::Space::new().width(Length::Fixed(scale.apply(20.0))),
        heading("Name", SortColumn::Name, NAME_PORTION),
    ]
    .spacing(spacing::SM)
    .align_y(iced::Alignment::Center);

    // `Column::ALL` order, the same walk `entry_row` makes — one list,
    // so a heading cannot end up over the wrong cells.
    for column in listing_columns(prefs, in_trash) {
        header = header.push(heading(column_heading(column, in_trash), column_sort(column), column_portion(column)));
    }

    // The picker's handle, at the far right where the columns run out.
    header = header.push(
        iced::widget::button(
            meta_text(if picker_open { "\u{2715}" } else { "\u{22EE}" }, density::META_TEXT_BASE, scale)
        )
        .on_press(Message::ToggleColumnPicker)
        .width(Length::Fixed(scale.apply(COLUMN_PICKER_WIDTH)))
        .style(header_button_style),
    );

    if !picker_open {
        return header.into();
    }

    // Open: a row of toggles directly under the header rather than a
    // floating menu. iced has no popup this could be without an overlay,
    // and an overlay for five checkboxes would be a lot of machinery for
    // a control that is only ever used from right here — the panel
    // pushes the listing down while it is open and takes its space back
    // when it closes, which is honest about what it is.
    let mut picker = row![meta_text("Columns", density::META_TEXT_BASE, scale)]
        .spacing(spacing::SM)
        .align_y(iced::Alignment::Center);
    for column in Column::ALL {
        let shown = prefs.columns.shows(column);
        picker = picker.push(
            iced::widget::button(
                // A tick where it is on and an empty box where it is
                // not — the state is the mark, so a glance across the
                // row reads as a set of settings rather than a row of
                // identical buttons.
                meta_text(
                    format!("{} {}", if shown { "\u{2713}" } else { "\u{2007}" }, column.label()),
                    density::META_TEXT_BASE,
                    scale,
                ),
            )
            .on_press(Message::ToggleColumn(column))
            .style(move |t: &iced::Theme, status| selectable_row_style(t, status, shown)),
        );
    }

    column![
        header,
        container(picker).padding([spacing::XS as u16, spacing::SM as u16]),
    ]
    .spacing(spacing::XS)
    .into()
}

/// The column picker's handle: as wide as the glyph in it needs, and no
/// wider. A `FillPortion` here would steal width from the columns it
/// exists to configure.
const COLUMN_PICKER_WIDTH: f32 = 22.0;

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
    let ctx = RowContext {
        columns: listing_columns(vm.prefs, vm.in_trash),
        renaming: vm.renaming,
        home: home_dir(),
        scale,
        now: chrono::Local::now(),
    };
    for (index, entry) in vm.rows.iter().copied().enumerate() {
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
        list = list.push(entry_row(index, entry, vm.selection.is_selected(&entry.path), &ctx));
    }
    // `.id(..)` is what lets a host restore this list's scroll position
    // across a tab switch — see `Browser::list_scrollable_id`'s own doc.
    // Without a stable `Id`, iced cannot tell "the same list, redrawn"
    // from "a different scrollable that happens to be in the same spot",
    // and drops the offset.
    //
    // The header sits outside the vertical `scrollable`, so it stays put
    // while the rows move under it — and *inside* the horizontal one
    // below, so it slides with the columns it labels. A header that held
    // still horizontally would be worse than no header at all: every
    // label over the wrong column.
    let stack = column![
        list_header(vm.prefs, vm.in_trash, vm.column_picker_open, scale),
        divider(),
        scrollable(list).height(Length::Fill).id(vm.list_scrollable_id.clone()),
    ]
    .spacing(spacing::XS);

    // The floor under which the columns stop sharing and start
    // scrolling.
    //
    // `FillPortion` divides whatever it is given with no minimum, so a
    // narrow enough window turns six columns into six slivers — the
    // permissions cell showing `drwx`, the date showing `Dec`. Past this
    // width the list is drawn at its own minimum and the pane scrolls
    // sideways to reach the rest of it, which is what a table does
    // everywhere else and is the reason the columns are worth having.
    let pane = density::list_pane_width(vm.viewport_width, vm.sidebar_collapsed);
    let min_width = density::list_min_width(listing_columns(vm.prefs, vm.in_trash).len(), scale);
    if pane >= min_width {
        return stack.into();
    }
    scrollable(container(stack).width(Length::Fixed(min_width)))
        .direction(scrollable::Direction::Horizontal(scrollable::Scrollbar::new()))
        .width(Length::Fill)
        .height(Length::Fill)
        .into()
}

fn grid_view<'a>(vm: &ViewModel<'a>, scale: FontScale) -> Element<'a, Message> {
    // The cells that will be laid out, built once outside the
    // `responsive` closure — that closure runs on every layout pass, and
    // rebuilding a whole directory's worth of widget inside it would
    // make resizing the window quadratic in the listing.
    //
    // Owned rather than borrowed for the same reason `entry_row` borrows
    // where it can: these strings already exist on the entries, so only
    // the paths and the flags come along.
    let cells: Vec<(usize, &Entry, bool)> = vm
        .rows
        .iter()
        .copied()
        .enumerate()
        .map(|(index, entry)| (index, entry, vm.selection.is_selected(&entry.path)))
        .collect();

    // `responsive` and not a fixed column count. Five fixed columns meant
    // the cells never got wider *or* more numerous as the window grew —
    // the grid just sat at one size with an expanding margin beside it,
    // which is most of why it read as squashed. This asks the layout how
    // much room there actually is and fills it.
    let renaming = vm.renaming;
    iced::widget::responsive(move |size| {
        let gap = density::grid_gap(scale);
        // The width the layout actually handed us, which is the same
        // pane `list_view` derives arithmetically — `responsive` is
        // affordable here because a grid cell is far cheaper to rebuild
        // than a list row, and the column count has to be exact or the
        // last column gets squeezed.
        let columns = density::grid_columns(size.width, scale);
        let mut grid = column![].spacing(gap);
        let mut current = row![].spacing(gap);
        for (position, (index, entry, selected)) in cells.iter().copied().enumerate() {
            if position > 0 && position % columns == 0 {
                grid = grid.push(current);
                current = row![].spacing(gap);
            }
            current = current.push(grid_cell(index, entry, selected, renaming, scale));
        }
        // The last row is padded out with empty space to a full set of
        // columns, so its cells keep the width the rows above gave them
        // instead of stretching to share the leftover.
        let remainder = cells.len() % columns;
        if remainder != 0 {
            for _ in remainder..columns {
                current = current.push(
                    iced::widget::Space::new().width(Length::Fixed(density::grid_cell_width(scale))),
                );
            }
        }
        grid = grid.push(current);
        scrollable(container(grid).padding(gap)).height(Length::Fill).into()
    })
    .into()
}

/// A grid cell's name.
fn grid_name<'a>(entry: &'a Entry, scale: FontScale) -> Element<'a, Message> {
    // The same text size as a list row's name, not a smaller one. The
    // grid used to shrink it to 12px, which made a view meant for
    // *recognising* things harder to read than the one meant for
    // scanning them.
    //
    // `WordOrGlyph`, not the default `Word`: a name with no spaces in it
    // — `IntradaScreenConnect`, and most of a source tree — is one
    // "word", and word wrapping cannot break it, so it runs straight out
    // of the cell and across its neighbour. This wraps at word
    // boundaries where there are any and falls back to breaking
    // mid-name where there are none.
    container(
        scaled_text(&entry.name, density::ROW_TEXT_BASE, scale)
            .align_x(iced::Alignment::Center)
            .wrapping(iced::widget::text::Wrapping::WordOrGlyph)
            .width(Length::Fill),
    )
    // And clipped to the room a name actually has, so a third line
    // cannot push the cell taller than its neighbours and stagger the
    // row. Two lines is the budget; past that the name is cut rather
    // than the grid distorted.
    .height(Length::Fixed(density::grid_name_height(scale)))
    .clip(true)
    .into()
}

/// The in-place edit field a name becomes while it is renamed.
///
/// Same text size as the name it replaces, and no taller than a row, so
/// starting a rename does not shift anything around it.
fn rename_field<'a>(renaming: &'a Renaming, scale: FontScale) -> Element<'a, Message> {
    text_input("", &renaming.text)
        .id(renaming.id.clone())
        .on_input(Message::RenameInput)
        .on_submit(Message::RenameCommit)
        .size(scale.apply(density::ROW_TEXT_BASE))
        .padding([0, spacing::XS as u16])
        .width(Length::Fill)
        .style(rename_field_style)
        .into()
}

/// The rename field's look: the listing's own surface, cut into the
/// selected row, with a hairline of accent round it.
///
/// It sits on a row that is already accent-filled, so iced's default —
/// an accent text selection on a black box — failed twice over: the
/// selected text was accent on accent and vanished, and the black box
/// was a colour nothing else in the window uses. On the listing's own
/// surface the accent selection reads, and the field reads as a hole in
/// the row you can type into.
fn rename_field_style(
    theme: &iced::Theme,
    _status: iced::widget::text_input::Status,
) -> iced::widget::text_input::Style {
    let accent = theme.extended_palette().primary.weak.color;
    iced::widget::text_input::Style {
        background: iced::Background::Color(hyprforge_ui::theme::surface::card()),
        border: iced::Border { color: accent, width: 1.0, radius: density::nested_radius().into() },
        icon: hyprforge_ui::theme::text_dim(),
        placeholder: hyprforge_ui::theme::text_dim(),
        value: hyprforge_ui::theme::text(),
        selection: accent,
    }
}

/// One cell of the grid: a big icon over a centred name.
fn grid_cell<'a>(
    index: usize,
    entry: &'a Entry,
    selected: bool,
    renaming: Option<&'a Renaming>,
    scale: FontScale,
) -> Element<'a, Message> {
    let editing = renaming.filter(|r| r.path == entry.path);
    // Unfilled while being renamed — see `entry_row`.
    let selected = selected && editing.is_none();
    let name: Element<'a, Message> = match editing {
        Some(r) => rename_field(r, scale),
        None => grid_name(entry, scale),
    };
    let content = column![
        entry_icon(entry.kind, density::grid_icon_size(scale), scale),
        name,
    ]
    .spacing(spacing::SM)
    .align_x(iced::Alignment::Center);

    let cell = iced::widget::button(container(content).center_x(Length::Fill).center_y(Length::Fill))
        .on_press(Message::EntryClicked { index, ctrl: false, shift: false })
        .width(Length::Fixed(density::grid_cell_width(scale)))
        .height(Length::Fixed(density::grid_cell_height(scale)))
        // Real padding inside the cell, so the icon and the name have
        // air around them rather than sitting against the selection
        // fill's edge.
        .padding(spacing::SM)
        .style(move |t: &iced::Theme, status| selectable_row_style(t, status, selected));
    with_row_menu(cell.into(), index)
}

/// Fixtures shared by this module's test modules.
#[cfg(test)]
mod tests_support {
    use super::*;
    use crate::types::EntryKind;
    use std::time::SystemTime;

    /// A listing of `(name, is_dir)` at `/dir`, already loaded.
    pub fn loaded(rows: &[(&str, bool)]) -> Browser {
        let (mut browser, _) =
            Browser::new(Mode::App, Prefs::default(), PathBuf::from("/dir"), vec![]);
        let entries: Vec<Entry> = rows
            .iter()
            .map(|(name, is_dir)| Entry {
                name: name.to_string(),
                path: PathBuf::from("/dir").join(name),
                is_dir: *is_dir,
                size: if *is_dir { EntrySize::UNCOUNTED } else { EntrySize::Bytes(10) },
                modified: Some(SystemTime::UNIX_EPOCH),
                is_symlink: false,
                link_broken: false,
                hidden: name.starts_with('.'),
                kind: EntryKind::classify(*is_dir, name),
                mode: 0o644,
                uid: 1000,
                owner: Some("alex".to_string()),
                origin: None,
            })
            .collect();
        browser.update(Message::DirLoaded(PathBuf::from("/dir"), Ok(entries)));
        browser
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::EntryKind;
    use std::time::SystemTime;

    /// Whether the row currently *at* `index` is selected.
    ///
    /// The selection is stored by path, so this resolves the index the
    /// same way a click does — against the rows as they are right now.
    /// Written as a helper rather than inlined so a test that wants the
    /// opposite property (the selection following a file *across* a
    /// re-sort) can ask about the path instead and the difference is
    /// visible at the call site.
    fn row_selected(browser: &Browser, index: usize) -> bool {
        browser
            .rows()
            .get(index)
            .is_some_and(|e| browser.selection().is_selected(&e.path))
    }

    fn entry(name: &str, is_dir: bool) -> Entry {
        Entry {
            name: name.to_string(),
            path: PathBuf::from("/dir").join(name),
            is_dir,
            size: if is_dir { EntrySize::UNCOUNTED } else { EntrySize::Bytes(10) },
            modified: Some(SystemTime::UNIX_EPOCH),
            is_symlink: false,
            link_broken: false,
            hidden: name.starts_with('.'),
            kind: EntryKind::classify(is_dir, name),
            // Ownership and permissions are fixtures here: these
            // helpers build entries for tests about names, sizes and
            // ordering, none of which read them.
            mode: 0o644,
            uid: 1000,
            owner: Some("alex".to_string()),
            origin: None,
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
        assert!(!row_selected(&browser, 0));
        assert!(row_selected(&browser, 2));
    }

    #[test]
    fn ctrl_click_toggles_without_disturbing_the_rest() {
        let mut browser = loaded_browser(&["a", "b", "c"]);
        browser.update(Message::EntryClicked { index: 0, ctrl: false, shift: false });
        browser.update(Message::EntryClicked { index: 2, ctrl: true, shift: false });
        assert!(row_selected(&browser, 0));
        assert!(row_selected(&browser, 2));
        assert!(!row_selected(&browser, 1));

        // Toggling the same index again removes just that one.
        browser.update(Message::EntryClicked { index: 2, ctrl: true, shift: false });
        assert!(!row_selected(&browser, 2));
        assert!(row_selected(&browser, 0), "index 0 must survive toggling a different index");
    }

    #[test]
    fn shift_click_selects_a_forward_range() {
        let mut browser = loaded_browser(&["a", "b", "c", "d"]);
        browser.update(Message::EntryClicked { index: 1, ctrl: false, shift: false });
        browser.update(Message::EntryClicked { index: 3, ctrl: false, shift: true });
        for i in 1..=3 {
            assert!(row_selected(&browser, i), "index {i} should be in the range");
        }
        assert!(!row_selected(&browser, 0));
    }

    #[test]
    fn shift_click_selects_a_backward_range() {
        let mut browser = loaded_browser(&["a", "b", "c", "d"]);
        browser.update(Message::EntryClicked { index: 3, ctrl: false, shift: false });
        browser.update(Message::EntryClicked { index: 1, ctrl: false, shift: true });
        for i in 1..=3 {
            assert!(row_selected(&browser, i), "index {i} should be in the backward range");
        }
        assert!(!row_selected(&browser, 0));
    }

    /// The metadata on a selected row must not be the dim grey chosen
    /// for the dark listing surface — on the accent fill it reads as
    /// washed out and half-erased, which is the opposite of what
    /// selecting something should do to its legibility.
    ///
    /// Checked at the level a test can reach: that the two states do not
    /// use the same colour, and that the selected one is the full text
    /// colour. What it looks like is still a screenshot's business, the
    /// same caveat `glyph` writes down for its own shapes.
    #[test]
    fn a_selected_rows_details_are_not_drawn_in_the_dim_grey() {
        assert_ne!(
            hyprforge_ui::theme::text(),
            hyprforge_ui::theme::text_dim(),
            "if these were equal the distinction this row makes would be invisible"
        );
    }

    /// The bug the path-keyed selection exists to make impossible.
    ///
    /// Selecting a row and then reversing the sort used to leave the
    /// selection on whatever moved into that *position* — and the host
    /// resolves the selection to paths and hands them to the trash, so
    /// this deleted a file the user never clicked.
    #[test]
    fn the_selection_follows_the_file_when_the_listing_is_re_sorted() {
        let mut browser = loaded_browser(&["a.txt", "b.txt", "c.txt"]);
        browser.update(Message::EntryClicked { index: 0, ctrl: false, shift: false });
        assert_eq!(browser.rows()[0].name, "a.txt");

        // Name is already the active column, so clicking its header
        // reverses the direction: a.txt goes from the top to the bottom.
        browser.update(Message::SortBy(SortColumn::Name));
        assert_eq!(browser.rows()[0].name, "c.txt", "the listing really did reverse");

        let selected: Vec<&Path> =
            browser.selection().selected_paths().iter().map(|p| p.as_path()).collect();
        assert_eq!(selected, [Path::new("/dir/a.txt")], "still the file that was clicked");
        assert!(!row_selected(&browser, 0), "and not whatever is at row 0 now");
    }

    /// The same property for a search, which rebuilds the view on every
    /// keystroke.
    #[test]
    fn the_selection_survives_typing_in_the_search_box() {
        let mut browser = loaded_browser(&["alpha.txt", "beta.txt", "gamma.txt"]);
        browser.update(Message::EntryClicked { index: 2, ctrl: false, shift: false });
        let chosen = browser.rows()[2].path.clone();

        browser.update(Message::SearchChanged("a".to_string()));
        assert!(
            browser.selection().is_selected(&chosen),
            "the file that was clicked is still the one selected"
        );
    }

    /// Enter activates the focused *file*, not whatever is at the row
    /// the focus was set from.
    #[test]
    fn enter_activates_the_focused_file_after_a_re_sort() {
        let mut browser = loaded_browser(&["a.txt", "b.txt", "c.txt"]);
        browser.update(Message::EntryClicked { index: 0, ctrl: false, shift: false });
        browser.update(Message::SortBy(SortColumn::Name));
        assert_eq!(browser.rows()[0].name, "c.txt", "the listing really did reverse");

        let outcome = browser.perform(Action::Open);
        assert_eq!(outcome, Outcome::Activated(PathBuf::from("/dir/a.txt")));
    }

    // --- columns -------------------------------------------------------------

    /// Toggling a column is a remembered choice, so it has to reach the
    /// host as a `PrefsChanged` — nothing else writes `files.toml`.
    #[test]
    fn toggling_a_column_asks_the_host_to_remember_it() {
        let mut browser = loaded_browser(&["a.txt"]);
        let outcome = browser.update(Message::ToggleColumn(Column::Owner));
        match outcome {
            Outcome::PrefsChanged(prefs) => assert!(!prefs.columns.shows(Column::Owner)),
            other => panic!("expected PrefsChanged, got {other:?}"),
        }
    }

    /// Hiding a column must not silently reorder the listing under the
    /// user, even when the column being hidden is the one being sorted
    /// by. Which columns are *shown* and how rows are *ordered* are
    /// separate questions.
    #[test]
    fn hiding_the_sorted_column_leaves_the_order_alone() {
        let mut browser = loaded_browser(&["b.txt", "a.txt", "c.txt"]);
        browser.update(Message::SortBy(SortColumn::Size));
        let before: Vec<String> = browser.rows().iter().map(|e| e.name.clone()).collect();

        browser.update(Message::ToggleColumn(Column::Size));
        let after: Vec<String> = browser.rows().iter().map(|e| e.name.clone()).collect();
        assert_eq!(before, after);
    }

    /// The picker is a thing you are doing, not a thing you configured —
    /// opening it must not write to `files.toml`.
    #[test]
    fn opening_the_column_picker_is_not_a_saved_preference() {
        let mut browser = loaded_browser(&["a.txt"]);
        assert_eq!(browser.update(Message::ToggleColumnPicker), Outcome::None);
        assert!(browser.column_picker_open);
        assert_eq!(browser.update(Message::ToggleColumnPicker), Outcome::None);
        assert!(!browser.column_picker_open);
    }

    // --- double click, and the sidebar -------------------------------------

    /// The thing that was missing: a double click opens what it is on.
    /// A folder navigates into itself; a file becomes the host's problem.
    #[test]
    fn a_double_click_opens_a_folder_and_activates_a_file() {
        let mut browser = loaded_browser(&["notes.txt"]);
        assert_eq!(
            browser.update(Message::EntryActivated(0)),
            Outcome::Activated(PathBuf::from("/dir/notes.txt"))
        );

        let (mut browser, _) =
            Browser::new(Mode::App, Prefs::default(), PathBuf::from("/dir"), vec![]);
        browser.update(Message::DirLoaded(PathBuf::from("/dir"), Ok(vec![entry("sub", true)])));
        assert_eq!(
            browser.update(Message::EntryActivated(0)),
            Outcome::ReadDir(PathBuf::from("/dir/sub"))
        );
    }

    /// A double click on a row that is no longer there does nothing —
    /// the index came from a view that may have been re-sorted since.
    #[test]
    fn a_double_click_past_the_end_of_the_listing_does_nothing() {
        let mut browser = loaded_browser(&["a.txt"]);
        assert_eq!(browser.update(Message::EntryActivated(7)), Outcome::None);
    }

    /// The toggle is persisted, and it sets the state the *view* worked
    /// out — see `Message::SetSidebar`'s own doc for why the message
    /// carries it rather than `update` deciding.
    #[test]
    fn showing_or_hiding_the_sidebar_is_remembered() {
        let mut browser = loaded_browser(&["a.txt"]);
        match browser.update(Message::SetSidebar(crate::prefs::SidebarPref::Hidden)) {
            Outcome::PrefsChanged(prefs) => {
                assert_eq!(prefs.sidebar, crate::prefs::SidebarPref::Hidden)
            }
            other => panic!("expected PrefsChanged, got {other:?}"),
        }
    }

    /// A refresh keeps the selection, minus anything that has gone —
    /// the path-keyed selection makes that free, and it is what lets a
    /// paste or a rename leave the things you were working on selected.
    #[test]
    fn refreshing_keeps_the_selection_of_what_is_still_there() {
        let mut browser = loaded_browser(&["a.txt", "b.txt", "c.txt"]);
        browser.perform(Action::SelectAll);
        assert_eq!(browser.perform(Action::Refresh), Outcome::ReadDir(PathBuf::from("/dir")));
        browser.update(Message::DirLoaded(
            PathBuf::from("/dir"),
            Ok(vec![entry("a.txt", false), entry("c.txt", false)]),
        ));
        assert_eq!(
            browser.selected_shown(),
            [PathBuf::from("/dir/a.txt"), PathBuf::from("/dir/c.txt")]
        );
    }

    #[test]
    fn copy_and_cut_put_the_visible_selection_on_the_clipboard() {
        use crate::clipboard::{ClipVerb, FileClip};
        let mut browser = loaded_browser(&["a.txt", "b.txt"]);
        browser.perform(Action::SelectAll);
        assert_eq!(
            browser.perform(Action::Cut),
            Outcome::SetClipboard(FileClip {
                paths: vec!["/dir/a.txt".into(), "/dir/b.txt".into()],
                verb: ClipVerb::Cut,
            })
        );
        assert_eq!(
            browser.perform(Action::CopyPath),
            Outcome::CopyText("/dir/a.txt\n/dir/b.txt".to_string())
        );
    }

    #[test]
    fn paste_asks_the_host_to_paste_here_only_when_there_is_something() {
        let mut browser = loaded_browser(&["a.txt"]);
        assert_eq!(browser.perform(Action::Paste), Outcome::None);
        browser.set_can_paste(true);
        assert_eq!(browser.perform(Action::Paste), Outcome::Paste(PathBuf::from("/dir")));
    }

    // --- rename and new folder ------------------------------------------------

    fn start_renaming(browser: &mut Browser, index: usize) -> Outcome {
        browser.update(Message::EntryClicked { index, ctrl: false, shift: false });
        browser.perform(Action::Rename)
    }

    #[test]
    fn f2_opens_the_name_with_everything_but_the_extension_selected() {
        let mut browser = loaded_browser(&["report.pdf"]);
        match start_renaming(&mut browser, 0) {
            Outcome::FocusRename { select, .. } => assert_eq!(select, 6),
            other => panic!("expected FocusRename, got {other:?}"),
        }
        assert_eq!(browser.renaming.as_ref().unwrap().text, "report.pdf");
    }

    #[test]
    fn rename_needs_exactly_one_selected() {
        let mut browser = loaded_browser(&["a.txt", "b.txt"]);
        assert_eq!(browser.perform(Action::Rename), Outcome::None, "nothing selected");
        browser.perform(Action::SelectAll);
        assert_eq!(browser.perform(Action::Rename), Outcome::None, "two selected");
    }

    #[test]
    fn enter_renames_to_what_was_typed() {
        let mut browser = loaded_browser(&["a.txt"]);
        start_renaming(&mut browser, 0);
        browser.update(Message::RenameInput("b.txt".into()));
        assert_eq!(
            browser.update(Message::RenameCommit),
            Outcome::Rename { from: "/dir/a.txt".into(), to: "/dir/b.txt".into() }
        );
        assert!(browser.renaming.is_none());
    }

    /// A bad name is said, and the field stays open to fix it.
    #[test]
    fn a_taken_name_keeps_the_field_open_with_a_reason() {
        let mut browser = loaded_browser(&["a.txt", "b.txt"]);
        start_renaming(&mut browser, 0);
        browser.update(Message::RenameInput("b.txt".into()));
        assert!(matches!(browser.update(Message::RenameCommit), Outcome::Notice(_)));
        assert!(browser.renaming.is_some());
    }

    /// Clearing the field and pressing Enter is how people back out.
    #[test]
    fn an_emptied_field_closes_without_renaming() {
        let mut browser = loaded_browser(&["a.txt"]);
        start_renaming(&mut browser, 0);
        browser.update(Message::RenameInput(String::new()));
        assert_eq!(browser.update(Message::RenameCommit), Outcome::None);
        assert!(browser.renaming.is_none());
    }

    #[test]
    fn escape_and_a_click_elsewhere_both_abandon_the_edit() {
        let mut browser = loaded_browser(&["a.txt", "b.txt"]);
        start_renaming(&mut browser, 0);
        browser.update(Message::RenameCancel);
        assert!(browser.renaming.is_none());

        start_renaming(&mut browser, 0);
        browser.update(Message::EntryClicked { index: 0, ctrl: false, shift: false });
        assert!(browser.renaming.is_some(), "a click inside the row being renamed keeps it");
        browser.update(Message::EntryClicked { index: 1, ctrl: false, shift: false });
        assert!(browser.renaming.is_none(), "a click on another row abandons it");
    }

    #[test]
    fn a_new_folder_gets_the_first_free_name() {
        let mut browser = loaded_browser(&["New folder", "a.txt"]);
        assert_eq!(
            browser.perform(Action::NewFolder),
            Outcome::CreateFolder("/dir/New folder 2".into())
        );
    }

    /// The host asks for the new folder to be named once the listing
    /// shows it; the listing that shows it selects it and opens the edit.
    #[test]
    fn a_new_folder_arrives_selected_and_being_named() {
        let (mut browser, _) =
            Browser::new(Mode::App, Prefs::default(), PathBuf::from("/dir"), vec![]);
        browser.update(Message::DirLoaded(PathBuf::from("/dir"), Ok(vec![entry("a.txt", false)])));
        browser.update(Message::AfterListing { path: "/dir/New folder".into(), rename: true });
        // A listing without it leaves the request waiting.
        browser.update(Message::DirLoaded(PathBuf::from("/dir"), Ok(vec![entry("a.txt", false)])));
        assert!(browser.renaming.is_none());

        let outcome = browser.update(Message::DirLoaded(
            PathBuf::from("/dir"),
            Ok(vec![entry("a.txt", false), entry("New folder", true)]),
        ));
        assert!(
            matches!(&outcome, Outcome::Many(v) if v.iter().any(|o| matches!(o, Outcome::FocusRename { .. }))),
            "{outcome:?}"
        );
        assert_eq!(browser.selected_shown(), [PathBuf::from("/dir/New folder")]);
        assert_eq!(browser.renaming.as_ref().unwrap().text, "New folder");
    }

    /// Navigating away drops a pending request, so it cannot select a
    /// same-named thing somewhere else.
    #[test]
    fn a_pending_selection_does_not_follow_you_to_another_folder() {
        let mut browser = loaded_browser(&["a.txt"]);
        browser.update(Message::AfterListing { path: "/dir/x".into(), rename: true });
        browser.update(Message::Navigate("/other".into()));
        assert!(browser.after_listing.is_none());
    }

    #[test]
    fn nothing_in_the_trash_can_be_renamed_or_made() {
        let ctx = ActionContext { selected: 1, in_trash: true, ..ActionContext::default() };
        assert!(!action::enabled(Action::Rename, &ctx));
        assert!(!action::enabled(Action::NewFolder, &ctx));
    }

    // --- the context menu -----------------------------------------------------

    fn enabled_labels(browser: &Browser) -> Vec<&'static str> {
        let menu = browser.menu.as_ref().expect("a menu is open");
        menu.items
            .iter()
            .filter_map(|i| match i {
                MenuItem::Action { label, enabled: true, .. } => Some(*label),
                _ => None,
            })
            .collect()
    }

    fn choose(browser: &mut Browser, label: &str) -> Outcome {
        let menu = browser.menu.as_ref().expect("a menu is open");
        let index = menu
            .items
            .iter()
            .position(|i| matches!(i, MenuItem::Action { label: l, .. } if *l == label))
            .unwrap_or_else(|| panic!("no {label} item"));
        browser.update(Message::MenuChose(index))
    }

    fn right_click(browser: &mut Browser, spot: MenuSpot) {
        browser.update(Message::OpenContextMenu { spot, at: (10.0, 10.0) });
    }

    fn menu_labels(browser: &Browser) -> Vec<&'static str> {
        browser
            .menu
            .as_ref()
            .map(|m| {
                m.items
                    .iter()
                    .filter_map(|i| match i {
                        MenuItem::Action { label, .. } => Some(*label),
                        MenuItem::Separator => None,
                    })
                    .collect()
            })
            .unwrap_or_default()
    }

    /// Right-clicking a row nobody selected makes it the selection, so
    /// the menu acts on what was clicked rather than on something
    /// selected earlier.
    #[test]
    fn right_clicking_an_unselected_row_selects_just_that_row() {
        let mut browser = loaded_browser(&["a.txt", "b.txt"]);
        browser.update(Message::EntryClicked { index: 0, ctrl: false, shift: false });
        right_click(&mut browser, MenuSpot::Row(1));
        assert_eq!(browser.selected_shown(), [PathBuf::from("/dir/b.txt")]);
        assert!(browser.menu_open());
    }

    /// Right-clicking inside a multi-selection keeps it — "select five,
    /// right-click, trash" has to trash five.
    #[test]
    fn right_clicking_inside_a_selection_keeps_the_whole_selection() {
        let mut browser = loaded_browser(&["a.txt", "b.txt", "c.txt"]);
        browser.perform(Action::SelectAll);
        right_click(&mut browser, MenuSpot::Row(1));
        assert_eq!(browser.selected_shown().len(), 3);
        let outcome = browser.update(Message::MenuChose(
            browser
                .menu
                .as_ref()
                .unwrap()
                .items
                .iter()
                .position(|i| matches!(i, MenuItem::Action { action: Action::Trash, .. }))
                .unwrap(),
        ));
        match outcome {
            Outcome::Trash(paths) => assert_eq!(paths.len(), 3),
            other => panic!("expected Trash, got {other:?}"),
        }
        assert!(!browser.menu_open(), "choosing an item closes the menu");
    }

    #[test]
    fn a_folder_a_file_and_the_background_each_get_their_own_menu() {
        let (mut browser, _) =
            Browser::new(Mode::App, Prefs::default(), PathBuf::from("/dir"), vec![]);
        browser.update(Message::DirLoaded(
            PathBuf::from("/dir"),
            Ok(vec![entry("sub", true), entry("f.txt", false)]),
        ));
        right_click(&mut browser, MenuSpot::Row(0));
        assert!(menu_labels(&browser).contains(&"Open in New Tab"), "a folder");
        right_click(&mut browser, MenuSpot::Row(1));
        assert!(!menu_labels(&browser).contains(&"Open in New Tab"), "a file");
        right_click(&mut browser, MenuSpot::Background);
        assert!(menu_labels(&browser).contains(&"Show Hidden Files"), "the folder itself");
        assert!(browser.selected_shown().is_empty(), "the background clears the selection");
    }

    /// With the menu open the arrows move through it and Enter runs the
    /// item — not the file behind it.
    #[test]
    fn the_keyboard_drives_an_open_menu() {
        let mut browser = loaded_browser(&["a.txt", "b.txt"]);
        right_click(&mut browser, MenuSpot::Row(0));
        // Walk down until Select All is highlighted, however the default
        // menu happens to be ordered.
        let highlighted = |b: &Browser| {
            let menu = b.menu.as_ref().unwrap();
            menu.highlighted.and_then(|i| match &menu.items[i] {
                MenuItem::Action { action, .. } => Some(*action),
                MenuItem::Separator => None,
            })
        };
        browser.perform(Action::FocusDown);
        assert_eq!(highlighted(&browser), Some(Action::Open), "Down starts at the top");
        while highlighted(&browser) != Some(Action::SelectAll) {
            browser.perform(Action::FocusDown);
        }
        assert_eq!(browser.selected_shown().len(), 1, "moving in the menu moves nothing else");
        assert_eq!(browser.perform(Action::Open), Outcome::None, "Select All ran");
        assert_eq!(browser.selected_shown().len(), 2);
        assert!(!browser.menu_open());
    }

    #[test]
    fn escape_closes_the_menu_and_leaves_the_search_alone() {
        let mut browser = loaded_browser(&["alpha.txt", "beta.txt"]);
        browser.update(Message::TypeToSearch('a'));
        right_click(&mut browser, MenuSpot::Background);
        browser.perform(Action::ClearSearch);
        assert!(!browser.menu_open());
        assert_eq!(browser.rows().len(), 2, "alpha and beta both contain an a");
        browser.perform(Action::ClearSearch);
        assert!(browser.search_query.is_empty(), "the second Escape clears it");
    }

    /// A disabled item does nothing when clicked, beyond closing.
    /// The context never claims a focused folder that is not there.
    ///
    /// It used to: a field set around a sidebar menu made
    /// `focused_is_dir` report `Some(true)` regardless of the listing,
    /// so every action consulting it quietly behaved as though a folder
    /// were focused.
    #[test]
    fn the_listings_context_describes_the_listing() {
        let mut browser = loaded_browser(&["a.txt"]);
        browser.update(Message::EntryClicked { index: 0, ctrl: false, shift: false });
        assert_eq!(
            browser.action_context().focused_is_dir,
            Some(false),
            "a file is focused, and the context says so"
        );
        assert_eq!(
            browser.action_context_for(Some(Path::new("/music"))).focused_is_dir,
            Some(true),
            "asked about a sidebar row, it answers about that row"
        );
    }

    /// A sidebar row is not in the listing, so its menu acts on the
    /// place the row names — never on whatever is selected beside it.
    #[test]
    fn a_sidebar_menu_acts_on_its_own_row() {
        use crate::sidebar::PinChange;
        let mut browser = loaded_browser(&["a.txt"]);
        browser.set_pins(vec!["/p1".into(), "/p2".into()]);
        right_click(&mut browser, MenuSpot::Sidebar("/p2".into()));
        let labels = enabled_labels(&browser);
        assert!(labels.contains(&"Move Up"), "{labels:?}");
        assert!(!labels.contains(&"Move Down"), "last pin: {labels:?}");
        assert_eq!(choose(&mut browser, "Unpin"), Outcome::Pins(PinChange::Unpin("/p2".into())));

        right_click(&mut browser, MenuSpot::Sidebar("/p1".into()));
        assert_eq!(choose(&mut browser, "Open in New Tab"), Outcome::OpenInNewTab("/p1".into()));

        right_click(&mut browser, MenuSpot::Sidebar("/music".into()));
        assert_eq!(choose(&mut browser, "Pin to Sidebar"), Outcome::Pins(PinChange::Pin("/music".into())));
    }

    /// The switch names the key it is bound to, since a shortcut is the
    /// half of a control nobody can guess — and follows a rebinding.
    #[test]
    fn the_dotfile_switch_names_its_configured_key() {
        use crate::config::Config;
        let mut browser = loaded_browser(&["a.txt"]);
        let (config, problems) =
            crate::config::parse("[keys]\nshow-hidden = \"Ctrl+J\"\n", Path::new("f.toml"));
        assert!(problems.is_empty(), "{problems:?}");
        browser.set_config(std::sync::Arc::new(config));
        assert_eq!(browser.view_model(1000.0).hidden_key.as_deref(), Some("Ctrl+J"));

        browser.set_config(std::sync::Arc::new(Config::default()));
        assert_eq!(browser.view_model(1000.0).hidden_key.as_deref(), Some("Ctrl+H"));
    }

    /// With nothing selected, Ctrl+D pins the folder being shown.
    #[test]
    fn pinning_from_the_keyboard_pins_the_folder_shown() {
        use crate::sidebar::PinChange;
        let mut browser = loaded_browser(&["a.txt"]);
        let shown = browser.current_dir.clone();
        assert_eq!(browser.perform(Action::Pin), Outcome::Pins(PinChange::Pin(shown.clone())));
        browser.set_pins(vec![shown]);
        assert!(!action::enabled(Action::Pin, &browser.action_context()), "already pinned");
    }

    #[test]
    fn choosing_a_disabled_item_only_closes_the_menu() {
        let mut browser = loaded_browser(&["a.txt"]);
        right_click(&mut browser, MenuSpot::Background);
        // Background clears the selection; nothing is selected, so an
        // `entry` menu's Trash would be off. Swap in a menu to check.
        browser.menu = Some(OpenMenu {
            items: menus::build(
                &[crate::menu::MenuEntry::Action(Action::Trash)],
                &browser.action_context(),
                &crate::keymap::Keymap::defaults(),
            ),
            at: (0.0, 0.0),
            highlighted: None,
            target: None,
        });
        assert_eq!(browser.update(Message::MenuChose(0)), Outcome::None);
        assert!(!browser.menu_open());
    }

    #[test]
    fn navigating_away_closes_the_menu() {
        let mut browser = loaded_browser(&["a.txt"]);
        right_click(&mut browser, MenuSpot::Background);
        browser.update(Message::Navigate(PathBuf::from("/elsewhere")));
        assert!(!browser.menu_open());
    }

    /// The Menu key asks the host for the pointer, then opens on the
    /// focused row.
    #[test]
    fn the_menu_key_opens_the_menu_for_the_focused_row() {
        let mut browser = loaded_browser(&["a.txt", "b.txt"]);
        browser.update(Message::EntryClicked { index: 1, ctrl: false, shift: false });
        assert_eq!(
            browser.perform(Action::ContextMenu),
            Outcome::OpenContextMenuAtPointer(MenuSpot::Focused)
        );
        browser.update(Message::OpenContextMenu { spot: MenuSpot::Focused, at: (5.0, 5.0) });
        assert!(browser.menu_open());
        assert_eq!(browser.selected_shown(), [PathBuf::from("/dir/b.txt")]);
    }

    /// The configured menu is the one shown.
    #[test]
    fn a_configured_menu_replaces_the_default() {
        let mut browser = loaded_browser(&["a.txt"]);
        let (config, _) = crate::config::parse(
            "[menu]\nentry = [\"trash\"]\n",
            Path::new("files-config.toml"),
        );
        browser.set_config(Arc::new(config));
        right_click(&mut browser, MenuSpot::Row(0));
        assert_eq!(menu_labels(&browser), ["Move to Trash"]);
    }

    /// The Trash listing shows where things came from, first, and calls
    /// its date column what it is.
    #[test]
    fn the_trash_listing_shows_original_location_and_deleted() {
        let prefs = Prefs::default();
        let columns = listing_columns(&prefs, true);
        assert_eq!(columns.first(), Some(&Column::Origin));
        assert_eq!(column_heading(Column::Modified, true), "Deleted");
        assert!(!listing_columns(&prefs, false).contains(&Column::Origin), "only in the Trash");
        assert_eq!(column_heading(Column::Modified, false), "Modified");
    }

    #[test]
    fn the_trash_is_called_the_trash_everywhere_it_is_named() {
        let trash = crate::sidebar::trash_path();
        assert_eq!(crate::sidebar::place_name(&trash), "Trash");
        assert_eq!(breadcrumb(&trash), [("Trash".to_string(), trash.clone())]);
        assert_eq!(crate::sidebar::place_name(Path::new("/srv/files")), "files", "only the Trash");
    }

    #[test]
    fn restore_hands_the_host_the_stored_paths() {
        let trash = crate::sidebar::trash_path();
        let (mut browser, _) = Browser::new(Mode::App, Prefs::default(), trash.clone(), vec![]);
        let mut item = entry("old.txt", false);
        item.path = trash.join("old.txt");
        browser.update(Message::DirLoaded(trash.clone(), Ok(vec![item])));
        browser.perform(Action::SelectAll);
        assert_eq!(browser.perform(Action::Restore), Outcome::Restore(vec![trash.join("old.txt")]));
    }

    #[test]
    fn nothing_in_the_trash_offers_to_be_trashed_again() {
        let trash = crate::sidebar::trash_path();
        let (mut browser, _) = Browser::new(Mode::App, Prefs::default(), trash.clone(), vec![]);
        let mut item = entry("old.txt", false);
        item.path = trash.join("old.txt");
        browser.update(Message::DirLoaded(trash, Ok(vec![item])));
        right_click(&mut browser, MenuSpot::Row(0));
        assert!(!menu_labels(&browser).contains(&"Move to Trash"));
        assert_eq!(browser.perform(Action::Trash), Outcome::None, "and Delete does nothing");
    }

    // --- the status bar's own claims ----------------------------------------

    /// What the bar says about how much is hidden has to count *both*
    /// filters, because from a reader's side they are one fact: there is
    /// more here than you can see.
    #[test]
    fn the_hidden_count_covers_both_the_search_and_the_dotfile_filter() {
        let (mut browser, _) =
            Browser::new(Mode::App, Prefs::default(), PathBuf::from("/dir"), vec![]);
        browser.update(Message::DirLoaded(
            PathBuf::from("/dir"),
            Ok(vec![entry("visible.txt", false), entry(".hidden", false), entry("other.txt", false)]),
        ));
        // One dotfile, with `show_hidden` off by default.
        assert_eq!(browser.hidden_count(), 1);

        // A search that matches one of the two remaining entries hides
        // the other, on top of the dotfile.
        browser.update(Message::SearchChanged("visible".to_string()));
        assert_eq!(browser.hidden_count(), 2);
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
        assert!(browser.rows().is_empty());
    }

    #[test]
    fn a_stale_dir_loaded_result_for_a_directory_left_behind_is_ignored() {
        let (mut browser, _) = Browser::new(Mode::App, Prefs::default(), PathBuf::from("/a"), vec![]);
        browser.update(Message::Navigate(PathBuf::from("/b")));
        // A slow result for /a arrives after the user already moved on.
        browser.update(Message::DirLoaded(PathBuf::from("/a"), Ok(vec![entry("late.txt", false)])));
        assert!(browser.rows().is_empty(), "a stale result must not appear in /b's listing");
    }

    // --- search filtering ----------------------------------------------------

    #[test]
    fn search_filters_then_clearing_restores_the_full_list() {
        let mut browser = loaded_browser(&["readme.txt", "license.txt", "notes.md"]);
        browser.update(Message::SearchChanged("readme".to_string()));
        assert_eq!(browser.rows().len(), 1);
        assert_eq!(browser.rows()[0].name, "readme.txt");

        browser.update(Message::SearchCleared);
        assert_eq!(browser.rows().len(), 3);
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
        let outcome = browser.perform(Action::Open);
        assert_eq!(outcome, Outcome::Activated(PathBuf::from("/dir/b.txt")));
    }

    #[test]
    fn typing_a_character_searches_and_escape_clears_it() {
        let mut browser = loaded_browser(&["alpha.txt", "beta.txt"]);
        browser.update(Message::TypeToSearch('a'));
        browser.update(Message::TypeToSearch('l'));
        assert_eq!(browser.rows().len(), 1);
        browser.perform(Action::ClearSearch);
        assert_eq!(browser.rows().len(), 2);
    }

    // --- actions ---------------------------------------------------------------

    /// The safety property the path-keyed selection makes necessary:
    /// select everything, narrow the view, press Delete — only what is
    /// still visible goes. Trashing a file the search had hidden would be
    /// acting on something the user could not see.
    #[test]
    fn trash_only_takes_what_the_search_left_visible() {
        let mut browser = loaded_browser(&["alpha.txt", "beta.txt", "gamma.txt"]);
        browser.perform(Action::SelectAll);
        assert_eq!(browser.selected_shown().len(), 3);

        browser.update(Message::TypeToSearch('b'));
        assert_eq!(
            browser.perform(Action::Trash),
            Outcome::Trash(vec![PathBuf::from("/dir/beta.txt")])
        );
    }

    #[test]
    fn trash_with_nothing_selected_does_nothing() {
        let mut browser = loaded_browser(&["a.txt"]);
        assert_eq!(browser.perform(Action::Trash), Outcome::None);
    }

    #[test]
    fn select_all_takes_every_shown_row() {
        let mut browser = loaded_browser(&["a.txt", "b.txt", "c.txt"]);
        browser.perform(Action::SelectAll);
        assert_eq!(browser.action_context().selected, 3);
    }

    #[test]
    fn shift_arrow_extends_the_selection_from_where_the_focus_was() {
        let mut browser = loaded_browser(&["a.txt", "b.txt", "c.txt", "d.txt"]);
        browser.update(Message::EntryClicked { index: 1, ctrl: false, shift: false });
        browser.perform(Action::ExtendDown);
        browser.perform(Action::ExtendDown);
        let names: Vec<String> = browser
            .selected_shown()
            .iter()
            .map(|p| p.file_name().unwrap().to_string_lossy().into_owned())
            .collect();
        assert_eq!(names, ["b.txt", "c.txt", "d.txt"]);

        // And back up past the anchor selects the other way.
        for _ in 0..3 {
            browser.perform(Action::ExtendUp);
        }
        assert_eq!(browser.selected_shown().len(), 2, "a.txt and b.txt");
    }

    #[test]
    fn show_hidden_is_a_remembered_toggle() {
        let (mut browser, _) =
            Browser::new(Mode::App, Prefs::default(), PathBuf::from("/dir"), vec![]);
        browser.update(Message::DirLoaded(
            PathBuf::from("/dir"),
            Ok(vec![entry("visible", false), entry(".secret", false)]),
        ));
        assert_eq!(browser.rows().len(), 1);
        match browser.perform(Action::ToggleHidden) {
            Outcome::PrefsChanged(prefs) => assert!(prefs.show_hidden),
            other => panic!("expected PrefsChanged, got {other:?}"),
        }
        assert_eq!(browser.rows().len(), 2);
    }

    /// Alt+Right used to fall through to "move right". Forward is a real
    /// action now, and it only works when there is somewhere to go.
    #[test]
    fn forward_goes_forward_and_only_when_there_is_history() {
        let (mut browser, _) = Browser::new(Mode::App, Prefs::default(), PathBuf::from("/a"), vec![]);
        assert_eq!(browser.perform(Action::GoForward), Outcome::None);
        browser.update(Message::Navigate(PathBuf::from("/b")));
        browser.perform(Action::GoBack);
        assert_eq!(browser.perform(Action::GoForward), Outcome::ReadDir(PathBuf::from("/b")));
    }

    #[test]
    fn open_in_new_tab_is_for_folders() {
        let (mut browser, _) =
            Browser::new(Mode::App, Prefs::default(), PathBuf::from("/dir"), vec![]);
        browser.update(Message::DirLoaded(
            PathBuf::from("/dir"),
            Ok(vec![entry("sub", true), entry("f.txt", false)]),
        ));
        browser.update(Message::EntryClicked { index: 1, ctrl: false, shift: false });
        assert_eq!(browser.perform(Action::OpenInNewTab), Outcome::None, "a file");
        browser.update(Message::EntryClicked { index: 0, ctrl: false, shift: false });
        assert_eq!(
            browser.perform(Action::OpenInNewTab),
            Outcome::OpenInNewTab(PathBuf::from("/dir/sub"))
        );
    }

    /// A window action asked of the browser is handed back, not dropped —
    /// that is how a menu item for one would reach the host.
    #[test]
    fn a_window_action_is_passed_back_to_the_host() {
        let mut browser = loaded_browser(&["a.txt"]);
        assert_eq!(browser.perform(Action::NewTab), Outcome::Window(Action::NewTab));
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

    /// A count lands on the folder it was asked about.
    #[test]
    fn a_count_that_arrives_lands_on_the_folder_it_was_asked_about() {
        let (mut browser, _) =
            Browser::new(Mode::App, Prefs::default(), PathBuf::from("/dir"), vec![]);
        browser.update(Message::DirLoaded(
            PathBuf::from("/dir"),
            Ok(vec![entry("sub", true), entry("other", true)]),
        ));
        assert_eq!(browser.rows()[1].size, EntrySize::UNCOUNTED, "nothing counted yet");

        browser.update(Message::CountsLoaded(vec![(PathBuf::from("/dir/sub"), Some(18))]));
        let sub = browser.rows().into_iter().find(|e| e.name == "sub").unwrap();
        assert_eq!(sub.size, EntrySize::Items(ItemCount::Known(18)));
        let other = browser.rows().into_iter().find(|e| e.name == "other").unwrap();
        assert_eq!(other.size, EntrySize::UNCOUNTED, "a folder nobody answered for stays blank");
    }

    /// A folder whose read failed is `Unreadable`, never `Known(0)` —
    /// CLAUDE.md's rule about never collapsing "could not be read" into
    /// "there is nothing there", at the level of one table cell.
    #[test]
    fn a_folder_that_could_not_be_counted_is_not_reported_as_empty() {
        let (mut browser, _) =
            Browser::new(Mode::App, Prefs::default(), PathBuf::from("/dir"), vec![]);
        browser.update(Message::DirLoaded(PathBuf::from("/dir"), Ok(vec![entry("sub", true)])));
        browser.update(Message::CountsLoaded(vec![(PathBuf::from("/dir/sub"), None)]));
        assert_eq!(browser.rows()[0].size, EntrySize::Items(ItemCount::Unreadable));
    }

    /// Counts from the previous directory cannot survive navigation,
    /// because they live on the entries and the entries were replaced.
    /// A number left over from somewhere else is worse than no number,
    /// because a number is read as current.
    #[test]
    fn navigating_away_forgets_the_counts_it_had() {
        let (mut browser, _) =
            Browser::new(Mode::App, Prefs::default(), PathBuf::from("/dir"), vec![]);
        browser.update(Message::DirLoaded(PathBuf::from("/dir"), Ok(vec![entry("sub", true)])));
        browser.update(Message::CountsLoaded(vec![(PathBuf::from("/dir/sub"), Some(18))]));
        assert_eq!(browser.rows()[0].size, EntrySize::Items(ItemCount::Known(18)));

        browser.update(Message::Navigate(PathBuf::from("/elsewhere")));
        // The same folder name, listed again somewhere else: its count
        // must start over rather than inherit the one from before.
        browser.update(Message::DirLoaded(
            PathBuf::from("/elsewhere"),
            Ok(vec![Entry { path: PathBuf::from("/elsewhere/sub"), ..entry("sub", true) }]),
        ));
        assert_eq!(browser.rows()[0].size, EntrySize::UNCOUNTED);
    }

    /// A count that arrives for a folder no longer listed is dropped,
    /// not applied to whatever is at that position now.
    #[test]
    fn a_count_for_a_folder_we_have_left_lands_nowhere() {
        let (mut browser, _) =
            Browser::new(Mode::App, Prefs::default(), PathBuf::from("/dir"), vec![]);
        browser.update(Message::DirLoaded(PathBuf::from("/dir"), Ok(vec![entry("sub", true)])));
        browser.update(Message::Navigate(PathBuf::from("/elsewhere")));
        browser.update(Message::DirLoaded(
            PathBuf::from("/elsewhere"),
            Ok(vec![Entry { path: PathBuf::from("/elsewhere/other"), ..entry("other", true) }]),
        ));
        // Keyed by path, so this finds no entry at all — where a
        // row-index key would have written 18 against "other".
        browser.update(Message::CountsLoaded(vec![(PathBuf::from("/dir/sub"), Some(18))]));
        assert_eq!(browser.rows()[0].size, EntrySize::UNCOUNTED);
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

    /// The root crumb is already a slash, so nothing follows it. Pinned
    /// against the way the path bar read on screen before this existed:
    /// `/ / … / countdemo`, three slashes deep before the first name.
    #[test]
    fn the_root_crumb_is_not_followed_by_another_slash() {
        assert!(!separates_from("/"));
        assert!(separates_from("~"));
        assert!(separates_from("home"));
    }

    /// The same path, spelled out as the bar draws it — every crumb
    /// joined by the separators the rule above decides on. Under home
    /// this was always right; from the root it was not.
    #[test]
    fn a_path_from_the_root_is_drawn_with_one_slash_between_names() {
        let drawn = |path: &str| {
            let segments = breadcrumb_from(Path::new(path), Some(Path::new("/home/alex")));
            let mut out = String::new();
            let mut previous: Option<String> = None;
            for (label, _) in segments {
                if previous.as_deref().is_some_and(separates_from) {
                    out.push('/');
                }
                out.push_str(&label);
                previous = Some(label);
            }
            out
        };
        assert_eq!(drawn("/tmp/work"), "/tmp/work");
        assert_eq!(drawn("/home/alex/Documents"), "~/Documents");
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
            app.view_model(1200.0),
            dialog.view_model(1200.0),
            "render()'s input must be identical regardless of Mode"
        );
    }

    // --- selection colour ----------------------------------------------------

    /// The design reserves purple for exactly one meaning: the current
    /// selection. A hovered-but-unselected row using the same colour
    /// would make "is this selected?" ambiguous the instant the pointer
    /// moved, so hover has to read as a plainly different colour — this
    /// pins that the accent (`palette.primary.weak.color`, what
    /// `selectable_row_style` selects on) never appears for any non-selected
    /// status, hover included.
    #[test]
    fn only_the_selected_row_ever_uses_the_accent_colour() {
        use iced::widget::button::Status;
        let theme = hyprforge_ui::theme::app_theme();
        let accent_bg = theme.extended_palette().primary.weak.color;

        let selected = selectable_row_style(&theme, Status::Active, true);
        assert_eq!(selected.background, Some(iced::Background::Color(accent_bg)));

        for status in [Status::Active, Status::Hovered, Status::Pressed, Status::Disabled] {
            let unselected = selectable_row_style(&theme, status, false);
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
        let plain = selectable_row_style(&theme, Status::Active, false);
        let hovered = selectable_row_style(&theme, Status::Hovered, false);
        let selected = selectable_row_style(&theme, Status::Active, true);
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
            in_trash: false,
            renaming: None,
            column_picker_open: false,
            sidebar_collapsed: false,
            viewport_width: 1000.0,
            hidden_count: 0,
            dotfiles: 0,
            hidden_key: Some("Ctrl+H".to_string()),
            show_trash: true,
            rows: Vec::new(),
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

    #[test]
    fn show_trash_off_leaves_the_trash_section_out() {
        let (selection, prefs, load_state) = (Selection::default(), Prefs::default(), LoadState::Loaded);
        let mut vm = view_model_with(&[], &[], Path::new("/x"), &selection, &prefs, &load_state);
        vm.show_trash = false;
        assert!(sidebar_sections(&vm).iter().all(|s| s.title != "Trash"));
    }

    /// A pin that cannot be read keeps its row, dimmed.
    #[test]
    fn a_missing_pin_is_shown_dimmed() {
        let (selection, prefs, load_state) = (Selection::default(), Prefs::default(), LoadState::Loaded);
        let pinned = vec![PinnedItem { label: "Gone".to_string(), path: PathBuf::from("/gone"), item_count: None }];
        let vm = view_model_with(&[], &pinned, Path::new("/x"), &selection, &prefs, &load_state);
        let sections = sidebar_sections(&vm);
        let row = &sections.iter().find(|s| s.title == "Pinned").unwrap().rows[0];
        assert!(row.missing);
        assert_eq!(row.shown_tint(), sidebar::Tint::Dim, "drawn dim");
        assert_eq!(
            row.tint,
            sidebar::Tint::Accent,
            "the row's own colour is untouched — one field decides, and it is `missing`"
        );
    }
}

#[cfg(test)]
mod archive_tests {
    use super::tests_support::*;
    use super::*;

    /// This suite browses archives, so opening one means going into it.
    /// Handing it to the desktop would open whatever else is installed —
    /// or, on a machine with nothing registered for `application/zip`,
    /// nothing at all.
    #[test]
    fn activating_an_archive_navigates_into_it_rather_than_handing_it_over() {
        let mut browser = loaded(&[("notes.txt", false), ("sample.zip", false)]);
        let outcome = browser.update(Message::EntryActivated(1));

        assert_eq!(outcome, Outcome::ReadDir(PathBuf::from("/dir/sample.zip")));
        assert_eq!(browser.current_dir(), Path::new("/dir/sample.zip"));
    }

    #[test]
    fn activating_an_ordinary_file_still_goes_to_the_host() {
        let mut browser = loaded(&[("notes.txt", false), ("sample.zip", false)]);
        assert_eq!(
            browser.update(Message::EntryActivated(0)),
            Outcome::Activated(PathBuf::from("/dir/notes.txt"))
        );
    }

    /// A folder called `backup.zip` is a folder. It already navigates,
    /// so the visible consequence is elsewhere: it must not be offered
    /// the archive menu or counted as an archive by Extract.
    #[test]
    fn a_folder_named_like_an_archive_is_treated_as_the_folder_it_is() {
        let mut browser = loaded(&[("backup.zip", true)]);
        browser.update(Message::EntryClicked { index: 0, ctrl: false, shift: false });
        assert!(
            !browser.action_context().selection_is_archives,
            "a directory is not an archive however it is named"
        );
    }

    /// The guess is made from the name, because a name is all a listing
    /// has. When it is wrong, the way out is the file opening normally —
    /// not an error page where the folder would have been.
    #[test]
    fn a_file_that_only_looked_like_an_archive_opens_the_ordinary_way() {
        let mut browser = loaded(&[("photo.zip", false)]);
        browser.update(Message::EntryActivated(0));
        assert_eq!(browser.current_dir(), Path::new("/dir/photo.zip"));

        let outcome = browser.update(Message::DirLoaded(
            PathBuf::from("/dir/photo.zip"),
            Err(DirError {
                message: "photo.zip isn't an archive.".to_string(),
                kind: DirErrorKind::NotAnArchive,
            }),
        ));

        assert!(
            matches!(&outcome, Outcome::Many(parts) if parts.contains(&Outcome::Activated(PathBuf::from("/dir/photo.zip")))),
            "the file has to reach the host to be opened: {outcome:?}"
        );
        assert_eq!(
            browser.current_dir(),
            Path::new("/dir"),
            "and the browser must not be left standing in a place that does not exist"
        );
        assert!(
            !matches!(browser.load_state, LoadState::Error(_)),
            "an error that was handled is not an error to show"
        );
    }

    /// Going *back* out of a failed navigation would leave the failure
    /// on the forward stack — a Forward button offering to do again the
    /// thing that just did not work.
    #[test]
    fn a_failed_archive_navigation_is_not_offered_again_by_forward() {
        let mut browser = loaded(&[("photo.zip", false)]);
        browser.update(Message::EntryActivated(0));
        browser.update(Message::DirLoaded(
            PathBuf::from("/dir/photo.zip"),
            Err(DirError {
                message: "photo.zip isn't an archive.".to_string(),
                kind: DirErrorKind::NotAnArchive,
            }),
        ));

        assert!(!browser.action_context().can_go_forward);
    }

    // --- what the menus and the actions offer ---------------------------

    #[test]
    fn an_archive_row_is_offered_extracting_and_an_ordinary_file_is_not() {
        let mut browser = loaded(&[("notes.txt", false), ("sample.zip", false)]);

        browser.update(Message::EntryClicked { index: 1, ctrl: false, shift: false });
        assert!(action::enabled(Action::Extract, &browser.action_context()));

        browser.update(Message::EntryClicked { index: 0, ctrl: false, shift: false });
        assert!(!action::enabled(Action::Extract, &browser.action_context()));
    }

    /// All of them, or the action would quietly skip the rest of the
    /// selection.
    #[test]
    fn extracting_a_mixed_selection_is_not_offered() {
        let mut browser = loaded(&[("notes.txt", false), ("sample.zip", false)]);
        browser.update(Message::EntryClicked { index: 0, ctrl: false, shift: false });
        browser.update(Message::EntryClicked { index: 1, ctrl: true, shift: false });

        assert_eq!(browser.action_context().selected, 2);
        assert!(!action::enabled(Action::Extract, &browser.action_context()));
    }

    #[test]
    fn inside_an_archive_deleting_is_offered_and_the_trash_is_not() {
        let mut browser = loaded(&[("guide.txt", false)]);
        browser.set_in_archive(true);
        browser.update(Message::EntryClicked { index: 0, ctrl: false, shift: false });

        let ctx = browser.action_context();
        assert!(
            !action::enabled(Action::Trash, &ctx),
            "there is no trash inside a zip to file a member in"
        );
        assert!(
            action::enabled(Action::DeletePermanently, &ctx),
            "and removing it from the archive is what Delete means here"
        );
        assert!(action::enabled(Action::Rename, &ctx), "renaming is an edit the formats allow");
        assert!(!action::enabled(Action::Cut, &ctx), "a move across a rewrite is not offered");
        assert!(!action::enabled(Action::Compress, &ctx), "an archive inside an archive is nobody's want");
        assert!(!action::enabled(Action::NewFolder, &ctx));
        assert!(!action::enabled(Action::Pin, &ctx), "a path through an archive is not a place to pin");
    }

    #[test]
    fn inside_an_archive_a_selection_can_be_extracted_out_of_it() {
        let mut browser = loaded(&[("guide.txt", false)]);
        browser.set_in_archive(true);
        browser.update(Message::EntryClicked { index: 0, ctrl: false, shift: false });

        assert!(action::enabled(Action::ExtractTo, &browser.action_context()));
        assert_eq!(
            browser.perform(Action::ExtractTo),
            Outcome::ExtractMembers {
                members: vec![PathBuf::from("/dir/guide.txt")],
                from: PathBuf::from("/dir"),
            }
        );
    }

    #[test]
    fn extracting_without_a_destination_leaves_the_naming_to_the_host() {
        let mut browser = loaded(&[("sample.zip", false)]);
        browser.update(Message::EntryClicked { index: 0, ctrl: false, shift: false });

        assert_eq!(
            browser.perform(Action::Extract),
            Outcome::Extract {
                archives: vec![PathBuf::from("/dir/sample.zip")],
                to: None,
            }
        );
    }

    #[test]
    fn compressing_names_the_folder_the_new_archive_goes_into() {
        let mut browser = loaded(&[("notes.txt", false)]);
        browser.update(Message::EntryClicked { index: 0, ctrl: false, shift: false });

        assert_eq!(
            browser.perform(Action::Compress),
            Outcome::Compress {
                sources: vec![PathBuf::from("/dir/notes.txt")],
                into: PathBuf::from("/dir"),
            }
        );
    }
}
