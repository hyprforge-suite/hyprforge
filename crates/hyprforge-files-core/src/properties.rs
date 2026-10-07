//! The Properties inspector: what the selection is, who may do what to
//! it, and what opens it — docked right of the listing, in the slot the
//! preview pane uses (mockup `1g`).
//!
//! # One docked panel at a time
//!
//! The inspector and the preview pane would sit in the same place, show
//! overlapping facts about the same selection, and each take a third of
//! a window that is mostly listing. Two at once would leave the listing
//! a strip. So opening Properties *replaces* the preview pane for as long
//! as it is open, and closing it brings the pane back exactly as the
//! preference left it. Asking for the preview while the inspector is up
//! is taken as "show me the preview instead", which closes the inspector
//! — see `Browser`'s `TogglePreview`.
//!
//! # The browser asks; the host looks
//!
//! Like everything else in this crate, the inspector does no I/O. Most of
//! what it shows is already in the listing — name, size, modified, mode,
//! owner — and it shows those at once. The rest is asked of the host as
//! one [`Request::Inspect`] and arrives as [`Message::Found`] and a stream
//! of [`Message::Tallied`]:
//!
//! - **Facts** about one path (`stat`, the group's name, created and
//!   accessed times, a link's target, the type by contents).
//! - **Applications** for its type, from the MIME database the host holds.
//! - **A recursive size** for any folders, which is the expensive one:
//!   it walks everything below, so it is *bounded* — at most
//!   [`MEASURE_MAX_ENTRIES`] entries and [`MEASURE_MAX_TIME`] — and
//!   *cancellable*: every measurement carries a [`Cancel`] that the
//!   browser trips the moment the selection moves on, so arrowing down a
//!   column of large folders runs one walk, not one per row passed.
//!
//! Every answer carries the generation it was asked under, and one that
//! no longer matches is dropped — the same rule the listing's own reads
//! follow.
//!
//! # What can be changed, and where
//!
//! Permissions (the nine rwx bits, one file at a time, only when you own
//! it) and the type's default application. Both go out as requests and
//! come back as answers, so a refusal from the kernel or a failed write
//! is shown in the panel rather than assumed away. Nothing can be changed
//! in the Trash, which is storage rather than somewhere to work, or inside
//! an archive, where a member's mode is a field in somebody's archive and
//! not a property of a file on this machine.
//!
//! Git status (the mockup's fourth tab), tags and a checksum are not
//! here: git and tags are deferred for the whole app (`DESIGN.md`), and a
//! checksum reads the entire file to answer a question nobody asked
//! by opening Properties.

use crate::format::{format_kind, format_permissions, human_readable_size, tilde_path};
use crate::icon::entry_icon;
use crate::preview::Picture;
use crate::types::{Entry, EntryKind, EntrySize, ItemCount};
use hyprforge_ui::theme::{spacing, FontScale, BASE_TEXT_SIZE};
use hyprforge_ui::widgets::{
    chip, config_line, divider, fact, fact_row, meta_text, panel_tabs, scaled_text, secondary_button,
    section_label, Tint,
};
use iced::widget::{button, checkbox, column, container, row, scrollable, Space};
use iced::{Element, Length};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, SystemTime};

/// The most entries one folder measurement visits before it stops and
/// reports what it has as a lower bound. A million is a home directory
/// with years of build trees in it; past that the walk is minutes of
/// disk for a number that is "very large" either way.
pub const MEASURE_MAX_ENTRIES: u64 = 1_000_000;

/// The longest one measurement runs. A network mount answering slowly
/// would otherwise keep a walk alive long after anyone was waiting for
/// it — CLAUDE.md's "never wait without a bound", applied to a walk
/// rather than a process.
pub const MEASURE_MAX_TIME: Duration = Duration::from_secs(60);

/// The panel's three views.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Tab {
    #[default]
    General,
    Permissions,
    OpenWith,
}

impl Tab {
    pub const ALL: [Tab; 3] = [Tab::General, Tab::Permissions, Tab::OpenWith];

    pub fn label(self) -> &'static str {
        match self {
            Tab::General => "General",
            Tab::Permissions => "Permissions",
            Tab::OpenWith => "Open with",
        }
    }
}

/// Where the inspected things live — which decides what can be asked
/// about them and what can be changed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Place {
    /// An ordinary folder: everything can be asked, and permissions and
    /// defaults can be changed.
    Folder,
    /// The Trash. Its items are real files and can be described, but
    /// they are storage: nothing is changed and nothing is opened.
    Trash,
    /// Inside an archive. A member has no path the kernel knows, so the
    /// host is asked nothing about it but the applications for its type.
    Archive,
}

/// One thing being inspected.
#[derive(Debug, Clone, PartialEq)]
pub struct Subject {
    pub path: PathBuf,
    /// What to call it — the listing's name, which in the Trash is the
    /// name it had rather than the one it is stored under.
    pub name: String,
    /// The listing's row for it. `None` for the folder in view, which is
    /// what Properties describes when nothing is selected and which no
    /// row stands for.
    pub entry: Option<Entry>,
}

impl Subject {
    pub fn is_dir(&self) -> bool {
        self.entry.as_ref().is_none_or(|e| e.is_dir)
    }

    fn kind(&self) -> EntryKind {
        self.entry.as_ref().map_or(EntryKind::Folder, |e| e.kind)
    }
}

/// What the host found by asking the filesystem about one path.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Facts {
    /// The type, by contents as well as name — `None` when the database
    /// has nothing to say.
    pub mime: Option<String>,
    /// The database's sentence for that type ("PNG image").
    pub description: Option<String>,
    /// The file's own size in bytes; meaningless for a folder.
    pub size: u64,
    /// Space it takes on disk, which differs for a sparse or compressed
    /// file.
    pub on_disk: u64,
    pub modified: Option<SystemTime>,
    pub accessed: Option<SystemTime>,
    /// Birth time. `None` on filesystems that do not record it, which is
    /// said as "not recorded" rather than left blank.
    pub created: Option<SystemTime>,
    /// The permission bits, `mode & 0o7777`.
    pub mode: u32,
    pub uid: u32,
    pub gid: u32,
    pub owner: Option<String>,
    pub group: Option<String>,
    pub inode: u64,
    /// Hard links to this inode.
    pub links: u64,
    /// Where a symbolic link points, as written in the link.
    pub link_target: Option<PathBuf>,
    /// Whether this process may change the mode: it owns the file or is
    /// root, and the file is not a link (whose own mode Linux ignores).
    pub can_change_mode: bool,
    /// Its tags, read off the file — see [`crate::tags`].
    pub tags: Vec<String>,
}

