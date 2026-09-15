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

use hyprforge_files_core::backend::{FsBackend, StdBackend};
use hyprforge_files_core::browser::Message as BrowserMessage;
use hyprforge_files_core::sidebar::SidebarItem;
use hyprforge_files_core::{
    keymap, sidebar, xdg_user_dirs, Browser, DirError, DirErrorKind, Entry, EntryKind, Mode,
    Outcome, Prefs,
};
use hyprforge_ui::theme::{app_theme, spacing, FontScale, BASE_TEXT_SIZE};
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

    let backend = StdBackend;
    let start_dir = resolve_start_dir(&backend, std::env::args().nth(1));

    // A `files.toml` that exists and will not parse must be reported,
    // never silently defaulted — see `hyprforge_files_core::prefs`'s own
    // doc and CLAUDE.md's rule on `hlconfig::storage`. Defaults are still
    // used so the window opens rather than refusing to start, but the
    // user is told, and nothing here saves over the broken file until
    // they fix it (the next `prefs::update` call refuses to write over a
    // file it can't reload — see `Message::PrefsSaved`'s handling below).
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
    let sidebar_items = sidebar::build(&backend, &user_dirs);

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
    let (browser, outcome) = Browser::new(Mode::App, prefs.clone(), start_dir, sidebar_items.clone());
    let mut app = App {
        tabs: vec![Tab::new(0, browser)],
        active: 0,
        next_tab_id: 1,
        sidebar_items,
        home_dir: backend.home_dir(),
        last_prefs: prefs,
        font_scale: FontScale(hyprforge_ui::theme::active().font_scale),
        status: prefs_status,
        last_window_size,
    };
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
fn resolve_start_dir(backend: &impl FsBackend, arg: Option<String>) -> PathBuf {
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
    let decoded = percent_decode(rest);
    if decoded.starts_with('/') {
        PathBuf::from(decoded)
    } else {
        // No leading slash means no `///` triple-slash form was used —
        // still an absolute local path per the scheme, so one is added
        // rather than resolving it against whatever the cwd happens to
        // be.
        PathBuf::from(format!("/{decoded}"))
    }
}

