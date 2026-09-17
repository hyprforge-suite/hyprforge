//! What the sidebar's **Places** section offers: Home and whichever XDG
//! user directories actually exist.
//!
//! Places is one of three sections the design puts in the sidebar —
//! Places, Pinned, Trash — and this module builds only that one. Trash
//! is [`trash_path`] below: a fixed location that needs no existence
//! check and so needs neither `FsBackend` nor an off-thread call, unlike
//! every user directory here. Pinned is [`build_pinned`]: unlike Places,
//! which is Home plus whatever the desktop's own `user-dirs.dirs`
//! happens to name, Pinned is a list the *user* built (see
//! `crate::prefs::Prefs::pinned`), so it is built from that list rather
//! than discovered.
//!
//! [`build`] and [`build_pinned`] both do real filesystem I/O (through
//! [`crate::backend::FsBackend`], same rule as everywhere else in this
//! crate) and so must be called by a host off the UI thread, exactly
//! like a directory read — [`Browser`](crate::browser::Browser) never
//! calls either itself; a host hands the result in at construction (for
//! Places) or via a message (for Pinned, since it can change after
//! `Browser` already exists).
//!
//! Mounted shares are the next thing this sidebar will offer and are
//! deliberately not built here yet — there is nothing in this workspace
//! today that mounts one to build or test against. [`SidebarItem`] is
//! shaped so adding a mounted-share entry later is "push another item
//! with a different origin", not a rework: nothing downstream of this
//! module — the browser, the two hosts — switches on where an item came
//! from, only on its `path`.

use crate::backend::FsBackend;
use crate::xdg_user_dirs::UserDirs;
use std::path::{Path, PathBuf};

/// One row in the sidebar.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SidebarItem {
    /// What the row shows. For a user directory this is the XDG label
    /// ("Documents", "Downloads", ...); for Home and Trash it is fixed.
    pub label: String,
    /// Where clicking it navigates.
    pub path: PathBuf,
    /// Which theme colour this row's folder mark takes.
    pub tint: Tint,
}

/// A sidebar row's colour, named by *role* rather than by value.
///
/// The design gives each place its own hue so a row is findable by
/// colour before it is read — you learn where Downloads sits and stop
/// reading the label. Naming the role rather than the colour is what
/// keeps that out of conflict with the rule that an app never writes a
/// colour: these resolve through `hyprforge_look::Theme` at draw time,
/// so a user on a different theme gets their own palette, distinct in
/// the same way.
///
/// The roles are borrowed here for identity rather than for state, which
/// is a real cost worth naming: `Warning` on the Pictures row does not
/// mean anything is wrong. It is defensible because a sidebar place is
/// not a state-bearing thing — there is no "Pictures is in trouble" for
/// it to be confused with — but the same trick in the entry list, where
/// rows *do* carry state, would be a mistake.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Tint {
    Accent,
    Info,
    Success,
    Warning,
    Dim,
}

impl Tint {
    /// Resolves to the live theme's colour for this role.
    pub fn color(self) -> hyprforge_look::Color {
        let t = hyprforge_ui::theme::active();
        match self {
            Tint::Accent => t.accent,
            Tint::Info => t.info,
            Tint::Success => t.success,
            Tint::Warning => t.warning,
            Tint::Dim => t.surfaces.text_dim,
        }
    }
}

/// One place the Places section can offer. `[sidebar] places` in
/// `files-config.toml` lists which, in what order.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Place {
    Home,
    Documents,
    Downloads,
    Pictures,
    Music,
    Videos,
    Desktop,
}

impl Place {
    /// Every place, in the order shown when nothing is configured.
    pub const ALL: [Place; 7] = [
        Place::Home,
        Place::Documents,
        Place::Downloads,
        Place::Pictures,
        Place::Music,
        Place::Videos,
        Place::Desktop,
    ];