/// How far a recursive measurement has got.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Tally {
    pub bytes: u64,
    pub files: u64,
    pub folders: u64,
    /// Folders the walk could not read. Never folded into the total as
    /// zero: a folder you may not open is not an empty one, and a total
    /// that silently left it out would claim to be the whole.
    pub unreadable: u64,
    pub state: TallyState,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum TallyState {
    /// Still walking; the numbers are a running total.
    #[default]
    Counting,
    /// Walked everything.
    Done,
    /// Stopped at [`MEASURE_MAX_ENTRIES`] or [`MEASURE_MAX_TIME`]: the
    /// numbers are a lower bound and say so.
    Bounded,
}

/// One application that could open the inspected file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AppChoice {
    /// The desktop entry's file name — what a default is recorded as.
    pub id: String,
    pub name: String,
    /// Registered for this type itself, rather than for a type it is a
    /// kind of — see `hyprforge_mime::Candidate::made_for`.
    pub made_for: bool,
    pub is_default: bool,
}

/// What can open the inspected file.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Apps {
    /// Its type, by name. `None` when no rule knows the name, which is
    /// also "nothing can be made the default for it".
    pub mime: Option<String>,
    pub choices: Vec<AppChoice>,
    /// A default that is recorded but whose application is not
    /// installed any more — worth saying, because otherwise the file
    /// quietly opens with something else.
    pub missing_default: Option<String>,
}

/// A measurement's off switch, shared between the browser that asked and
/// the host doing the walking.
///
/// Flipping a flag is not I/O, so the browser can own the decision —
/// "this measurement is no longer wanted" — while the host owns the
/// walking. Equal only to itself, so two requests are the same request
/// only if they would stop together.
#[derive(Debug, Clone, Default)]
pub struct Cancel(Arc<AtomicBool>);

impl Cancel {
    pub fn new() -> Cancel {
        Cancel::default()
    }

    pub fn cancel(&self) {
        self.0.store(true, Ordering::Relaxed);
    }

    pub fn is_cancelled(&self) -> bool {
        self.0.load(Ordering::Relaxed)
    }
}

impl PartialEq for Cancel {
    fn eq(&self, other: &Cancel) -> bool {
        Arc::ptr_eq(&self.0, &other.0)
    }
}

/// What the inspector needs the host to do.
#[derive(Debug, Clone, PartialEq)]
pub enum Request {
    /// Find out about these, off the UI thread, and answer with one
    /// [`Message::Found`] and — when `measure` is not empty — a stream of
    /// [`Message::Tallied`] ending in one that is not `Counting`.
    Inspect {
        generation: u64,
        /// `stat` and sniff this one path. Only ever one: facts about a
        /// thousand files at once are not something anyone reads.
        facts: Option<PathBuf>,
        /// Look up the applications for this file's type.
        apps: Option<PathBuf>,
        /// Walk these folders for a total, bounded and cancellable.
        measure: Vec<PathBuf>,
        cancel: Cancel,
    },
    /// Set `path`'s permission bits to `mode`, then answer with
    /// [`Message::ModeSet`] carrying what `stat` says afterwards.
    SetMode { generation: u64, path: PathBuf, mode: u32 },
    /// Make `app` the default for `mime`, read the database again, and
    /// answer with [`Message::DefaultSet`] carrying `path`'s applications
    /// as they now stand.
    SetDefault { generation: u64, path: PathBuf, mime: String, app: String },
}

/// What the inspector is told — by its own buttons, and by the host.
#[derive(Debug, Clone, PartialEq)]
pub enum Message {
    ShowTab(Tab),
    Close,
    /// The answer to [`Request::Inspect`]'s `facts` and `apps`.
    Found {
        generation: u64,
        facts: Option<Result<Facts, String>>,
        apps: Option<Apps>,
    },
    /// A measurement's running or final total.
    Tallied { generation: u64, tally: Tally },
    /// One of the nine rwx boxes, by its bit (`0o400` is the owner's
    /// read), set to `on`.
    SetBit { bit: u32, on: bool },
    ModeSet { generation: u64, result: Result<Facts, String> },
    /// "Make default" beside an application, by its id.
    MakeDefault(String),
    DefaultSet { generation: u64, result: Result<Apps, String> },
    /// "Other applications…": the window's own chooser, which can also
    /// open the file there and then.
    ChooseApp,
    /// Compress the selection — the mockup's button on a multiple
    /// selection, and the one action there that reads naturally from a
    /// summary of several things.
    Compress,
    /// "Edit" beside the tags: the window's Tags sheet, for what is
    /// being described.
    EditTags,
}

/// What the browser should do after the inspector has updated itself.
#[derive(Debug, Clone, PartialEq)]
pub enum Effect {
    None,
    Ask(Request),
    Close,
    /// The window's Open With chooser, for this file.
    OpenWith(PathBuf),
    Compress,
    /// The Tags sheet, for these.
    EditTags(Vec<PathBuf>),
    /// Read the folder again — a mode changed, so the listing's
    /// Permissions column is out of date.
    Refresh,
}

/// The inspector's state, owned by the browser while it is open.
#[derive(Debug, Clone, PartialEq)]
pub struct Inspector {
    generation: u64,
    subjects: Vec<Subject>,
    place: Place,
    tab: Tab,
    /// `None` while asked and not yet answered, and when not asked at
    /// all — [`Inspector::asked_facts`] tells the two apart.
    facts: Option<Result<Facts, String>>,
    asked_facts: bool,
    apps: Option<Apps>,
    asked_apps: bool,
    /// The folders' total, when there are folders to measure.
    tally: Option<Tally>,
    cancel: Cancel,
    /// A change on its way to the host — the boxes and buttons are off
    /// meanwhile, so a second click cannot race the first.
    busy: bool,
    /// What the last change came back with, when it was a refusal.
    problem: Option<String>,
}

impl Inspector {
    /// An inspector for `subjects`, and the request that fills it in.
    ///
    /// `tab` carries over from the inspector this replaces, so moving the
    /// selection while on Permissions stays on Permissions.
    pub fn new(generation: u64, subjects: Vec<Subject>, place: Place, tab: Tab) -> (Inspector, Request) {
        let cancel = Cancel::new();
        let single = match subjects.as_slice() {
            [one] => Some(one),
            _ => None,
        };
        // Inside an archive there is no file to `stat` and no folder to
        // walk; the listing's own row is everything there is to know.
        let real = place != Place::Archive;
        let facts = single.filter(|_| real).map(|s| s.path.clone());
        // Folders open in here, and the Trash opens nothing.
        let apps = single.filter(|s| !s.is_dir() && place != Place::Trash).map(|s| s.path.clone());
        let measure: Vec<PathBuf> =
            if real { subjects.iter().filter(|s| s.is_dir()).map(|s| s.path.clone()).collect() } else { Vec::new() };
        let inspector = Inspector {
            generation,
            place,
            tab,
            facts: None,
            asked_facts: facts.is_some(),
            apps: None,
            asked_apps: apps.is_some(),
            tally: (!measure.is_empty()).then(Tally::default),
            cancel: cancel.clone(),
            busy: false,
            problem: None,
            subjects,
        };
        (inspector, Request::Inspect { generation, facts, apps, measure, cancel })
    }

