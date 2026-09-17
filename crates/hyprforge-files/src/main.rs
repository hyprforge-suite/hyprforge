//! The file manager window.
//!
//! Thin on purpose. The browsing view — the sidebar, the path bar, the
//! entry list, the keyboard grammar — lives in `hyprforge-files-core`,
//! because the portal's open/save dialog has to render the *same* one.
//! What is here is the window around it and the things a dialog
//! deliberately does not have: file operations, and launching what you
//! double-click.
//!
//! # No single-instance lock, unlike the Settings app
//!
//! `hyprforge-settings` takes an `flock` so a second invocation hands
//! its request to the first rather than opening a second window, because
//! two Settings windows editing the same config would be a genuine
//! hazard. A file manager is the opposite: two windows onto two
//! directories is how people move files, and every file manager anyone
//! has used allows it. There is deliberately no lock here.
//!
//! # `Browser` never touches a filesystem — this does, off the UI thread
//!
//! Every [`Outcome::ReadDir`] this window receives is satisfied by
//! [`read_dir_task`], which runs [`read_dir_sync`] on a background
//! thread via `tokio::task::spawn_blocking` — never inline in `update`,
//! because a directory of 100k entries on a slow mount would hold the
//! frame for as long as it took to read.
//!
//! That call works only because this crate asks for iced's `tokio`
//! feature (see its Cargo.toml, which explains why it is declared there
//! rather than inherited). Without it, `Task` futures run on a
//! `futures` thread pool with no ambient tokio runtime and
//! `spawn_blocking` panics with "there is no reactor running" — a
//! failure that would appear on the first directory read rather than at
//! startup, which is exactly the kind that survives a smoke test.
//! `hyprforge-settings` makes the same addition for the same reason.
//!
//! A second `ReadDir` racing a slow first one — the user clicks fast, or
//! a network mount answers slowly — is guarded by `read_generation`: every
//! read this window issues is stamped with the generation current at the
//! moment it was issued, and a result whose stamp no longer matches is
//! dropped before it ever reaches `Browser`. This is a *host*-level guard
//! and a different one from `Browser::apply_dir_loaded`'s own "the user
//! already navigated elsewhere" check — that one compares the result's
//! path against the current directory, which does nothing for two reads
//! of the *same* directory racing each other (a stale reload landing after
//! a fresh one). See [`tests::a_stale_dir_loaded_result_is_ignored`].

use hyprforge_files::jobs::{self, JobControl, JobEvent, JobId, JobSummary};
use hyprforge_files_core::backend::FsBackend;
use hyprforge_files_core::clipboard::{self as file_clipboard, ClipVerb, FileClipboard, MemoryClipboard};
use hyprforge_fileops::{Collision, CollisionDecision, CollisionPolicy, Progress};
use hyprforge_files_core::trash::RoutingBackend;
use hyprforge_files_core::browser::Message as BrowserMessage;
use hyprforge_files_core::keymap::Resolved;
use hyprforge_files_core::sidebar::SidebarItem;
use hyprforge_files_core::{
    keymap, sidebar, xdg_user_dirs, Action, Browser, Click, ClickTracker, DirError, DirErrorKind,
    Entry, Mode, Outcome, Prefs, Scope,
};
use hyprforge_ui::theme::{app_theme, spacing, FontScale, BASE_TEXT_SIZE};
use std::sync::Arc;
use std::time::Instant;
use hyprforge_ui::widgets::{scaled_text, secondary_button};
use iced::keyboard::{self, key, Key};
use iced::widget::{button, column, container, row};
use iced::{window, Background, Border, Color, Element, Length, Size, Subscription, Task, Theme};
use std::path::{Path, PathBuf};

/// This window's application id (X11 `WM_CLASS` / Wayland `app_id`).
/// Must match `packaging/hyprforge-files.desktop`'s basename — see
/// `hyprforge-settings`'s `APP_ID` doc for the bug this avoids: without
/// setting `platform_specific.application_id`, iced's default is an
/// empty string and Hyprland has nothing to select this window by.
const APP_ID: &str = "hyprforge-files";

fn main() -> iced::Result {
    // `warn` unless RUST_LOG says otherwise — see hyprforge-settings's
    // identical note: a GUI launched from a menu has no terminal, and
    // defaulting to ERROR would silence every `warn!` below, including
    // the one for a Files settings file that exists but won't parse.
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("warn")),
        )
        .init();

    // Resolve the shared look before the first frame — the one place
    // that joins hyprforge-appearance (what the user set) and
    // hyprforge-ui (how to draw it), exactly as every other app in this
    // suite does it.
    hyprforge_ui::theme::init(hyprforge_appearance::look::resolve());

    // One backend, held for the life of the window and handed to every
    // read. `RoutingBackend` is what decides that the trash lists by its
    // records rather than by its storage directory — see
    // `hyprforge_files_core::trash`. Behind `Arc<dyn FsBackend>` because
    // each read runs on a background thread and because that is the type
    // that lets a test drive this whole window against `MockBackend`.
    let backend: Arc<dyn FsBackend> = Arc::new(RoutingBackend::default());
    let start_dir = resolve_start_dir(backend.as_ref(), std::env::args().nth(1));

    // A `files.toml` that exists and will not parse must be reported,
    // never silently defaulted — see `hyprforge_files_core::prefs`'s own
    // doc and CLAUDE.md's rule on `hlconfig::storage`. Defaults are still
    // used so the window opens rather than refusing to start, but the
    // user is told, and nothing here saves over the broken file until
    // they fix it (the next `prefs::update` call refuses to write over a
    // file it can't reload — see `Message::PrefsSaved`'s handling below).
    // The hand-written config: key bindings today, menus and behaviour
    // later. Read once, before the window opens — a small local file,
    // the same budget `prefs::load` spends just below. Problems are
    // shown, never fatal: a typo in a binding must not stop the window
    // opening, and must not cost the bindings around it.
    let (config, config_problems) = hyprforge_files_core::config::load();
    let config = Arc::new(config);

    let (prefs, prefs_status) = match hyprforge_files_core::prefs::load() {
        Ok(prefs) => (prefs, None),
        Err(e) => (
            Prefs::default(),
            Some(format!(
                "Your Files settings couldn't be read, so defaults are being used until this \
                 is fixed: {e}"
            )),
        ),
    };

    // Sidebar construction does real filesystem I/O (a `stat` per XDG
    // user directory) — see `sidebar::build`'s own doc for why. It is a
    // handful of syscalls done once before the window opens, the same
    // budget `main()` already spends on the singleton lock in
    // hyprforge-settings and on PAM setup in the greeter; it is not the
    // per-navigation directory read that has to stay off the UI thread.
    let user_dirs = xdg_user_dirs::load(&hyprforge_paths::config_home(), &backend.home_dir());
    let sidebar_items = sidebar::build(backend.as_ref(), &user_dirs);

    let initial_size = Size::new(
        (prefs.window_width as f32).max(480.0),
        (prefs.window_height as f32).max(360.0),
    );
    let last_window_size = (prefs.window_width, prefs.window_height);

    // Tab restoration (re-opening the tabs that were open last time) is
    // deferred: it needs a field on `Prefs`, and another agent is working
    // on `hyprforge-files-core::prefs` at the same time this crate was
    // built — see this crate's task report for why that edit was left
    // for reconciliation rather than made here. So the window always
    // starts with exactly one tab, at `start_dir`, same as before tabs
    // existed.
    let (mut browser, outcome) =
        Browser::new(Mode::App, prefs.clone(), start_dir, sidebar_items.clone());
    browser.set_config(config.clone());
    let mut app = App {
        home_dir: backend.home_dir(),
        backend,
        tabs: vec![Tab::new(0, browser)],
        active: 0,
        next_tab_id: 1,
        sidebar_items,
        last_prefs: prefs,
        font_scale: FontScale(hyprforge_ui::theme::active().font_scale),
        status: startup_status(prefs_status, &config_problems),
        last_window_size,
        resize_generation: 0,
        clipboard: Arc::new(MemoryClipboard::new()),
        jobs: Vec::new(),
        next_job_id: 1,
        modifiers: keyboard::Modifiers::default(),
        clicks: ClickTracker::new(),
        config: config.clone(),
        #[cfg(debug_assertions)]
        debug_menu: debug_menu_request(),
    };
    #[cfg(debug_assertions)]
    debug_conflict(&mut app);
    // Fulfils the `Outcome::ReadDir` `Browser::new` always returns —
    // otherwise the window opens showing nothing at all, forever, for
    // the same reason CLAUDE.md's "a five-second gap" rule exists: an
    // outcome nobody satisfies is silent, not merely slow.
    let boot_task = app.handle_outcome(0, outcome);

    // `iced::application` calls this closure exactly once; a `RefCell`
    // lets `main` build the real starting state above (which needs I/O)
    // instead of the zero-argument `Fn() -> (State, Task)` iced wants —
    // the same shape `hyprforge-greet` uses for its own one-shot state.
    let boot = std::cell::RefCell::new(Some((app, boot_task)));

    iced::application(
        move || boot.borrow_mut().take().expect("hyprforge-files boots once"),
        App::update,
        App::view,
    )
    .title(App::title)
    .theme(App::theme)
    .subscription(App::subscription)
    .window(window::Settings {
        size: initial_size,
        min_size: Some(Size::new(480.0, 360.0)),
        platform_specific: window::settings::PlatformSpecific {
            application_id: APP_ID.to_string(),
            ..window::settings::PlatformSpecific::default()
        },
        ..window::Settings::default()
    })
    .run()
}

// ---------------------------------------------------------------------
// Startup path resolution
// ---------------------------------------------------------------------

/// Resolves the optional CLI argument into a directory to open.
///
/// With no argument, the user's home directory. With one, it may be a
/// plain path or a `file://` URI (the `.desktop` entry passes `%U`,
/// which a launcher can hand either as) — see [`resolve_arg_path`]. If
/// that path names a file rather than a directory, its *parent* is
/// opened instead — the convention every graphical file manager follows
/// for "open containing folder" and for a `%U` that named a document.
///
/// A path that does not exist at all is deliberately **not** validated
/// here and **not** silently replaced with home: it is handed to
/// `Browser` as the starting directory unchanged, and the ordinary
/// `ReadDir` -> `StdBackend::read_dir` -> `NotFound` path produces
/// `Browser`'s own `LoadState::Error` — the actionable "doesn't exist"
/// sentence the brief asks for, already built, not a second copy of it
/// invented here. See [`tests::a_nonexistent_start_path_is_not_short_circuited`].
fn resolve_start_dir(backend: &dyn FsBackend, arg: Option<String>) -> PathBuf {
    let Some(arg) = arg else {
        return backend.home_dir();
    };
    let path = resolve_arg_path(&arg);
    match backend.stat(&path) {
        Ok(entry) if !entry.is_dir => path.parent().map(Path::to_path_buf).unwrap_or(path),
        _ => path,
    }
}

/// Plain path, or a `file://` URI — percent-decoded, with an optional
/// `localhost` authority stripped (`file://localhost/home/x` is valid
/// per RFC 8089; a bare `file:///home/x` is what most tools actually
/// emit). Not a general URI parser: this crate does not depend on one,
/// and the `.desktop` entry's `%U` for a local path never needs more.
fn resolve_arg_path(arg: &str) -> PathBuf {
    let Some(rest) = arg.strip_prefix("file://") else {
        return PathBuf::from(arg);
    };
    let rest = rest.strip_prefix("localhost").unwrap_or(rest);
    // `hyprforge_fileops::percent` decodes over raw bytes rather than
    // `str`, so a filename that is not valid UTF-8 survives the trip —
    // the same reason that module gives for doing it that way. A `%`
    // that is not a valid escape makes it refuse; the argument is then
    // taken literally, which is the only remaining honest reading of it.
    let Ok(decoded) = hyprforge_fileops::percent::decode_path(rest) else {
        return PathBuf::from(arg);
    };
    if decoded.is_absolute() {
        decoded
    } else {
        // No leading slash means no `///` triple-slash form was used —
        // still an absolute local path per the scheme, so one is added
        // rather than resolving it against whatever the cwd happens to
        // be.
        Path::new("/").join(decoded)
    }
}

// ---------------------------------------------------------------------
// Directory reads, off the UI thread
// ---------------------------------------------------------------------

/// Reads one directory off the UI thread, through whichever backend
/// claims it.
///
/// The backend is handed in rather than constructed here. That is what
/// makes everything above this function reachable with the mock: a
/// window's whole update loop — stale generations, tab routing, folder
/// counts — used to be untestable without a real disk, because this one
/// function named `StdBackend` as a concrete type and nothing else in
/// the app held a backend at all.
async fn read_dir_task(
    backend: Arc<dyn FsBackend>,
    path: PathBuf,
) -> Result<Vec<Entry>, DirError> {
    match tokio::task::spawn_blocking(move || {
        backend.read_dir(&path).map_err(|e| DirError::from(&e))
    })
    .await
    {
        Ok(result) => result,
        Err(e) => Err(DirError {
            message: format!("Reading this folder was interrupted: {e}"),
            kind: DirErrorKind::Other,
        }),
    }
}

