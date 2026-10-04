//! Dropping files onto the browser: where a drop lands, and what it does.
//!
//! Two questions, both answered here without I/O so they can be tested.
//!
//! **Where.** During a drag the compositor reports the pointer to the
//! window's data device, not to its pointer, so iced never sees it move
//! and no widget can say "the pointer is over me". What the window has is
//! a point. [`HitTest`] takes that point to iced's own layout: every
//! container a drop can land in — a folder's row, a sidebar place, the
//! listing's background — carries a [`target_id`], and the operation asks
//! which of them holds the point. Asking the layout rather than doing
//! arithmetic on row heights is what keeps this right through scrolling,
//! font scaling and the grid.
//!
//! **What.** [`plan`] decides, the way Nautilus and Dolphin do: from
//! another application, always a copy; from this window, a move within
//! one filesystem and a copy across two, with Ctrl and Shift to say
//! otherwise; onto the Trash, to the Trash; and back into the folder the
//! files came from, nothing at all.

use crate::clipboard::{ClipVerb, FileClip};
use iced::advanced::widget::operation::{Operation, Outcome};
use iced::advanced::widget::Id;
use iced::{Point, Rectangle, Vector};
use std::collections::HashMap;
use std::path::{Path, PathBuf};

/// The widget id of the container a drop onto `path` lands in, in the
/// view of browser `instance` ([`crate::Browser::instance`]). The same
/// path gives the same id wherever one browser draws it — a folder in the
/// listing and the same folder in the sidebar are one target.
///
/// Per browser, because a split tab draws two in one window and both can
/// show the same folder: the host merges both browsers' targets, and an id
/// that only named the path would leave it unable to tell which pane's row
/// the drag is over — so it could light only both, or the wrong one.
pub fn target_id(instance: u64, path: &Path) -> Id {
    Id::from(format!("hyprforge-drop:{instance}:{}", path.display()))
}

/// Finds the innermost drop target holding a point.
///
/// Innermost, because targets nest: a folder's row sits inside the
/// listing's background, and a drop on the row means that folder, not the
/// one in view.
///
/// Scrolled content is laid out in its own coordinates, so a container
/// inside a scrollable reports bounds that have not moved with the
/// scroll. The operation keeps the translation of every scrollable it is
/// inside, and its visible bounds too: a row scrolled out of sight still
/// has a layout, and must not catch a drop aimed at whatever is drawn
/// where it would have been.
///
/// Generic over what a target *is* to the host: a path for a window of
/// one listing, a pane and a path for a window of two — see [`target_id`].
pub struct HitTest<T = PathBuf> {
    point: Point,
    targets: HashMap<Id, T>,
    /// The innermost hit so far, and how deep it was.
    found: Option<(usize, T)>,
    /// Per level being walked: the accumulated scroll translation, and
    /// the region actually visible at that level.
    frames: Vec<Frame>,
    /// Set by `scrollable`, consumed by the `traverse` that follows it.
    entering: Option<Frame>,
}

#[derive(Clone, Copy)]
struct Frame {
    translation: Vector,
    visible: Option<Rectangle>,
}

impl<T> HitTest<T> {
    pub fn new(point: Point, targets: HashMap<Id, T>) -> HitTest<T> {
        HitTest {
            point,
            targets,
            found: None,
            frames: vec![Frame { translation: Vector::ZERO, visible: None }],
            entering: None,
        }
    }

    fn frame(&self) -> Frame {
        *self.frames.last().expect("the root frame is never popped")
    }
}