    pub fn subjects(&self) -> &[Subject] {
        &self.subjects
    }

    pub fn place(&self) -> Place {
        self.place
    }

    pub fn tab(&self) -> Tab {
        self.tab
    }

    pub fn generation(&self) -> u64 {
        self.generation
    }

    pub fn tally(&self) -> Option<Tally> {
        self.tally
    }

    pub fn facts(&self) -> Option<&Result<Facts, String>> {
        self.facts.as_ref()
    }

    /// Stops a measurement still running for this inspector. Called by
    /// the browser when the inspector closes or is replaced.
    pub fn stop(&self) {
        self.cancel.cancel();
    }

    pub fn update(&mut self, message: Message) -> Effect {
        match message {
            Message::ShowTab(tab) => {
                self.tab = tab;
                Effect::None
            }
            Message::Close => Effect::Close,
            Message::Found { generation, facts, apps } => {
                if generation == self.generation {
                    if facts.is_some() {
                        self.facts = facts;
                    }
                    if apps.is_some() {
                        self.apps = apps;
                    }
                }
                Effect::None
            }
            Message::Tallied { generation, tally } => {
                if generation == self.generation && self.tally.is_some() {
                    self.tally = Some(tally);
                }
                Effect::None
            }
            Message::SetBit { bit, on } => {
                let Some(path) = self.mode_target() else { return Effect::None };
                let Some(Ok(facts)) = &self.facts else { return Effect::None };
                let mode = with_bit(facts.mode, bit, on);
                if mode == facts.mode {
                    return Effect::None;
                }
                self.busy = true;
                self.problem = None;
                Effect::Ask(Request::SetMode { generation: self.generation, path, mode })
            }
            Message::ModeSet { generation, result } => {
                if generation != self.generation {
                    return Effect::None;
                }
                self.busy = false;
                match result {
                    Ok(facts) => {
                        self.facts = Some(Ok(facts));
                        Effect::Refresh
                    }
                    Err(why) => {
                        self.problem = Some(why);
                        Effect::None
                    }
                }
            }
            Message::MakeDefault(app) => {
                let (Some(path), Some(mime)) = (
                    self.single().map(|s| s.path.clone()),
                    self.apps.as_ref().and_then(|a| a.mime.clone()),
                ) else {
                    return Effect::None;
                };
                if !self.can_set_default() {
                    return Effect::None;
                }
                self.busy = true;
                self.problem = None;
                Effect::Ask(Request::SetDefault { generation: self.generation, path, mime, app })
            }
            Message::DefaultSet { generation, result } => {
                if generation != self.generation {
                    return Effect::None;
                }
                self.busy = false;
                match result {
                    Ok(apps) => self.apps = Some(apps),
                    Err(why) => self.problem = Some(why),
                }
                Effect::None
            }
            Message::ChooseApp => match self.single() {
                Some(subject) if !subject.is_dir() && self.place != Place::Trash => {
                    Effect::OpenWith(subject.path.clone())
                }
                _ => Effect::None,
            },
            Message::Compress => {
                if self.place == Place::Folder && self.subjects.iter().all(|s| s.entry.is_some()) {
                    Effect::Compress
                } else {
                    Effect::None
                }
            }
            // On real files only: a trashed item's path is its storage
            // name, and an archive's member has no attributes to hold one.
            Message::EditTags => match self.place {
                Place::Folder => Effect::EditTags(self.subjects.iter().map(|s| s.path.clone()).collect()),
                _ => Effect::None,
            },
        }
    }

    fn single(&self) -> Option<&Subject> {
        match self.subjects.as_slice() {
            [one] => Some(one),
            _ => None,
        }
    }

    /// The one path whose mode the boxes would change, when they may.
    fn mode_target(&self) -> Option<PathBuf> {
        self.can_change_mode().then(|| self.single().map(|s| s.path.clone())).flatten()
    }

    /// Whether the permission boxes are live: one ordinary file this
    /// process owns, nothing already on its way, and not in the Trash or
    /// an archive.
    pub fn can_change_mode(&self) -> bool {
        self.place == Place::Folder
            && !self.busy
            && self.single().is_some()
            && matches!(&self.facts, Some(Ok(facts)) if facts.can_change_mode)
    }

    /// Whether "Make default" is offered: one file of a known type,
    /// outside the Trash. Inside an archive it is — the default decides
    /// what opens the member once it is unpacked to be opened, which is
    /// how a member is ever opened at all.
    pub fn can_set_default(&self) -> bool {
        self.place != Place::Trash
            && !self.busy
            && self.single().is_some_and(|s| !s.is_dir())
            && self.apps.as_ref().is_some_and(|a| a.mime.is_some())
    }
}

/// `mode` with `bit` set or cleared — only ever one of the nine rwx
/// bits, so setuid, setgid and sticky survive an edit untouched. A
/// person ticking "group can write" has not been asked about setuid and
/// must not lose it.
pub fn with_bit(mode: u32, bit: u32, on: bool) -> u32 {
    let bit = bit & 0o777;
    if on {
        mode | bit
    } else {
        mode & !bit
    }
}

/// What several selected things add up to.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Summary {
    pub items: usize,
    pub files: usize,
    pub folders: usize,
    /// The selected files' own bytes plus whatever the folders' walk has
    /// found so far.
    pub bytes: u64,
    /// Whether `bytes` is the whole answer: no folder still being
    /// walked, none stopped at the bound, none unreadable, and no
    /// file whose size the listing did not know.
    pub exact: bool,
    /// "3 Rust source · 1 TOML" — the kinds, commonest first.
    pub kinds: String,
}

/// Adds up `subjects` and, if there are folders among them, their
/// measured `tally`.
pub fn summarise(subjects: &[Subject], tally: Option<Tally>) -> Summary {
    let mut files = 0;
    let mut folders = 0;
    let mut bytes = 0u64;
    let mut exact = true;
    let mut kinds: Vec<(String, usize)> = Vec::new();
    for subject in subjects {
        let kind = match &subject.entry {
            Some(entry) => format_kind(entry),
            None => "Folder".to_string(),
        };
        match kinds.iter_mut().find(|(k, _)| *k == kind) {
            Some((_, n)) => *n += 1,
            None => kinds.push((kind, 1)),
        }
        if subject.is_dir() {
            folders += 1;
            continue;
        }
        files += 1;
        match subject.entry.as_ref().map(|e| e.size) {
            Some(EntrySize::Bytes(b)) => bytes += b,
            _ => exact = false,
        }
    }
    if folders > 0 {
        match tally {
            Some(t) => {
                bytes += t.bytes;
                exact &= t.state == TallyState::Done && t.unreadable == 0;
            }
            None => exact = false,
        }
    }
    // Commonest first; ties keep the order they were met in, which is
    // the listing's — so the summary reads the way the rows do.
    kinds.sort_by_key(|(_, n)| std::cmp::Reverse(*n));
    let kinds = kinds.into_iter().map(|(kind, n)| format!("{n} {kind}")).collect::<Vec<_>>().join(" \u{b7} ");
    Summary { items: subjects.len(), files, folders, bytes, exact, kinds }
}