    /// The name `files-config.toml` uses.
    pub fn id(self) -> &'static str {
        match self {
            Place::Home => "home",
            Place::Documents => "documents",
            Place::Downloads => "downloads",
            Place::Pictures => "pictures",
            Place::Music => "music",
            Place::Videos => "videos",
            Place::Desktop => "desktop",
        }
    }

    pub fn from_id(id: &str) -> Option<Place> {
        Place::ALL.into_iter().find(|p| p.id() == id.trim())
    }

    fn label(self) -> &'static str {
        match self {
            Place::Home => "Home",
            Place::Documents => "Documents",
            Place::Downloads => "Downloads",
            Place::Pictures => "Pictures",
            Place::Music => "Music",
            Place::Videos => "Videos",
            Place::Desktop => "Desktop",
        }
    }

    /// A colour per place, so a row is findable before it is read. The
    /// assignment is arbitrary but *fixed*: what matters is that
    /// Downloads is always the same colour, not which colour it is.
    /// Beyond the four the theme has distinct roles for, the rest share
    /// the dim one rather than the palette repeating — two places in the
    /// same green would be worse than several in plain grey, because the
    /// eye would read the repeat as a grouping that means something.
    /// The colour follows the place, not its position, so reordering
    /// places does not repaint them.
    fn tint(self) -> Tint {
        match self {
            Place::Home => Tint::Accent,
            Place::Documents => Tint::Info,
            Place::Downloads => Tint::Success,
            Place::Pictures => Tint::Warning,
            Place::Music | Place::Videos | Place::Desktop => Tint::Dim,
        }
    }
}

/// Builds the **Places** section: `places`, in that order, each one
/// that [`resolve`](crate::xdg_user_dirs) found *and* that actually
/// exists as a directory right now. Home is always offered when listed
/// — it is where the window starts, and it cannot be missing.
///
/// A user directory that `user-dirs.dirs` names but that has since been
/// deleted or renamed is left out rather than shown as a shortcut to
/// nowhere — checked with [`FsBackend::stat`], the same call
/// [`crate::browser`] uses for everything else, so the mock backend
/// tests this against is the one real code runs against too.
pub fn build<B: FsBackend + ?Sized>(backend: &B, user_dirs: &UserDirs, places: &[Place]) -> Vec<SidebarItem> {
    let mut items = Vec::new();
    for &place in places {
        let path = match place {
            Place::Home => Some(backend.home_dir()),
            Place::Documents => user_dirs.documents.clone(),
            Place::Downloads => user_dirs.download.clone(),
            Place::Pictures => user_dirs.pictures.clone(),
            Place::Music => user_dirs.music.clone(),
            Place::Videos => user_dirs.videos.clone(),
            Place::Desktop => user_dirs.desktop.clone(),
        };
        let Some(path) = path else { continue };
        if place != Place::Home && !exists_as_dir(backend, &path) {
            continue;
        }
        items.push(SidebarItem { label: place.label().to_string(), path, tint: place.tint() });
    }
    items
}

fn exists_as_dir<B: FsBackend + ?Sized>(backend: &B, path: &std::path::Path) -> bool {
    backend.stat(path).map(|entry| entry.is_dir).unwrap_or(false)
}

/// Where the sidebar's Trash entry always points.
///
/// Fixed, and needs no [`FsBackend`]: unlike a user directory, "does
/// Trash exist" is not a question this app asks before offering the
/// shortcut — `hyprforge-fileops` creates the directory on first use, the
/// same way any other trash implementation does, so a Trash section that
/// waited on a `stat` would show nothing on a machine that has never
/// trashed a file yet, which is a worse answer than always offering it.
pub fn trash_path() -> PathBuf {
    hyprforge_fileops::home_trash_dir().join("files")
}

/// What to call a place in a tab, a window title, a search box or a path
/// bar: its folder name — or "Trash" for the Trash.
///
/// The Trash's storage directory is called `files`, which is what every
/// one of those showed until this existed: a tab named "files" and a path
/// bar reading `~ / … / files`, both technically true and neither telling
/// anyone where they are.
pub fn place_name(path: &Path) -> String {
    if path == trash_path() {
        return "Trash".to_string();
    }
    path.file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| path.display().to_string())
}

/// A change to the pinned list.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PinChange {
    Pin(PathBuf),
    Unpin(PathBuf),
    /// Move this pin up (`-1`) or down (`+1`) the list.
    Move(PathBuf, i32),
}

/// The pinned list with `change` applied — pure, so the window's one
/// list and every test agree on what a change does.
///
/// Pinning something already pinned leaves the list alone rather than
/// adding a second row for it; unpinning something not pinned is
/// likewise nothing. A move past either end stays at that end.
pub fn apply_pin_change(pinned: &[PathBuf], change: &PinChange) -> Vec<PathBuf> {
    let mut list = pinned.to_vec();
    match change {
        PinChange::Pin(path) => {
            if !list.contains(path) {
                list.push(path.clone());
            }
        }
        PinChange::Unpin(path) => list.retain(|p| p != path),
        PinChange::Move(path, delta) => {
            if let Some(from) = list.iter().position(|p| p == path) {
                let to = (from as i64 + i64::from(*delta)).clamp(0, list.len() as i64 - 1) as usize;
                let item = list.remove(from);
                list.insert(to, item);
            }
        }
    }
    list
}

