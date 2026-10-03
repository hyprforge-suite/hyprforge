//! Keeping the keyboard's row on screen.
//!
//! The browser knows *which* row the keyboard is on and nothing about
//! pixels — it does no layout, and the rows' height depends on the font
//! scale, the grid's column count on the window, the list's divider on a
//! setting. The layout knows all of it. So the view tags the one focused
//! row with [`focused_row`], and [`Reveal`] — a widget operation the host
//! runs — finds that row and the scrollable it sits in, and scrolls only
//! as far as it takes to bring the row fully into sight: nothing at all
//! when it already is, so walking down a screenful of rows never moves
//! the list under you.
//!
//! One id for the focused row, not one per row: tagging every row would
//! allocate an id per row per frame to find one of them.

use iced::advanced::widget::operation::{self, Operation, Outcome};
use iced::advanced::widget::Id;
use iced::{Rectangle, Vector};

/// The id the view gives the row the keyboard is on.
pub fn focused_row() -> Id {
    Id::from("hyprforge-focused-row")
}

/// How far to scroll a viewport `height` tall, now at `offset`, so the
/// span `top..bottom` of its content is in sight — `None` when it is.
///
/// Up to its top when it is above, down to its bottom when it is below;
/// a row taller than the viewport shows its top.
pub fn offset_for(top: f32, bottom: f32, offset: f32, height: f32) -> Option<f32> {
    if top < offset {
        Some(top)
    } else if bottom > offset + height {
        Some((bottom - height).min(top))
    } else {
        None
    }
}

/// Scrolls whichever scrollable holds the [`focused_row`] so the row is
/// in sight. See the module doc.
pub struct Reveal {
    /// Per level being walked: the scrollable that level is inside.
    frames: Vec<Option<Viewport>>,
    /// Set by `scrollable`, consumed by the `traverse` that follows it.
    entering: Option<Viewport>,
    /// The scrollable the row was found in, and where it wants to be.
    found: Option<(Id, f32)>,
}

#[derive(Clone)]
struct Viewport {
    id: Option<Id>,
    bounds: Rectangle,
    content: Rectangle,
    translation: Vector,
}

impl Reveal {
    pub fn new() -> Reveal {
        Reveal { frames: vec![None], entering: None, found: None }
    }
}

impl Default for Reveal {
    fn default() -> Reveal {
        Reveal::new()
    }
}