/// A size that may be a lower bound: "at least 4.1 GiB" until it is
/// known to be the whole.
pub fn size_text(bytes: u64, exact: bool) -> String {
    if exact {
        human_readable_size(bytes)
    } else {
        format!("at least {}", human_readable_size(bytes))
    }
}

/// A folder's measured contents, in words: "1.2 GiB · 3,104 files in 212
/// folders", with how it stands when that is not yet the whole answer.
pub fn tally_text(tally: &Tally) -> String {
    let exact = tally.state == TallyState::Done && tally.unreadable == 0;
    let mut text = format!(
        "{} \u{b7} {} in {}",
        size_text(tally.bytes, exact),
        counted(tally.files, "file", "files"),
        counted(tally.folders, "folder", "folders"),
    );
    match tally.state {
        TallyState::Counting => text.push_str(" \u{2014} still counting"),
        TallyState::Bounded => text.push_str(" \u{2014} stopped counting here"),
        TallyState::Done => {}
    }
    if tally.unreadable > 0 {
        text.push_str(&format!(
            "; {} couldn't be read",
            counted(tally.unreadable, "folder", "folders")
        ));
    }
    text
}

fn counted(n: u64, one: &str, many: &str) -> String {
    format!("{} {}", grouped(n), if n == 1 { one } else { many })
}

/// `4196331` as `4,196,331` — an inode or a file count is read digit by
/// digit, and a run of seven is where the eye loses its place.
pub fn grouped(n: u64) -> String {
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

/// A timestamp in full — "14 Aug 2026, 09:12". The list's relative form
/// ("Yesterday") answers "how recent"; a Properties panel is asked
/// "when, exactly", and has the room to say.
pub fn full_time(time: Option<SystemTime>) -> String {
    match time {
        Some(t) => chrono::DateTime::<chrono::Local>::from(t).format("%-d %b %Y, %H:%M").to_string(),
        None => "Not recorded".to_string(),
    }
}

/// `0644` beside `-rw-r--r--` — the octal is what a person types into a
/// terminal, the letters are what they read.
pub fn mode_text(mode: u32, is_dir: bool, is_symlink: bool) -> String {
    format!("{:04o} \u{b7} {}", mode & 0o7777, format_permissions(mode, is_dir, is_symlink))
}

/// `ada:users`, with a number standing in for a name this system cannot
/// resolve — the same choice `format_owner` makes for the listing.
pub fn owner_text(facts: &Facts) -> String {
    let user = facts.owner.clone().unwrap_or_else(|| facts.uid.to_string());
    let group = facts.group.clone().unwrap_or_else(|| facts.gid.to_string());
    format!("{user}:{group}")
}

/// The three classes the boxes are laid out in, and their bits.
const CLASSES: [(&str, u32); 3] = [("Owner", 6), ("Group", 3), ("Others", 0)];

/// Whether every subject has `bit` set, none has, or they differ.
fn shared_bit(modes: &[u32], bit: u32) -> Option<bool> {
    let set = modes.iter().filter(|m| *m & bit != 0).count();
    match set {
        0 => Some(false),
        n if n == modes.len() => Some(true),
        _ => None,
    }
}

/// Draws the inspector `width` logical pixels wide. `icon` is the theme
/// icon for a single subject, when the host has found one.
pub fn view<'a>(
    inspector: &'a Inspector,
    icon: Option<&Picture>,
    home: Option<&Path>,
    width: f32,
    scale: FontScale,
) -> Element<'a, Message> {
    let inner = width - 2.0 * spacing::MD;
    let close = button(meta_text("Close", hyprforge_ui::density::META_TEXT_BASE, scale))
        .padding([2.0, spacing::XS])
        .on_press(Message::Close)
        .style(|t: &iced::Theme, status| hyprforge_ui::widgets::selectable_row_style(t, status, false));

    let head: Element<'a, Message> = match inspector.subjects.as_slice() {
        [one] => {
            let mark = entry_icon(one.kind(), icon, 56.0, scale);
            let line = match (&one.entry, inspector.facts.as_ref()) {
                (Some(entry), _) if !entry.is_dir => {
                    format!("{} \u{b7} {}", format_kind(entry), crate::format::format_size(entry.size))
                }
                _ => match inspector.tally {
                    Some(t) => format!("Folder \u{b7} {}", size_text(t.bytes, t.state == TallyState::Done && t.unreadable == 0)),
                    None => "Folder".to_string(),
                },
            };
            column![
                container(mark).center_x(Length::Fixed(inner)),
                container(
                    scaled_text(one.name.clone(), 16.0, scale)
                        .wrapping(iced::widget::text::Wrapping::WordOrGlyph)
                        .align_x(iced::alignment::Horizontal::Center)
                )
                .center_x(Length::Fixed(inner)),
                container(config_line(line, scale)).center_x(Length::Fixed(inner)),
            ]
            .spacing(spacing::SM)
            .into()
        }
        many => {
            let summary = summarise(many, inspector.tally);
            column![scaled_text(format!("{} items selected", summary.items), 16.0, scale)].into()
        }
    };

    let tabs: Vec<(Tab, &'static str)> = Tab::ALL.iter().map(|t| (*t, t.label())).collect();
    let mut body = column![
        row![Space::new().width(Length::Fill), close].align_y(iced::Alignment::Center),
        head,
        panel_tabs(&tabs, &inspector.tab, Message::ShowTab, scale),
        divider(),
    ]
    .spacing(spacing::MD);

    body = body.push(match inspector.tab {
        Tab::General => general(inspector, home, scale),
        Tab::Permissions => permissions(inspector, scale),
        Tab::OpenWith => open_with(inspector, scale),
    });
    if let Some(problem) = &inspector.problem {
        body = body.push(
            scaled_text(problem.clone(), hyprforge_ui::density::META_TEXT_BASE, scale)
                .color(hyprforge_ui::theme::error())
                .wrapping(iced::widget::text::Wrapping::WordOrGlyph),
        );
    }

    container(scrollable(container(body).padding(spacing::MD).width(Length::Fixed(width))).height(Length::Fill))
        .width(Length::Fixed(width))
        .height(Length::Fill)
        .into()
}

