//! Column view: the folders above the one in view, each a pane to its
//! left, with the folder you are in highlighted in the pane before it.
//!
//! The folder in view is still the browser's one listing — selection,
//! rename, drag, menus and the preview pane all work on it exactly as in
//! the list. The panes to its left are *context*: each is a folder on
//! the way here, read once, showing where you came from. Clicking in one
//! goes there. That keeps column view a way of drawing the browser
//! rather than a second browser with its own idea of where you are.
//!
//! The metadata rail the design draws to the right is the preview pane,
//! which column view does not need a copy of.
//!
//! Pure: which folders get a pane, and which row in each is the trail.
//! The reading is the host's, through [`crate::browser::Outcome::ReadColumns`].

use std::path::{Path, PathBuf};

/// The most panes kept to the left of the folder in view. A window shows
/// as many of the nearest as fit; past a dozen levels the far ones are
/// never on screen, and each is a directory read.
pub const MAX_ANCESTORS: usize = 12;

/// The folders that get a pane, outermost first, ending with the parent
/// of `current`.
///
/// They start at home when `current` is inside it, because home is where
/// a person thinks a path starts — `/` and `/home` above it would be two
/// panes of nothing anyone browses. Anywhere else they start at `/`. In
/// view itself, home or `/` has no panes at all.
pub fn ancestors(current: &Path, home: Option<&Path>) -> Vec<PathBuf> {
    if home == Some(current) {
        return Vec::new();
    }
    let start = match home {
        Some(home) if current != home && current.starts_with(home) => home,
        _ => Path::new("/"),
    };
    let mut dirs: Vec<PathBuf> = current
        .ancestors()
        .skip(1)
        .take_while(|dir| dir.starts_with(start))
        .map(Path::to_path_buf)
        .collect();
    dirs.reverse();
    if dirs.len() > MAX_ANCESTORS {
        dirs.drain(..dirs.len() - MAX_ANCESTORS);
    }
    dirs
}

/// The entry in `dir`'s pane that leads toward `current`: the folder you
/// went into from there. `None` when `current` is not below `dir`.
pub fn trail(dir: &Path, current: &Path) -> Option<PathBuf> {
    let rest = current.strip_prefix(dir).ok()?;
    let first = rest.components().next()?;
    Some(dir.join(first))
}

/// How far down a pane of `rows` rows to scroll so row `index` is in
/// view, as a fraction of the pane's scroll range — the same fraction
/// iced's `snap_to` takes. Proportional, so the first row lands at the
/// top, the last at the bottom, and one in the middle in the middle:
/// always on screen, whatever height the pane turned out to be.
pub fn reveal(index: usize, rows: usize) -> f32 {
    if rows <= 1 {
        return 0.0;
    }
    index.min(rows - 1) as f32 / (rows - 1) as f32
}

#[cfg(test)]
mod tests {
    use super::*;

    fn paths(list: &[&str]) -> Vec<PathBuf> {
        list.iter().map(PathBuf::from).collect()
    }

    #[test]
    fn inside_home_the_panes_start_at_home() {
        let home = Path::new("/home/me");
        assert_eq!(
            ancestors(Path::new("/home/me/Documents/Projects"), Some(home)),
            paths(&["/home/me", "/home/me/Documents"]),
        );
    }

    #[test]
    fn outside_home_the_panes_start_at_the_root() {
        let home = Path::new("/home/me");
        assert_eq!(ancestors(Path::new("/usr/share/icons"), Some(home)), paths(&["/", "/usr", "/usr/share"]));
    }

    #[test]
    fn home_and_the_root_have_nothing_to_their_left() {
        let home = Path::new("/home/me");
        assert!(ancestors(home, Some(home)).is_empty());
        assert!(ancestors(Path::new("/"), Some(home)).is_empty());
    }

    #[test]
    fn a_folder_beside_home_is_not_mistaken_for_one_inside_it() {
        // `/home/me2` starts with the *text* `/home/me`; paths compare by
        // component, so it is outside home and starts at the root.
        let home = Path::new("/home/me");
        assert_eq!(ancestors(Path::new("/home/me2/x"), Some(home)), paths(&["/", "/home", "/home/me2"]));
    }

    #[test]
    fn a_deep_folder_keeps_only_the_nearest_panes() {
        let deep: PathBuf = (0..20).fold(PathBuf::from("/"), |p, i| p.join(format!("d{i}")));
        let dirs = ancestors(&deep, None);
        assert_eq!(dirs.len(), MAX_ANCESTORS);
        assert_eq!(dirs.last().map(PathBuf::as_path), deep.parent(), "the parent is always kept");
    }

    #[test]
    fn the_trail_is_the_folder_you_went_into() {
        let current = Path::new("/home/me/Documents/Projects");
        assert_eq!(trail(Path::new("/home/me"), current), Some(PathBuf::from("/home/me/Documents")));
        assert_eq!(trail(Path::new("/home/me/Documents"), current), Some(PathBuf::from("/home/me/Documents/Projects")));
        assert_eq!(trail(Path::new("/usr"), current), None);
        assert_eq!(trail(current, current), None, "a folder is not on its own trail");
    }

    #[test]
    fn revealing_a_row_scrolls_in_proportion_and_never_past_the_end() {
        assert_eq!(reveal(0, 50), 0.0);
        assert_eq!(reveal(49, 50), 1.0);
        assert_eq!(reveal(99, 50), 1.0);
        assert_eq!(reveal(0, 1), 0.0);
        assert_eq!(reveal(0, 0), 0.0);
    }
}