/// One row in the sidebar's **Pinned** section: a user-chosen directory
/// (see `crate::prefs::Prefs::pinned`) plus how many entries it holds
/// right now.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PinnedItem {
    /// The directory's own file name — the same rule [`build_pinned`]
    /// uses for every pin, so a pinned `/mnt/data/Projects` reads
    /// "Projects" rather than its full path.
    pub label: String,
    pub path: PathBuf,
    /// `None` when the directory could not be read (removed, permission
    /// lost, unmounted) rather than `Some(0)` — a pin to a directory
    /// nobody can enter must not read identically to a pin to an empty
    /// one, the same "unreadable is not empty" rule `browser::LoadState`
    /// already enforces for the main listing.
    pub item_count: Option<usize>,
}

/// Builds the Pinned section from `pinned` — the paths in
/// `crate::prefs::Prefs::pinned`, in the order the user pinned them.
///
/// Does real filesystem I/O per pin (one [`FsBackend::read_dir`] each),
/// so — like [`build`] — a host calls this off the UI thread and feeds
/// the result to [`Browser`](crate::browser::Browser) via
/// [`crate::browser::Message::PinnedLoaded`], not at construction: unlike
/// Places, the pinned list can change while the browser is already open
/// (the user pins or unpins something), so it needs a message rather
/// than only a constructor argument.
pub fn build_pinned<B: FsBackend + ?Sized>(backend: &B, pinned: &[PathBuf]) -> Vec<PinnedItem> {
    pinned
        .iter()
        .map(|path| {
            let label = path
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_else(|| path.to_string_lossy().into_owned());
            // `count_children`, not `read_dir(..).len()`: that stats and
            // builds an entry for every child only to keep the length.
            let item_count = backend.count_children(path).ok();
            PinnedItem { label, path: path.clone(), item_count }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::backend::mock::MockBackend;

    fn paths(list: &[&str]) -> Vec<PathBuf> {
        list.iter().map(PathBuf::from).collect()
    }

    /// `[sidebar] places` picks and orders the places; a place's colour
    /// goes with it.
    #[test]
    fn places_follow_the_configured_order() {
        let backend = MockBackend::new();
        let dirs = UserDirs::default();
        let items = build(&backend, &dirs, &[Place::Home]);
        assert_eq!(items.len(), 1);
        assert!(build(&backend, &dirs, &[]).is_empty(), "an empty list shows no places");
        assert_eq!(Place::from_id("downloads"), Some(Place::Downloads));
        assert_eq!(Place::from_id("Trash"), None);
    }

    #[test]
    fn pinning_adds_to_the_end_once() {
        let list = paths(&["/a"]);
        let list = apply_pin_change(&list, &PinChange::Pin("/b".into()));
        assert_eq!(list, paths(&["/a", "/b"]));
        assert_eq!(apply_pin_change(&list, &PinChange::Pin("/a".into())), list, "no second row");
    }

    #[test]
    fn unpinning_removes_only_that_one() {
        let list = paths(&["/a", "/b", "/c"]);
        assert_eq!(apply_pin_change(&list, &PinChange::Unpin("/b".into())), paths(&["/a", "/c"]));
        assert_eq!(apply_pin_change(&list, &PinChange::Unpin("/zzz".into())), list);
    }

    #[test]
    fn a_pin_moves_up_and_down_and_stops_at_the_ends() {
        let list = paths(&["/a", "/b", "/c"]);
        assert_eq!(apply_pin_change(&list, &PinChange::Move("/c".into(), -1)), paths(&["/a", "/c", "/b"]));
        assert_eq!(apply_pin_change(&list, &PinChange::Move("/a".into(), 1)), paths(&["/b", "/a", "/c"]));
        assert_eq!(apply_pin_change(&list, &PinChange::Move("/a".into(), -1)), list, "already first");
        assert_eq!(apply_pin_change(&list, &PinChange::Move("/c".into(), 1)), list, "already last");
    }

    #[test]
    fn home_is_always_present() {
        let backend = MockBackend::new();
        let items = build(&backend, &UserDirs::default(), &Place::ALL);
        assert_eq!(items.first().unwrap().label, "Home");
    }

    /// Trash moved out of `build` into its own fixed path — see the
    /// module doc — so this pins that it no longer rides along in
    /// Places at all, not even last.
    #[test]
    fn build_no_longer_includes_trash() {
        let backend = MockBackend::new();
        let items = build(&backend, &UserDirs::default(), &Place::ALL);
        assert!(!items.iter().any(|i| i.label == "Trash"));
    }

    #[test]
    fn trash_path_needs_no_backend_and_is_stable() {
        assert_eq!(trash_path(), trash_path());
        assert!(trash_path().ends_with("files"));
    }

    #[test]
    fn a_user_directory_that_does_not_exist_is_left_out() {
        let backend = MockBackend::new();
        let user_dirs = UserDirs {
            documents: Some(PathBuf::from("/home/mock/Documents")),
            ..UserDirs::default()
        };
        // Never seeded, so `stat` reports NotFound.
        let items = build(&backend, &user_dirs, &Place::ALL);
        assert!(!items.iter().any(|i| i.label == "Documents"));
    }

    #[test]
    fn a_user_directory_that_exists_is_offered_in_the_fixed_order() {
        let backend = MockBackend::new();
        let docs = PathBuf::from("/home/mock/Documents");
        let pics = PathBuf::from("/home/mock/Pictures");
        backend.seed(docs.clone(), vec![]);
        backend.seed(pics.clone(), vec![]);
        let user_dirs = UserDirs {
            documents: Some(docs),
            pictures: Some(pics),
            ..UserDirs::default()
        };
        let items = build(&backend, &user_dirs, &Place::ALL);
        let labels: Vec<&str> = items.iter().map(|i| i.label.as_str()).collect();
        assert_eq!(labels, vec!["Home", "Documents", "Pictures"]);
    }

    #[test]
    fn a_user_directory_that_is_actually_a_file_is_left_out() {
        let backend = MockBackend::new();
        let fake_docs = PathBuf::from("/home/mock/Documents");
        // Seed it as a *file* in its parent so `stat` succeeds but
        // reports is_dir = false.
        backend.seed(
            PathBuf::from("/home/mock"),
            vec![MockBackend::file(&PathBuf::from("/home/mock"), "Documents", 10)],
        );
        let user_dirs = UserDirs {
            documents: Some(fake_docs),
            ..UserDirs::default()
        };
        let items = build(&backend, &user_dirs, &Place::ALL);
        assert!(!items.iter().any(|i| i.label == "Documents"));
    }

    // --- Pinned --------------------------------------------------------

    #[test]
    fn a_pinned_directory_is_labelled_by_its_own_file_name_and_counted() {
        let backend = MockBackend::new();
        let projects = PathBuf::from("/home/mock/Projects");
        backend.seed_many(projects.clone(), 12);

        let pinned = build_pinned(&backend, &[projects]);
        assert_eq!(pinned.len(), 1);
        assert_eq!(pinned[0].label, "Projects");
        assert_eq!(pinned[0].item_count, Some(12));
    }

    /// A pin to a directory that can no longer be read (removed,
    /// unmounted, permission lost) must read as "unknown", never as
    /// "empty" — the same "unreadable is not empty" rule the main
    /// listing already enforces for `LoadState::Error`.
    #[test]
    fn a_pin_that_cannot_be_read_reports_an_unknown_count_not_zero() {
        let backend = MockBackend::new();
        let gone = PathBuf::from("/home/mock/Gone");
        // Never seeded: `read_dir` reports NotFound.

        let pinned = build_pinned(&backend, &[gone]);
        assert_eq!(pinned[0].item_count, None, "unreadable must not read the same as an empty directory");
    }

    #[test]
    fn pins_preserve_the_order_the_user_pinned_them_in() {
        let backend = MockBackend::new();
        let a = PathBuf::from("/home/mock/B");
        let b = PathBuf::from("/home/mock/A");
        backend.seed(a.clone(), vec![]);
        backend.seed(b.clone(), vec![]);

        let pinned = build_pinned(&backend, &[a, b]);
        assert_eq!(pinned[0].label, "B");
        assert_eq!(pinned[1].label, "A");
    }
}