/// The second pass behind a folder's Size cell: how many things are in
/// each of these directories.
///
/// One call per folder, which is why this is a second pass rather than
/// part of the listing — see `Outcome::CountFolders`'s own doc.
///
/// Through the same backend as the listing itself, so the trash's count
/// agrees with the trash's listing rather than counting its storage
/// directory behind its back.
///
/// Bounded by [`COUNT_BUDGET`]. A directory of ten thousand
/// subdirectories would otherwise spend ten thousand reads filling in a
/// column nobody has scrolled to, on a machine the user is trying to do
/// something else with. Past the budget the rest simply stay
/// `ItemCount::Pending`, which the Size cell already renders as blank —
/// the alternative, counting forever, is invisible until it is
/// someone's fan spinning up.
async fn count_folders(
    backend: Arc<dyn FsBackend>,
    folders: Vec<PathBuf>,
) -> Vec<(PathBuf, Option<usize>)> {
    tokio::task::spawn_blocking(move || {
        folders
            .into_iter()
            .take(COUNT_BUDGET)
            .map(|path| {
                // `None` rather than `Some(0)` when the read fails: a
                // folder you have no permission to open is not an empty
                // one, and `ItemCount::Unreadable` says so with an em
                // dash where `Known(0)` would say "0 items".
                let count = backend.count_children(&path).ok();
                (path, count)
            })
            .collect()
    })
    .await
    .unwrap_or_default()
}

// ---------------------------------------------------------------------
// The window
// ---------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq)]
enum Message {
    Browser(BrowserMessage),
    /// A key nobody else took — resolved in `update` against
    /// [`App::keymap`], which is state a subscription closure cannot see.
    KeyPressed(keymap::KeyPress),
    /// The tab a directory read was issued for, the generation it was
    /// issued under, the directory it was for, and what came back — see
    /// the module doc's note on `read_generation`, and [`Tab`]'s own doc
    /// for why the guard is per-tab rather than per-window now that a
    /// window can have several reads in flight at once, one per tab.
    DirLoaded(u64, u64, PathBuf, Result<Vec<Entry>, DirError>),
    /// The tab a trash operation was started from, the directory to
    /// refresh, and what (if anything) failed.
    TrashDone(u64, PathBuf, Vec<String>),
    PrefsSaved(Result<Prefs, String>),
    WindowResized(Size),
    /// Ctrl/shift went down or came up. Tracked so a click on a row can
    /// be told what was held at the time — see [`App::modifiers`].
    ModifiersChanged(keyboard::Modifiers),
    /// A resize has gone quiet — the generation this was armed with, so
    /// a drag that is still going can ignore it. See
    /// [`App::resize_generation`].
    WindowSettled(u64),
    DismissStatus,
    /// The titlebar's `+`. `Ctrl+T` is `Action::NewTab`.
    NewTab,
    /// `Ctrl+W`, or a tab's own close affordance — by position in
    /// `App::tabs` at the moment the message was produced (view and
    /// update run on the same state between one click and the next, the
    /// same assumption every other index-carrying message in this file
    /// already makes).
    CloseTab(usize),
    /// Something a paste job reported.
    Job(JobEvent),
    /// The answer to the conflict a job is paused on.
    AnswerConflict(JobId, CollisionPolicy),
    /// "Do this for the rest" was ticked or unticked.
    SetApplyToRest(JobId, bool),
    CancelJob(JobId),
    /// A click on a tab. The keyboard's tab actions go through
    /// [`App::perform_window`] instead.
    SwitchTab(usize),
}

/// One open directory tree view, with the state that makes it a tab
/// rather than a window-wide value someone reopens by accident: its own
/// [`Browser`] (directory, selection, back/forward history all live
/// inside that), and its own read-generation counter.
///
/// `id` is a value distinct from this tab's position in `App::tabs` —
/// closing an earlier tab shifts every later one's position, and an
/// async `DirLoaded`/`TrashDone` result must still find the *same* tab
/// it was issued for, not whatever now sits at the index it remembers.
/// See [`App::tab_index`].
struct Tab {
    id: u64,
    browser: Browser,
    /// Stamped on every read this tab issues, and checked against on
    /// return — the same generation guard the window used to keep for
    /// itself, now kept per tab so a slow read racing a fast one in tab 2
    /// cannot land in tab 1, or vice versa: each tab's counter only ever
    /// advances for reads *that tab* asked for.
    read_generation: u64,
}

impl Tab {
    fn new(id: u64, browser: Browser) -> Tab {
        Tab { id, browser, read_generation: 0 }
    }
}

/// Where "close the tab at `closed_index`" leaves things, decided as
/// plain data so the choice of neighbour is one function tests can pin
/// directly — not a fact only visible by inspecting an `iced::Task`,
/// which carries no equality of its own to assert against.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum TabCloseOutcome {
    /// That was the last tab; the host should close the window.
    WindowShouldClose,
    /// Tabs remain; this position is the sensible one to select next —
    /// the tab that slid into the closed one's place, or the new last
    /// tab if the closed one was rightmost.
    Activate(usize),
    /// `closed_index` named no open tab (already closed, or never
    /// existed) — nothing to do.
    NoSuchTab,
}

/// The decision [`App::close_tab`] applies. A free function, not a
/// method, so it can be pinned by tests without building an `App` (which
/// needs a real `Browser`, which needs a starting directory) — see
/// [`tests::closing_a_tab_activates_a_sensible_neighbour`] and its
/// siblings.
fn neighbour_after_close(tab_count_before: usize, closed_index: usize) -> TabCloseOutcome {
    if closed_index >= tab_count_before {
        return TabCloseOutcome::NoSuchTab;
    }
    let remaining = tab_count_before - 1;
    if remaining == 0 {
        return TabCloseOutcome::WindowShouldClose;
    }
    // The tab that slid into `closed_index`'s slot, unless the closed
    // tab was the rightmost — then there is no such slot and the new
    // last tab (what used to be its left neighbour) is the sensible pick.
    TabCloseOutcome::Activate(closed_index.min(remaining - 1))
}

struct App {
    tabs: Vec<Tab>,
    /// Index into `tabs` of the one currently drawn and receiving
    /// keyboard input. Never out of bounds while `tabs` is non-empty —
    /// closing the window is how `tabs` becomes empty, and that happens
    /// in the same step that would otherwise leave `active` dangling.
    active: usize,
    /// Monotonic, never reused — see [`Tab::id`]'s own doc for why a
    /// tab needs an identity independent of its position.
    next_tab_id: u64,
    /// Built once at startup (real filesystem `stat`s — see `main`'s own
    /// comment on why that's acceptable there) and cloned into every new
    /// tab's `Browser`; the XDG user directories it lists don't change
    /// while this window is open.
    sidebar_items: Vec<SidebarItem>,
    /// Where `Ctrl+T` and the titlebar `+` open a new tab.
    home_dir: PathBuf,
    /// The most recently loaded-or-saved `Prefs`, used as the starting
    /// point for a newly opened tab so it doesn't revert to defaults —
    /// each tab's `Browser` carries its own copy from here on, per the
    /// brief's scope (Phase A asks for per-tab directory/selection/
    /// history, not per-tab view preferences).
    last_prefs: Prefs,
    font_scale: FontScale,
    /// One line shown at the bottom of the window: what
    /// `launch::open`/trashing/saving preferences said, if anything did.
    /// Pillar 3 — every error reaching the user is a sentence here, never
    /// a log line they are expected to go find.
    status: Option<String>,
    /// Copied and cut files, shared by every tab.
    clipboard: Arc<dyn FileClipboard>,
    /// Pastes in progress, oldest first.
    jobs: Vec<RunningJob>,
    next_job_id: JobId,
    last_window_size: (u32, u32),
    /// Bumped by every resize event, so only the *last* one in a drag
    /// writes the new size to disk.
    ///
    /// `window::resize_events` fires continuously while a window edge is
    /// being dragged, and each event whose rounded size differs writes
    /// the whole of `files.toml` — read, parse, re-serialise, temp file,
    /// rename. A drag is tens of those a second, for a number only the
    /// final one of them is right about. The `last_window_size` guard
    /// cannot help: it only suppresses an *identical* size, which a drag
    /// never produces.
    ///
    /// The same generation guard the directory reads use, for the same
    /// reason: a stale answer has to be recognisable as stale rather
    /// than raced against.
    resize_generation: u64,
    /// Which modifiers are held right now.
    ///
    /// A row click has to know, because ctrl-click toggles and
    /// shift-click ranges — and a click in iced 0.14 carries no modifier
    /// state of its own, on either `button` or `mouse_area`. So
    /// `Message::EntryClicked` arrives from the view with both flags
    /// `false`, and `update` fills them in from here before the browser
    /// sees it.
    ///
    /// That leaves `Browser` a pure function of the message it is given
    /// — its own tests pass ctrl/shift explicitly — and puts "what was
    /// held" in the one place that actually observes the keyboard.
    modifiers: keyboard::Modifiers,
    /// What `files-config.toml` configured: every key's meaning, and
    /// the menus. Shared with every tab's browser rather than copied.
    config: Arc<hyprforge_files_core::config::Config>,
    /// Tracks click timing, so two quick presses on one row open it.
    ///
    /// Here rather than in the view, because iced's `button` captures a
    /// left press before any `mouse_area` wrapped around it can see one
    /// — so `on_double_click` on the outside never fires, and moving the
    /// `mouse_area` inside would break the single click that selects.
    /// See `hyprforge_files_core::click` for the whole argument.
    clicks: ClickTracker,
    /// Debug builds only: open a context menu on the first row as soon
    /// as the first listing arrives. See [`debug_menu_request`].
    #[cfg(debug_assertions)]
    debug_menu: Option<(f32, f32)>,
    /// The filesystem this window reads through, shared with every
    /// background read it issues.
    ///
    /// `App` used to hold none at all: the one in `main()` was dropped
    /// after startup and the read function named `StdBackend` inline. So
    /// `backend::MockBackend`'s carefully-built cases — an unreadable
    /// directory, an entry that vanishes mid-read, fifty thousand
    /// entries — were properties of the mock and of nothing that ships.
    /// Holding the trait object is what puts this window's own update
    /// loop within reach of them.
    backend: Arc<dyn FsBackend>,
}

/// A paste the window is waiting on.
struct RunningJob {
    id: JobId,
    control: JobControl,
    verb: ClipVerb,
    /// Every folder whose listing the job changes — where things landed,
    /// and, for a move, where they came from — refreshed when it ends.
    dirs: Vec<PathBuf>,
    progress: Option<Progress>,
    /// The conflict the job is paused on, if any.
    conflict: Option<Collision>,
    apply_to_rest: bool,
}

impl App {
    fn active_tab(&self) -> &Tab {
        &self.tabs[self.active]
    }

    fn active_tab_mut(&mut self) -> &mut Tab {
        &mut self.tabs[self.active]
    }

    /// The position of the tab with this id, if it's still open. `None`
    /// for a `DirLoaded`/`TrashDone` that outlived the tab that asked for
    /// it — a real case, not a defensive-only branch: closing a tab
    /// doesn't cancel a read already in flight for it.
    fn tab_index(&self, id: u64) -> Option<usize> {
        self.tabs.iter().position(|t| t.id == id)
    }

    fn spawn_read_dir(&mut self, tab_index: usize, path: PathBuf) -> Task<Message> {
        let tab = &mut self.tabs[tab_index];
        tab.read_generation += 1;
        let generation = tab.read_generation;
        let tab_id = tab.id;
        Task::perform(read_dir_task(self.backend.clone(), path.clone()), move |result| {
            Message::DirLoaded(tab_id, generation, path.clone(), result)
        })
    }

    /// Carries out everything `Browser::update`/`perform` handed back
    /// but could not do itself — see `hyprforge_files_core::browser::Outcome`'s
    /// own doc for why each of these belongs to the host. `tab_index` is
    /// which tab produced the outcome, needed only for `ReadDir` (every
    /// other variant is window-wide: opening a file, or saving prefs).
    fn handle_outcome(&mut self, tab_index: usize, outcome: Outcome) -> Task<Message> {
        match outcome {
            Outcome::None => Task::none(),
            Outcome::ReadDir(path) => self.spawn_read_dir(tab_index, path),
            Outcome::Activated(path) => {
                let opened = hyprforge_files::launch::open(&path);
                self.status = opened.message(&path);
                Task::none()
            }
            Outcome::PrefsChanged(prefs) => {
                Task::perform(save_prefs(move |p: &mut Prefs| *p = prefs.clone()), Message::PrefsSaved)
            }
            Outcome::CountFolders(folders) => {
                Task::perform(count_folders(self.backend.clone(), folders), |counts| {
                    Message::Browser(BrowserMessage::CountsLoaded(counts))
                })
            }
            Outcome::Trash(paths) => {
                if paths.is_empty() {
                    return Task::none();
                }
                let tab = &self.tabs[tab_index];
                let dir = tab.browser.current_dir().to_path_buf();
                let tab_id = tab.id;
                Task::perform(trash_many(paths), move |errors| {
                    Message::TrashDone(tab_id, dir.clone(), errors)
                })
            }
            Outcome::OpenInNewTab(path) => self.open_tab(path),
            Outcome::SetClipboard(clip) => {
                if let Err(e) = self.clipboard.set(clip) {
                    self.status = Some(format!("Couldn't copy: {e}"));
                }
                self.sync_can_paste();
                Task::none()
            }
            Outcome::CopyText(text) => iced::clipboard::write(text),
            Outcome::Paste(into) => self.paste_into(into),
            Outcome::OpenContextMenuAtPointer(spot) => {
                let outcome = self.tabs[tab_index]
                    .browser
                    .update(BrowserMessage::OpenContextMenu { spot, at: pointer::last() });
                self.handle_outcome(tab_index, outcome)
            }
            Outcome::Window(action) => self.perform_window(action),
        }
    }

