//! Starred files, and the two lists the sidebar shows that are not
//! folders: Recent and Starred.
//!
//! A star is a path in `files.toml`'s `starred` — see
//! [`crate::prefs::Prefs::starred`] — and like the pinned list it is the
//! window's one list rather than a tab's: every tab shows the same stars,
//! a change goes to the window as a [`StarChange`], and the window writes
//! it through `prefs::update` because the portal's dialog shares the file.
//!
//! Pins are folders and stay folders; a star is anything. The two differ
//! in one more way that is the reason this module has [`follow`]: a star
//! is kept on a *file*, and files get renamed and moved far more often
//! than the folders people pin. A star that stayed behind on the old
//! name would be a star quietly lost the first time someone tidied up.
//! Only moves made inside Files are followed — nothing else tells this
//! window a file moved — and a star whose file has gone some other way is
//! shown as missing, the way a pin is, rather than dropped behind the
//! person's back.

use crate::backend::FsBackend;
use crate::types::Entry;
use std::path::{Path, PathBuf};

/// A list the sidebar offers that is not a folder: what it shows comes
/// from many folders, the way a search's results do.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Collection {
    /// What was opened lately, by anything — see [`crate::recent`].
    Recent,
    /// What the person starred.
    Starred,
}

impl Collection {
    pub fn label(self) -> &'static str {
        match self {
            Collection::Recent => "Recent",
            Collection::Starred => "Starred",
        }
    }

    /// The freedesktop icon names for the sidebar row, best first, as one
    /// [`crate::icon::themed_key`] name list.
    ///
    /// The folder-shaped ones first, so the row is drawn in the same
    /// style as the places under it. And a list rather than one name,
    /// because the lookup tries every name in a theme before it moves on
    /// to the next one, and the host's last resort is `folder`: asked for
    /// `starred` alone, a theme such as Dracula — which has no `starred`
    /// but does have `folder` — answered with its plain folder before the
    /// star its parent theme has was ever reached.
    pub fn icon_name(self) -> &'static str {
        match self {
            Collection::Recent => "folder-recent,document-open-recent",
            Collection::Starred => "folder-favorites,starred,emblem-favorite",
        }
    }
}

/// A change to the starred list.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StarChange {
    /// Star these. One already starred stays where it is.
    Star(Vec<PathBuf>),
    /// Unstar these.
    Unstar(Vec<PathBuf>),
    /// These moved, `(from, to)`: a star on `from`, or on anything inside
    /// it when it is a folder, moves with it.
    Follow(Vec<(PathBuf, PathBuf)>),
}

/// The list with `change` applied. Pure, like
/// [`crate::sidebar::apply_pin_change`], so the window and every test
/// agree — and idempotent: starring twice is starring once.
pub fn apply_star_change(list: &[PathBuf], change: &StarChange) -> Vec<PathBuf> {
    let mut out = list.to_vec();
    match change {
        StarChange::Star(paths) => {
            for path in paths {
                if !out.contains(path) {
                    out.push(path.clone());
                }
            }
        }
        StarChange::Unstar(paths) => out.retain(|p| !paths.contains(p)),
        StarChange::Follow(moves) => out = follow(&out, moves),
    }
    out
}

/// `list` after `moves` — see [`StarChange::Follow`]. A star whose path
/// two moves both claim keeps its place once.
pub fn follow(list: &[PathBuf], moves: &[(PathBuf, PathBuf)]) -> Vec<PathBuf> {
    let mut out: Vec<PathBuf> = Vec::with_capacity(list.len());
    for star in list {
        let moved = moves.iter().find_map(|(from, to)| {
            if star == from {
                Some(to.clone())
            } else {
                // `Path::strip_prefix` compares whole components, so a
                // folder `a` moving does not drag a star on `ab` along.
                star.strip_prefix(from).ok().map(|rest| to.join(rest))
            }
        });
        let now = moved.unwrap_or_else(|| star.clone());
        if !out.contains(&now) {
            out.push(now);
        }
    }
    out
}

