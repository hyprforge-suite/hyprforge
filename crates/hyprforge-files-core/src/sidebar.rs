//! What the sidebar offers: Home, whichever XDG user directories
//! actually exist, and Trash.
//!
//! [`build`] does real filesystem I/O (through [`crate::backend::FsBackend`],
//! same rule as everywhere else in this crate) and so must be called by
//! a host off the UI thread, exactly like a directory read — [`Browser`](crate::browser::Browser)
//! never calls it itself; a host hands the result in at construction or
//! via a message.
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

/// Builds the sidebar: Home, then each of Documents/Downloads/Pictures/
/// Music/Videos/Desktop that [`resolve`](crate::xdg_user_dirs) found *and*
/// that actually exists as a directory right now, in that fixed order,
/// then Trash last.
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

    items.push(SidebarItem {
        label: "Trash".to_string(),
        path: hyprforge_fileops::home_trash_dir().join("files"),
    });

    items
}

fn exists_as_dir<B: FsBackend + ?Sized>(backend: &B, path: &std::path::Path) -> bool {
    backend.stat(path).map(|entry| entry.is_dir).unwrap_or(false)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::backend::mock::MockBackend;
    use std::path::PathBuf;

    #[test]
    fn home_and_trash_are_always_present() {
        let backend = MockBackend::new();
        let items = build(&backend, &UserDirs::default());
        assert_eq!(items.first().unwrap().label, "Home");
        assert_eq!(items.last().unwrap().label, "Trash");
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
        assert_eq!(labels, vec!["Home", "Documents", "Pictures", "Trash"]);
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
}
