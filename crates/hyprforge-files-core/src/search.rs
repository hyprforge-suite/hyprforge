//! Searching below a folder: where a search runs, the walk that runs it,
//! and the searches a person saved.
//!
//! The search box has always filtered the folder in view, and still
//! does — that costs nothing, because the listing is already in memory.
//! Searching *below* a folder means reading every folder under it, which
//! is the most expensive thing this app can be asked to do: a home
//! directory here is tens of thousands of folders. So the shape is the
//! path bar's ([`crate::jump`]), made stricter:
//!
//! - **The browser asks; the host walks.** [`crate::browser::Browser`]
//!   emits a [`Request`] and does no I/O. The host runs [`walk`] on a
//!   blocking worker, through the same [`FsBackend`] as the listing, and
//!   streams what it finds back as [`SearchMessage::Found`] batches.
//! - **Latest wins.** Each request carries a `run` number. A keystroke
//!   is "look again", never a job of its own: the host cancels the walk
//!   that is running and starts the newest, and the browser drops any
//!   batch whose number is not the one it is showing.
//! - **Bounded four ways.** Time, folders read, results kept and depth —
//!   see [`Budget`]. Hitting one is a [`End`] the view says out loud,
//!   never a silent "that's all there is".
//! - **Never quietly incomplete.** A folder that could not be read and a
//!   folder on another filesystem that was not entered are both counted
//!   and both reported. Leaving either out would be CLAUDE.md's "could
//!   not read it, so there is nothing there".
//!
//! # What the walk does not enter
//!
//! - **Archives.** A `.zip` met on the way is a file: it is matched by
//!   name like any other and not opened. Listing an archive means
//!   decompressing it — a tar has no table of contents — so a search of
//!   `~` would otherwise unpack every tarball in Downloads, and the
//!   archive backend keeps only four indexes. Searching *inside* an
//!   archive still works: start from a folder in one, and the walk goes
//!   through the archive backend, whose index is already read.
//! - **Hidden folders**, unless dotfiles are showing or the query asks
//!   for them (`is:hidden`). A result inside `.cache` would be one the
//!   listing hides anyway, and those folders are most of a home
//!   directory's bulk.
//! - **Symbolic links to folders**, which is how a walk loops forever.
//! - **Other filesystems** — the host decides, through [`walk`]'s
//!   `elsewhere`. A network mount can stop answering, and a `read_dir`
//!   blocked on one cannot be cancelled from here.
//! - **The Trash's storage.** A trashed file is not where anything lives,
//!   and its stored name is not its name — see [`crate::trash`].

use crate::backend::FsBackend;
use crate::query::{Filter, Matcher};
use crate::types::Entry;
use serde::{Deserialize, Serialize};
use std::collections::VecDeque;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

/// Where the search box looks.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Scope {
    /// The folder in view, as loaded — instant, and what the box has
    /// always done.
    #[default]
    Folder,
    /// The folder in view and everything below it.
    Below,
    /// The home directory and everything below it, wherever you are.
    Home,
}

impl Scope {
    /// Whether this reads folders the listing has not — the expensive
    /// kind, with a walk behind it.
    pub fn walks(self) -> bool {
        self != Scope::Folder
    }
}

/// How far one walk may go. Each limit is a different way a search over a
/// big tree can cost too much, and hitting any of them ends the walk with
/// that [`End`] rather than an ordinary finish.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Budget {
    /// Wall-clock, from the first read.
    pub time: Duration,
    /// Folders read. Also caps how many folders wait to be read, so the
    /// queue cannot outgrow what the walk would ever get to.
    pub folders: usize,
    /// Results kept. Past this the walk stops: a list of more than this
    /// is not one anybody reads, and every result is an [`Entry`] held
    /// in memory.
    pub found: usize,
    /// Levels below the root.
    pub depth: usize,
}

impl Default for Budget {
    /// Measured on this machine, warm cache: the whole home directory
    /// without its dotfolders (17,865 folders) is walked in 0.8s — see
    /// `crates/hyprforge-files/DESIGN.md`, phase F. The limits sit well
    /// past that, so they bound a pathological tree rather than an
    /// ordinary one: the same home *with* its dotfolders is 110,000
    /// folders and a million entries, and stops at the folder limit
    /// after about ten seconds with the walk's peak memory under 20MB.
    fn default() -> Self {
        Budget { time: Duration::from_secs(15), folders: 100_000, found: 5_000, depth: 48 }
    }
}

