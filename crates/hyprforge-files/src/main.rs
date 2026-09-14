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
use hyprforge_files_core::prefs::ViewMode;
use hyprforge_files_core::sort::{SortColumn, SortDirection};
use hyprforge_files_core::{
    keymap, sidebar, xdg_user_dirs, Browser, DirError, DirErrorKind, Entry, EntryKind, Mode,
    Outcome, Prefs,
};
use hyprforge_ui::theme::{app_theme, spacing, FontScale, BASE_TEXT_SIZE};
use hyprforge_ui::widgets::{primary_button, scaled_text, secondary_button};
use iced::keyboard::{self, key, Key};
use iced::widget::{column, container, row};
use iced::{window, Element, Length, Size, Subscription, Task, Theme};
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

    let (browser, outcome) = Browser::new(Mode::App, prefs, start_dir, sidebar_items);
    let mut app = App {
        browser,
        font_scale: FontScale(hyprforge_ui::theme::active().font_scale),
        read_generation: 0,
        status: prefs_status,
        last_window_size,
    };
    // Fulfils the `Outcome::ReadDir` `Browser::new` always returns —
    // otherwise the window opens showing nothing at all, forever, for
    // the same reason CLAUDE.md's "a five-second gap" rule exists: an
    // outcome nobody satisfies is silent, not merely slow.
    let boot_task = app.handle_outcome(outcome);

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

#[derive(Debug, Clone)]
enum Message {
    Browser(BrowserMessage),
    BrowserKey(keymap::Key, keymap::Modifiers),
    /// The generation this read was issued under, the directory it was
    /// for, and what came back — see the module doc's note on
    /// `read_generation`.
    DirLoaded(u64, PathBuf, Result<Vec<Entry>, DirError>),
    DeleteSelected,
    TrashDone(PathBuf, Vec<String>),
    PrefsSaved(Result<Prefs, String>),
    WindowResized(Size),
    DismissStatus,
}

struct App {
    browser: Browser,
    font_scale: FontScale,
    read_generation: u64,
    /// One line shown at the bottom of the window: what
    /// `launch::open`/trashing/saving preferences said, if anything did.
    /// Pillar 3 — every error reaching the user is a sentence here, never
    /// a log line they are expected to go find.
    status: Option<String>,
    last_window_size: (u32, u32),
}

impl App {
    fn spawn_read_dir(&mut self, path: PathBuf) -> Task<Message> {
        self.read_generation += 1;
        let generation = self.read_generation;
        Task::perform(read_dir_task(path.clone()), move |result| {
            Message::DirLoaded(generation, path.clone(), result)
        })
    }

    /// Carries out everything `Browser::update`/`handle_key` handed back
    /// but could not do itself — see `hyprforge_files_core::browser::Outcome`'s
    /// own doc for why each of these belongs to the host.
    fn handle_outcome(&mut self, outcome: Outcome) -> Task<Message> {
        match outcome {
            Outcome::None => Task::none(),
            Outcome::ReadDir(path) => self.spawn_read_dir(path),
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

    fn update(&mut self, message: Message) -> Task<Message> {
        match message {
            Message::Browser(msg) => {
                let outcome = self.browser.update(msg);
                self.handle_outcome(outcome)
            }
            Message::BrowserKey(key, mods) => {
                let outcome = self.browser.handle_key(key, mods);
                self.handle_outcome(outcome)
            }
            Message::DirLoaded(generation, path, result) => {
                // Stale: a newer read has already been issued for
                // *some* directory (possibly this same one — see the
                // module doc) since this one went out. Dropping it here
                // means `Browser` never even sees it, which is stronger
                // than relying on `Browser::apply_dir_loaded`'s own
                // path-based staleness check alone.
                if generation != self.read_generation {
                    return Task::none();
                }
                let outcome = self.browser.update(BrowserMessage::DirLoaded(path, result));
                self.handle_outcome(outcome)
            }
            Message::DeleteSelected => {
                let paths: Vec<PathBuf> = self
                    .browser
                    .selection()
                    .selected_indices()
                    .iter()
                    .filter_map(|&i| self.browser.view_entries().get(i).map(|e| e.path.clone()))
                    .collect();
                if paths.is_empty() {
                    return Task::none();
                }
                let dir = self.browser.current_dir().to_path_buf();
                Task::perform(trash_many(paths), move |errors| Message::TrashDone(dir.clone(), errors))
            }
            Message::TrashDone(dir, errors) => {
                if !errors.is_empty() {
                    self.status =
                        Some(format!("Couldn't move everything to Trash: {}", errors.join("; ")));
                }
                // Refresh regardless of whether anything failed, so what
                // did succeed disappears from the listing. Safe even if
                // the user has since navigated elsewhere: this issues a
                // plain read (not a navigation), and `Browser` ignores a
                // `DirLoaded` for a directory that is no longer current.
                self.spawn_read_dir(dir)
            }
            Message::PrefsSaved(Ok(_)) => Task::none(),
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
        }
    }

    fn theme(&self) -> Theme {
        app_theme()
    }

    fn title(&self) -> String {
        let dir = self.browser.current_dir();
        let name = dir
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_else(|| dir.display().to_string());
        format!("{name} \u{2014} Hyprforge Files")
    }

    fn view(&self) -> Element<'_, Message> {
        let scale = self.font_scale;
        let prefs = self.browser.prefs();

        // The chrome `Browser::view` does not draw itself — back/forward/
        // up, the path bar and the search field are already inside it
        // (see `browser::render`); a sort order and a view mode are
        // preferences `Browser::update` handles but nothing in its own
        // `render` emits a control for, so the host adds one.
        let sort_button = |column: SortColumn, label: &'static str| {
            let active = prefs.sort_column() == column;
            let button = if active { primary_button(label) } else { secondary_button(label) };
            button.on_press(Message::Browser(BrowserMessage::SortBy(column)))
        };
        let direction_label = match prefs.sort_direction() {
            SortDirection::Ascending => "Sort \u{2191}",
            SortDirection::Descending => "Sort \u{2193}",
        };
        let hidden_label = if prefs.show_hidden { "Hide hidden" } else { "Show hidden" };
        let (view_label, view_target) = match prefs.view_mode {
            ViewMode::List => ("Grid view", ViewMode::Grid),
            ViewMode::Grid => ("List view", ViewMode::List),
        };

        let controls = row![
            sort_button(SortColumn::Name, "Name"),
            sort_button(SortColumn::Size, "Size"),
            sort_button(SortColumn::Modified, "Modified"),
            sort_button(SortColumn::Kind, "Kind"),
            secondary_button(direction_label).on_press(Message::Browser(BrowserMessage::ToggleSortDirection)),
            secondary_button(hidden_label).on_press(Message::Browser(BrowserMessage::ToggleShowHidden)),
            secondary_button(view_label).on_press(Message::Browser(BrowserMessage::SetViewMode(view_target))),
        ]
        .spacing(spacing::SM)
        .padding(spacing::SM);

        let mut content = column![controls, self.browser.view(scale).map(Message::Browser)]
            .width(Length::Fill)
            .height(Length::Fill);

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

        content.into()
    }