/// Whether a star changing `paths` stars them or unstars them: unstar
/// when every one is already starred, star otherwise — so a selection of
/// one starred file and one not is starred whole rather than flipped
/// file by file into a mixture.
pub fn toggle(starred: &[PathBuf], paths: Vec<PathBuf>) -> StarChange {
    if !paths.is_empty() && paths.iter().all(|p| starred.contains(p)) {
        StarChange::Unstar(paths)
    } else {
        StarChange::Star(paths)
    }
}

/// What the Starred view shows: an entry for each star that is still
/// there, in the order they were starred, and the stars that are not.
/// Each entry's `origin` is its folder. Blocking — a host runs it off
/// the UI thread.
pub fn read_starred<B: FsBackend + ?Sized>(backend: &B, starred: &[PathBuf]) -> (Vec<Entry>, Vec<PathBuf>) {
    let mut found = Vec::new();
    let mut missing = Vec::new();
    for path in starred {
        match backend.stat(path) {
            Ok(mut entry) => {
                entry.origin = path.parent().map(Path::to_path_buf);
                found.push(entry);
            }
            Err(_) => missing.push(path.clone()),
        }
    }
    (found, missing)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn paths(list: &[&str]) -> Vec<PathBuf> {
        list.iter().map(PathBuf::from).collect()
    }

    #[test]
    fn starring_is_idempotent() {
        let once = apply_star_change(&[], &StarChange::Star(paths(&["/a", "/b"])));
        let twice = apply_star_change(&once, &StarChange::Star(paths(&["/b", "/a"])));
        assert_eq!(twice, paths(&["/a", "/b"]), "the same list, in the order first starred");
        let doubled = apply_star_change(&[], &StarChange::Star(paths(&["/a", "/a"])));
        assert_eq!(doubled, paths(&["/a"]));
    }

    #[test]
    fn unstarring_takes_out_only_what_was_named() {
        let list = paths(&["/a", "/b", "/c"]);
        assert_eq!(apply_star_change(&list, &StarChange::Unstar(paths(&["/b", "/zzz"]))), paths(&["/a", "/c"]));
    }

    #[test]
    fn a_star_follows_a_rename() {
        let list = paths(&["/docs/draft.txt", "/docs/other.txt"]);
        let moved = apply_star_change(&list, &StarChange::Follow(vec![("/docs/draft.txt".into(), "/docs/final.txt".into())]));
        assert_eq!(moved, paths(&["/docs/final.txt", "/docs/other.txt"]), "and keeps its place");
    }

    /// A folder moved carries every star inside it — and only those: a
    /// sibling whose name merely starts the same way stays put.
    #[test]
    fn a_star_inside_a_moved_folder_moves_with_it() {
        let list = paths(&["/p/a/x.txt", "/p/ab/y.txt"]);
        let moved = follow(&list, &[("/p/a".into(), "/q/a".into())]);
        assert_eq!(moved, paths(&["/q/a/x.txt", "/p/ab/y.txt"]));
    }

    #[test]
    fn a_mixed_selection_is_starred_whole_and_an_all_starred_one_unstarred() {
        let starred = paths(&["/a"]);
        assert_eq!(toggle(&starred, paths(&["/a", "/b"])), StarChange::Star(paths(&["/a", "/b"])));
        assert_eq!(toggle(&starred, paths(&["/a"])), StarChange::Unstar(paths(&["/a"])));
    }

    /// Gone is not forgotten: a star whose file cannot be found is
    /// reported as missing, for the view to say so, not dropped.
    #[test]
    fn a_star_whose_file_is_gone_is_reported_missing() {
        let backend = crate::backend::mock::MockBackend::new();
        backend.seed("/d", vec![crate::backend::mock::MockBackend::file(Path::new("/d"), "here.txt", 1)]);
        let (found, missing) = read_starred(&backend, &paths(&["/d/gone.txt", "/d/here.txt"]));
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].origin.as_deref(), Some(Path::new("/d")));
        assert_eq!(missing, paths(&["/d/gone.txt"]));
    }
}