/// Percent-decoding, the one piece of URI handling `resolve_arg_path`
/// needs. `hyprforge-fileops::percent` does the same job for `.trashinfo`
/// files but is a private module of that crate — this crate is scoped to
/// `hyprforge-files/` only, so this is its own small copy rather than a
/// cross-crate change out of scope for this pass.
fn percent_decode(s: &str) -> String {
    let bytes = s.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' && i + 2 < bytes.len() {
            if let (Some(hi), Some(lo)) = (hex_val(bytes[i + 1]), hex_val(bytes[i + 2])) {
                out.push(hi * 16 + lo);
                i += 3;
                continue;
            }
        }
        out.push(bytes[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

fn hex_val(b: u8) -> Option<u8> {
    match b {
        b'0'..=b'9' => Some(b - b'0'),
        b'a'..=b'f' => Some(b - b'a' + 10),
        b'A'..=b'F' => Some(b - b'A' + 10),
        _ => None,
    }
}

// ---------------------------------------------------------------------
// Directory reads, off the UI thread
// ---------------------------------------------------------------------

async fn read_dir_task(path: PathBuf) -> Result<Vec<Entry>, DirError> {
    match tokio::task::spawn_blocking(move || read_dir_sync(&path)).await {
        Ok(result) => result,
        Err(e) => Err(DirError {
            message: format!("Reading this folder was interrupted: {e}"),
            kind: DirErrorKind::Other,
        }),
    }
}

/// The synchronous half of a directory read. Runs on the background
/// runtime, never on the UI thread.
///
/// One special case: the Trash sidebar item points at the trash's own
/// `files/` directory, and reading that with a plain `read_dir` shows
/// the *stored* names — `hyprland.conf.bak` with nothing saying it came
/// from `~/.config/hypr/`, and a `Monkey Around.2.mp4` that was never
/// called that (the `.2` is the trash's own collision suffix, not part
/// of the real name). [`is_trash_dir`] recognises that one path and
/// [`trash_entries`] builds the listing from `hyprforge_fileops::trash::list`
/// instead, using each item's *original* name.
fn read_dir_sync(path: &Path) -> Result<Vec<Entry>, DirError> {
    let trash_files_dir = hyprforge_fileops::home_trash_dir().join("files");
    if is_trash_dir(path, &trash_files_dir) {
        return trash_entries().map_err(|message| DirError { message, kind: DirErrorKind::Other });
    }
    StdBackend.read_dir(path).map_err(|e| DirError::from(&e))
}

/// Whether `candidate` is the trash's `files/` directory — compared
/// through `canonicalize` first, so a symlink to it or a spelling with a
/// redundant `..` still matches, and falling back to a plain equality
/// check when either side cannot be canonicalized (most commonly: the
/// trash has never been used, so `files/` does not exist yet — that must
/// not make the comparison silently say "not the trash" and fall through
/// to an ordinary, always-empty `read_dir`, which would look identical
/// to "you have nothing in the trash" for the wrong reason).
fn is_trash_dir(candidate: &Path, trash_files_dir: &Path) -> bool {
    match (std::fs::canonicalize(candidate), std::fs::canonicalize(trash_files_dir)) {
        (Ok(a), Ok(b)) => a == b,
        _ => candidate == trash_files_dir,
    }
}

fn trash_entries() -> Result<Vec<Entry>, String> {
    trash_entries_from(&hyprforge_fileops::home_trash_dir())
}

/// [`trash_entries`], parameterised over the trash directory — the seam
/// [`tests`] uses to point this at a throwaway directory instead of the
/// real `$XDG_DATA_HOME/Trash`.
fn trash_entries_from(trash_dir: &Path) -> Result<Vec<Entry>, String> {
    let items = hyprforge_fileops::list(trash_dir).map_err(|e| e.to_string())?;
    Ok(items.iter().filter_map(trash_entry).collect())
}

/// One [`hyprforge_fileops::TrashedItem`] as a listing [`Entry`] — name
/// from `original_path` (never the on-disk stored name), path pointing
/// at the real trashed file (so activating it still works), kind/size/
/// dir-ness read from that file's own metadata the same way
/// `StdBackend::read_dir` builds every other entry.
///
/// `None` for an item whose trashed file has itself vanished since
/// `list()` walked `info/` — the same "one bad entry must not sink the
/// whole listing" rule `StdBackend::read_dir` already follows, applied
/// here for the same reason.
fn trash_entry(item: &hyprforge_fileops::TrashedItem) -> Option<Entry> {
    let name = item.original_path.file_name()?.to_string_lossy().into_owned();
    let meta = std::fs::symlink_metadata(&item.trashed_file).ok()?;
    let is_symlink = meta.file_type().is_symlink();
    let is_dir = if is_symlink {
        std::fs::metadata(&item.trashed_file).map(|m| m.is_dir()).unwrap_or(false)
    } else {
        meta.is_dir()
    };
    let size = if is_dir { 0 } else { meta.len() };
    // `deleted_at` first — it is what the row is actually about ("when
    // did this leave"), the trashed file's own mtime second only as a
    // fallback if that timestamp somehow fails to parse.
    let modified = parse_deletion_date(&item.deleted_at).or_else(|| meta.modified().ok());
    Some(Entry {
        is_dir,
        size,
        modified,
        is_symlink,
        // A restored/re-trashed item's link brokenness is not tracked by
        // the trash spec at all; treating it as never-broken matches
        // what `list()` itself reports and keeps this mapping simple —
        // a broken-link badge on a trashed item is a nicety this pass
        // does not attempt.
        link_broken: false,
        hidden: name.starts_with('.'),
        kind: EntryKind::classify(is_dir, &name),
        name,
        path: item.trashed_file.clone(),
    })
}

/// Parses a `.trashinfo` `DeletionDate=` value (`YYYY-MM-DDThh:mm:ss`,
/// local time, no timezone suffix — see `hyprforge_fileops::trash`'s own
/// doc) into a [`SystemTime`].
///
/// Treated as UTC rather than the host's real local time: this crate has
/// no timezone database dependency, and the error that introduces is
/// bounded by the host's own UTC offset (at most about 14 hours) — an
/// acceptable approximation for "when was this put in the trash", a
/// value this crate only ever displays, never sorts against a strict
/// deadline. `None` for anything that does not parse as that exact
/// shape; the caller falls back to the trashed file's own mtime.
fn parse_deletion_date(s: &str) -> Option<std::time::SystemTime> {
    let bytes = s.as_bytes();
    if bytes.len() != 19 {
        return None;
    }
    let field = |range: std::ops::Range<usize>| s.get(range)?.parse::<i64>().ok();
    if &s[4..5] != "-" || &s[7..8] != "-" || &s[10..11] != "T" || &s[13..14] != ":" || &s[16..17] != ":" {
        return None;
    }
    let year = field(0..4)?;
    let month = field(5..7)?;
    let day = field(8..10)?;
    let hour = field(11..13)?;
    let minute = field(14..16)?;
    let second = field(17..19)?;

    let days = days_from_civil(year, month, day)?;
    let secs = days.checked_mul(86_400)?.checked_add(hour * 3600 + minute * 60 + second)?;
    if secs < 0 {
        return None;
    }
    Some(std::time::UNIX_EPOCH + std::time::Duration::from_secs(secs as u64))
}

/// Days since the Unix epoch for a proleptic-Gregorian `(year, month,
/// day)` — Howard Hinnant's `days_from_civil` algorithm, chosen over
/// adding a date/time crate dependency to this crate for the sake of one
/// conversion. `None` for a month or day out of range.
fn days_from_civil(y: i64, m: i64, d: i64) -> Option<i64> {
    if !(1..=12).contains(&m) || !(1..=31).contains(&d) {
        return None;
    }
    let y = if m <= 2 { y - 1 } else { y };
    let era = if y >= 0 { y } else { y - 399 } / 400;
    let yoe = y - era * 400; // [0, 399]
    let mp = (m + 9) % 12; // [0, 11]
    let doy = (153 * mp + 2) / 5 + d - 1; // [0, 365]
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy; // [0, 146096]
    Some(era * 146_097 + doe - 719_468)
}

// ---------------------------------------------------------------------
// The window
// ---------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq)]
enum Message {
    Browser(BrowserMessage),
    BrowserKey(keymap::Key, keymap::Modifiers),
    /// The tab a directory read was issued for, the generation it was
    /// issued under, the directory it was for, and what came back — see
    /// the module doc's note on `read_generation`, and [`Tab`]'s own doc
    /// for why the guard is per-tab rather than per-window now that a
    /// window can have several reads in flight at once, one per tab.
    DirLoaded(u64, u64, PathBuf, Result<Vec<Entry>, DirError>),
    DeleteSelected,
    /// The tab a trash operation was started from, the directory to
    /// refresh, and what (if anything) failed.
    TrashDone(u64, PathBuf, Vec<String>),
    PrefsSaved(Result<Prefs, String>),
    WindowResized(Size),
    DismissStatus,
    /// `Ctrl+T`, or the titlebar's `+`.
    NewTab,
    /// `Ctrl+W`, or a tab's own close affordance — by position in
    /// `App::tabs` at the moment the message was produced (view and
    /// update run on the same state between one click and the next, the
    /// same assumption every other index-carrying message in this file
    /// already makes).
    CloseTab(usize),
    SwitchTab(usize),
    /// `Ctrl+Tab` (`+1`) / `Ctrl+Shift+Tab` (`-1`).
    CycleTab(i32),
    /// `Ctrl+1`..`Ctrl+9`, zero-based. Silently does nothing past the
    /// last tab — see [`App::jump_to_tab`].
    JumpToTab(usize),
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
    last_window_size: (u32, u32),
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
        Task::perform(read_dir_task(path.clone()), move |result| {
            Message::DirLoaded(tab_id, generation, path.clone(), result)
        })
    }

    /// Carries out everything `Browser::update`/`handle_key` handed back
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
        }
    }

    /// Opens a new tab at `start_dir` and makes it active — `Ctrl+T` and
    /// the titlebar `+` both funnel here.
    fn open_tab(&mut self, start_dir: PathBuf) -> Task<Message> {
        let id = self.next_tab_id;
        self.next_tab_id += 1;
        let (browser, outcome) =
            Browser::new(Mode::App, self.last_prefs.clone(), start_dir, self.sidebar_items.clone());
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
            Message::Browser(msg) => {
                let outcome = self.active_tab_mut().browser.update(msg);
                self.handle_outcome(self.active, outcome)
            }
            Message::BrowserKey(key, mods) => {
                let outcome = self.active_tab_mut().browser.handle_key(key, mods);
                self.handle_outcome(self.active, outcome)
            }
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
                self.handle_outcome(index, outcome)
            }
            Message::DeleteSelected => {
                let tab = self.active_tab();
                let paths: Vec<PathBuf> = tab
                    .browser
                    .selection()
                    .selected_indices()
                    .iter()
                    .filter_map(|&i| tab.browser.view_entries().get(i).map(|e| e.path.clone()))
                    .collect();
                if paths.is_empty() {
                    return Task::none();
                }
                let dir = tab.browser.current_dir().to_path_buf();
                let tab_id = tab.id;
                Task::perform(trash_many(paths), move |errors| {
                    Message::TrashDone(tab_id, dir.clone(), errors)
                })
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
            Message::SwitchTab(index) => {
                if index < self.tabs.len() {
                    self.active = index;
                }
                Task::none()
            }
            Message::CycleTab(delta) => {
                self.cycle_tab(delta);
                Task::none()
            }
            Message::JumpToTab(index) => {
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
        let mut bar = row![].spacing(spacing::SM).align_y(iced::Alignment::Center).padding(spacing::SM);
        for (index, tab) in self.tabs.iter().enumerate() {
            bar = bar.push(tab_widget(index, tab, index == self.active, scale));
        }
        bar = bar.push(secondary_button("+").on_press(Message::NewTab));
        container(bar).width(Length::Fill).into()
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

        content = content.push(tab.browser.view(scale).map(Message::Browser));

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
        container(content).width(Length::Fill).height(Length::Fill).style(window_frame_style).into()
    }

    /// The keyboard grammar: everything `keymap::resolve` gives meaning
    /// to is forwarded to `Browser::handle_key` (see that module's own
    /// doc — this is the host wiring it names), plus Delete (trashing is
    /// this app's own concern, never the shared browsing view's or the
    /// portal dialog's) and the tab shortcuts below (same reasoning: a
    /// dialog has no tabs, so this grammar belongs to the window, not to
    /// `Browser`).
    fn subscription(&self) -> Subscription<Message> {
        // `Subscription::filter_map`/`map` require a non-capturing (zero-
        // sized) closure — `with` is iced's own way to thread state like
        // `self.active` into one anyway, as a tuple element instead of a
        // capture.
        let keys = keyboard::listen().with(self.active).filter_map(|(active, event)| {
            let keyboard::Event::KeyPressed { key, modifiers, .. } = &event else {
                return None;
            };
            if let Key::Named(key::Named::Delete) = key {
                return Some(Message::DeleteSelected);
            }
            if modifiers.control() {
                if let Some(msg) = tab_shortcut(key, modifiers.shift(), active) {
                    return Some(msg);
                }
            }
            to_browser_key(&event).map(|(k, m)| Message::BrowserKey(k, m))
        });
        Subscription::batch([
            keys,
            window::resize_events().map(|(_, size)| Message::WindowResized(size)),
        ])
    }
}