    /// The keyboard grammar: everything `keymap::resolve` gives meaning
    /// to is forwarded to `Browser::handle_key` (see that module's own
    /// doc — this is the host wiring it names), plus Delete, which
    /// `Browser`'s own keymap has no notion of because trashing is this
    /// app's own concern, never the shared browsing view's or the
    /// portal dialog's.
    fn subscription(&self) -> Subscription<Message> {
        let keys = keyboard::listen().filter_map(|event| {
            if let keyboard::Event::KeyPressed { key: Key::Named(key::Named::Delete), .. } = &event {
                return Some(Message::DeleteSelected);
            }
            to_browser_key(&event).map(|(k, m)| Message::BrowserKey(k, m))
        });
        Subscription::batch([
            keys,
            window::resize_events().map(|(_, size)| Message::WindowResized(size)),
        ])
    }
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
    use hyprforge_files_core::sidebar::SidebarItem;

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

    // --- stale DirLoaded, at the App/generation level ---------------------

    fn app_for_test(browser: Browser) -> App {
        App {
            browser,
            font_scale: FontScale::default(),
            read_generation: 0,
            status: None,
            last_window_size: (900, 600),
        }
    }

    /// The race this module's doc calls out: two reads for the very same
    /// directory, the first slow, the second fast. Without the
    /// generation guard, `Browser::apply_dir_loaded`'s own path check
    /// would happily accept the stale first result because its path
    /// still matches `current_dir`.
    #[test]
    fn a_stale_dir_loaded_result_is_ignored() {
        let (browser, _) = Browser::new(Mode::App, Prefs::default(), PathBuf::from("/dir"), vec![]);
        let mut app = app_for_test(browser);

        // Simulate two reads having been issued for /dir: generation 1
        // (slow — about to arrive) and generation 2 (fast — arrives
        // first, below).
        app.read_generation = 2;

        let fresh = vec![Entry {
            name: "fresh.txt".to_string(),
            path: PathBuf::from("/dir/fresh.txt"),
            is_dir: false,
            size: 1,
            modified: None,
            is_symlink: false,
            link_broken: false,
            hidden: false,
            kind: EntryKind::Other,
        }];
        let _ = app.update(Message::DirLoaded(2, PathBuf::from("/dir"), Ok(fresh.clone())));
        assert_eq!(app.browser.view_entries(), fresh.as_slice());

        // The stale generation-1 result now arrives, for the same path.
        let stale = vec![Entry {
            name: "stale.txt".to_string(),
            path: PathBuf::from("/dir/stale.txt"),
            is_dir: false,
            size: 1,
            modified: None,
            is_symlink: false,
            link_broken: false,
            hidden: false,
            kind: EntryKind::Other,
        }];
        let _ = app.update(Message::DirLoaded(1, PathBuf::from("/dir"), Ok(stale)));

        assert_eq!(
            app.browser.view_entries(),
            fresh.as_slice(),
            "the stale generation-1 result must not have overwritten the fresh listing"
        );
    }

    #[test]
    fn a_matching_generation_is_applied() {
        let (browser, _) = Browser::new(Mode::App, Prefs::default(), PathBuf::from("/dir"), vec![]);
        let mut app = app_for_test(browser);
        app.read_generation = 1;

        let entries = vec![Entry {
            name: "a.txt".to_string(),
            path: PathBuf::from("/dir/a.txt"),
            is_dir: false,
            size: 1,
            modified: None,
            is_symlink: false,
            link_broken: false,
            hidden: false,
            kind: EntryKind::Other,
        }];
        let _ = app.update(Message::DirLoaded(1, PathBuf::from("/dir"), Ok(entries.clone())));
        assert_eq!(app.browser.view_entries(), entries.as_slice());
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