    /// Tells every tab whether there is something to paste. Called
    /// whenever the clipboard changes, and for every new tab.
    fn sync_can_paste(&mut self) {
        let can = self.clipboard.get().is_some();
        for tab in &mut self.tabs {
            tab.browser.set_can_paste(can);
        }
    }

    /// Re-reads every tab showing one of `dirs`.
    fn refresh_dirs(&mut self, dirs: &[PathBuf]) -> Task<Message> {
        let indices: Vec<usize> = self
            .tabs
            .iter()
            .enumerate()
            .filter(|(_, tab)| dirs.iter().any(|d| d == tab.browser.current_dir()))
            .map(|(i, _)| i)
            .collect();
        Task::batch(indices.into_iter().map(|i| {
            let dir = self.tabs[i].browser.current_dir().to_path_buf();
            self.spawn_read_dir(i, dir)
        }))
    }

    /// Plans and starts pasting the clipboard into `into`.
    fn paste_into(&mut self, into: PathBuf) -> Task<Message> {
        let Some(clip) = self.clipboard.get() else {
            return Task::none();
        };
        let plan = file_clipboard::plan(&clip, &into);
        if !plan.refused.is_empty() {
            self.status = Some(
                plan.refused.iter().map(|(_, why)| why.as_str()).collect::<Vec<_>>().join(" "),
            );
        }
        if plan.steps.is_empty() {
            return Task::none();
        }
        let mut dirs = vec![into];
        if clip.verb == ClipVerb::Cut {
            for step in &plan.steps {
                if let Some(parent) = step.source.parent() {
                    if !dirs.iter().any(|d| d == parent) {
                        dirs.push(parent.to_path_buf());
                    }
                }
            }
        }
        let id = self.next_job_id;
        self.next_job_id += 1;
        let (control, events) = jobs::start(id, plan.steps, self.config.behaviour.on_conflict);
        self.jobs.push(RunningJob {
            id,
            control,
            verb: clip.verb,
            dirs,
            progress: None,
            conflict: None,
            apply_to_rest: false,
        });
        Task::run(events, Message::Job)
    }

    /// Updates the window for something a job reported.
    fn job_event(&mut self, event: JobEvent) -> Task<Message> {
        match event {
            JobEvent::Progress { job, progress } => {
                if let Some(running) = self.jobs.iter_mut().find(|j| j.id == job) {
                    running.progress = Some(progress);
                }
                Task::none()
            }
            JobEvent::Collision { job, collision } => {
                if let Some(running) = self.jobs.iter_mut().find(|j| j.id == job) {
                    running.conflict = Some(collision);
                }
                Task::none()
            }
            JobEvent::Finished { job, summary } => {
                let Some(at) = self.jobs.iter().position(|j| j.id == job) else {
                    return Task::none();
                };
                let finished = self.jobs.remove(at);
                if let Some(message) = job_report(finished.verb, &summary) {
                    self.status = Some(message);
                }
                // A cut that fully landed leaves the clipboard pointing at
                // files that are no longer there.
                if finished.verb == ClipVerb::Cut && summary.complete() {
                    self.clipboard.clear();
                    self.sync_can_paste();
                }
                self.refresh_dirs(&finished.dirs)
            }
        }
    }

    /// Answers the conflict job `id` is paused on.
    fn answer_conflict(&mut self, id: JobId, policy: CollisionPolicy) {
        if let Some(running) = self.jobs.iter_mut().find(|j| j.id == id) {
            if running.conflict.take().is_some() {
                running
                    .control
                    .answer(CollisionDecision { policy, apply_to_rest: running.apply_to_rest });
            }
        }
    }

    /// Carries out a window-scope action: the tab strip's half of the
    /// action vocabulary, which the dialog host does not have.
    fn perform_window(&mut self, action: Action) -> Task<Message> {
        match action {
            Action::NewTab => self.open_tab(self.home_dir.clone()),
            Action::CloseTab => self.close_tab(self.active),
            Action::NextTab => {
                self.cycle_tab(1);
                Task::none()
            }
            Action::PreviousTab => {
                self.cycle_tab(-1);
                Task::none()
            }
            Action::Tab(n) => {
                self.jump_to_tab(usize::from(n).saturating_sub(1));
                Task::none()
            }
            // A browser action has no business here; `Browser::perform`
            // never hands one back. Doing nothing is the safe reading.
            _ => Task::none(),
        }
    }

    /// Opens a new tab at `start_dir` and makes it active — `Ctrl+T` and
    /// the titlebar `+` both funnel here.
    fn open_tab(&mut self, start_dir: PathBuf) -> Task<Message> {
        let id = self.next_tab_id;
        self.next_tab_id += 1;
        let (mut browser, outcome) =
            Browser::new(Mode::App, self.last_prefs.clone(), start_dir, self.sidebar_items.clone());
        browser.set_config(self.config.clone());
        browser.set_can_paste(self.clipboard.get().is_some());
        self.tabs.push(Tab::new(id, browser));
        self.active = self.tabs.len() - 1;
        let index = self.active;
        self.handle_outcome(index, outcome)
    }

    /// `Ctrl+W`, or a tab's own close button. See [`neighbour_after_close`]
    /// for the "which neighbour" decision this applies.
    fn close_tab(&mut self, index: usize) -> Task<Message> {
        match neighbour_after_close(self.tabs.len(), index) {
            TabCloseOutcome::NoSuchTab => Task::none(),
            TabCloseOutcome::WindowShouldClose => {
                self.tabs.clear();
                // A one-window process: closing its only tab closes the
                // window by closing the whole application, same as the
                // titlebar's own close button would.
                iced::exit()
            }
            TabCloseOutcome::Activate(next) => {
                self.tabs.remove(index);
                self.active = next;
                Task::none()
            }
        }
    }

    /// `Ctrl+Tab` (`delta = 1`) / `Ctrl+Shift+Tab` (`delta = -1`), wrapping
    /// at either end rather than stopping — the grammar every tabbed
    /// editor and browser already uses.
    fn cycle_tab(&mut self, delta: i32) {
        if self.tabs.is_empty() {
            return;
        }
        let len = self.tabs.len() as i32;
        let next = (self.active as i32 + delta).rem_euclid(len);
        self.active = next as usize;
    }

    /// `Ctrl+1`..`Ctrl+9`. Beyond the number of open tabs this is
    /// silently a no-op — the brief's own wording — never a panicking
    /// index, because nothing about a keybind guarantees the tab count
    /// it was written against.
    fn jump_to_tab(&mut self, index: usize) {
        if index < self.tabs.len() {
            self.active = index;
        }
    }

    fn update(&mut self, message: Message) -> Task<Message> {
        match message {
            Message::ModifiersChanged(modifiers) => {
                self.modifiers = modifiers;
                Task::none()
            }
            Message::Browser(msg) => {
                // A click arrives from the view bare: no modifiers, and
                // no notion of whether it is the second of a pair. Both
                // are facts about the world rather than about the model,
                // and this is where the window knows them.
                let msg = match msg {
                    // A right press carries no position; the pointer
                    // tracker has it. See `pointer`.
                    BrowserMessage::OpenContextMenu { spot, .. } => {
                        self.clicks.reset();
                        BrowserMessage::OpenContextMenu { spot, at: pointer::last() }
                    }
                    BrowserMessage::EntryClicked { index, .. } => {
                        let ctrl = self.modifiers.control();
                        let shift = self.modifiers.shift();
                        if ctrl || shift {
                            // A modifier-held click is a selection
                            // gesture, never half of an open — and it
                            // must not leave a press behind that a
                            // following plain click could pair with.
                            self.clicks.reset();
                            BrowserMessage::EntryClicked { index, ctrl, shift }
                        } else {
                            match self.clicks.press(index, Instant::now()) {
                                Click::Double => BrowserMessage::EntryActivated(index),
                                Click::Single => {
                                    BrowserMessage::EntryClicked { index, ctrl, shift }
                                }
                            }
                        }
                    }
                    other => other,
                };
                let outcome = self.active_tab_mut().browser.update(msg);
                // Arriving somewhere new ends any click sequence: the row
                // under the pointer is a different file now, and pairing
                // the next press with the one that got us here would open
                // whatever happens to be in that position.
                if matches!(outcome, Outcome::ReadDir(_)) {
                    self.clicks.reset();
                }
                self.handle_outcome(self.active, outcome)
            }
            // A conflict dialog has the keyboard. Escape cancels the job;
            // Enter keeps both, the one answer that neither loses a file
            // nor leaves one behind.
            Message::KeyPressed(press)
                if self.jobs.iter().any(|j| j.conflict.is_some())
                    && matches!(press.key, keymap::Key::Escape | keymap::Key::Enter) =>
            {
                let Some(id) = self.jobs.iter().find(|j| j.conflict.is_some()).map(|j| j.id) else {
                    return Task::none();
                };
                if press.key == keymap::Key::Escape {
                    self.update(Message::CancelJob(id))
                } else {
                    self.answer_conflict(id, CollisionPolicy::KeepBoth);
                    Task::none()
                }
            }
            Message::KeyPressed(press) => match self.config.keymap.resolve(&press) {
                // Window actions never reach the browser, which the
                // dialog host also renders and which has no tabs.
                Some(Resolved::Action(action)) if action.scope() == Scope::Window => {
                    self.perform_window(action)
                }
                Some(Resolved::Action(action)) => {
                    self.clicks.reset();
                    let outcome = self.active_tab_mut().browser.perform(action);
                    self.handle_outcome(self.active, outcome)
                }
                Some(Resolved::Type(c)) => {
                    let outcome =
                        self.active_tab_mut().browser.update(BrowserMessage::TypeToSearch(c));
                    self.handle_outcome(self.active, outcome)
                }
                None => Task::none(),
            },
            Message::DirLoaded(tab_id, generation, path, result) => {
                let Some(index) = self.tab_index(tab_id) else {
                    // The tab that asked for this closed before the read
                    // came back. Not an error — see `Tab::id`'s doc.
                    return Task::none();
                };
                // Stale: a newer read has already been issued for *this
                // tab* (possibly for the same directory — see the module
                // doc) since this one went out. Dropping it here means
                // `Browser` never even sees it, which is stronger than
                // relying on `Browser::apply_dir_loaded`'s own path-based
                // staleness check alone — and, critically, it is keyed on
                // this tab's own counter, so a slow read for a different
                // tab can never be mistaken for stale or fresh against
                // the wrong tab's generation.
                if generation != self.tabs[index].read_generation {
                    return Task::none();
                }
                let outcome = self.tabs[index].browser.update(BrowserMessage::DirLoaded(path, result));
                #[cfg(debug_assertions)]
                if let Some(at) = self.debug_menu.take() {
                    self.tabs[index].browser.update(BrowserMessage::OpenContextMenu {
                        spot: hyprforge_files_core::browser::MenuSpot::Row(0),
                        at,
                    });
                }
                self.handle_outcome(index, outcome)
            }
            Message::Job(event) => self.job_event(event),
            Message::AnswerConflict(id, policy) => {
                self.answer_conflict(id, policy);
                Task::none()
            }
            Message::SetApplyToRest(id, on) => {
                if let Some(running) = self.jobs.iter_mut().find(|j| j.id == id) {
                    running.apply_to_rest = on;
                }
                Task::none()
            }
            Message::CancelJob(id) => {
                if let Some(running) = self.jobs.iter_mut().find(|j| j.id == id) {
                    running.conflict = None;
                    running.control.cancel();
                }
                Task::none()
            }
            Message::TrashDone(tab_id, dir, errors) => {
                if !errors.is_empty() {
                    self.status =
                        Some(format!("Couldn't move everything to Trash: {}", errors.join("; ")));
                }
                let Some(index) = self.tab_index(tab_id) else {
                    // The tab this delete was started from has since
                    // closed. The files are already trashed either way;
                    // there is simply no listing left to refresh.
                    return Task::none();
                };
                // Refresh regardless of whether anything failed, so what
                // did succeed disappears from the listing. Safe even if
                // the user has since navigated elsewhere within this tab:
                // this issues a plain read (not a navigation), and
                // `Browser` ignores a `DirLoaded` for a directory that is
                // no longer current.
                self.spawn_read_dir(index, dir)
            }
            Message::PrefsSaved(Ok(prefs)) => {
                // Feeds the next `Ctrl+T`/`+` — see `last_prefs`'s own
                // doc.
                self.last_prefs = prefs;
                Task::none()
            }
            Message::PrefsSaved(Err(e)) => {
                self.status = Some(format!("Couldn't save your Files settings: {e}"));
                Task::none()
            }
            Message::WindowResized(size) => {
                let (w, h) = (size.width.round() as u32, size.height.round() as u32);
                if (w, h) == self.last_window_size {
                    return Task::none();
                }
                self.last_window_size = (w, h);
                // Nothing is written here — only a timer armed. See
                // `resize_generation`.
                self.resize_generation += 1;
                let generation = self.resize_generation;
                Task::perform(
                    async move {
                        tokio::time::sleep(std::time::Duration::from_millis(RESIZE_SETTLE)).await;
                        generation
                    },
                    Message::WindowSettled,
                )
            }
            Message::WindowSettled(generation) => {
                // A later resize has already armed its own timer; this
                // one is about a size the window no longer has.
                if generation != self.resize_generation {
                    return Task::none();
                }
                let (w, h) = self.last_window_size;
                Task::perform(
                    save_prefs(move |p: &mut Prefs| {
                        p.window_width = w;
                        p.window_height = h;
                    }),
                    Message::PrefsSaved,
                )
            }
            Message::DismissStatus => {
                self.status = None;
                Task::none()
            }
            Message::NewTab => self.open_tab(self.home_dir.clone()),
            Message::CloseTab(index) => self.close_tab(index),
            // One bounds check, not two spellings of it: clicking a tab
            // and `Ctrl+N` differ only in which affordance asked, and
            // nothing downstream cares which.
            Message::SwitchTab(index) => {
                self.jump_to_tab(index);
                Task::none()
            }
        }
    }