/// A tab's display name: its directory's own name, or the full path for
/// a directory with none (`/`).
fn tab_display_name(dir: &Path) -> String {
    dir.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_else(|| dir.display().to_string())
}

/// The Ctrl-held keys the titlebar's tab strip gives meaning to — new,
/// close, cycle, jump — kept separate from `to_browser_key`'s browsing
/// grammar so, for instance, Ctrl+T can never be misread as "type T into
/// the search box" (`keymap::resolve` already refuses a Ctrl-held
/// character for exactly that reason; this is the window-level analogue
/// of the same rule for keys `Browser` has no notion of at all).
fn tab_shortcut(key: &Key, shift: bool, active: usize) -> Option<Message> {
    match key.as_ref() {
        Key::Character("t") => Some(Message::NewTab),
        Key::Character("w") => Some(Message::CloseTab(active)),
        Key::Named(key::Named::Tab) => Some(Message::CycleTab(if shift { -1 } else { 1 })),
        Key::Character(c) => {
            let mut chars = c.chars();
            match (chars.next(), chars.next()) {
                (Some(d), None) if d.is_ascii_digit() && d != '0' => {
                    Some(Message::JumpToTab(d.to_digit(10).unwrap() as usize - 1))
                }
                _ => None,
            }
        }
        _ => None,
    }
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

fn tab_button_style(_theme: &Theme, status: button::Status, is_active: bool) -> button::Style {
    let base = button::Style {
        background: Some(Background::Color(if is_active {
            hyprforge_ui::theme::surface::card()
        } else {
            hyprforge_ui::theme::surface::sidebar()
        })),
        text_color: hyprforge_ui::theme::text(),
        border: Border { radius: 6.0.into(), width: 0.0, color: Color::TRANSPARENT },
        ..button::Style::default()
    };
    match status {
        button::Status::Hovered if !is_active => {
            button::Style { background: Some(Background::Color(hyprforge_ui::theme::surface::row())), ..base }
        }
        _ => base,
    }
}


/// The floating window's outer chrome: background at the root surface,
/// corners rounded to the compositor's own `decoration:rounding` (via
/// `Theme::rounding`) rather than a value this app invented.
fn window_frame_style(_theme: &Theme) -> container::Style {
    let radius = hyprforge_ui::theme::active().rounding as f32;
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
    let dot = container(column![])
        .width(Length::Fixed(8.0))
        .height(Length::Fixed(8.0))
        .style(move |_theme: &Theme| container::Style {
            background: Some(Background::Color(tab_dot_color(is_active))),
            border: Border { radius: 4.0.into(), ..Border::default() },
            ..container::Style::default()
        });
    let name = tab_display_name(tab.browser.current_dir());
    let label = row![dot, scaled_text(name, BASE_TEXT_SIZE, scale)]
        .spacing(spacing::XS)
        .align_y(iced::Alignment::Center);
    let select = button(label)
        .on_press(Message::SwitchTab(index))
        .style(move |t: &Theme, status| tab_button_style(t, status, is_active));
    // "\u{00d7}" — a plain multiplication sign, the close-affordance glyph
    // every one of these tab strips uses; not a character a keystroke
    // could produce, so it needs no `scaled_text`/FontScale route of its
    // own beyond what `secondary_button` already gives every label.
    let close = secondary_button("\u{00d7}").on_press(Message::CloseTab(index));
    row![select, close].spacing(2.0).align_y(iced::Alignment::Center).into()
}

/// Maps one iced key press to this browser's own [`keymap::Key`], or
/// `None` if it means nothing here (Delete is handled by the caller
/// separately — see [`App::subscription`]).
///
/// Named keys (Enter/Backspace/Escape/arrows) are read from `key`; a
/// printable character is read from `text`, never from `key` — the
/// CLAUDE.md rule on the three things iced reports for a keypress: `key`
/// is unmodified, so reading it for typed text would turn `Shift+/` into
/// `/` instead of `?` and silently mistype a search. This browser has no
/// password field, but the rule is the right default everywhere text
/// gets typed, not only where it happens to matter today.
fn to_browser_key(event: &keyboard::Event) -> Option<(keymap::Key, keymap::Modifiers)> {
    let keyboard::Event::KeyPressed { key: pressed, modifiers, text, .. } = event else {
        return None;
    };
    let mods =
        keymap::Modifiers { ctrl: modifiers.control(), alt: modifiers.alt(), shift: modifiers.shift() };
    let mapped = match pressed {
        Key::Named(key::Named::Enter) => Some(keymap::Key::Enter),
        Key::Named(key::Named::Backspace) => Some(keymap::Key::Backspace),
        Key::Named(key::Named::Escape) => Some(keymap::Key::Escape),
        Key::Named(key::Named::ArrowUp) => Some(keymap::Key::ArrowUp),
        Key::Named(key::Named::ArrowDown) => Some(keymap::Key::ArrowDown),
        Key::Named(key::Named::ArrowLeft) => Some(keymap::Key::ArrowLeft),
        Key::Named(key::Named::ArrowRight) => Some(keymap::Key::ArrowRight),
        _ => text.as_ref().and_then(|t| t.chars().next()).map(keymap::Key::Character),
    };
    mapped.map(|k| (k, mods))
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
        let backend = StdBackend;
        assert_eq!(resolve_start_dir(&backend, None), backend.home_dir());
    }

    #[test]
    fn resolve_start_dir_of_a_file_opens_its_parent() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("notes.txt");
        std::fs::write(&file, "hi").unwrap();
        let backend = StdBackend;
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
        let backend = StdBackend;
        assert_eq!(
            resolve_start_dir(&backend, Some(missing.to_string_lossy().into_owned())),
            missing
        );
    }

    // --- reading a nonexistent directory: error state, not empty --------

    #[test]
    fn read_dir_sync_of_a_nonexistent_directory_is_an_error() {
        let dir = tempfile::tempdir().unwrap();
        let missing = dir.path().join("nope");
        let err = read_dir_sync(&missing).expect_err("a directory that was never there is an error");
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

        let result = read_dir_sync(&missing);
        browser.update(BrowserMessage::DirLoaded(missing, result));

        assert!(browser.view_entries().is_empty());
        // Reaching into the same crate's own `LoadState` the way
        // `hyprforge_files_core::browser`'s own tests do — this asserts
        // the *kind* of empty, not just that nothing is shown.
        let is_error = format!("{:?}", browser).contains("Error(DirError");
        assert!(is_error, "expected an Error load state, browser debug: {browser:?}");
    }

    // --- tabs: per-tab state, generation guard, close/cycle/jump ----------

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
            tabs,
            active: 0,
            next_tab_id,
            sidebar_items: vec![],
            home_dir: PathBuf::from("/home/alex"),
            last_prefs: Prefs::default(),
                font_scale: FontScale::default(),
            status: None,
            last_window_size: (900, 600),
        }
    }

    fn entry_named(name: &str) -> Entry {
        Entry {
            name: name.to_string(),
            path: PathBuf::from("/dir").join(name),
            is_dir: false,
            size: 1,
            modified: None,
            is_symlink: false,
            link_broken: false,
            hidden: false,
            kind: EntryKind::Other,
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
        assert_eq!(app.tabs[0].browser.view_entries(), fresh.as_slice());

        // The stale generation-1 result now arrives, for the same path.
        let stale = vec![entry_named("stale.txt")];
        let _ = app.update(Message::DirLoaded(tab_id, 1, PathBuf::from("/dir"), Ok(stale)));

        assert_eq!(
            app.tabs[0].browser.view_entries(),
            fresh.as_slice(),
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
        assert_eq!(app.tabs[0].browser.view_entries(), entries.as_slice());
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
            app.tabs[1].browser.view_entries(),
            two_entries.as_slice(),
            "the result must still reach tab 2, even though it is no longer visible"
        );
        assert!(
            app.tabs[0].browser.view_entries().is_empty(),
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
    fn tab_shortcut_reads_ctrl_1_through_9_as_zero_based_indices() {
        assert_eq!(tab_shortcut(&Key::Character("1".into()), false, 0), Some(Message::JumpToTab(0)));
        assert_eq!(tab_shortcut(&Key::Character("9".into()), false, 0), Some(Message::JumpToTab(8)));
        assert_eq!(tab_shortcut(&Key::Character("0".into()), false, 0), None, "there is no tab 0");
        assert_eq!(tab_shortcut(&Key::Character("t".into()), false, 0), Some(Message::NewTab));
        assert_eq!(tab_shortcut(&Key::Character("w".into()), false, 3), Some(Message::CloseTab(3)));
    }

    // --- the Trash sidebar item -------------------------------------------

    #[test]
    fn is_trash_dir_matches_the_real_path() {
        let dir = tempfile::tempdir().unwrap();
        let trash_files = dir.path().join("Trash").join("files");
        std::fs::create_dir_all(&trash_files).unwrap();
        assert!(is_trash_dir(&trash_files, &trash_files));
    }

    /// The exact wording of the brief: "including via a symlinked or
    /// non-canonical spelling".
    #[test]
    fn is_trash_dir_recognises_a_symlink_and_a_noncanonical_spelling() {
        let dir = tempfile::tempdir().unwrap();
        let trash_files = dir.path().join("Trash").join("files");
        std::fs::create_dir_all(&trash_files).unwrap();

        let symlink = dir.path().join("trash-link");
        std::os::unix::fs::symlink(&trash_files, &symlink).unwrap();
        assert!(is_trash_dir(&symlink, &trash_files), "a symlink to the trash must still match");

        let noncanonical = dir.path().join("Trash").join(".").join("files");
        assert!(
            is_trash_dir(&noncanonical, &trash_files),
            "a `.`-laden spelling of the same path must still match"
        );

        let unrelated = dir.path().join("Documents");
        std::fs::create_dir_all(&unrelated).unwrap();
        assert!(!is_trash_dir(&unrelated, &trash_files));
    }

    /// The defect the brief names directly: the stored file in `files/`
    /// can carry a collision suffix (`thing.2.txt`) that was never part
    /// of the real name, and the directory a file was trashed *from* is
    /// nowhere in the stored name at all. The listing must show the
    /// *original* name from the `.trashinfo` file, not the on-disk one —
    /// while still pointing at the real on-disk path so activating it
    /// works.
    #[test]
    fn trash_entries_show_the_original_name_not_the_collision_suffixed_stored_one() {
        let dir = tempfile::tempdir().unwrap();
        let trash_dir = dir.path().join("Trash");
        let files_dir = trash_dir.join("files");
        let info_dir = trash_dir.join("info");
        std::fs::create_dir_all(&files_dir).unwrap();
        std::fs::create_dir_all(&info_dir).unwrap();

        // Stored under a disambiguated name, as if a second "thing.txt"
        // had already been trashed once before this one.
        std::fs::write(files_dir.join("thing.2.txt"), b"second copy").unwrap();
        std::fs::write(
            info_dir.join("thing.2.txt.trashinfo"),
            "[Trash Info]\nPath=/home/alex/projects/thing.txt\nDeletionDate=2024-03-01T12:30:00\n",
        )
        .unwrap();

        let entries = trash_entries_from(&trash_dir).unwrap();
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].name, "thing.txt", "must show the original name, not thing.2.txt");
        assert_eq!(entries[0].path, files_dir.join("thing.2.txt"), "must still point at the real file");
        assert!(!entries[0].is_dir);
        assert_eq!(entries[0].size, "second copy".len() as u64);
    }

    #[test]
    fn trash_entries_skip_an_item_whose_stored_file_has_vanished() {
        let dir = tempfile::tempdir().unwrap();
        let trash_dir = dir.path().join("Trash");
        let info_dir = trash_dir.join("info");
        std::fs::create_dir_all(&info_dir).unwrap();
        // No corresponding file under files/ — as if it were removed by
        // hand outside this crate.
        std::fs::write(
            info_dir.join("gone.txt.trashinfo"),
            "[Trash Info]\nPath=/home/alex/gone.txt\nDeletionDate=2024-03-01T12:30:00\n",
        )
        .unwrap();

        let entries = trash_entries_from(&trash_dir).unwrap();
        assert!(entries.is_empty(), "a vanished trashed file must be skipped, not error the whole listing");
    }

    #[test]
    fn a_missing_trash_directory_is_an_empty_listing_not_an_error() {
        let dir = tempfile::tempdir().unwrap();
        let never_used = dir.path().join("never-used-trash");
        assert_eq!(trash_entries_from(&never_used).unwrap(), Vec::new());
    }

    // --- deletion-date parsing ----------------------------------------------

    #[test]
    fn the_unix_epoch_parses_to_the_unix_epoch() {
        assert_eq!(parse_deletion_date("1970-01-01T00:00:00"), Some(std::time::UNIX_EPOCH));
    }

    #[test]
    fn a_later_date_parses_to_the_correct_offset() {
        // 2024-01-02T03:04:05 UTC is 1704164645 seconds after the epoch —
        // checked against a standard epoch converter, not derived from
        // this same algorithm.
        let parsed = parse_deletion_date("2024-01-02T03:04:05").unwrap();
        let secs = parsed.duration_since(std::time::UNIX_EPOCH).unwrap().as_secs();
        assert_eq!(secs, 1_704_164_645);
    }

    #[test]
    fn a_malformed_deletion_date_is_none_not_a_panic() {
        assert_eq!(parse_deletion_date("not-a-date-at-all"), None);
        assert_eq!(parse_deletion_date("2024/01/02 03:04:05"), None);
    }

    // --- sanity: unused-import guard for SidebarItem in future tests ----
    #[test]
    fn sidebar_item_type_is_reachable_from_this_crate() {
        let _ = SidebarItem { label: "Home".to_string(), path: PathBuf::from("/home") };
    }
}