/// A path in the mono font, the way the path bar writes it.
fn path_value<'a>(path: &Path, home: Option<&Path>, scale: FontScale) -> Element<'a, Message> {
    config_line(tilde_path(path, home), scale)
        .color(hyprforge_ui::theme::text())
        .wrapping(iced::widget::text::Wrapping::WordOrGlyph)
        .into()
}

fn general<'a>(inspector: &'a Inspector, home: Option<&Path>, scale: FontScale) -> Element<'a, Message> {
    let mut lines = column![].spacing(spacing::SM);
    let facts = match &inspector.facts {
        Some(Ok(facts)) => Some(facts),
        _ => None,
    };
    match inspector.subjects.as_slice() {
        [one] => {
            // Where: the folder it is in. In the Trash that is where it
            // came from, which is the only "where" a person means there.
            match (&one.entry, inspector.place) {
                (Some(entry), Place::Trash) => {
                    if let Some(origin) = &entry.origin {
                        lines = lines.push(fact_row("Came from", path_value(origin, home, scale), scale));
                    }
                }
                // Inside an archive the parent is a path through it —
                // `~/x.zip/docs` — which is exactly what the path bar
                // shows, so it is shown the same way. Not split into
                // archive and member here: `archive::split` asks the
                // filesystem, and this runs every frame.
                _ => {
                    if let Some(parent) = one.path.parent() {
                        lines = lines.push(fact_row("Where", path_value(parent, home, scale), scale));
                    }
                }
            }
            let kind = match (facts.and_then(|f| f.description.clone()), &one.entry) {
                (Some(description), _) => description,
                (None, Some(entry)) => format_kind(entry),
                (None, None) => "Folder".to_string(),
            };
            lines = lines.push(fact("Kind", kind, scale));
            if let (Some(facts), Place::Folder) = (facts, inspector.place) {
                let said = if facts.tags.is_empty() { "None".to_string() } else { facts.tags.join(", ") };
                lines = lines.push(fact_row(
                    "Tags",
                    row![
                        hyprforge_ui::widgets::scaled_text(said, BASE_TEXT_SIZE, scale)
                            .wrapping(iced::widget::text::Wrapping::WordOrGlyph)
                            .width(Length::Fill),
                        hyprforge_ui::widgets::secondary_button("Edit").on_press(Message::EditTags),
                    ]
                    .spacing(spacing::SM)
                    .align_y(iced::Alignment::Center),
                    scale,
                ));
            }
            if let Some(mime) = facts.and_then(|f| f.mime.clone()) {
                lines = lines.push(fact_row("Type", config_line(mime, scale).color(hyprforge_ui::theme::text()), scale));
            }
            if let Some(target) = facts.and_then(|f| f.link_target.clone()) {
                let broken = one.entry.as_ref().is_some_and(|e| e.link_broken);
                let mut value = row![config_line(target.display().to_string(), scale)
                    .color(hyprforge_ui::theme::text())
                    .wrapping(iced::widget::text::Wrapping::WordOrGlyph)]
                .spacing(spacing::XS);
                if broken {
                    value = value.push(chip("missing", Tint::Error, scale));
                }
                lines = lines.push(fact_row("Links to", value, scale));
            }
            if one.is_dir() {
                match (inspector.tally.as_ref(), &one.entry) {
                    (Some(tally), _) => lines = lines.push(fact("Contents", tally_text(tally), scale)),
                    // Inside an archive the listing's count is the
                    // whole answer there is.
                    (None, Some(entry)) => {
                        let count = match entry.size {
                            EntrySize::Items(ItemCount::Known(n)) => counted(n as u64, "item", "items"),
                            _ => "Not counted".to_string(),
                        };
                        lines = lines.push(fact("Contents", count, scale));
                    }
                    (None, None) => {}
                }
            } else {
                let size = match (facts, &one.entry) {
                    (Some(f), _) => {
                        let mut text = format!("{} ({} bytes)", human_readable_size(f.size), grouped(f.size));
                        if f.on_disk != f.size {
                            text.push_str(&format!(", {} on disk", human_readable_size(f.on_disk)));
                        }
                        text
                    }
                    (None, Some(entry)) => crate::format::format_size(entry.size),
                    (None, None) => String::new(),
                };
                lines = lines.push(fact("Size", size, scale));
                if let Some(packed) = one.entry.as_ref().and_then(|e| e.packed) {
                    lines = lines.push(fact("Packed", human_readable_size(packed), scale));
                }
            }
            let modified = facts.and_then(|f| f.modified).or(one.entry.as_ref().and_then(|e| e.modified));
            // In the Trash the listing's date is when it was deleted.
            let modified_label = if inspector.place == Place::Trash && facts.is_none() { "Deleted" } else { "Modified" };
            lines = lines.push(fact(modified_label, full_time(modified), scale));
            if let Some(f) = facts {
                lines = lines.push(fact("Accessed", full_time(f.accessed), scale));
                lines = lines.push(fact("Created", full_time(f.created), scale));
                lines = lines.push(fact_row("Owner", config_line(owner_text(f), scale).color(hyprforge_ui::theme::text()), scale));
                lines = lines.push(fact_row("Inode", config_line(grouped(f.inode), scale).color(hyprforge_ui::theme::text()), scale));
                if f.links > 1 && !one.is_dir() {
                    lines = lines.push(fact("Hard links", grouped(f.links), scale));
                }
            }
            if inspector.asked_facts && inspector.facts.is_none() {
                lines = lines.push(meta_text("Looking\u{2026}", hyprforge_ui::density::META_TEXT_BASE, scale));
            }
            if let Some(Err(why)) = &inspector.facts {
                lines = lines.push(
                    scaled_text(why.clone(), hyprforge_ui::density::META_TEXT_BASE, scale)
                        .color(hyprforge_ui::theme::error())
                        .wrapping(iced::widget::text::Wrapping::WordOrGlyph),
                );
            }
        }
        many => {
            let summary = summarise(many, inspector.tally);
            lines = lines.push(fact("Total size", size_text(summary.bytes, summary.exact), scale));
            lines = lines.push(fact("Kinds", summary.kinds.clone(), scale));
            if let Some(tally) = &inspector.tally {
                lines = lines.push(fact("In folders", tally_text(tally), scale));
            }
            if inspector.place == Place::Folder {
                lines = lines.push(
                    secondary_button("Compress\u{2026}")
                        .on_press(Message::Compress),
                );
            }
        }
    }
    lines.into()
}