    fn theme(&self) -> Theme {
        app_theme()
    }

    fn title(&self) -> String {
        let name = tab_display_name(self.active_tab().browser.current_dir());
        format!("{name} \u{2014} Hyprforge Files")
    }

    /// The titlebar's row of tabs — a coloured dot (accent for the active
    /// tab, muted for the rest: the design's "coloured dot" made to mean
    /// something rather than decorate, per CLAUDE.md's rule that a state
    /// colour stops meaning anything the moment it's reused for looks),
    /// the directory's name, a close affordance, and a trailing `+`.
    fn tabs_bar(&self, scale: FontScale) -> Element<'_, Message> {
        use hyprforge_files::tabstrip;

        // Bottom-aligned, not centred. This is the mechanic the strip
        // depends on: tabs of different heights then share a baseline
        // with the pane below and grow *upward*, so the active one rises
        // out of the content. Centred, they float in the middle of the
        // strip with a gap beneath and the taller one grows both ways,
        // which reads as a toolbar of pills rather than as tabs.
        let mut bar = row![].spacing(2.0).align_y(tabstrip::STRIP_ALIGNMENT);
        for (index, tab) in self.tabs.iter().enumerate() {
            bar = bar.push(tab_widget(index, tab, index == self.active, scale));
        }

        let plus = tabstrip::new_tab_size();
        bar = bar.push(
            // Present but unpainted, the same treatment an inactive tab
            // gets, and the same height so it shares their baseline.
            button(
                container(scaled_text("+", BASE_TEXT_SIZE, scale).color(hyprforge_ui::theme::text_dim()))
                    .center_x(Length::Fill)
                    .center_y(Length::Fill),
            )
            .width(Length::Fixed(scale.apply(plus.width)))
            .height(Length::Fixed(scale.apply(plus.height)))
            .padding(0)
            .on_press(Message::NewTab)
            .style(|_t: &Theme, status| button::Style {
                background: matches!(status, button::Status::Hovered)
                    .then(|| Background::Color(hyprforge_ui::theme::surface::row())),
                text_color: hyprforge_ui::theme::text_dim(),
                border: Border { radius: iced::border::top(hyprforge_files::tabstrip::TAB_RADIUS), ..Border::default() },
                ..button::Style::default()
            }),
        );

        container(bar)
            .width(Length::Fill)
            .height(Length::Fixed(scale.apply(tabstrip::STRIP_HEIGHT)))
            // Horizontal only. The space *above* the tabs comes from the
            // strip being taller than its tallest tab, not from padding
            // here — padding would push the tabs off the strip's bottom
            // edge too, and that edge is where they meet the pane.
            .padding([0, spacing::MD as u16])
            .style(|_t: &Theme| container::Style {
                // The strip and the header below it are one piece of
                // chrome; the active tab is the card lifted out of it.
                background: Some(Background::Color(hyprforge_ui::theme::surface::sidebar())),
                ..container::Style::default()
            })
            .into()
    }

    fn view(&self) -> Element<'_, Message> {
        let scale = self.font_scale;
        let tab = self.active_tab();

        let tabs_row = self.tabs_bar(scale);

        // No toolbar here, deliberately.
        //
        // This host used to draw its own row of List/Grid/Columns
        // buttons and a "…" overflow carrying sort and hidden-files.
        // That was a mistake with a visible cost: `browser::render`
        // already drew back/forward/up, the path bar and the search
        // field, so the window ended up with three stacked bands of
        // chrome where the design has one, and the view toggles sat
        // above the path bar instead of beside it.
        //
        // The deeper reason it was wrong is that none of those controls
        // are *this crate's* state. View mode is `Prefs::view_mode`,
        // sort is `Prefs::sort_column`, hidden files are
        // `Prefs::show_hidden` — all browser state, all with a
        // `browser::Message` variant already. Drawing them here meant
        // the open/save dialog, which renders the same `Browser` through
        // a different host, would never have had them at all.
        //
        // So the whole toolbar belongs to `hyprforge-files-core`, and
        // what is left here is what genuinely is the window's own: tabs,
        // and the status bar below.
        let mut content = column![tabs_row].width(Length::Fill).height(Length::Fill);

        // The window's own width, so the browser can collapse the
        // sidebar and decide whether the list has to scroll sideways —
        // both properties of the window, which is this crate's business
        // to know and not the model's to go looking for.
        let viewport_width = self.last_window_size.0 as f32;
        content = content.push(tab.browser.view(scale, viewport_width).map(Message::Browser));

        if let Some(job) = self.jobs.iter().find(|j| j.conflict.is_none()) {
            content = content.push(job_progress_bar(job, scale));
        }

        if let Some(status) = &self.status {
            let bar = row![
                scaled_text(status.clone(), BASE_TEXT_SIZE, scale).width(Length::Fill),
                secondary_button("Dismiss").on_press(Message::DismissStatus),
            ]
            .spacing(spacing::SM)
            .align_y(iced::Alignment::Center)
            .padding(spacing::SM);
            content = content.push(container(bar).width(Length::Fill));
        }

        // The floating window frame: DESIGN.md's 12px outer radius, taken
        // from `Theme::rounding` (the same value hyprforge-look already
        // resolved from Hyprland's own `decoration:rounding`) rather than
        // hardcoded — see DESIGN.md's note on density being ratios, not
        // constants.
        let window: Element<'_, Message> =
            container(content).width(Length::Fill).height(Length::Fill).style(window_frame_style).into();

        // The context menu floats over everything — tabs, header and
        // listing alike — so it is stacked over the whole window rather
        // than drawn inside the browser's own area.
        let size = (self.last_window_size.0 as f32, self.last_window_size.1 as f32);
        // A conflict is a question the job is paused on, so it goes
        // above everything, the context menu included.
        if let Some((job, conflict)) =
            self.jobs.iter().find_map(|j| j.conflict.as_ref().map(|c| (j, c)))
        {
            return iced::widget::stack![window, conflict_dialog(job, conflict, scale)].into();
        }
        match tab.browser.menu_overlay(scale, size) {
            Some(overlay) => iced::widget::stack![window, overlay.map(Message::Browser)].into(),
            None => window,
        }
    }

    /// Keys become `Message::KeyPressed`, and what they mean is decided
    /// in `update` against [`App::keymap`] — the one table. This used to
    /// decide in three places here (Delete first, then the Ctrl tab
    /// shortcuts, then the browser's own grammar), and that precedence
    /// could only be learned by reading this function.
    ///
    /// `keyboard::listen` only delivers keys no widget captured, so while
    /// the search field has focus, Ctrl+A selects its text rather than
    /// the files and Backspace deletes a character rather than going up
    /// a level. That is the behaviour wanted, and it comes free.
    fn subscription(&self) -> Subscription<Message> {
        let keys = keyboard::listen().filter_map(|event| {
            // Which modifiers are held is tracked separately from which
            // key was pressed, because a *click* needs to know and a
            // click carries none of its own: neither `button::on_press`
            // nor `mouse_area::on_press` takes modifier state in iced
            // 0.14. The window is the thing that watches the keyboard,
            // so the window is what supplies them — see
            // `Message::ModifiersChanged`.
            if let keyboard::Event::ModifiersChanged(modifiers) = &event {
                return Some(Message::ModifiersChanged(*modifiers));
            }
            key_press(&event).map(Message::KeyPressed)
        });
        Subscription::batch([
            keys,
            window::resize_events().map(|(_, size)| Message::WindowResized(size)),
            pointer::track(),
        ])
    }
}

/// What the status bar says when a paste ends, or `None` when there is
/// nothing worth interrupting anyone for — a paste that simply worked is
/// visible in the listing already.
fn job_report(verb: ClipVerb, summary: &JobSummary) -> Option<String> {
    let doing = match verb {
        ClipVerb::Copy => "copied",
        ClipVerb::Cut => "moved",
    };
    let mut parts = Vec::new();
    if summary.cancelled {
        parts.push(format!("Stopped after {} {doing}.", plural(summary.done, "item", "items")));
    }
    if summary.skipped > 0 {
        parts.push(format!("{} skipped.", plural(summary.skipped, "item", "items")));
    }
    if !summary.failed.is_empty() {
        parts.push(format!("Some items weren't {doing}: {}", summary.failed.join("; ")));
    }
    (!parts.is_empty()).then(|| parts.join(" "))
}

fn plural(n: usize, one: &str, many: &str) -> String {
    if n == 1 {
        format!("1 {one}")
    } else {
        format!("{n} {many}")
    }
}

/// One line under the listing while a paste runs: what it is doing, how
/// far it has got, and a way to stop it.
fn job_progress_bar<'a>(job: &RunningJob, scale: FontScale) -> Element<'a, Message> {
    let doing = match job.verb {
        ClipVerb::Copy => "Copying",
        ClipVerb::Cut => "Moving",
    };
    let detail = match &job.progress {
        // Totals are unknown while the source tree is still being
        // walked — `Progress` says so rather than guessing.
        Some(Progress { entries_done, entries_total: Some(total), bytes_done, bytes_total, .. }) => {
            let mut text = format!("{entries_done} of {total}");
            if let Some(bytes_total) = bytes_total {
                text.push_str(&format!(
                    " \u{00B7} {} of {}",
                    hyprforge_files_core::human_readable_size(*bytes_done),
                    hyprforge_files_core::human_readable_size(*bytes_total)
                ));
            }
            text
        }
        _ => "counting\u{2026}".to_string(),
    };
    row![
        scaled_text(format!("{doing} \u{00B7} {detail}"), BASE_TEXT_SIZE, scale).width(Length::Fill),
        secondary_button("Cancel").on_press(Message::CancelJob(job.id)),
    ]
    .spacing(spacing::SM)
    .align_y(iced::Alignment::Center)
    .padding([spacing::XS as u16, spacing::MD as u16])
    .into()
}

/// The question a paused paste is asking.
///
/// Over a full-window layer that swallows clicks, because the job is
/// paused on this and nothing else in the window should look clickable
/// meanwhile. Keep Both is the primary button: it is the one answer that
/// neither loses a file nor leaves one behind, and it is what Enter does.
fn conflict_dialog<'a>(job: &RunningJob, conflict: &Collision, scale: FontScale) -> Element<'a, Message> {
    use hyprforge_ui::widgets::primary_button;
    let name = conflict
        .dest
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| conflict.dest.display().to_string());
    let folder = conflict
        .dest
        .parent()
        .map(|p| p.display().to_string())
        .unwrap_or_default();
    let id = job.id;
    let card = container(
        column![
            scaled_text(format!("\u{201C}{name}\u{201D} already exists"), 17.0, scale),
            hyprforge_ui::widgets::meta_text(
                format!("In {folder}. Replace it, keep both, or skip this one?"),
                BASE_TEXT_SIZE,
                scale,
            ),
            iced::widget::checkbox(job.apply_to_rest)
                .label("Do this for the rest")
                .on_toggle(move |on| Message::SetApplyToRest(id, on))
                .text_size(scale.apply(BASE_TEXT_SIZE)),
            row![
                secondary_button("Cancel").on_press(Message::CancelJob(id)),
                iced::widget::Space::new().width(Length::Fill),
                secondary_button("Skip").on_press(Message::AnswerConflict(id, CollisionPolicy::Skip)),
                secondary_button("Replace")
                    .on_press(Message::AnswerConflict(id, CollisionPolicy::Replace)),
                primary_button("Keep Both")
                    .on_press(Message::AnswerConflict(id, CollisionPolicy::KeepBoth)),
            ]
            .spacing(spacing::SM)
            .align_y(iced::Alignment::Center),
        ]
        .spacing(spacing::MD),
    )
    .padding(spacing::LG)
    .max_width(scale.apply(460.0))
    .style(|_t: &Theme| container::Style {
        background: Some(Background::Color(hyprforge_ui::theme::surface::sidebar())),
        border: Border {
            color: hyprforge_ui::theme::surface::card_border(),
            width: 1.0,
            radius: hyprforge_files_core::density::outer_radius().into(),
        },
        ..container::Style::default()
    });
    iced::widget::opaque(
        container(card)
            .center_x(Length::Fill)
            .center_y(Length::Fill)
            .width(Length::Fill)
            .height(Length::Fill)
            // The window behind, dimmed with its own root colour rather
            // than an invented black: the listing still reads as there,
            // and plainly not what the keyboard or pointer is on.
            .style(|_t: &Theme| container::Style {
                background: Some(Background::Color(Color {
                    a: SCRIM_ALPHA,
                    ..hyprforge_ui::theme::surface::root()
                })),
                ..container::Style::default()
            }),
    )
}