impl<T: Clone + Send + 'static> Operation<Option<T>> for HitTest<T> {
    fn traverse(&mut self, operate: &mut dyn FnMut(&mut dyn Operation<Option<T>>)) {
        let next = self.entering.take().unwrap_or_else(|| self.frame());
        self.frames.push(next);
        operate(self);
        self.frames.pop();
    }

    fn container(&mut self, id: Option<&Id>, bounds: Rectangle) {
        let Some(path) = id.and_then(|id| self.targets.get(id)) else { return };
        let frame = self.frame();
        if frame.visible.is_some_and(|v| !v.contains(self.point)) {
            return;
        }
        // Scrolled by `translation`: content at `bounds` is drawn at
        // `bounds - translation`, so the point is moved the other way.
        if !bounds.contains(self.point + frame.translation) {
            return;
        }
        let depth = self.frames.len();
        if self.found.as_ref().is_none_or(|(d, _)| depth >= *d) {
            self.found = Some((depth, path.clone()));
        }
    }

    fn scrollable(
        &mut self,
        _id: Option<&Id>,
        bounds: Rectangle,
        _content_bounds: Rectangle,
        translation: Vector,
        _state: &mut dyn iced::advanced::widget::operation::Scrollable,
    ) {
        let outer = self.frame();
        // The viewport itself is drawn in the outer frame's space.
        let viewport = Rectangle { x: bounds.x - outer.translation.x, y: bounds.y - outer.translation.y, ..bounds };
        let visible = match outer.visible {
            Some(v) => v.intersection(&viewport),
            None => Some(viewport),
        };
        self.entering = Some(Frame {
            translation: outer.translation + translation,
            // Nothing visible at all: an empty rectangle no point is in.
            visible: Some(visible.unwrap_or(Rectangle::new(Point::ORIGIN, iced::Size::ZERO))),
        });
    }

    fn finish(&self) -> Outcome<Option<T>> {
        Outcome::Some(self.found.as_ref().map(|(_, target)| target.clone()))
    }
}

/// Where dropped files came from.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DropFrom {
    /// Dragged out of this window.
    Inside(Vec<PathBuf>),
    /// Another application's `text/uri-list`.
    Outside(Vec<PathBuf>),
}

/// The keys held when the drop happened.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Held {
    /// Copy, whatever the filesystems.
    pub ctrl: bool,
    /// Move, whatever the filesystems.
    pub shift: bool,
}

/// What a drop does.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DropPlan {
    /// Nothing to do, and nothing worth saying.
    Nothing,
    /// Paste these, exactly as a paste from the clipboard would — into
    /// this folder or this archive.
    Paste { clip: FileClip, into: PathBuf },
    /// Move these to the Trash.
    Trash(Vec<PathBuf>),
    /// Not done, and why — a sentence for the status bar.
    Refused(String),
}

/// Where a drop lands, as far as deciding what it does needs to know.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Landing<'a> {
    pub path: &'a Path,
    /// Whether `path` goes through an archive — answered by the caller,
    /// because only a `stat` can say (`archive::split`).
    pub in_archive: bool,
}

