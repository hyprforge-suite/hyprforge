//! A vertical stack of fixed-height lines — section headers and the rows
//! under them — and the one set of numbers both drawing and hit-testing
//! read.
//!
//! Both popups used to be a single run of identical rows, so "row `n`
//! starts at `n * stride`" was all the arithmetic there was. A list with
//! headers breaks that: a "Pinned" label and a clipboard row are not the
//! same height, and an emoji grid's "Smileys & emotion" label is not the
//! height of a row of cells. The moment line heights differ, `n * stride`
//! is a second, independent answer to "where is line `n`", and CLAUDE.md
//! is explicit about what two answers to that question do: the highlight
//! lands on one row while the click lands on another.
//!
//! So a [`Stack`] is built once from every line's height, and every
//! question — where does line `n` start, which line is under this `y`,
//! which lines are on screen at this scroll offset, how far must the view
//! move to show line `n` — is answered from the same `tops`. A view that
//! draws each line into a container of exactly [`Stack::height`], with
//! exactly [`Stack::spacing`] between them, is then drawing the same
//! geometry the hit-test measures, because there is only one.
//!
//! Pure arithmetic: no iced, no Wayland — the same discipline as
//! [`crate::scrollbar`], for the same reason.

use std::ops::Range;

/// Lines stacked top to bottom with a fixed gap between each pair (none
/// after the last).
#[derive(Debug, Clone, PartialEq)]
pub struct Stack {
    tops: Vec<f64>,
    heights: Vec<f64>,
    spacing: f64,
}

impl Stack {
    /// Builds the stack from each line's height, in order. A negative or
    /// non-finite height is treated as zero rather than trusted — a line
    /// that claims to be `-5px` tall would make every line after it
    /// overlap the one before.
    pub fn new(heights: impl IntoIterator<Item = f64>, spacing: f64) -> Stack {
        let spacing = if spacing.is_finite() { spacing.max(0.0) } else { 0.0 };
        let heights: Vec<f64> = heights.into_iter().map(|h| if h.is_finite() { h.max(0.0) } else { 0.0 }).collect();
        let mut tops = Vec::with_capacity(heights.len());
        let mut y = 0.0;
        for height in &heights {
            tops.push(y);
            y += height + spacing;
        }
        Stack { tops, heights, spacing }
    }

    pub fn len(&self) -> usize {
        self.heights.len()
    }

    pub fn is_empty(&self) -> bool {
        self.heights.is_empty()
    }

    pub fn spacing(&self) -> f64 {
        self.spacing
    }

    /// Where line `index` starts, measured from the top of the content.
    /// Past the end is the content's own bottom, so a caller asking about
    /// a line that just stopped existing gets a sane answer rather than a
    /// panic.
    pub fn top(&self, index: usize) -> f64 {
        self.tops.get(index).copied().unwrap_or_else(|| self.content_height())
    }

    pub fn height(&self, index: usize) -> f64 {
        self.heights.get(index).copied().unwrap_or(0.0)
    }

    /// Every line stacked, with the gaps between them but none trailing
    /// the last — the "content height" [`crate::clamp_offset`] and
    /// [`crate::Scrollbar`] measure against.
    pub fn content_height(&self) -> f64 {
        match (self.tops.last(), self.heights.last()) {
            (Some(top), Some(height)) => top + height,
            _ => 0.0,
        }
    }

    /// The line under content-space `y`, or `None` above the first line,
    /// in the gap between two lines, or past the last. A gap is not
    /// rounded to whichever line is nearer: a click that selected a
    /// neighbour at random would be worse than one that did nothing.
    pub fn line_at(&self, y: f64) -> Option<usize> {
        if !y.is_finite() || y < 0.0 {
            return None;
        }
        // The last line whose top is at or above `y`.
        let index = self.tops.partition_point(|&top| top <= y).checked_sub(1)?;
        (y <= self.tops[index] + self.heights[index]).then_some(index)
    }

    /// The lines at least partly inside a viewport `viewport` tall
    /// scrolled `offset` down — exactly the lines worth building widgets
    /// for. The view draws this range starting at [`Stack::top`] of its
    /// first line, shifted up by `offset - top(first)`.
    pub fn visible(&self, offset: f64, viewport: f64) -> Range<usize> {
        if self.is_empty() {
            return 0..0;
        }
        let offset = offset.max(0.0);
        let start = self.tops.partition_point(|&top| top <= offset).saturating_sub(1);
        // A line ending in the gap above `offset` is not visible.
        let start = if self.tops[start] + self.heights[start] < offset { start + 1 } else { start };
        let bottom = offset + viewport.max(0.0);
        let end = self.tops.partition_point(|&top| top < bottom).max(start + 1).min(self.len());
        start.min(self.len())..end
    }