/// How much of the window a dialog's dim layer hides.
const SCRIM_ALPHA: f32 = 0.6;

/// Where the pointer last was, kept without a message per mouse move.
///
/// A context menu opens at the pointer, and a right press carries no
/// position. The obvious fix — a `Message::PointerMoved` on every cursor
/// event — would run `update` and rebuild the whole window's widget tree
/// every time the mouse moved, listing and all, to learn one number that
/// matters only at the moment of a right click.
///
/// So the subscription's filter records the position in two atomics and
/// returns `None`, which iced treats as "no message": nothing updates,
/// nothing is rebuilt. `update` reads the atomics when a menu is asked
/// for. It is process-wide state, and that is fine for exactly one
/// reason: this process has one window — closing its last tab exits.
mod pointer {
    use iced::{event, mouse, Event, Subscription};
    use std::sync::atomic::{AtomicU32, Ordering};

    static X: AtomicU32 = AtomicU32::new(0);
    static Y: AtomicU32 = AtomicU32::new(0);

    /// Listens for pointer movement and never produces a message.
    pub fn track<Message: 'static + Send>() -> Subscription<Message> {
        event::listen_with(|event, _status, _window| {
            if let Event::Mouse(mouse::Event::CursorMoved { position }) = event {
                X.store(position.x.to_bits(), Ordering::Relaxed);
                Y.store(position.y.to_bits(), Ordering::Relaxed);
            }
            None
        })
    }

    /// The last pointer position seen, in window coordinates.
    pub fn last() -> (f32, f32) {
        (
            f32::from_bits(X.load(Ordering::Relaxed)),
            f32::from_bits(Y.load(Ordering::Relaxed)),
        )
    }
}

/// `HYPRFORGE_FILES_DEBUG_MENU=x,y` opens a context menu on the first
/// row at that point once the first listing loads.
///
/// For looking at the menu. This machine has no way to synthesise a
/// right click, and the menu's placement and look are only checkable in
/// a screenshot. Compiled into debug builds only, the same way the lock
/// screen's `--type-in` is: nothing in a release build reads it.
#[cfg(debug_assertions)]
fn debug_menu_request() -> Option<(f32, f32)> {
    let raw = std::env::var("HYPRFORGE_FILES_DEBUG_MENU").ok()?;
    let (x, y) = raw.split_once(',')?;
    Some((x.trim().parse().ok()?, y.trim().parse().ok()?))
}

/// `HYPRFORGE_FILES_DEBUG_CONFLICT=1` opens the paste-conflict dialog on
/// a job that does nothing, for the same reason as
/// [`debug_menu_request`]: the dialog only appears mid-paste, and this
/// machine cannot drive a paste by hand. Debug builds only.
#[cfg(debug_assertions)]
fn debug_conflict(app: &mut App) {
    if std::env::var_os("HYPRFORGE_FILES_DEBUG_CONFLICT").is_none() {
        return;
    }
    let (control, _events) = jobs::start(0, Vec::new(), hyprforge_files_core::config::OnConflict::Ask);
    app.jobs.push(RunningJob {
        id: 0,
        control,
        verb: ClipVerb::Copy,
        dirs: Vec::new(),
        progress: None,
        conflict: Some(Collision {
            source: PathBuf::from("/tmp/quarterly report.pdf"),
            dest: app.home_dir.join("Documents").join("quarterly report.pdf"),
        }),
        apply_to_rest: false,
    });
}

/// What the status bar says when the window opens: a broken
/// `files.toml`, and anything in `files-config.toml` that could not be
/// used.
///
/// All of it, in one line. A person who mistyped two bindings should
/// learn about both at once rather than fix one, restart and find the
/// other.
fn startup_status(
    prefs_status: Option<String>,
    config_problems: &[hyprforge_files_core::config::ConfigProblem],
) -> Option<String> {
    let mut parts: Vec<String> = prefs_status.into_iter().collect();
    match config_problems {
        [] => {}
        [one] => parts.push(format!("files-config.toml: {one}")),
        many => parts.push(format!(
            "files-config.toml has {} problems: {}",
            many.len(),
            many.iter().map(|p| p.to_string()).collect::<Vec<_>>().join("; ")
        )),
    }
    (!parts.is_empty()).then(|| parts.join(" \u{00B7} "))
}

/// A tab's display name: its directory's own name, or the full path for
/// a directory with none (`/`).
fn tab_display_name(dir: &Path) -> String {
    dir.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_else(|| dir.display().to_string())
}

fn tab_dot_color(is_active: bool) -> Color {
    if is_active {
        // The active tab's dot is `accent` — the same purple the mockup
        // reserves for selection — because "which tab is this" is
        // exactly that kind of state. Reusing a role for a second,
        // decorative meaning (an arbitrary hue per tab) is the failure
        // CLAUDE.md's colour-role rule exists to prevent, so the inactive
        // dot is plain muted text instead of inventing one.
        hyprforge_ui::color::to_iced(hyprforge_ui::theme::active().accent)
    } else {
        hyprforge_ui::theme::text_dim()
    }
}

/// The clickable layer over a tab's drawn shape.
///
/// Transparent when active, because the canvas below has already painted
/// the fill and the outline; painting again here would double the fill
/// over the stroke and eat the open bottom edge. An inactive tab is
/// unpainted until hovered, and then gets only a fill — no outline, so
/// hovering never looks like activating.
fn tab_button_style(status: button::Status, is_active: bool) -> button::Style {
    let hovered = matches!(status, button::Status::Hovered);
    button::Style {
        // The active tab's fill lives here rather than in the canvas
        // above it, because the canvas draws on top now and a fill there
        // would cover the label.
        //
        // Hovering an inactive tab shows the same fill the active one
        // has — a preview of what clicking would do, which is what makes
        // the strip feel like it is made of targets. The active tab's
        // outline and its extra height are what still tell the two
        // apart, so the shared fill does not make them ambiguous.
        background: (is_active || hovered)
            .then(|| Background::Color(hyprforge_ui::theme::surface::row())),
        text_color: hyprforge_ui::theme::text(),
        border: Border {
            // Top corners only. A tab meets the pane below it; it does
            // not sit on it.
            radius: iced::border::top(hyprforge_files::tabstrip::TAB_RADIUS),
            width: 0.0,
            color: Color::TRANSPARENT,
        },
        ..button::Style::default()
    }
}



/// The floating window's outer chrome: background at the root surface,
/// corners rounded to the compositor's own `decoration:rounding` (via
/// `Theme::rounding`) rather than a value this app invented.
fn window_frame_style(_theme: &Theme) -> container::Style {
    // `density::outer_radius`, not a second read of `Theme::rounding`:
    // the radius ladder (12 outer / 6 inner / 4 nested) has one source,
    // and every other radius in this app already goes through it.
    let radius = hyprforge_files_core::density::outer_radius();
    container::Style {
        background: Some(Background::Color(hyprforge_ui::theme::surface::root())),
        border: Border { radius: radius.into(), width: 0.0, color: Color::TRANSPARENT },
        ..container::Style::default()
    }
}

/// One tab's widget: a coloured dot + name (switches to this tab) beside
/// a close button — two siblings, not a button nested in a button (iced
/// doesn't allow that), so the close affordance gets its own click target
/// distinct from the switch one.
fn tab_widget(index: usize, tab: &Tab, is_active: bool, scale: FontScale) -> Element<'_, Message> {
    let height = scale.apply(hyprforge_files::tabstrip::height(is_active));
    let mark = hyprforge_files::tabstrip::identity_mark(tab_dot_color(is_active), scale);
    let name = tab_display_name(tab.browser.current_dir());

    // Dim text on an inactive tab, full brightness on the active one.
    // The identity mark above keeps its colour either way — see
    // `tabstrip`'s own doc for why that asymmetry is deliberate.
    let label = scaled_text(name, BASE_TEXT_SIZE, scale)
        .color(if is_active {
            hyprforge_ui::theme::text()
        } else {
            hyprforge_ui::theme::text_dim()
        })
        // One line, clipped at the tab's edge. A name too long for a
        // fixed-width tab has to lose its tail somewhere, and wrapping
        // it would make the tab grow — which is exactly what a fixed
        // width exists to prevent.
        .wrapping(iced::widget::text::Wrapping::None)
        .width(Length::Fill);

    let mut content = row![mark, label].spacing(spacing::XS).align_y(iced::Alignment::Center);

    // The close affordance exists only on the active tab. On an inactive
    // one it would be a control for a thing you are not looking at, and
    // three of them across the strip is three chances to close the wrong
    // window.
    if is_active {
        content = content.push(
            button(scaled_text("\u{00d7}", BASE_TEXT_SIZE, scale).color(hyprforge_ui::theme::text_dim()))
                .on_press(Message::CloseTab(index))
                .padding(0)
                .style(|_t: &Theme, _status| button::Style {
                    background: None,
                    text_color: hyprforge_ui::theme::text_dim(),
                    ..button::Style::default()
                }),
        );
    }

    // The drawn shape underneath, the contents on top. `stack` rather
    // than a styled container because the active tab's outline is open
    // at the bottom, which iced's `Border` cannot express — see
    // `tabstrip`'s module doc.
    // The button first, so the stack sizes to the label rather than
    // stretching across the strip — see `tabstrip::outline`'s own doc.
    let width = scale.apply(hyprforge_files::tabstrip::TAB_WIDTH);
    iced::widget::stack![
        button(container(content).center_y(Length::Fill).padding([0, TAB_PADDING_X]))
            .on_press(Message::SwitchTab(index))
            .width(Length::Fixed(width))
            .height(Length::Fixed(height))
            .padding(0)
            .style(move |_t: &Theme, status| tab_button_style(status, is_active)),
        hyprforge_files::tabstrip::outline(is_active, width, height),
    ]
    .into()
}

/// How long a resize has to stay still before the new window size is
/// written to `files.toml`. Long enough that a drag writes once at the
/// end rather than continuously, short enough that letting go and
/// closing the window immediately still saves.
const RESIZE_SETTLE: u64 = 400;

/// A tab's horizontal padding. Declared for inactive tabs too, even
/// though nothing paints their background, so activating one makes a
/// fill appear in exactly the right shape with no reflow.
const TAB_PADDING_X: u16 = 14;

/// One iced key press, as the keymap reads it — or `None` for a key no
/// binding can name.
///
/// Two fields, deliberately from two places — the CLAUDE.md rule on the
/// three things iced reports for a key press:
///
/// - `key` comes from iced's *unmodified* `key`, which is what a binding
///   is matched against. Ctrl+Shift+N arrives as `n` with Ctrl and Shift
///   held, which is exactly how the binding is written.
/// - `text` comes from iced's `text`, what the press actually typed, and
///   is only ever used for typing into search. Reading `key` for that
///   would turn `Shift+/` into `/`.
fn key_press(event: &keyboard::Event) -> Option<keymap::KeyPress> {
    let keyboard::Event::KeyPressed { key: pressed, modifiers, text, .. } = event else {
        return None;
    };
    let mods = keymap::Modifiers {
        ctrl: modifiers.control(),
        alt: modifiers.alt(),
        shift: modifiers.shift(),
        logo: modifiers.logo(),
    };
    let key = match pressed.as_ref() {
        Key::Named(named) => match named {
            key::Named::Enter => keymap::Key::Enter,
            key::Named::Backspace => keymap::Key::Backspace,
            key::Named::Escape => keymap::Key::Escape,
            key::Named::Delete => keymap::Key::Delete,
            key::Named::Tab => keymap::Key::Tab,
            key::Named::Space => keymap::Key::Space,
            key::Named::ArrowUp => keymap::Key::Up,
            key::Named::ArrowDown => keymap::Key::Down,
            key::Named::ArrowLeft => keymap::Key::Left,
            key::Named::ArrowRight => keymap::Key::Right,
            key::Named::Home => keymap::Key::Home,
            key::Named::End => keymap::Key::End,
            key::Named::PageUp => keymap::Key::PageUp,
            key::Named::PageDown => keymap::Key::PageDown,
            key::Named::Insert => keymap::Key::Insert,
            key::Named::ContextMenu => keymap::Key::Menu,
            other => keymap::Key::F(function_key_number(other)?),
        },
        Key::Character(c) => {
            let mut chars = c.chars();
            match (chars.next(), chars.next()) {
                (Some(ch), None) => keymap::Key::Char(ch.to_lowercase().next().unwrap_or(ch)),
                _ => return None,
            }
        }
        _ => return None,
    };
    let text = text.as_ref().and_then(|t| {
        let mut chars = t.chars();
        match (chars.next(), chars.next()) {
            (Some(ch), None) => Some(ch),
            _ => None,
        }
    });
    Some(keymap::KeyPress { key, mods, text })
}