/// Everything a host needs to run one search — see the module doc.
#[derive(Debug, Clone, PartialEq)]
pub struct Request {
    /// Which search this is. Batches come back carrying it, and the
    /// browser drops any that are not for the search on screen.
    pub run: u64,
    pub root: PathBuf,
    pub matcher: Matcher,
    /// The dotfile switch, as it was when the search started.
    pub show_hidden: bool,
    /// Folders never entered — the Trash's storage.
    pub skip: Vec<PathBuf>,
    pub budget: Budget,
}

/// How a walk ended.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum End {
    /// Every folder in reach was read.
    Complete,
    /// Cancelled — a newer search, the Stop button, or a navigation.
    Stopped,
    /// [`Budget::time`] ran out.
    TimeLimit,
    /// [`Budget::folders`] were read.
    FolderLimit,
    /// [`Budget::found`] results were kept.
    FoundLimit,
}

/// What a finished walk reports, beside its results.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Summary {
    pub end: End,
    /// Folders read.
    pub folders: usize,
    /// Folders that could not be read — permission, or gone mid-walk.
    pub unreadable: usize,
    /// Folders on another filesystem, not entered.
    pub elsewhere: usize,
    pub found: usize,
    pub elapsed: Duration,
}

impl Summary {
    /// A walk that never started — what a host reports for one it could
    /// not run at all, so the view still leaves "Searching…".
    pub fn stopped() -> Summary {
        Summary { end: End::Stopped, folders: 0, unreadable: 0, elsewhere: 0, found: 0, elapsed: Duration::ZERO }
    }
}

/// How often a walk hands over what it has found, at most, and how many
/// results make a batch worth sending early. Results arrive as a few
/// messages a second rather than one per match: a window redrawing per
/// result would spend more time drawing than the walk spends reading.
pub const BATCH_EVERY: Duration = Duration::from_millis(120);
pub const BATCH_SIZE: usize = 250;

/// Walks `request.root` breadth first, so the nearest results arrive
/// first, handing each batch of matches to `emit` — see the module doc
/// for what it does not enter and how it is bounded.
///
/// `elsewhere` says whether a folder is on another filesystem than the
/// root; the host knows how to ask (a `stat`), this crate does not. A
/// result's [`Entry::origin`] is set to the folder it was found in: the
/// listing already draws that field as a column of folders, and it is
/// the same fact the Trash puts there — where this thing is.
///
/// `emit` returning `false` means nobody is listening any more, and the
/// walk stops. `cancel` is checked before every folder.
pub fn walk<B: FsBackend + ?Sized>(
    backend: &B,
    request: &Request,
    cancel: &AtomicBool,
    elsewhere: &dyn Fn(&Path) -> bool,
    emit: &mut dyn FnMut(Vec<Entry>) -> bool,
) -> Summary {
    let started = Instant::now();
    let budget = request.budget;
    let include_hidden = request.show_hidden || request.matcher.wants_hidden();
    let mut summary = Summary { end: End::Complete, ..Summary::stopped() };
    let mut queue: VecDeque<(PathBuf, usize)> = VecDeque::from([(request.root.clone(), 0)]);
    let mut batch: Vec<Entry> = Vec::new();
    let mut last_sent = started;

    'walk: while let Some((dir, depth)) = queue.pop_front() {
        if cancel.load(Ordering::Relaxed) {
            summary.end = End::Stopped;
            break;
        }
        if started.elapsed() >= budget.time {
            summary.end = End::TimeLimit;
            break;
        }
        if summary.folders >= budget.folders {
            summary.end = End::FolderLimit;
            break;
        }
        summary.folders += 1;
        let entries = match backend.read_dir(&dir) {
            Ok(entries) => entries,
            Err(_) => {
                summary.unreadable += 1;
                continue;
            }
        };
        for mut entry in entries {
            if entry.hidden && !include_hidden {
                continue;
            }
            if entry.is_dir && !entry.is_symlink && depth + 1 < budget.depth && !request.skip.contains(&entry.path) {
                if elsewhere(&entry.path) {
                    summary.elsewhere += 1;
                } else if queue.len() < budget.folders {
                    queue.push_back((entry.path.clone(), depth + 1));
                }
            }
            if request.matcher.matches(&entry) {
                entry.origin = Some(dir.clone());
                batch.push(entry);
                summary.found += 1;
                if summary.found >= budget.found {
                    summary.end = End::FoundLimit;
                    break 'walk;
                }
            }
        }
        if batch.len() >= BATCH_SIZE || (!batch.is_empty() && last_sent.elapsed() >= BATCH_EVERY) {
            last_sent = Instant::now();
            if !emit(std::mem::take(&mut batch)) {
                summary.end = End::Stopped;
                break;
            }
        }
    }
    if !batch.is_empty() {
        emit(batch);
    }
    summary.elapsed = started.elapsed();
    summary
}

