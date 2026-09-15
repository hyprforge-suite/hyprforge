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
use std::path::PathBuf;

/// One row in the sidebar.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SidebarItem {
    /// What the row shows. For a user directory this is the XDG label
    /// ("Documents", "Downloads", ...); for Home and Trash it is fixed.
    pub label: String,
    /// Where clicking it navigates.
    pub path: PathBuf,
}

/// Builds the **Places** section: Home, then each of Documents/Downloads/
/// Pictures/Music/Videos/Desktop that [`resolve`](crate::xdg_user_dirs)
/// found *and* that actually exists as a directory right now, in that
/// fixed order.
///
/// A user directory that `user-dirs.dirs` names but that has since been
/// deleted or renamed is left out rather than shown as a shortcut to
/// nowhere — checked with [`FsBackend::stat`], the same call
/// [`crate::browser`] uses for everything else, so the mock backend
/// tests this against is the one real code runs against too.
pub fn build<B: FsBackend + ?Sized>(backend: &B, user_dirs: &UserDirs) -> Vec<SidebarItem> {
    let mut items = vec![SidebarItem {
        label: "Home".to_string(),
        path: backend.home_dir(),
    }];

    let candidates: [(&str, &Option<PathBuf>); 6] = [
        ("Documents", &user_dirs.documents),
        ("Downloads", &user_dirs.download),
        ("Pictures", &user_dirs.pictures),
        ("Music", &user_dirs.music),
        ("Videos", &user_dirs.videos),
        ("Desktop", &user_dirs.desktop),
    ];
    for (label, path) in candidates {
        if let Some(path) = path {
            if exists_as_dir(backend, path) {
                items.push(SidebarItem {
                    label: label.to_string(),
                    path: path.clone(),
                });
            }
        }
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
            let item_count = backend.read_dir(path).ok().map(|entries| entries.len());
            PinnedItem { label, path: path.clone(), item_count }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::backend::mock::MockBackend;
    use std::path::PathBuf;

    #[test]
    fn home_is_always_present() {
        let backend = MockBackend::new();
        let items = build(&backend, &UserDirs::default());
        assert_eq!(items.first().unwrap().label, "Home");
    }

    /// Trash moved out of `build` into its own fixed path — see the
    /// module doc — so this pins that it no longer rides along in
    /// Places at all, not even last.
    #[test]
    fn build_no_longer_includes_trash() {
        let backend = MockBackend::new();
        let items = build(&backend, &UserDirs::default());
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
        let items = build(&backend, &user_dirs);
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
        let items = build(&backend, &user_dirs);
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
        let items = build(&backend, &user_dirs);
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