/// F1 to F24 as their number, `None` for any other named key.
fn function_key_number(named: key::Named) -> Option<u8> {
    use key::Named as N;
    let n = match named {
        N::F1 => 1,
        N::F2 => 2,
        N::F3 => 3,
        N::F4 => 4,
        N::F5 => 5,
        N::F6 => 6,
        N::F7 => 7,
        N::F8 => 8,
        N::F9 => 9,
        N::F10 => 10,
        N::F11 => 11,
        N::F12 => 12,
        N::F13 => 13,
        N::F14 => 14,
        N::F15 => 15,
        N::F16 => 16,
        N::F17 => 17,
        N::F18 => 18,
        N::F19 => 19,
        N::F20 => 20,
        N::F21 => 21,
        N::F22 => 22,
        N::F23 => 23,
        N::F24 => 24,
        _ => return None,
    };
    Some(n)
}

async fn trash_many(paths: Vec<PathBuf>) -> Vec<String> {
    match tokio::task::spawn_blocking(move || {
            paths
                .into_iter()
                .filter_map(|p| hyprforge_fileops::trash(&p).err().map(|e| e.to_string()))
                .collect::<Vec<_>>()
        })
        .await
    {
        Ok(errors) => errors,
        Err(e) => vec![format!("Deleting was interrupted: {e}")],
    }
}

/// How many folders one listing will count before giving up.
const COUNT_BUDGET: usize = 400;