fn permissions<'a>(inspector: &'a Inspector, scale: FontScale) -> Element<'a, Message> {
    let mut lines = column![].spacing(spacing::SM);
    let facts = match (&inspector.facts, inspector.subjects.as_slice()) {
        (Some(Ok(facts)), [_]) => Some(facts),
        _ => None,
    };
    // The modes the boxes show: `stat`'s when there is an answer, the
    // listing's otherwise — which is all there is inside an archive and
    // for a selection of several.
    let modes: Vec<u32> = match facts {
        Some(f) => vec![f.mode],
        None => inspector.subjects.iter().filter_map(|s| s.entry.as_ref().map(|e| e.mode)).collect(),
    };
    let any_dir = inspector.subjects.iter().any(Subject::is_dir);
    let any_file = inspector.subjects.iter().any(|s| !s.is_dir());

    if let Some(f) = facts {
        lines = lines.push(fact_row("Owner", config_line(owner_text(f), scale).color(hyprforge_ui::theme::text()), scale));
        let is_link = f.link_target.is_some();
        lines = lines.push(fact_row(
            "Mode",
            config_line(mode_text(f.mode, any_dir, is_link), scale).color(hyprforge_ui::theme::text()),
            scale,
        ));
    }

    if modes.is_empty() {
        let text = if inspector.asked_facts && inspector.facts.is_none() {
            "Looking\u{2026}"
        } else {
            "Nothing here says what its permissions are."
        };
        return lines.push(meta_text(text, hyprforge_ui::density::META_TEXT_BASE, scale)).into();
    }

    // What "x" means depends on what it is on: a folder's is "may go
    // in", a file's is "may run". A mixed selection gets both words.
    let run = match (any_dir, any_file) {
        (true, false) => "Enter",
        (false, true) => "Run",
        _ => "Run/Enter",
    };
    let live = inspector.can_change_mode();
    let heading = |text: &'static str| -> Element<'a, Message> {
        container(meta_text(text, hyprforge_ui::density::META_TEXT_BASE, scale))
            .width(Length::Fixed(scale.apply(56.0)))
            .center_x(Length::Fixed(scale.apply(56.0)))
            .into()
    };
    let mut grid = column![row![
        Space::new().width(Length::Fixed(scale.apply(64.0))),
        heading("Read"),
        heading("Write"),
        heading(run),
    ]]
    .spacing(spacing::XS);
    for (class, shift) in CLASSES {
        let mut line = row![container(scaled_text(class, hyprforge_ui::density::META_TEXT_BASE, scale))
            .width(Length::Fixed(scale.apply(64.0)))]
        .align_y(iced::Alignment::Center);
        for bit in [0o4 << shift, 0o2 << shift, 0o1 << shift] {
            // A box that is mixed across a selection shows unticked —
            // iced's checkbox has no third state — and the line under
            // the grid says the selection differs.
            let state = shared_bit(&modes, bit);
            let mut tick = checkbox(state.unwrap_or(false)).size(scale.apply(16.0));
            if live {
                tick = tick.on_toggle(move |on| Message::SetBit { bit, on });
            }
            line = line.push(container(tick).center_x(Length::Fixed(scale.apply(56.0))));
        }
        grid = grid.push(line);
    }
    lines = lines.push(grid);

    let mixed = (0..9).any(|i| shared_bit(&modes, 1 << i).is_none());
    if mixed {
        lines = lines.push(meta_text(
            "The selected items differ; an unticked box may be set on some of them.",
            hyprforge_ui::density::META_TEXT_BASE,
            scale,
        ));
    }
    // Special bits are shown, never offered: setuid on the wrong file is
    // a privilege escalation, and a box for it is one click from that.
    if let Some(f) = facts {
        let special: Vec<&str> = [(0o4000, "setuid"), (0o2000, "setgid"), (0o1000, "sticky")]
            .into_iter()
            .filter(|(bit, _)| f.mode & bit != 0)
            .map(|(_, name)| name)
            .collect();
        if !special.is_empty() {
            lines = lines.push(fact("Special", special.join(", "), scale));
        }
    }
    if !live {
        lines = lines.push(meta_text(
            read_only_reason(inspector),
            hyprforge_ui::density::META_TEXT_BASE,
            scale,
        ));
    }
    lines.into()
}

/// Why the boxes cannot be changed, in the person's terms.
fn read_only_reason(inspector: &Inspector) -> String {
    match inspector.place {
        Place::Trash => "Items in the Trash keep the permissions they had; restore one to change them.".to_string(),
        Place::Archive => {
            "These are recorded in the archive. They apply to what is extracted, and can't be changed here."
                .to_string()
        }
        Place::Folder if inspector.subjects.len() > 1 => "Select one item to change its permissions.".to_string(),
        Place::Folder if inspector.busy => "Changing\u{2026}".to_string(),
        Place::Folder => match &inspector.facts {
            Some(Ok(f)) if f.link_target.is_some() => {
                "A link's own permissions are not used; change those of what it points to.".to_string()
            }
            Some(Ok(f)) => format!(
                "Only {} can change these.",
                f.owner.clone().unwrap_or_else(|| format!("user {}", f.uid))
            ),
            _ => String::new(),
        },
    }
}