/// A search somebody saved: what was typed, and where it looked. Shown
/// in the sidebar under Saved Searches, and run again when clicked.
///
/// The query as text, the way it would be typed, rather than as parsed
/// filters: the text is what a person wrote and can read in
/// `files.toml`, and a filter this version does not understand is then
/// reported when the search runs instead of failing the whole file.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SmartFolder {
    pub name: String,
    pub query: String,
    /// Where it looks from.
    pub folder: PathBuf,
    /// Whether it looks below `folder` too. Defaults to on: a saved
    /// search over one folder is rarely what anybody saves.
    #[serde(default = "yes")]
    pub subfolders: bool,
}

fn yes() -> bool {
    true
}

/// A change to the saved searches. The window owns the list — every tab
/// shows the same one — so a change goes to it, as a pin does.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SmartChange {
    /// Add this, or replace the one with the same name.
    Save(SmartFolder),
    /// Remove the one with this name.
    Forget(String),
}

/// The list with `change` applied. Pure, like
/// [`crate::sidebar::apply_pin_change`], so the window and every test
/// agree. A name is the identity: saving one that exists replaces it in
/// place rather than adding a second row nobody could tell apart.
pub fn apply_smart_change(list: &[SmartFolder], change: &SmartChange) -> Vec<SmartFolder> {
    let mut list = list.to_vec();
    match change {
        SmartChange::Save(saved) => match list.iter_mut().find(|s| s.name == saved.name) {
            Some(existing) => *existing = saved.clone(),
            None => list.push(saved.clone()),
        },
        SmartChange::Forget(name) => list.retain(|s| &s.name != name),
    }
    list
}

/// What a browser hands its host about searching — one variant of
/// [`crate::browser::Outcome`], so a host has one arm for all of it.
#[derive(Debug, Clone, PartialEq)]
pub enum Ask {
    /// Run this search, cancelling whatever this tab was running.
    Run(Request),
    /// Cancel whatever this tab is running.
    Stop,
    /// Change the saved searches, for every tab, and on disk.
    Smart(SmartChange),
}

/// Everything about searching a browser can be told — one variant of
/// [`crate::browser::Message`].
#[derive(Debug, Clone, PartialEq)]
pub enum SearchMessage {
    /// Look somewhere else.
    Scope(Scope),
    /// A chip's × — by position among the chips.
    RemoveChip(usize),
    /// Results from a walk the host is running.
    Found { run: u64, entries: Vec<Entry> },
    /// That walk ended.
    Finished { run: u64, summary: Summary },
    /// The Stop button.
    Stop,
    /// "Save search…": ask for a name.
    SaveStart,
    SaveName(String),
    SaveCommit,
    SaveCancel,
    /// Take the saved search on screen out of the sidebar.
    Forget,
    /// A saved search in the sidebar was clicked, by position.
    Open(usize),
}