impl<T: 'static> Operation<T> for Reveal {
    fn traverse(&mut self, operate: &mut dyn FnMut(&mut dyn Operation<T>)) {
        let next = match self.entering.take() {
            Some(viewport) => Some(viewport),
            None => self.frames.last().cloned().flatten(),
        };
        self.frames.push(next);
        operate(self);
        self.frames.pop();
    }

    fn container(&mut self, id: Option<&Id>, bounds: Rectangle) {
        if id != Some(&focused_row()) || self.found.is_some() {
            return;
        }
        // The innermost scrollable, and only one with an id: an unnamed
        // one cannot be told to scroll.
        let Some(Some(viewport)) = self.frames.last() else { return };
        let Some(scroller) = viewport.id.clone() else { return };
        // A scrollable lays its content out unmoved and draws it shifted,
        // so the row's bounds are already in content coordinates.
        let top = bounds.y - viewport.content.y;
        let bottom = top + bounds.height;
        if let Some(offset) = offset_for(top, bottom, viewport.translation.y, viewport.bounds.height) {
            self.found = Some((scroller, offset));
        }
    }

    fn scrollable(
        &mut self,
        id: Option<&Id>,
        bounds: Rectangle,
        content_bounds: Rectangle,
        translation: Vector,
        _state: &mut dyn operation::Scrollable,
    ) {
        self.entering = Some(Viewport { id: id.cloned(), bounds, content: content_bounds, translation });
    }

    fn finish(&self) -> Outcome<T> {
        match &self.found {
            Some((id, y)) => Outcome::Chain(Box::new(operation::scrollable::scroll_to(
                id.clone(),
                operation::scrollable::AbsoluteOffset { x: None, y: Some(*y) },
            ))),
            None => Outcome::None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_row_already_in_sight_does_not_move_the_list() {
        assert_eq!(offset_for(100.0, 128.0, 0.0, 400.0), None);
        assert_eq!(offset_for(372.0, 400.0, 0.0, 400.0), None, "touching the bottom edge is in sight");
    }

    #[test]
    fn a_row_below_scrolls_just_far_enough_to_sit_at_the_bottom() {
        assert_eq!(offset_for(400.0, 428.0, 0.0, 400.0), Some(28.0));
    }

    #[test]
    fn a_row_above_scrolls_up_to_its_top() {
        assert_eq!(offset_for(56.0, 84.0, 200.0, 400.0), Some(56.0));
    }

    #[test]
    fn a_row_taller_than_the_view_shows_its_top() {
        assert_eq!(offset_for(500.0, 1500.0, 0.0, 400.0), Some(500.0));
    }

    fn rect(x: f32, y: f32, w: f32, h: f32) -> Rectangle {
        Rectangle { x, y, width: w, height: h }
    }

    struct NoScroll;
    impl operation::Scrollable for NoScroll {
        fn snap_to(&mut self, _: operation::scrollable::RelativeOffset<Option<f32>>) {}
        fn scroll_to(&mut self, _: operation::scrollable::AbsoluteOffset<Option<f32>>) {}
        fn scroll_by(&mut self, _: operation::scrollable::AbsoluteOffset, _: Rectangle, _: Rectangle) {}
    }

    /// A listing laid out the way iced reports it: a 400-tall scrollable
    /// at y=80 whose content starts there too, the focused row at
    /// content row `row` of 28px each.
    fn walk(op: &mut Reveal, id: Option<Id>, row: f32, scrolled: f32) {
        Operation::<()>::traverse(op, &mut |op| {
            op.scrollable(id.as_ref(), rect(0.0, 80.0, 600.0, 400.0), rect(0.0, 80.0, 600.0, 5000.0), Vector::new(0.0, scrolled), &mut NoScroll);
            op.traverse(&mut |op| {
                op.container(None, rect(0.0, 80.0, 600.0, 28.0));
                op.container(Some(&focused_row()), rect(0.0, 80.0 + row * 28.0, 600.0, 28.0));
            });
        });
    }

    fn wants(op: &Reveal) -> Option<f32> {
        op.found.as_ref().map(|(_, y)| *y)
    }

    #[test]
    fn the_operation_finds_the_row_in_its_scrollable_and_asks_for_the_least_scroll() {
        let mut op = Reveal::new();
        walk(&mut op, Some(Id::from("list")), 20.0, 0.0);
        // Row 20 spans 560..588 of the content; a 400 view shows it
        // with its bottom at the bottom.
        assert_eq!(wants(&op), Some(188.0));
        assert!(matches!(Operation::<()>::finish(&op), Outcome::Chain(_)));
    }

    #[test]
    fn a_row_on_screen_asks_for_nothing() {
        let mut op = Reveal::new();
        walk(&mut op, Some(Id::from("list")), 3.0, 0.0);
        assert_eq!(wants(&op), None);
        assert!(matches!(Operation::<()>::finish(&op), Outcome::None));
    }

    #[test]
    fn scrolled_past_the_row_brings_it_back_from_above() {
        let mut op = Reveal::new();
        walk(&mut op, Some(Id::from("list")), 2.0, 300.0);
        assert_eq!(wants(&op), Some(56.0));
    }

    #[test]
    fn a_scrollable_with_no_id_cannot_be_scrolled_and_is_left_alone() {
        let mut op = Reveal::new();
        walk(&mut op, None, 20.0, 0.0);
        assert_eq!(wants(&op), None);
    }
}