    /// The offset that shows lines `first..=last` whole, moving from
    /// `offset` as little as possible — the stack's own version of
    /// [`crate::scroll_into_view`], taking a *range* so a caller can ask
    /// for a row together with the header above it.
    pub fn reveal(&self, first: usize, last: usize, offset: f64, viewport: f64) -> f64 {
        if self.is_empty() {
            return 0.0;
        }
        let last = last.min(self.len() - 1);
        let first = first.min(last);
        let item_top = self.top(first);
        let item_bottom = self.top(last) + self.height(last);
        let moved = crate::scroll_into_view(offset, item_top, item_bottom, viewport);
        crate::clamp_offset(moved, self.content_height(), viewport)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A header 26 tall, then three rows 34 tall, 2 apart.
    fn sample() -> Stack {
        Stack::new([26.0, 34.0, 34.0, 34.0], 2.0)
    }

    #[test]
    fn lines_of_different_heights_start_where_the_previous_one_ended_plus_the_gap() {
        let stack = sample();
        assert_eq!(stack.top(0), 0.0);
        assert_eq!(stack.top(1), 28.0);
        assert_eq!(stack.top(2), 64.0);
        assert_eq!(stack.top(3), 100.0);
        assert_eq!(stack.content_height(), 134.0, "no gap after the last line");
    }

    #[test]
    fn a_point_resolves_to_the_line_drawn_there_and_a_gap_to_nothing() {
        let stack = sample();
        assert_eq!(stack.line_at(0.0), Some(0));
        assert_eq!(stack.line_at(26.0), Some(0), "a line's own bottom edge is still that line");
        assert_eq!(stack.line_at(27.0), None, "the gap between two lines hits neither");
        assert_eq!(stack.line_at(28.0), Some(1));
        assert_eq!(stack.line_at(133.9), Some(3));
        assert_eq!(stack.line_at(134.5), None);
        assert_eq!(stack.line_at(-1.0), None);
    }

    /// The property the whole module exists for: for every line, the
    /// point in the middle of where it is drawn hits that line.
    #[test]
    fn every_line_is_hit_where_it_is_drawn() {
        let stack = Stack::new([26.0, 34.0, 26.0, 34.0, 34.0, 12.0], 2.0);
        for index in 0..stack.len() {
            let middle = stack.top(index) + stack.height(index) / 2.0;
            assert_eq!(stack.line_at(middle), Some(index));
        }
    }

    #[test]
    fn the_visible_range_is_every_line_that_overlaps_the_viewport() {
        let stack = sample();
        assert_eq!(stack.visible(0.0, 50.0), 0..2);
        // Scrolled so the header is gone and row 1 is cut off at the top.
        assert_eq!(stack.visible(40.0, 50.0), 1..3);
        // Scrolled into the gap after the header: the header is not visible.
        assert_eq!(stack.visible(27.0, 10.0), 1..2);
        assert_eq!(stack.visible(0.0, 1000.0), 0..4);
    }

    #[test]
    fn an_empty_stack_has_nothing_visible_and_no_height() {
        let stack = Stack::new([], 2.0);
        assert_eq!(stack.visible(0.0, 100.0), 0..0);
        assert_eq!(stack.content_height(), 0.0);
        assert_eq!(stack.line_at(0.0), None);
        assert_eq!(stack.reveal(0, 0, 50.0, 100.0), 0.0);
    }

    #[test]
    fn revealing_a_row_below_the_viewport_scrolls_just_far_enough() {
        let stack = sample();
        let offset = stack.reveal(3, 3, 0.0, 100.0);
        assert_eq!(offset, 34.0, "row 3 ends at 134, so the view must start at 34");
    }

    #[test]
    fn revealing_a_row_together_with_its_header_brings_the_header_into_view_too() {
        let stack = sample();
        assert_eq!(stack.reveal(0, 1, 60.0, 100.0), 0.0);
    }

    #[test]
    fn revealing_something_already_visible_does_not_move_the_view() {
        let stack = sample();
        assert_eq!(stack.reveal(2, 2, 30.0, 100.0), 30.0);
    }

    #[test]
    fn a_hostile_height_cannot_make_lines_overlap() {
        let stack = Stack::new([10.0, -50.0, f64::NAN, 10.0], 2.0);
        for pair in 0..stack.len() - 1 {
            assert!(stack.top(pair + 1) >= stack.top(pair));
        }
    }
}