/// The chips a saved query starts with, and the text left for the field.
pub fn load_query(text: &str) -> (Vec<Filter>, String) {
    crate::query::all_chips(text)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::backend::mock::MockBackend;
    use crate::types::EntryKind;

    fn request(root: &str, query: &str) -> Request {
        Request {
            run: 1,
            root: PathBuf::from(root),
            matcher: crate::query::parse(query).matcher(chrono::Local::now()),
            show_hidden: false,
            skip: Vec::new(),
            budget: Budget::default(),
        }
    }

    /// `/r` with `a.rs`, `b.txt`, `.hidden/` (holding `h.rs`), `sub/`
    /// (holding `c.rs` and `deep/d.rs`), and `link/`, a symlinked folder.
    fn tree() -> MockBackend {
        let backend = MockBackend::new();
        let r = Path::new("/r");
        let mut hidden = MockBackend::dir(r, ".hidden");
        hidden.hidden = true;
        let mut link = MockBackend::dir(r, "link");
        link.is_symlink = true;
        backend.seed("/r", vec![
            MockBackend::file(r, "a.rs", 10),
            MockBackend::file(r, "b.txt", 10),
            hidden,
            MockBackend::dir(r, "sub"),
            link,
        ]);
        backend.seed("/r/.hidden", vec![MockBackend::file(Path::new("/r/.hidden"), "h.rs", 1)]);
        backend.seed("/r/sub", vec![MockBackend::file(Path::new("/r/sub"), "c.rs", 1), MockBackend::dir(Path::new("/r/sub"), "deep")]);
        backend.seed("/r/sub/deep", vec![MockBackend::file(Path::new("/r/sub/deep"), "d.rs", 1)]);
        backend.seed("/r/link", vec![MockBackend::file(Path::new("/r/link"), "loop.rs", 1)]);
        backend
    }

    fn run(backend: &MockBackend, request: &Request) -> (Vec<Entry>, Summary) {
        let mut found = Vec::new();
        let summary = walk(backend, request, &AtomicBool::new(false), &|_| false, &mut |batch| {
            found.extend(batch);
            true
        });
        (found, summary)
    }

    fn names(found: &[Entry]) -> Vec<&str> {
        let mut names: Vec<&str> = found.iter().map(|e| e.name.as_str()).collect();
        names.sort();
        names
    }

    #[test]
    fn a_walk_finds_matches_at_every_depth_and_says_it_finished() {
        let (found, summary) = run(&tree(), &request("/r", "ext:rs"));
        assert_eq!(names(&found), ["a.rs", "c.rs", "d.rs"]);
        assert_eq!(summary.end, End::Complete);
        assert_eq!(summary.found, 3);
        assert_eq!(summary.folders, 3, "/r, /r/sub and /r/sub/deep — not the link, not the dotfolder");
    }

    /// Breadth first, so what is nearest the folder you are in arrives
    /// before what is buried.
    #[test]
    fn nearer_results_arrive_first() {
        let (found, _) = run(&tree(), &request("/r", "ext:rs"));
        assert_eq!(found.first().map(|e| e.name.as_str()), Some("a.rs"));
        assert_eq!(found.last().map(|e| e.name.as_str()), Some("d.rs"));
    }

    #[test]
    fn each_result_names_the_folder_it_was_found_in() {
        let (found, _) = run(&tree(), &request("/r", "d.rs"));
        assert_eq!(found[0].origin.as_deref(), Some(Path::new("/r/sub/deep")));
    }

    #[test]
    fn a_walk_does_not_enter_hidden_folders_or_follow_links_to_folders() {
        let (found, _) = run(&tree(), &request("/r", "ext:rs"));
        assert!(!found.iter().any(|e| e.name == "h.rs" || e.name == "loop.rs"));
    }

    #[test]
    fn asking_for_dotfiles_or_showing_them_lets_the_walk_into_hidden_folders() {
        let (found, _) = run(&tree(), &request("/r", "h.rs"));
        assert!(found.is_empty());
        let mut showing = request("/r", "h.rs");
        showing.show_hidden = true;
        assert_eq!(names(&run(&tree(), &showing).0), ["h.rs"]);
    }

    /// An archive met on the way is a file, matched by name — never
    /// unpacked. Pinned with an entry the mock would happily "list" if
    /// asked: the walk must not ask.
    #[test]
    fn an_archive_on_the_way_is_matched_by_name_and_never_opened() {
        let backend = MockBackend::new();
        let r = Path::new("/r");
        let zip = MockBackend::file(r, "photos.zip", 10);
        assert_eq!(zip.kind, EntryKind::Archive);
        backend.seed("/r", vec![zip]);
        backend.seed("/r/photos.zip", vec![MockBackend::file(Path::new("/r/photos.zip"), "inside.jpg", 1)]);
        let (found, summary) = run(&backend, &request("/r", "kind:archive|image"));
        assert_eq!(names(&found), ["photos.zip"]);
        assert_eq!(summary.folders, 1);
    }

    /// The two kinds of "not searched" are counted, never dropped.
    #[test]
    fn unreadable_folders_and_other_filesystems_are_counted() {
        let backend = tree();
        backend.make_unreadable("/r/sub/deep");
        let request = request("/r", "ext:rs");
        let mut found = Vec::new();
        let summary = walk(&backend, &request, &AtomicBool::new(false), &|_| false, &mut |b| {
            found.extend(b);
            true
        });
        assert_eq!(summary.unreadable, 1);
        assert_eq!(summary.end, End::Complete);

        let summary = walk(&tree(), &request, &AtomicBool::new(false), &|p| p.ends_with("sub"), &mut |_| true);
        assert_eq!(summary.elsewhere, 1);
        assert_eq!(summary.folders, 1, "the other filesystem was not entered");
    }

    #[test]
    fn a_skipped_folder_is_never_entered() {
        let mut request = request("/r", "ext:rs");
        request.skip = vec![PathBuf::from("/r/sub")];
        let (found, _) = run(&tree(), &request);
        assert_eq!(names(&found), ["a.rs"]);
    }

    #[test]
    fn cancelling_stops_before_the_next_folder() {
        let summary = walk(&tree(), &request("/r", "ext:rs"), &AtomicBool::new(true), &|_| false, &mut |_| true);
        assert_eq!(summary.end, End::Stopped);
        assert_eq!(summary.folders, 0);
    }

    #[test]
    fn a_listener_that_has_gone_stops_the_walk() {
        let backend = MockBackend::new();
        backend.seed_many("/big", BATCH_SIZE + 1);
        let mut request = request("/big", "");
        request.root = PathBuf::from("/big");
        let summary = walk(&backend, &request, &AtomicBool::new(false), &|_| false, &mut |_| false);
        assert_eq!(summary.end, End::Stopped);
    }

    /// Each limit is its own ending, so the view can say which.
    #[test]
    fn every_limit_ends_the_walk_with_its_own_reason() {
        let backend = tree();
        let mut by_found = request("/r", "ext:rs");
        by_found.budget.found = 2;
        let (found, summary) = run(&backend, &by_found);
        assert_eq!((summary.end, found.len()), (End::FoundLimit, 2), "and keeps no more than the limit");

        let mut by_folders = request("/r", "ext:rs");
        by_folders.budget.folders = 1;
        assert_eq!(run(&backend, &by_folders).1.end, End::FolderLimit);

        let mut by_time = request("/r", "ext:rs");
        by_time.budget.time = Duration::ZERO;
        assert_eq!(run(&backend, &by_time).1.end, End::TimeLimit);

        let mut by_depth = request("/r", "ext:rs");
        by_depth.budget.depth = 2;
        let (found, summary) = run(&backend, &by_depth);
        assert_eq!(names(&found), ["a.rs", "c.rs"], "two levels: the root and one below");
        assert_eq!(summary.end, End::Complete, "depth bounds the tree, it is not a failure");
    }

    /// The queue of folders waiting to be read is bounded by the folder
    /// budget, so a root with a hundred thousand subfolders cannot grow
    /// it past what the walk would ever reach. Resource, not result.
    #[test]
    fn the_queue_never_holds_more_folders_than_the_walk_may_read() {
        let backend = MockBackend::new();
        let root = Path::new("/wide");
        backend.seed("/wide", (0..500).map(|i| MockBackend::dir(root, &format!("d{i}"))).collect());
        let mut request = request("/wide", "nothing-matches-this");
        request.budget.folders = 10;
        let summary = walk(&backend, &request, &AtomicBool::new(false), &|_| false, &mut |_| true);
        assert_eq!(summary.end, End::FolderLimit);
        assert_eq!(summary.folders, 10);
    }

    #[test]
    fn results_arrive_in_batches_not_one_message_each() {
        let backend = MockBackend::new();
        backend.seed_many("/big", BATCH_SIZE * 3);
        let request = request("/big", "");
        let mut batches = 0;
        let mut total = 0;
        walk(&backend, &request, &AtomicBool::new(false), &|_| false, &mut |b| {
            batches += 1;
            total += b.len();
            true
        });
        assert_eq!(total, BATCH_SIZE * 3);
        assert!(batches <= 2, "one folder is one batch at most, plus the remainder: {batches}");
    }

    // --- saved searches --------------------------------------------------

    fn saved(name: &str, query: &str) -> SmartFolder {
        SmartFolder { name: name.into(), query: query.into(), folder: "/home/a".into(), subfolders: true }
    }

    #[test]
    fn saving_a_name_that_exists_replaces_it_in_place() {
        let list = vec![saved("Rust", "ext:rs"), saved("Big", "size:>1G")];
        let list = apply_smart_change(&list, &SmartChange::Save(saved("Rust", "ext:rs kind:code")));
        assert_eq!(list.len(), 2);
        assert_eq!(list[0].query, "ext:rs kind:code", "same place, new query");
        let list = apply_smart_change(&list, &SmartChange::Save(saved("Photos", "kind:image")));
        assert_eq!(list.iter().map(|s| s.name.as_str()).collect::<Vec<_>>(), ["Rust", "Big", "Photos"]);
    }

    #[test]
    fn forgetting_removes_only_that_one() {
        let list = vec![saved("Rust", "ext:rs"), saved("Big", "size:>1G")];
        assert_eq!(apply_smart_change(&list, &SmartChange::Forget("Rust".into())), [saved("Big", "size:>1G")]);
        assert_eq!(apply_smart_change(&list, &SmartChange::Forget("Nope".into())), list);
    }

    /// A saved search written without `subfolders` — by hand, or by a
    /// later version that dropped it — still loads, looking below.
    #[test]
    fn a_saved_search_without_subfolders_looks_below() {
        let parsed: SmartFolder = toml::from_str("name = \"x\"\nquery = \"ext:rs\"\nfolder = \"/p\"\n").unwrap();
        assert!(parsed.subfolders);
    }
}