fn open_with<'a>(inspector: &'a Inspector, scale: FontScale) -> Element<'a, Message> {
    let meta = |text: String| -> Element<'a, Message> {
        meta_text(text, hyprforge_ui::density::META_TEXT_BASE, scale)
            .wrapping(iced::widget::text::Wrapping::WordOrGlyph)
            .into()
    };
    let Some(subject) = inspector.single() else {
        return meta("Select one file to see what opens it.".to_string());
    };
    if subject.is_dir() {
        return meta("Folders open here, in Files.".to_string());
    }
    if inspector.place == Place::Trash {
        return meta("Items in the Trash can't be opened; restore one first.".to_string());
    }
    let Some(apps) = &inspector.apps else {
        return meta("Looking\u{2026}".to_string());
    };
    let mut lines = column![].spacing(spacing::SM);
    match &apps.mime {
        Some(mime) => {
            lines = lines.push(fact_row("Opens as", config_line(mime.clone(), scale).color(hyprforge_ui::theme::text()), scale))
        }
        None => lines = lines.push(meta("Nothing here knows what kind of file this is by its name.".to_string())),
    }
    if let Some(missing) = &apps.missing_default {
        lines = lines.push(
            scaled_text(
                format!("The default, {missing}, isn't installed any more."),
                hyprforge_ui::density::META_TEXT_BASE,
                scale,
            )
            .color(hyprforge_ui::theme::warning())
            .wrapping(iced::widget::text::Wrapping::WordOrGlyph),
        );
    }
    if apps.choices.is_empty() {
        lines = lines.push(meta("Nothing installed is registered to open it.".to_string()));
    }
    let settable = inspector.can_set_default();
    let mut related_heading = false;
    for choice in &apps.choices {
        if !choice.made_for && !related_heading {
            related_heading = true;
            lines = lines.push(section_label("Made for a related kind", scale));
        }
        let mut line = row![scaled_text(choice.name.clone(), BASE_TEXT_SIZE * 0.9, scale)
            .wrapping(iced::widget::text::Wrapping::WordOrGlyph)
            .width(Length::Fill)]
        .spacing(spacing::SM)
        .align_y(iced::Alignment::Center);
        if choice.is_default {
            line = line.push(chip("default", Tint::Accent, scale));
        } else {
            let mut make = button(meta_text("Make default", hyprforge_ui::density::META_TEXT_BASE, scale))
                .padding([2.0, spacing::XS])
                .style(|t: &iced::Theme, status| hyprforge_ui::widgets::selectable_row_style(t, status, false));
            if settable {
                make = make.on_press(Message::MakeDefault(choice.id.clone()));
            }
            line = line.push(make);
        }
        lines = lines.push(line);
    }
    lines = lines.push(
        secondary_button("Other applications\u{2026}")
            .on_press(Message::ChooseApp),
    );
    lines.into()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(name: &str, is_dir: bool, size: EntrySize) -> Entry {
        Entry {
            name: name.to_string(),
            path: PathBuf::from("/w").join(name),
            is_dir,
            size,
            modified: None,
            is_symlink: false,
            link_broken: false,
            hidden: false,
            kind: EntryKind::classify(is_dir, name),
            mode: if is_dir { 0o755 } else { 0o644 },
            uid: 1000,
            owner: Some("ada".to_string()),
            origin: None,
            packed: None,
        }
    }

    fn subject(e: Entry) -> Subject {
        Subject { path: e.path.clone(), name: e.name.clone(), entry: Some(e) }
    }

    fn file(name: &str, bytes: u64) -> Subject {
        subject(entry(name, false, EntrySize::Bytes(bytes)))
    }

    fn folder(name: &str) -> Subject {
        subject(entry(name, true, EntrySize::UNCOUNTED))
    }

    fn found(inspector: &mut Inspector, facts: Facts) {
        let generation = inspector.generation();
        inspector.update(Message::Found { generation, facts: Some(Ok(facts)), apps: None });
    }

    fn mine() -> Facts {
        Facts { mode: 0o644, uid: 1000, can_change_mode: true, ..Facts::default() }
    }

    #[test]
    fn one_file_asks_for_its_facts_and_applications_and_measures_nothing() {
        let (_, request) = Inspector::new(1, vec![file("a.rs", 10)], Place::Folder, Tab::General);
        let Request::Inspect { facts, apps, measure, .. } = request else { panic!("{request:?}") };
        assert_eq!(facts.as_deref(), Some(Path::new("/w/a.rs")));
        assert_eq!(apps.as_deref(), Some(Path::new("/w/a.rs")));
        assert!(measure.is_empty());
    }

    #[test]
    fn folders_are_measured_and_never_asked_what_opens_them() {
        let (inspector, request) =
            Inspector::new(1, vec![folder("src"), file("a.rs", 1)], Place::Folder, Tab::General);
        let Request::Inspect { facts, apps, measure, .. } = request else { panic!() };
        assert_eq!(facts, None, "facts are for one thing at a time");
        assert_eq!(apps, None);
        assert_eq!(measure, vec![PathBuf::from("/w/src")]);
        assert_eq!(inspector.tally(), Some(Tally::default()), "counting from the start");
    }

    /// A member of an archive has no path the kernel knows: asking the
    /// host to `stat` or walk it would only produce errors.
    #[test]
    fn inside_an_archive_only_the_applications_are_asked_for() {
        let (_, request) = Inspector::new(1, vec![file("a.rs", 1), ], Place::Archive, Tab::General);
        let Request::Inspect { facts, apps, measure, .. } = request else { panic!() };
        assert_eq!(facts, None);
        assert!(apps.is_some());
        let (_, request) = Inspector::new(1, vec![folder("src")], Place::Archive, Tab::General);
        let Request::Inspect { measure: folders, .. } = request else { panic!() };
        assert!(measure.is_empty() && folders.is_empty());
    }

    #[test]
    fn nothing_in_the_trash_is_asked_what_opens_it() {
        let (_, request) = Inspector::new(1, vec![file("a.rs", 1)], Place::Trash, Tab::General);
        let Request::Inspect { facts, apps, .. } = request else { panic!() };
        assert!(facts.is_some(), "a trashed file is still a file to describe");
        assert_eq!(apps, None);
    }

    /// The point of the generation: an answer about the selection before
    /// last must not land on this one.
    #[test]
    fn an_answer_for_an_earlier_selection_is_dropped() {
        let (mut inspector, _) = Inspector::new(7, vec![folder("src")], Place::Folder, Tab::General);
        let late = Tally { bytes: 99, state: TallyState::Done, ..Tally::default() };
        inspector.update(Message::Tallied { generation: 6, tally: late });
        assert_eq!(inspector.tally(), Some(Tally::default()));
        inspector.update(Message::Tallied { generation: 7, tally: late });
        assert_eq!(inspector.tally(), Some(late));
    }

    #[test]
    fn stopping_an_inspector_trips_the_cancel_its_host_was_given() {
        let (inspector, request) = Inspector::new(1, vec![folder("src")], Place::Folder, Tab::General);
        let Request::Inspect { cancel, .. } = request else { panic!() };
        assert!(!cancel.is_cancelled());
        inspector.stop();
        assert!(cancel.is_cancelled());
    }

    #[test]
    fn two_cancels_are_equal_only_when_they_are_one_switch() {
        let a = Cancel::new();
        assert_eq!(a, a.clone());
        assert_ne!(a, Cancel::new());
    }

    /// One box changes one bit, and the host is asked for exactly the
    /// mode that results — with setuid left where it was.
    #[test]
    fn ticking_a_box_asks_for_that_one_bit_and_keeps_the_special_ones() {
        let (mut inspector, _) = Inspector::new(1, vec![file("run.sh", 1)], Place::Folder, Tab::Permissions);
        found(&mut inspector, Facts { mode: 0o4644, ..mine() });
        let effect = inspector.update(Message::SetBit { bit: 0o100, on: true });
        assert_eq!(
            effect,
            Effect::Ask(Request::SetMode { generation: 1, path: PathBuf::from("/w/run.sh"), mode: 0o4744 })
        );
    }

    #[test]
    fn a_second_click_waits_for_the_first_to_come_back() {
        let (mut inspector, _) = Inspector::new(1, vec![file("a", 1)], Place::Folder, Tab::Permissions);
        found(&mut inspector, mine());
        assert!(matches!(inspector.update(Message::SetBit { bit: 0o020, on: true }), Effect::Ask(_)));
        assert_eq!(inspector.update(Message::SetBit { bit: 0o002, on: true }), Effect::None);
        let after = Facts { mode: 0o664, ..mine() };
        assert_eq!(inspector.update(Message::ModeSet { generation: 1, result: Ok(after) }), Effect::Refresh);
        assert!(inspector.can_change_mode(), "live again once answered");
    }

    /// A refusal is shown, not swallowed — and the boxes come back.
    #[test]
    fn a_refused_change_is_said_in_the_panel() {
        let (mut inspector, _) = Inspector::new(1, vec![file("a", 1)], Place::Folder, Tab::Permissions);
        found(&mut inspector, mine());
        inspector.update(Message::SetBit { bit: 0o020, on: true });
        let why = "The file system is read-only.".to_string();
        assert_eq!(inspector.update(Message::ModeSet { generation: 1, result: Err(why.clone()) }), Effect::None);
        assert_eq!(inspector.problem.as_deref(), Some(why.as_str()));
        assert!(inspector.can_change_mode());
    }

    #[test]
    fn permissions_are_read_only_unless_one_owned_file_in_an_ordinary_folder() {
        let not_mine = Facts { can_change_mode: false, ..mine() };
        let (mut inspector, _) = Inspector::new(1, vec![file("a", 1)], Place::Folder, Tab::Permissions);
        found(&mut inspector, not_mine);
        assert!(!inspector.can_change_mode(), "somebody else's file");
        assert_eq!(inspector.update(Message::SetBit { bit: 0o002, on: true }), Effect::None);

        for place in [Place::Trash, Place::Archive] {
            let (mut inspector, _) = Inspector::new(1, vec![file("a", 1)], place, Tab::Permissions);
            found(&mut inspector, mine());
            assert!(!inspector.can_change_mode(), "{place:?}");
        }

        let (inspector, _) = Inspector::new(1, vec![file("a", 1), file("b", 1)], Place::Folder, Tab::Permissions);
        assert!(!inspector.can_change_mode(), "several at once");
    }

    #[test]
    fn a_default_is_set_for_the_files_type_and_never_from_the_trash() {
        let apps = Apps {
            mime: Some("text/x-rust".to_string()),
            choices: vec![AppChoice { id: "ed.desktop".into(), name: "Ed".into(), made_for: true, is_default: false }],
            missing_default: None,
        };
        let (mut inspector, _) = Inspector::new(3, vec![file("a.rs", 1)], Place::Folder, Tab::OpenWith);
        inspector.update(Message::Found { generation: 3, facts: None, apps: Some(apps.clone()) });
        assert_eq!(
            inspector.update(Message::MakeDefault("ed.desktop".into())),
            Effect::Ask(Request::SetDefault {
                generation: 3,
                path: PathBuf::from("/w/a.rs"),
                mime: "text/x-rust".into(),
                app: "ed.desktop".into(),
            })
        );

        let (mut trashed, _) = Inspector::new(3, vec![file("a.rs", 1)], Place::Trash, Tab::OpenWith);
        trashed.update(Message::Found { generation: 3, facts: None, apps: Some(apps) });
        assert_eq!(trashed.update(Message::MakeDefault("ed.desktop".into())), Effect::None);
    }

    #[test]
    fn a_name_no_rule_knows_has_no_default_to_set() {
        let (mut inspector, _) = Inspector::new(1, vec![file("a.qqq", 1)], Place::Folder, Tab::OpenWith);
        inspector.update(Message::Found { generation: 1, facts: None, apps: Some(Apps::default()) });
        assert!(!inspector.can_set_default());
    }

    #[test]
    fn a_bit_edit_never_touches_setuid_setgid_or_sticky() {
        assert_eq!(with_bit(0o7644, 0o002, true), 0o7646);
        assert_eq!(with_bit(0o7644, 0o400, false), 0o7244);
        // Asked to clear a special bit, it declines: only rwx are boxes.
        assert_eq!(with_bit(0o4755, 0o4000, false), 0o4755);
    }

    #[test]
    fn a_selection_adds_its_files_to_its_folders_and_says_when_that_is_not_all() {
        let subjects = vec![file("a.rs", 1000), file("b.rs", 24), folder("src")];
        let counting = summarise(&subjects, Some(Tally { bytes: 500, ..Tally::default() }));
        assert_eq!(counting.bytes, 1524);
        assert!(!counting.exact, "still counting");
        assert_eq!((counting.files, counting.folders), (2, 1));

        let done = summarise(&subjects, Some(Tally { bytes: 500, state: TallyState::Done, ..Tally::default() }));
        assert!(done.exact);

        let unreadable =
            summarise(&subjects, Some(Tally { bytes: 500, state: TallyState::Done, unreadable: 1, ..Tally::default() }));
        assert!(!unreadable.exact, "a folder that could not be read is not part of a whole");
    }

    #[test]
    fn the_kinds_are_counted_commonest_first() {
        let subjects = vec![file("Cargo.toml", 1), file("a.rs", 1), file("b.rs", 1), file("c.rs", 1)];
        let summary = summarise(&subjects, None);
        assert!(summary.kinds.starts_with("3 "), "{}", summary.kinds);
        assert!(summary.kinds.contains("1 "), "{}", summary.kinds);
    }

    #[test]
    fn a_lower_bound_says_it_is_one() {
        assert_eq!(size_text(2048, true), "2.0 KiB");
        assert_eq!(size_text(2048, false), "at least 2.0 KiB");
        let bounded = Tally { bytes: 2048, files: 3, folders: 1, state: TallyState::Bounded, ..Tally::default() };
        assert!(tally_text(&bounded).contains("at least"));
        assert!(tally_text(&bounded).contains("stopped"));
        let unreadable = Tally { unreadable: 2, state: TallyState::Done, ..bounded };
        assert!(tally_text(&unreadable).contains("2 folders couldn't be read"));
    }

    #[test]
    fn long_numbers_are_grouped_in_threes() {
        assert_eq!(grouped(4_196_331), "4,196,331");
        assert_eq!(grouped(999), "999");
        assert_eq!(grouped(1000), "1,000");
        assert_eq!(grouped(0), "0");
    }

    #[test]
    fn a_mode_reads_as_octal_and_as_letters() {
        assert_eq!(mode_text(0o644, false, false), "0644 \u{b7} -rw-r--r--");
        assert_eq!(mode_text(0o4755, false, false), "4755 \u{b7} -rwsr-xr-x");
    }

    #[test]
    fn an_owner_nobody_here_can_name_is_shown_by_number() {
        let facts = Facts { uid: 1234, gid: 99, owner: None, group: Some("users".into()), ..Facts::default() };
        assert_eq!(owner_text(&facts), "1234:users");
    }

    #[test]
    fn a_time_the_filesystem_did_not_record_says_so() {
        assert_eq!(full_time(None), "Not recorded");
    }

    #[test]
    fn a_box_shared_by_part_of_a_selection_is_mixed() {
        assert_eq!(shared_bit(&[0o644, 0o664], 0o020), None);
        assert_eq!(shared_bit(&[0o644, 0o664], 0o400), Some(true));
        assert_eq!(shared_bit(&[0o644, 0o664], 0o001), Some(false));
    }
}