/// What dropping `from` into `into` does. `trash` is the Trash's own
/// path; `same_filesystem` answers whether a file and a folder are on one
/// filesystem. Both of those, and `into.in_archive`, are I/O, and are
/// the caller's — this decides, and reads nothing.
pub fn plan(
    from: DropFrom,
    into: Landing<'_>,
    trash: &Path,
    held: Held,
    same_filesystem: impl Fn(&Path, &Path) -> bool,
) -> DropPlan {
    let (into, into_archive) = (into.path, into.in_archive);
    let (inside, paths) = match from {
        DropFrom::Inside(paths) => (true, paths),
        DropFrom::Outside(paths) => (false, paths),
    };
    if paths.is_empty() {
        return DropPlan::Nothing;
    }
    // Things already in the Trash come back with Restore, which puts each
    // where it was and removes its record. Dropped anywhere — onto the
    // Trash again, or onto a folder — they would be trashed a second time
    // or moved out from under their record, and the record left behind
    // would name a file that is not there. Found by review.
    if paths.iter().any(|p| p.starts_with(trash)) {
        return if into == trash {
            DropPlan::Nothing
        } else {
            DropPlan::Refused("To take something out of the Trash, use Restore.".to_string())
        };
    }
    if into == trash {
        return DropPlan::Trash(paths);
    }
    // Let go where it started: a drag that changed its mind. Pasting
    // would make a duplicate of every file beside itself — unless Ctrl
    // asked for exactly that.
    if !held.ctrl && paths.iter().all(|p| p.parent() == Some(into)) {
        return DropPlan::Nothing;
    }
    // Into an archive is always a copy: removing the originals because an
    // archive rewrite succeeded is not something the paste does either.
    let verb = if !inside || into_archive || held.ctrl {
        // Another program's files are copied, never moved: a move would
        // delete them out from under the program that offered them.
        ClipVerb::Copy
    } else if held.shift || paths.iter().all(|p| same_filesystem(p, into)) {
        ClipVerb::Cut
    } else {
        ClipVerb::Copy
    };
    DropPlan::Paste { clip: FileClip { paths, verb }, into: into.to_path_buf() }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn p(s: &str) -> PathBuf {
        PathBuf::from(s)
    }

    const TRASH: &str = "/home/a/.local/share/Trash/files";

    fn planned(from: DropFrom, into: &str, held: Held, same_fs: bool) -> DropPlan {
        let into = Landing { path: Path::new(into), in_archive: into.contains(".zip") };
        plan(from, into, Path::new(TRASH), held, |_, _| same_fs)
    }

    fn verb(plan: DropPlan) -> ClipVerb {
        match plan {
            DropPlan::Paste { clip, .. } => clip.verb,
            other => panic!("not a paste: {other:?}"),
        }
    }

    #[test]
    fn a_drop_from_inside_on_one_filesystem_moves() {
        let plan = planned(DropFrom::Inside(vec![p("/home/a/x.txt")]), "/home/a/docs", Held::default(), true);
        assert_eq!(verb(plan), ClipVerb::Cut);
    }

    #[test]
    fn a_drop_from_inside_across_filesystems_copies() {
        let plan = planned(DropFrom::Inside(vec![p("/home/a/x.txt")]), "/mnt/usb", Held::default(), false);
        assert_eq!(verb(plan), ClipVerb::Copy);
    }

    #[test]
    fn ctrl_copies_and_shift_moves_whatever_the_filesystems() {
        let inside = || DropFrom::Inside(vec![p("/home/a/x.txt")]);
        let ctrl = Held { ctrl: true, shift: false };
        let shift = Held { ctrl: false, shift: true };
        assert_eq!(verb(planned(inside(), "/home/a/docs", ctrl, true)), ClipVerb::Copy);
        assert_eq!(verb(planned(inside(), "/mnt/usb", shift, false)), ClipVerb::Cut);
    }

    #[test]
    fn another_programs_files_are_copied_never_moved() {
        let shift = Held { ctrl: false, shift: true };
        let plan = planned(DropFrom::Outside(vec![p("/tmp/shot.png")]), "/home/a", shift, true);
        assert_eq!(verb(plan), ClipVerb::Copy);
    }

    #[test]
    fn letting_go_where_the_drag_started_does_nothing() {
        let plan = planned(DropFrom::Inside(vec![p("/home/a/x.txt")]), "/home/a", Held::default(), true);
        assert_eq!(plan, DropPlan::Nothing);
        let ctrl = Held { ctrl: true, shift: false };
        let plan = planned(DropFrom::Inside(vec![p("/home/a/x.txt")]), "/home/a", ctrl, true);
        assert_eq!(verb(plan), ClipVerb::Copy, "unless Ctrl asks for a duplicate");
    }

    #[test]
    fn a_drop_on_the_trash_trashes() {
        let plan = planned(DropFrom::Inside(vec![p("/home/a/x.txt")]), TRASH, Held::default(), true);
        assert_eq!(plan, DropPlan::Trash(vec![p("/home/a/x.txt")]));
    }

    #[test]
    fn something_already_in_the_trash_is_restored_not_dropped() {
        let trashed = || DropFrom::Inside(vec![PathBuf::from(TRASH).join("old.txt")]);
        assert_eq!(planned(trashed(), TRASH, Held::default(), true), DropPlan::Nothing, "not trashed twice");
        assert!(matches!(planned(trashed(), "/home/a", Held::default(), true), DropPlan::Refused(_)));
    }

    #[test]
    fn a_drop_into_an_archive_copies() {
        let plan = planned(DropFrom::Inside(vec![p("/home/a/x.txt")]), "/home/a/b.zip/docs", Held::default(), true);
        assert_eq!(verb(plan), ClipVerb::Copy);
    }

    // --- the hit test, against the calls iced makes -------------------

    fn targets(paths: &[&str]) -> HashMap<Id, PathBuf> {
        paths.iter().map(|s| (target_id(0, Path::new(s)), p(s))).collect()
    }

    fn rect(x: f32, y: f32, w: f32, h: f32) -> Rectangle {
        Rectangle { x, y, width: w, height: h }
    }

    fn found(op: &HitTest) -> Option<PathBuf> {
        match op.finish() {
            Outcome::Some(found) => found,
            _ => None,
        }
    }

    /// A listing background holding a scrollable of two rows, as the
    /// browser lays it out — rows at content y 0 and 28.
    fn walk(op: &mut HitTest, scrolled: f32) {
        op.container(Some(&target_id(0, Path::new("/here"))), rect(200.0, 50.0, 600.0, 400.0));
        op.traverse(&mut |op| {
            op.scrollable(None, rect(200.0, 80.0, 600.0, 56.0), rect(200.0, 80.0, 600.0, 2000.0), Vector::new(0.0, scrolled), &mut NoScroll);
            op.traverse(&mut |op| {
                op.container(Some(&target_id(0, Path::new("/here/a"))), rect(200.0, 80.0, 600.0, 28.0));
                op.container(Some(&target_id(0, Path::new("/here/b"))), rect(200.0, 108.0, 600.0, 28.0));
            });
        });
    }

    struct NoScroll;
    impl iced::advanced::widget::operation::Scrollable for NoScroll {
        fn snap_to(&mut self, _: iced::advanced::widget::operation::scrollable::RelativeOffset<Option<f32>>) {}
        fn scroll_to(&mut self, _: iced::advanced::widget::operation::scrollable::AbsoluteOffset<Option<f32>>) {}
        fn scroll_by(&mut self, _: iced::advanced::widget::operation::scrollable::AbsoluteOffset, _: Rectangle, _: Rectangle) {}
    }

    #[test]
    fn a_row_inside_the_listing_wins_over_the_listing() {
        let mut op = HitTest::new(Point::new(300.0, 90.0), targets(&["/here", "/here/a", "/here/b"]));
        walk(&mut op, 0.0);
        assert_eq!(found(&op), Some(p("/here/a")));
    }

    #[test]
    fn scrolling_moves_which_row_is_under_the_point() {
        // Scrolled by one row: what is drawn at y=90 is the second row.
        let mut op = HitTest::new(Point::new(300.0, 90.0), targets(&["/here", "/here/a", "/here/b"]));
        walk(&mut op, 28.0);
        assert_eq!(found(&op), Some(p("/here/b")));
    }

    #[test]
    fn a_row_scrolled_out_of_sight_catches_nothing() {
        // Below the scrollable's 56px viewport, where row `b` would be at
        // y=108 if nothing clipped it — the drop belongs to the listing.
        let mut op = HitTest::new(Point::new(300.0, 140.0), targets(&["/here", "/here/a", "/here/b"]));
        walk(&mut op, 0.0);
        assert_eq!(found(&op), Some(p("/here")));
    }

    #[test]
    fn a_point_outside_every_target_finds_nothing() {
        let mut op = HitTest::new(Point::new(10.0, 10.0), targets(&["/here", "/here/a"]));
        walk(&mut op, 0.0);
        assert_eq!(found(&op), None);
    }

    /// Two panes showing the same folder: each draws its own targets, and
    /// the hit says which pane's row the point is over.
    #[test]
    fn the_same_folder_in_two_panes_is_two_targets() {
        assert_ne!(target_id(1, Path::new("/here")), target_id(2, Path::new("/here")));
        let targets: HashMap<Id, (u64, PathBuf)> = [1, 2]
            .into_iter()
            .map(|pane| (target_id(pane, Path::new("/here")), (pane, p("/here"))))
            .collect();
        let mut op = HitTest::new(Point::new(700.0, 90.0), targets);
        op.container(Some(&target_id(1, Path::new("/here"))), rect(0.0, 50.0, 500.0, 400.0));
        op.container(Some(&target_id(2, Path::new("/here"))), rect(500.0, 50.0, 500.0, 400.0));
        let found = match op.finish() {
            Outcome::Some(found) => found,
            _ => None,
        };
        assert_eq!(found, Some((2, p("/here"))), "the right-hand pane's");
    }
}