/// Read-modify-write against `files.toml` off the UI thread, via
/// `hyprforge_files_core::prefs::update` as the brief names — see that
/// function's own doc for why a read-modify-write beats saving a copy
/// loaded earlier (two processes, the app and the portal dialog, can
/// each have their own idea of what the file last said).
async fn save_prefs(mutate: impl FnOnce(&mut Prefs) + Send + 'static) -> Result<Prefs, String> {
    match tokio::task::spawn_blocking(move || hyprforge_files_core::prefs::update(mutate)).await {
        Ok(Ok(prefs)) => Ok(prefs),
        Ok(Err(e)) => Err(e.to_string()),
        Err(e) => Err(format!("Saving your Files settings was interrupted: {e}")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // --- CLI argument / start-path resolution -----------------------------

    #[test]
    fn a_plain_path_resolves_unchanged() {
        assert_eq!(resolve_arg_path("/home/alex/Documents"), PathBuf::from("/home/alex/Documents"));
    }

    #[test]
    fn a_file_uri_decodes_to_the_same_plain_path() {
        assert_eq!(
            resolve_arg_path("file:///home/alex/Documents"),
            PathBuf::from("/home/alex/Documents")
        );
    }

    #[test]
    fn a_file_uri_with_a_localhost_authority_is_accepted() {
        assert_eq!(
            resolve_arg_path("file://localhost/home/alex/Documents"),
            PathBuf::from("/home/alex/Documents")
        );
    }

    #[test]
    fn a_file_uri_percent_decodes_a_space_in_the_path() {
        assert_eq!(
            resolve_arg_path("file:///home/alex/My%20Documents"),
            PathBuf::from("/home/alex/My Documents")
        );
    }

    #[test]
    fn resolve_start_dir_with_no_argument_is_home() {
        let backend = RoutingBackend::default();
        assert_eq!(resolve_start_dir(&backend, None), backend.home_dir());
    }

    #[test]
    fn resolve_start_dir_of_a_file_opens_its_parent() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("notes.txt");
        std::fs::write(&file, "hi").unwrap();
        let backend = RoutingBackend::default();
        assert_eq!(
            resolve_start_dir(&backend, Some(file.to_string_lossy().into_owned())),
            dir.path()
        );
    }

    /// The design choice this module's doc calls out by name: a
    /// nonexistent path is handed to `Browser` as-is rather than
    /// pre-validated and replaced with something safe, because
    /// `Browser`'s own error state already exists to say "this doesn't
    /// exist" and a second copy of that logic here would be redundant —
    /// and could disagree with it.
    #[test]
    fn a_nonexistent_start_path_is_not_short_circuited() {
        let dir = tempfile::tempdir().unwrap();
        let missing = dir.path().join("does-not-exist");
        let backend = RoutingBackend::default();
        assert_eq!(
            resolve_start_dir(&backend, Some(missing.to_string_lossy().into_owned())),
            missing
        );
    }

    // --- reading a nonexistent directory: error state, not empty --------

    #[test]
    fn reading_a_nonexistent_directory_is_an_error() {
        let dir = tempfile::tempdir().unwrap();
        let missing = dir.path().join("nope");
        let err = RoutingBackend::default()
            .read_dir(&missing)
            .map_err(|e| DirError::from(&e))
            .expect_err("a directory that was never there is an error");
        assert_eq!(err.kind, DirErrorKind::NotFound);
    }

    /// End to end through the same path the window takes: the CLI
    /// argument resolves to a path that doesn't exist, and the browser
    /// that opens on it ends up in `LoadState::Error`, never showing an
    /// empty, readable-looking folder.
    #[test]
    fn a_nonexistent_path_produces_the_error_state_not_an_empty_listing() {
        let dir = tempfile::tempdir().unwrap();
        let missing = dir.path().join("gone");
        let (mut browser, outcome) = Browser::new(Mode::App, Prefs::default(), missing.clone(), vec![]);
        assert_eq!(outcome, Outcome::ReadDir(missing.clone()));

        let result = RoutingBackend::default().read_dir(&missing).map_err(|e| DirError::from(&e));
        browser.update(BrowserMessage::DirLoaded(missing, result));

        assert!(browser.rows().is_empty());
        // Reaching into the same crate's own `LoadState` the way
        // `hyprforge_files_core::browser`'s own tests do — this asserts
        // the *kind* of empty, not just that nothing is shown.
        let is_error = format!("{:?}", browser).contains("Error(DirError");
        assert!(is_error, "expected an Error load state, browser debug: {browser:?}");
    }

    /// The names of a browser's rows, in order — what these tests are
    /// actually asserting about. `rows()` borrows out of the browser's
    /// one owned listing, so it cannot be compared against an owned
    /// `Vec<Entry>` directly.
    fn row_names(browser: &hyprforge_files_core::Browser) -> Vec<&str> {
        browser.rows().into_iter().map(|e| e.name.as_str()).collect()
    }

    fn names_of(entries: &[hyprforge_files_core::Entry]) -> Vec<&str> {
        entries.iter().map(|e| e.name.as_str()).collect()
    }

    // --- tabs: per-tab state, generation guard, close/cycle/jump ----------

    use hyprforge_files_core::backend::mock::MockBackend;
    // Test-only: the window builds no `Entry` of its own any more —
    // every listing comes from a backend.
    use hyprforge_files_core::{EntryKind, EntrySize};

    fn browser_at(dir: &str) -> Browser {
        let (browser, _) = Browser::new(Mode::App, Prefs::default(), PathBuf::from(dir), vec![]);
        browser
    }

    /// One tab per given directory, ids `0..n`, `next_tab_id` past the
    /// last one — the shape `main()` builds, minus the real I/O.
    fn app_for_test(dirs: &[&str]) -> App {
        let tabs: Vec<Tab> =
            dirs.iter().enumerate().map(|(id, dir)| Tab::new(id as u64, browser_at(dir))).collect();
        let next_tab_id = tabs.len() as u64;
        App {
            // The mock, not the real filesystem: these tests are about
            // the window's own loop, and nothing in them should depend
            // on what happens to be on the machine running them.
            backend: Arc::new(hyprforge_files_core::backend::mock::MockBackend::new()),
            tabs,
            active: 0,
            next_tab_id,
            sidebar_items: vec![],
            home_dir: PathBuf::from("/home/alex"),
            last_prefs: Prefs::default(),
                font_scale: FontScale::default(),
            status: None,
            last_window_size: (900, 600),
            resize_generation: 0,
            clipboard: Arc::new(MemoryClipboard::new()),
            jobs: Vec::new(),
            next_job_id: 1,
            modifiers: keyboard::Modifiers::default(),
            clicks: ClickTracker::new(),
            config: Arc::new(hyprforge_files_core::config::Config::default()),
            #[cfg(debug_assertions)]
            debug_menu: None,
        }
    }

    fn entry_named(name: &str) -> Entry {
        Entry {
            name: name.to_string(),
            path: PathBuf::from("/dir").join(name),
            is_dir: false,
            size: EntrySize::Bytes(1),
            modified: None,
            is_symlink: false,
            link_broken: false,
            hidden: false,
            kind: EntryKind::Other,
            // Fixtures: these tests are about tab routing and read
            // generations, none of which look at ownership.
            mode: 0o644,
            uid: 1000,
            owner: Some("alex".to_string()),
        }
    }

    /// The race this module's doc calls out: two reads for the very same
    /// directory, the first slow, the second fast. Without the
    /// generation guard, `Browser::apply_dir_loaded`'s own path check
    /// would happily accept the stale first result because its path
    /// still matches `current_dir`.
    #[test]
    fn a_stale_dir_loaded_result_is_ignored() {
        let mut app = app_for_test(&["/dir"]);
        let tab_id = app.tabs[0].id;

        // Simulate two reads having been issued for /dir: generation 1
        // (slow — about to arrive) and generation 2 (fast — arrives
        // first, below).
        app.tabs[0].read_generation = 2;

        let fresh = vec![entry_named("fresh.txt")];
        let _ = app.update(Message::DirLoaded(tab_id, 2, PathBuf::from("/dir"), Ok(fresh.clone())));
        assert_eq!(row_names(&app.tabs[0].browser), names_of(&fresh));

        // The stale generation-1 result now arrives, for the same path.
        let stale = vec![entry_named("stale.txt")];
        let _ = app.update(Message::DirLoaded(tab_id, 1, PathBuf::from("/dir"), Ok(stale)));

        assert_eq!(
            row_names(&app.tabs[0].browser),
            names_of(&fresh),
            "the stale generation-1 result must not have overwritten the fresh listing"
        );
    }

    #[test]
    fn a_matching_generation_is_applied() {
        let mut app = app_for_test(&["/dir"]);
        let tab_id = app.tabs[0].id;
        app.tabs[0].read_generation = 1;

        let entries = vec![entry_named("a.txt")];
        let _ = app.update(Message::DirLoaded(tab_id, 1, PathBuf::from("/dir"), Ok(entries.clone())));
        assert_eq!(row_names(&app.tabs[0].browser), names_of(&entries));
    }

    /// The bug the brief names as the one "most likely to exist and
    /// hardest to see by hand": tab 2's slow read must land in tab 2 even
    /// after the user has switched back to tab 1 and tab 1 is what's on
    /// screen when the result finally arrives.
    #[test]
    fn a_read_that_completes_after_switching_tabs_lands_in_the_tab_that_asked_for_it() {
        let mut app = app_for_test(&["/one", "/two"]);
        let tab_two_id = app.tabs[1].id;

        // Tab 2 issues a read (its generation becomes 1) while it's the
        // active tab...
        app.active = 1;
        let _ = app.spawn_read_dir(1, PathBuf::from("/two"));
        assert_eq!(app.tabs[1].read_generation, 1);

        // ...then the user switches back to tab 1 before it comes back.
        app.active = 0;

        let two_entries = vec![entry_named("from-tab-two.txt")];
        let _ = app.update(Message::DirLoaded(tab_two_id, 1, PathBuf::from("/two"), Ok(two_entries.clone())));

        assert_eq!(
            row_names(&app.tabs[1].browser),
            names_of(&two_entries),
            "the result must still reach tab 2, even though it is no longer visible"
        );
        assert!(
            app.tabs[0].browser.rows().is_empty(),
            "tab 1, the one actually on screen, must not have received tab 2's listing"
        );
    }

    /// A `DirLoaded`/`TrashDone` for a tab that has since closed must be
    /// a silent no-op, not a panic on a stale index — the same "the read
    /// outlived what asked for it" case as the generation guard, but at
    /// the level of the tab itself having gone away entirely.
    #[test]
    fn a_dir_loaded_for_a_since_closed_tab_is_ignored_without_panicking() {
        let mut app = app_for_test(&["/one", "/two"]);
        let closed_id = app.tabs[1].id;
        let _ = app.close_tab(1);
        assert_eq!(app.tabs.len(), 1, "the tab must actually be gone");

        let _ = app.update(Message::DirLoaded(closed_id, 1, PathBuf::from("/two"), Ok(vec![])));
        // No panic reaching here is most of what this test checks; the
        // rest is that the surviving tab was left alone.
        assert_eq!(app.tabs[0].browser.current_dir(), Path::new("/one"));
    }

    // --- tabs: switching keeps each tab's own directory and history -------

    #[test]
    fn each_tab_keeps_its_own_directory_and_history_across_a_switch() {
        let mut app = app_for_test(&["/one", "/two"]);

        // Navigate tab 1 somewhere else, then switch to tab 2 and
        // navigate it somewhere else too.
        let _ = app.update(Message::Browser(BrowserMessage::Navigate(PathBuf::from("/one/sub"))));
        assert_eq!(app.tabs[0].browser.current_dir(), Path::new("/one/sub"));

        app.active = 1;
        let _ = app.update(Message::Browser(BrowserMessage::Navigate(PathBuf::from("/two/sub"))));
        assert_eq!(app.tabs[1].browser.current_dir(), Path::new("/two/sub"));

        // Switching back to tab 1 must not have disturbed where it was,
        // nor its back-history (built by the `Navigate` above).
        app.active = 0;
        assert_eq!(
            app.tabs[0].browser.current_dir(),
            Path::new("/one/sub"),
            "tab 1 must still be where it was navigated, unaffected by tab 2's own navigation"
        );
        let outcome = app.tabs[0].browser.update(BrowserMessage::GoBack);
        assert_eq!(
            outcome,
            Outcome::ReadDir(PathBuf::from("/one")),
            "tab 1's own back-history, recorded before the switch, must still be there"
        );
    }

    // --- tabs: closing ------------------------------------------------------

    #[test]
    fn closing_a_middle_tab_activates_the_one_that_slides_into_its_place() {
        let mut app = app_for_test(&["/a", "/b", "/c"]);
        app.active = 2; // doesn't matter which tab is active for this
        let _ = app.close_tab(1);
        assert_eq!(app.tabs.len(), 2);
        assert_eq!(app.active, 1, "the tab that slid into slot 1 (originally /c) is the sensible neighbour");
        assert_eq!(app.tabs[app.active].browser.current_dir(), Path::new("/c"));
    }

    #[test]
    fn closing_the_rightmost_tab_activates_the_new_last_tab() {
        let mut app = app_for_test(&["/a", "/b", "/c"]);
        let _ = app.close_tab(2);
        assert_eq!(app.tabs.len(), 2);
        assert_eq!(app.active, 1, "there is no slot to slide into past the end; the new last tab is picked");
    }

    #[test]
    fn closing_the_last_tab_signals_the_window_should_close() {
        let mut app = app_for_test(&["/only"]);
        let _ = app.close_tab(0);
        assert!(app.tabs.is_empty(), "the tab list must actually be emptied so a later message can't reach a phantom tab");
    }

    #[test]
    fn closing_a_tab_index_that_no_longer_exists_does_nothing() {
        let mut app = app_for_test(&["/a"]);
        let _ = app.close_tab(5);
        assert_eq!(app.tabs.len(), 1, "an out-of-range close must be a no-op, not a panic");
    }

    /// [`neighbour_after_close`] pinned directly, independent of `App` —
    /// the decision `App::close_tab` above applies.
    #[test]
    fn neighbour_after_close_picks_the_slid_in_tab_or_the_new_last_one() {
        assert_eq!(neighbour_after_close(3, 0), TabCloseOutcome::Activate(0));
        assert_eq!(neighbour_after_close(3, 1), TabCloseOutcome::Activate(1));
        assert_eq!(neighbour_after_close(3, 2), TabCloseOutcome::Activate(1));
        assert_eq!(neighbour_after_close(1, 0), TabCloseOutcome::WindowShouldClose);
        assert_eq!(neighbour_after_close(3, 9), TabCloseOutcome::NoSuchTab);
    }

    // --- tabs: cycling and jumping ------------------------------------------

    #[test]
    fn ctrl_tab_cycles_forward_and_wraps() {
        let mut app = app_for_test(&["/a", "/b", "/c"]);
        app.cycle_tab(1);
        assert_eq!(app.active, 1);
        app.cycle_tab(1);
        assert_eq!(app.active, 2);
        app.cycle_tab(1);
        assert_eq!(app.active, 0, "cycling past the last tab wraps to the first");
    }

    #[test]
    fn ctrl_shift_tab_cycles_backward_and_wraps() {
        let mut app = app_for_test(&["/a", "/b", "/c"]);
        app.cycle_tab(-1);
        assert_eq!(app.active, 2, "cycling back from the first tab wraps to the last");
    }

    #[test]
    fn ctrl_1_through_9_beyond_the_tab_count_does_nothing() {
        let mut app = app_for_test(&["/a", "/b"]);
        app.jump_to_tab(8); // Ctrl+9, zero-based — no ninth tab exists
        assert_eq!(app.active, 0, "an out-of-range jump must leave the active tab exactly where it was");
        app.jump_to_tab(1); // Ctrl+2, in range
        assert_eq!(app.active, 1);
    }

    #[test]
    fn a_clean_start_says_nothing() {
        assert_eq!(startup_status(None, &[]), None);
    }

    /// Every problem at once — fixing one and restarting to discover the
    /// next is the experience this avoids.
    #[test]
    fn every_config_problem_is_reported_together() {
        let (_, problems) = hyprforge_files_core::config::parse(
            "[keys]\ntrash = \"Ctrl+Banana\"\nnope = \"Ctrl+K\"\n",
            Path::new("files-config.toml"),
        );
        let status = startup_status(None, &problems).unwrap();
        assert!(status.contains("2 problems"), "{status}");
        assert!(status.contains("Banana") && status.contains("nope"), "{status}");
    }

    #[test]
    fn a_broken_prefs_file_and_a_config_problem_are_both_shown() {
        let (_, problems) = hyprforge_files_core::config::parse(
            "[keys]\nnope = \"Ctrl+K\"\n",
            Path::new("files-config.toml"),
        );
        let status = startup_status(Some("files.toml is broken".into()), &problems).unwrap();
        assert!(status.starts_with("files.toml is broken"), "{status}");
        assert!(status.contains("nope"), "{status}");
    }

    // --- keys, through the one table ------------------------------------------
    //
    // Sent as `Message::KeyPressed` through `App::update`, the road a real
    // key takes, rather than asserting on `Keymap::resolve` alone — the
    // double-click bug was a unit that passed while the wiring was dead.

    fn key(binding: &str) -> Message {
        let combo = keymap::Combo::parse(binding).unwrap();
        Message::KeyPressed(keymap::KeyPress { key: combo.key, mods: combo.mods, text: None })
    }

    fn typed(c: char) -> Message {
        Message::KeyPressed(keymap::KeyPress {
            key: keymap::Key::Char(c),
            mods: keymap::Modifiers::default(),
            text: Some(c),
        })
    }

    #[test]
    fn ctrl_and_a_digit_jumps_to_that_tab_counted_from_one() {
        let mut app = app_for_test(&["/a", "/b", "/c"]);
        let _ = app.update(key("Ctrl+3"));
        assert_eq!(app.active, 2);
        let _ = app.update(key("Ctrl+1"));
        assert_eq!(app.active, 0);
        let _ = app.update(key("Ctrl+9"));
        assert_eq!(app.active, 0, "no ninth tab: nothing happens");
        let _ = app.update(key("Ctrl+0"));
        assert_eq!(app.active, 0, "there is no tab 0");
    }

    #[test]
    fn ctrl_tab_and_ctrl_shift_tab_cycle_and_wrap() {
        let mut app = app_for_test(&["/a", "/b"]);
        let _ = app.update(key("Ctrl+Tab"));
        assert_eq!(app.active, 1);
        let _ = app.update(key("Ctrl+Tab"));
        assert_eq!(app.active, 0, "wraps forward");
        let _ = app.update(key("Ctrl+Shift+Tab"));
        assert_eq!(app.active, 1, "wraps backward");
    }

    #[test]
    fn ctrl_t_opens_a_tab_and_ctrl_w_closes_the_active_one() {
        let mut app = app_for_test(&["/a"]);
        let _ = app.update(key("Ctrl+T"));
        assert_eq!(app.tabs.len(), 2);
        assert_eq!(app.active, 1, "the new tab is the active one");
        let _ = app.update(key("Ctrl+W"));
        assert_eq!(app.tabs.len(), 1);
    }

    /// A plain character types into the search box of the active tab,
    /// and Escape clears it — the two ends of type-to-search.
    #[test]
    fn a_typed_character_searches_the_active_tab() {
        let mut app = app_for_test(&["/dir"]);
        let _ = app.update(Message::Browser(BrowserMessage::DirLoaded(
            PathBuf::from("/dir"),
            Ok(vec![entry_named("alpha.txt"), entry_named("beta.txt")]),
        )));
        let _ = app.update(typed('b'));
        assert_eq!(app.active_tab().browser.rows().len(), 1);
        let _ = app.update(key("Escape"));
        assert_eq!(app.active_tab().browser.rows().len(), 2);
    }

    #[test]
    fn ctrl_a_selects_every_shown_file() {
        let mut app = app_for_test(&["/dir"]);
        let _ = app.update(Message::Browser(BrowserMessage::DirLoaded(
            PathBuf::from("/dir"),
            Ok(vec![entry_named("a.txt"), entry_named("b.txt"), entry_named("c.txt")]),
        )));
        let _ = app.update(key("Ctrl+A"));
        assert_eq!(app.active_tab().browser.selected_shown().len(), 3);
    }

    /// The Menu key goes out to the window for the pointer and back in
    /// as an open menu — the whole round trip, through `update`.
    #[test]
    fn the_menu_key_opens_a_context_menu() {
        let mut app = app_for_test(&["/dir"]);
        let _ = app.update(Message::Browser(BrowserMessage::DirLoaded(
            PathBuf::from("/dir"),
            Ok(vec![entry_named("a.txt")]),
        )));
        let _ = app.update(key("Menu"));
        assert!(app.active_tab().browser.menu_open());
        let _ = app.update(key("Escape"));
        assert!(!app.active_tab().browser.menu_open());
    }

    #[test]
    fn a_right_click_from_the_view_opens_a_menu() {
        let mut app = app_for_test(&["/dir"]);
        let _ = app.update(Message::Browser(BrowserMessage::DirLoaded(
            PathBuf::from("/dir"),
            Ok(vec![entry_named("a.txt")]),
        )));
        let _ = app.update(Message::Browser(BrowserMessage::OpenContextMenu {
            spot: hyprforge_files_core::browser::MenuSpot::Row(0),
            at: (0.0, 0.0),
        }));
        assert!(app.active_tab().browser.menu_open());
    }

    /// A Ctrl-held letter nobody bound must not type into search — it is
    /// somebody's shortcut, just not ours.
    #[test]
    fn an_unbound_ctrl_letter_does_nothing() {
        let mut app = app_for_test(&["/dir"]);
        let _ = app.update(Message::Browser(BrowserMessage::DirLoaded(
            PathBuf::from("/dir"),
            Ok(vec![entry_named("a.txt")]),
        )));
        let _ = app.update(Message::KeyPressed(keymap::KeyPress {
            key: keymap::Key::Char('q'),
            mods: keymap::Modifiers { ctrl: true, ..keymap::Modifiers::default() },
            text: Some('q'),
        }));
        assert_eq!(app.active_tab().browser.rows().len(), 1, "no search was typed");
    }

    // --- ctrl/shift click ---------------------------------------------------

    /// Multi-select is only reachable if the modifiers held at click
    /// time reach the browser. The view cannot supply them — a click in
    /// iced 0.14 carries none — so the window does, and this is the
    /// property that says it still does.
    #[test]
    fn a_row_click_is_told_which_modifiers_were_held() {
        let mut app = app_for_test(&["/dir"]);
        let entries = vec![
            entry_named("a.txt"),
            entry_named("b.txt"),
            entry_named("c.txt"),
        ];
        let _ = app.update(Message::Browser(BrowserMessage::DirLoaded(
            PathBuf::from("/dir"),
            Ok(entries),
        )));

        // A plain click selects one.
        let _ = app.update(Message::Browser(BrowserMessage::EntryClicked {
            index: 0,
            ctrl: false,
            shift: false,
        }));
        assert_eq!(app.active_tab().browser.selection().selected_paths().len(), 1);

        // Ctrl goes down, and the same message — which the view still
        // builds with `ctrl: false` — now adds to the selection instead
        // of replacing it.
        let _ = app.update(Message::ModifiersChanged(
            keyboard::Modifiers::CTRL,
        ));
        let _ = app.update(Message::Browser(BrowserMessage::EntryClicked {
            index: 2,
            ctrl: false,
            shift: false,
        }));
        assert_eq!(
            app.active_tab().browser.selection().selected_paths().len(),
            2,
            "ctrl-click must add to the selection, not replace it"
        );
    }

    /// Shift-click ranges, through the same route.
    #[test]
    fn shift_click_selects_a_range_through_the_windows_modifier_state() {
        let mut app = app_for_test(&["/dir"]);
        let entries = vec![entry_named("a.txt"), entry_named("b.txt"), entry_named("c.txt")];
        let _ = app.update(Message::Browser(BrowserMessage::DirLoaded(
            PathBuf::from("/dir"),
            Ok(entries),
        )));
        let _ = app.update(Message::Browser(BrowserMessage::EntryClicked {
            index: 0,
            ctrl: false,
            shift: false,
        }));
        let _ = app.update(Message::ModifiersChanged(keyboard::Modifiers::SHIFT));
        let _ = app.update(Message::Browser(BrowserMessage::EntryClicked {
            index: 2,
            ctrl: false,
            shift: false,
        }));
        assert_eq!(app.active_tab().browser.selection().selected_paths().len(), 3);
    }

    // --- double click to open ------------------------------------------------
    //
    // These go through `App::update` with real `Message::Browser` values
    // rather than testing `ClickTracker` alone, because the bug they
    // exist to catch was not in the timing at all — it was that the
    // click never reached anything. `mouse_area::on_double_click`
    // wrapped around a `button` is silently inert, since `button`
    // captures the press first. A test of the tracker would have passed
    // throughout.

    fn clicked(index: usize) -> Message {
        Message::Browser(BrowserMessage::EntryClicked { index, ctrl: false, shift: false })
    }

    /// Two quick clicks on a folder navigate into it.
    #[test]
    fn double_clicking_a_folder_opens_it() {
        let mut app = app_for_test(&["/dir"]);
        let _ = app.update(Message::Browser(BrowserMessage::DirLoaded(
            PathBuf::from("/dir"),
            Ok(vec![Entry {
                name: "sub".to_string(),
                path: PathBuf::from("/dir/sub"),
                is_dir: true,
                size: EntrySize::UNCOUNTED,
                kind: EntryKind::Folder,
                ..entry_named("sub")
            }]),
        )));

        let _ = app.update(clicked(0));
        assert_eq!(
            app.active_tab().browser.current_dir(),
            Path::new("/dir"),
            "one click selects and stays put"
        );

        let _ = app.update(clicked(0));
        assert_eq!(
            app.active_tab().browser.current_dir(),
            Path::new("/dir/sub"),
            "the second click opens the folder"
        );
    }

    /// And the first click still selects, which is what makes it
    /// possible to see what you are about to open.
    #[test]
    fn the_first_click_of_a_pair_still_selects() {
        let mut app = app_for_test(&["/dir"]);
        let _ = app.update(Message::Browser(BrowserMessage::DirLoaded(
            PathBuf::from("/dir"),
            Ok(vec![entry_named("a.txt"), entry_named("b.txt")]),
        )));
        let _ = app.update(clicked(1));
        assert_eq!(app.active_tab().browser.selection().selected_paths().len(), 1);
    }

    /// A ctrl-click is a selection gesture, never half of an open — and
    /// two of them in quick succession must not open anything.
    #[test]
    fn a_modifier_held_click_never_opens_anything() {
        let mut app = app_for_test(&["/dir"]);
        let _ = app.update(Message::Browser(BrowserMessage::DirLoaded(
            PathBuf::from("/dir"),
            Ok(vec![Entry {
                name: "sub".to_string(),
                path: PathBuf::from("/dir/sub"),
                is_dir: true,
                size: EntrySize::UNCOUNTED,
                kind: EntryKind::Folder,
                ..entry_named("sub")
            }]),
        )));
        let _ = app.update(Message::ModifiersChanged(keyboard::Modifiers::CTRL));
        let _ = app.update(clicked(0));
        let _ = app.update(clicked(0));
        assert_eq!(
            app.active_tab().browser.current_dir(),
            Path::new("/dir"),
            "ctrl-clicking twice selects and deselects; it does not open"
        );
    }

    /// Navigating ends a click sequence. Otherwise the press that opened
    /// a folder could pair with the first press in the folder it opened,
    /// and that one would open too.
    #[test]
    fn a_click_in_a_newly_opened_folder_does_not_pair_with_the_one_that_opened_it() {
        let mut app = app_for_test(&["/dir"]);
        let _ = app.update(Message::Browser(BrowserMessage::DirLoaded(
            PathBuf::from("/dir"),
            Ok(vec![Entry {
                name: "sub".to_string(),
                path: PathBuf::from("/dir/sub"),
                is_dir: true,
                size: EntrySize::UNCOUNTED,
                kind: EntryKind::Folder,
                ..entry_named("sub")
            }]),
        )));
        let _ = app.update(clicked(0));
        let _ = app.update(clicked(0));
        // Now inside /dir/sub, holding one folder at index 0 again.
        let _ = app.update(Message::Browser(BrowserMessage::DirLoaded(
            PathBuf::from("/dir/sub"),
            Ok(vec![Entry {
                name: "deeper".to_string(),
                path: PathBuf::from("/dir/sub/deeper"),
                is_dir: true,
                size: EntrySize::UNCOUNTED,
                kind: EntryKind::Folder,
                ..entry_named("deeper")
            }]),
        )));
        let _ = app.update(clicked(0));
        assert_eq!(
            app.active_tab().browser.current_dir(),
            Path::new("/dir/sub"),
            "the first click in the new folder selects; it must not open"
        );
    }

    // --- copy, cut and paste ---------------------------------------------------

    /// A window on a real temporary folder, with its listing loaded.
    fn app_on(dir: &Path) -> App {
        let mut app = app_for_test(&[dir.to_str().unwrap()]);
        let entries = RoutingBackend::default().read_dir(dir).unwrap();
        let _ = app.update(Message::Browser(BrowserMessage::DirLoaded(dir.to_path_buf(), Ok(entries))));
        app
    }

    fn wait_until(what: &str, mut done: impl FnMut() -> bool) {
        let deadline = Instant::now() + std::time::Duration::from_secs(10);
        while !done() {
            assert!(Instant::now() < deadline, "timed out waiting for {what}");
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
    }

    /// The whole road from the keyboard to the disk, bar the stream iced
    /// would deliver job events on: select, copy, paste, and a second
    /// file appears.
    #[test]
    fn ctrl_c_then_ctrl_v_duplicates_a_file_on_disk() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("a.txt"), "hello").unwrap();
        let mut app = app_on(dir.path());

        let _ = app.update(key("Ctrl+A"));
        let _ = app.update(key("Ctrl+C"));
        assert!(app.active_tab().browser.action_context().can_paste, "the tab knows there is something");
        let _ = app.update(key("Ctrl+V"));
        assert_eq!(app.jobs.len(), 1, "a paste job is running");

        wait_until("the copy", || std::fs::read_dir(dir.path()).unwrap().count() == 2);
    }

    /// A cut that fully landed empties the clipboard — its files are no
    /// longer where it says they are — and every tab stops offering Paste.
    #[test]
    fn a_completed_cut_empties_the_clipboard() {
        let from = tempfile::tempdir().unwrap();
        let to = tempfile::tempdir().unwrap();
        std::fs::write(from.path().join("a.txt"), "x").unwrap();
        let mut app = app_on(from.path());
        let _ = app.update(key("Ctrl+A"));
        let _ = app.update(key("Ctrl+X"));

        let _ = app.update(Message::Browser(BrowserMessage::Navigate(to.path().to_path_buf())));
        let _ = app.update(key("Ctrl+V"));
        let id = app.jobs[0].id;
        assert_eq!(app.jobs[0].dirs, [to.path().to_path_buf(), from.path().to_path_buf()]);

        let _ = app.update(Message::Job(JobEvent::Finished {
            job: id,
            summary: JobSummary { done: 1, ..JobSummary::default() },
        }));
        assert!(app.jobs.is_empty());
        assert_eq!(app.clipboard.get(), None);
        assert!(!app.active_tab().browser.action_context().can_paste);
        wait_until("the move", || to.path().join("a.txt").exists());
    }

    /// A cut that did not fully land keeps the clipboard, so what is
    /// left can be pasted again.
    #[test]
    fn a_partial_cut_keeps_the_clipboard() {
        let mut app = app_for_test(&["/dir"]);
        app.clipboard
            .set(hyprforge_files_core::clipboard::FileClip {
                paths: vec!["/nowhere/a".into()],
                verb: ClipVerb::Cut,
            })
            .unwrap();
        let (control, _events) = jobs::start(9, vec![], hyprforge_files_core::config::OnConflict::Ask);
        app.jobs.push(RunningJob {
            id: 9,
            control,
            verb: ClipVerb::Cut,
            dirs: vec![],
            progress: None,
            conflict: None,
            apply_to_rest: false,
        });
        let _ = app.update(Message::Job(JobEvent::Finished {
            job: 9,
            summary: JobSummary { done: 1, failed: vec!["disk full".into()], ..JobSummary::default() },
        }));
        assert!(app.clipboard.get().is_some());
        assert!(app.status.as_deref().unwrap().contains("disk full"));
    }

    #[test]
    fn escape_during_a_conflict_cancels_the_job_rather_than_the_search() {
        let mut app = app_for_test(&["/dir"]);
        let (control, _events) = jobs::start(3, vec![], hyprforge_files_core::config::OnConflict::Ask);
        app.jobs.push(RunningJob {
            id: 3,
            control,
            verb: ClipVerb::Copy,
            dirs: vec![],
            progress: None,
            conflict: Some(Collision { source: "/a/x".into(), dest: "/dir/x".into() }),
            apply_to_rest: false,
        });
        let _ = app.update(key("Escape"));
        assert!(app.jobs[0].conflict.is_none(), "the dialog is gone");
    }

    #[test]
    fn a_paste_that_simply_worked_says_nothing() {
        assert_eq!(job_report(ClipVerb::Copy, &JobSummary { done: 3, ..JobSummary::default() }), None);
    }

    #[test]
    fn a_paste_report_names_what_went_wrong() {
        let summary = JobSummary {
            done: 1,
            skipped: 2,
            failed: vec!["x.txt: permission denied".into()],
            cancelled: true,
        };
        let text = job_report(ClipVerb::Cut, &summary).unwrap();
        assert!(text.contains("Stopped after 1 item moved"), "{text}");
        assert!(text.contains("2 items skipped"), "{text}");
        assert!(text.contains("permission denied"), "{text}");
    }

    /// Pasting a folder into itself is refused before any job starts.
    #[test]
    fn pasting_a_folder_into_itself_is_refused_up_front() {
        let dir = tempfile::tempdir().unwrap();
        let sub = dir.path().join("sub");
        std::fs::create_dir(&sub).unwrap();
        let mut app = app_on(dir.path());
        let _ = app.update(key("Ctrl+A"));
        let _ = app.update(key("Ctrl+C"));
        let _ = app.update(Message::Browser(BrowserMessage::Navigate(sub.clone())));
        let _ = app.update(key("Ctrl+V"));
        assert!(app.jobs.is_empty());
        assert!(app.status.as_deref().unwrap().contains("inside itself"));
    }

    // --- the backend seam --------------------------------------------------
    //
    // These are the tests the seam exists for. The window used to name
    // `StdBackend` inline in its read function and hold no backend at
    // all, so `MockBackend`'s carefully-built cases — an unreadable
    // directory, an entry that vanishes mid-read — were properties of
    // the mock and of nothing that ships. Reaching them from here is
    // what makes the trait more than a shape.

    fn mock_backend() -> hyprforge_files_core::backend::mock::MockBackend {
        hyprforge_files_core::backend::mock::MockBackend::new()
    }

    #[tokio::test]
    async fn the_window_reads_a_listing_through_the_backend_it_was_given() {
        let mock = mock_backend();
        mock.seed(
            "/dir",
            vec![
                MockBackend::file(Path::new("/dir"), "notes.txt", 12),
                MockBackend::dir(Path::new("/dir"), "sub"),
            ],
        );
        let backend: Arc<dyn FsBackend> = Arc::new(mock);

        let entries = read_dir_task(backend, PathBuf::from("/dir")).await.expect("the mock lists");
        assert_eq!(entries.len(), 2);
        assert_eq!(entries[0].size, EntrySize::Bytes(12));
        assert_eq!(entries[1].size, EntrySize::UNCOUNTED, "a folder starts uncounted");
    }

    /// A directory that exists and cannot be read is an error, never an
    /// empty listing — CLAUDE.md's `hlconfig::storage` rule. Provable
    /// here without arranging a real permission failure on the machine
    /// running the tests, which is the whole argument for the mock.
    #[tokio::test]
    async fn an_unreadable_directory_reaches_the_window_as_an_error_not_as_empty() {
        let mock = mock_backend();
        mock.seed("/locked", vec![]);
        mock.make_unreadable("/locked");
        let backend: Arc<dyn FsBackend> = Arc::new(mock);

        let err = read_dir_task(backend, PathBuf::from("/locked"))
            .await
            .expect_err("a directory we cannot read is an error");
        assert_eq!(err.kind, DirErrorKind::PermissionDenied);
    }

    /// The second pass goes through the same backend as the listing, so
    /// a folder's count is answered by whatever described the folder.
    #[tokio::test]
    async fn folder_counts_are_answered_by_the_same_backend_as_the_listing() {
        let mock = mock_backend();
        mock.seed("/dir", vec![MockBackend::dir(Path::new("/dir"), "sub")]);
        mock.seed(
            "/dir/sub",
            vec![
                MockBackend::file(Path::new("/dir/sub"), "a", 1),
                MockBackend::file(Path::new("/dir/sub"), "b", 1),
            ],
        );
        mock.seed("/dir/locked", vec![]);
        mock.make_unreadable("/dir/locked");
        let backend: Arc<dyn FsBackend> = Arc::new(mock);

        let counts = count_folders(
            backend,
            vec![PathBuf::from("/dir/sub"), PathBuf::from("/dir/locked")],
        )
        .await;
        assert_eq!(counts[0], (PathBuf::from("/dir/sub"), Some(2)));
        // `None`, which becomes `ItemCount::Unreadable` — never
        // `Some(0)`, which would render as "0 items".
        assert_eq!(counts[1], (PathBuf::from("/dir/locked"), None));
    }

    // The Trash's own listing behaviour — that it shows original names
    // rather than stored ones, skips a vanished item, and is recognised
    // through a symlink — is tested in `hyprforge_files_core::trash`,
    // where it now lives as a backend rather than as a branch here.

    // --- sanity: unused-import guard for SidebarItem in future tests ----
    #[test]
    fn sidebar_item_type_is_reachable_from_this_crate() {
        let _ = SidebarItem {
            label: "Home".to_string(),
            path: PathBuf::from("/home"),
            tint: hyprforge_files_core::sidebar::Tint::Accent,
        };
    }
}
