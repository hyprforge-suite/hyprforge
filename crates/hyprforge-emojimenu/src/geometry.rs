//! The emoji grid's own cell geometry — which cell a pointer position
//! lands on, how many columns fit a popup of a given width, and how many
//! rows fit a popup of a given height.
//!
//! This is the grid analogue of `hyprforge-clipmenu::geometry::RowLayout`,
//! and deliberately not shared with it — `hyprforge-popup::popup`'s
//! module doc explains why a list's row geometry and a grid's cell
//! geometry are not the same shape, and why each `PopupApp` gets its own.
//! The one property this module exists to guarantee, the same one
//! `RowLayout` guarantees for the clipboard popup, is CLAUDE.md's rule
//! that the thing drawn, the thing hit-tested, and the number of things
//! that fit must never be three different numbers: [`GridLayout::columns`]
//! is the single source both [`view`](crate::view) (how many cells per
//! row to draw) and [`GridLayout::cell_at`] (which cell a pointer lands
//! on) read, and [`GridLayout::rows_that_fit`] is the same kind of
//! single source for how many rows [`crate::model::Model`] is allowed to
//! build into its own visible window.
//!
//! Everything here is free of `hyprctl`, Wayland or iced — pure
//! arithmetic over points and sizes, the same discipline `RowLayout`
//! follows for the same reason: the piece most likely to be wrong (an
//! off-by-one at an edge, a hit that lands in the gap between two cells)
//! is exactly the piece a unit test can pin directly.

/// Where the grid's cells are, in the same logical-pixel space
/// `view::view` draws into and [`crate::popup_app::EmojiApp`]'s pointer
/// handling reads positions in.
///
/// Every number here is a value `view.rs` sets explicitly on the widgets
/// it builds — a fixed square cell, a fixed spacing — rather than
/// anything read back from iced's own layout, for the same reason
/// `RowLayout` does this: this crate builds a fresh `UserInterface` every
/// frame and throws it away (see `hyprforge_popup::popup::Popup::draw`),
/// so there is no live layout tree to ask "what's under the cursor" —
/// this module has to already agree with `view.rs` on where everything
/// is, from the same theme-derived numbers, rather than reconstruct it.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct GridLayout {
    /// Padding around the whole popup's content, all four sides — the
    /// same role `RowLayout::padding` plays.
    pub padding: f64,
    /// Height of the header (the search field) plus the gap under it,
    /// before the grid's first row starts — computed the same way
    /// `RowLayout::header_height` is, from the theme's font size alone,
    /// so the header is exactly as tall in this popup as it is in the
    /// clipboard one at the same font size.
    pub header_height: f64,
    /// Side length of one (square) grid cell.
    pub cell_size: f64,
    /// Gap between two cells, horizontally and vertically alike.
    pub spacing: f64,
}

impl GridLayout {
    pub const PADDING: f64 = 10.0;
    pub const HEADER_GAP: f64 = 6.0;
    pub const ROW_PADDING: f64 = 6.0;
    pub const SPACING: f64 = 4.0;

    /// Derives the layout from the theme's font size — the one variable
    /// this module and `view.rs` already agree on, the same way
    /// `RowLayout::for_font_size` does for the clipboard popup.
    ///
    /// A cell needs to be comfortably bigger than a line of text: an
    /// emoji glyph reads smaller at a given font size than a Latin
    /// letter does (most renderers scale a color/COLR glyph to roughly
    /// the font's em-box, but the visual mark inside it is usually
    /// smaller than a letter's own ascender-to-descender height), and
    /// this is also the pointer's click target, which wants to stay
    /// comfortably tappable even at a small theme font size — hence the
    /// `28.0` floor, not just a multiple of the font size.
    pub fn for_font_size(font_size: f32) -> GridLayout {
        let font_size = font_size as f64;
        let line = font_size * 1.2;
        GridLayout {
            padding: Self::PADDING,
            header_height: line + Self::ROW_PADDING * 2.0 + Self::HEADER_GAP,
            // A floor of 40 rather than 28: an emoji is the content, not
            // a decoration beside text, so the cell is sized for the
            // glyph rather than for the theme's line height. At a small
            // UI font the old floor made a grid of postage stamps.
            cell_size: (font_size * 2.8).max(40.0),
            spacing: Self::SPACING,
        }
    }

    /// How many columns fit a popup `width` logical pixels wide.
    ///
    /// The `+ self.spacing` before dividing is the same "the last cell
    /// needs no trailing gap" adjustment `RowLayout::rows_that_fit` makes
    /// for rows: `n` cells need `n` cell widths and only `n - 1` gaps
    /// between them, so adding one spacing's worth of slack up front
    /// before dividing by a full stride is what makes the arithmetic
    /// come out to `n`, not `n - 1`, for a width that fits `n` exactly.
    pub fn columns(&self, width: f64) -> usize {
        let available = width - self.padding * 2.0 + self.spacing;
        let stride = self.cell_size + self.spacing;
        if available <= 0.0 || stride <= 0.0 {
            return 1;
        }
        ((available / stride).floor() as usize).max(1)
    }

    /// How many rows fit a popup `height` logical pixels tall, below the
    /// header — the grid's exact analogue of `RowLayout::rows_that_fit`,
    /// and it exists for the identical reason: rows built and rows
    /// hit-tested have to be the same rows, so both this module and
    /// `crate::model::Model`'s own visible window read this one function
    /// rather than each guessing a count.
    pub fn rows_that_fit(&self, height: f64) -> usize {
        let available = height - self.padding * 2.0 - self.header_height + self.spacing;
        let stride = self.cell_size + self.spacing;
        if available <= 0.0 || stride <= 0.0 {
            return 0;
        }
        ((available / stride).floor() as usize).max(1)
    }

    /// The cell index (row-major: `row * columns + col`) under
    /// `position`, among `visible_cells` currently built — the same
    /// "which of the cells actually on screen" scoping
    /// `RowLayout::row_at` applies with its own `visible_count`.
    ///
    /// `None` covers every way a position is not over a cell: above the
    /// first row (still in the header), in a gap between cells, past the
    /// last column, or past the last cell actually built — all of them
    /// "do nothing", the same as an unrecognised key.
    /// Where the first column starts, horizontally.
    ///
    /// Whole cells rarely divide a popup's width exactly, and the
    /// remainder used to sit entirely on the right — a grid pushed
    /// against its left edge with a ragged gap down the other side.
    /// Splitting it centres the grid.
    ///
    /// **Both the drawing and the hit-test read this.** That is the
    /// whole reason it is a function rather than an `align_x` on a
    /// container: centring the grid visually while `cell_at` still
    /// measured from the padding would put every click half a gap to
    /// the left of the cell under the pointer — the same class of
    /// silent disagreement between what is drawn and what is measured
    /// that this module exists to prevent.
    pub fn left_margin(&self, width: f64, columns: usize) -> f64 {
        let stride = self.cell_size + self.spacing;
        // The last column needs no spacing after it.
        let grid_width = columns as f64 * stride - self.spacing;
        let slack = width - self.padding * 2.0 - grid_width;
        self.padding + (slack.max(0.0) / 2.0)
    }

    pub fn cell_at(
        &self,
        position: (f64, f64),
        width: f64,
        columns: usize,
        visible_cells: usize,
    ) -> Option<usize> {
        if columns == 0 || visible_cells == 0 {
            return None;
        }
        let x = position.0 - self.left_margin(width, columns);
        let y = position.1 - self.padding - self.header_height;
        if x < 0.0 || y < 0.0 {
            return None;
        }
        let stride = self.cell_size + self.spacing;
        if stride <= 0.0 {
            return None;
        }

        let col = (x / stride) as usize;
        if col >= columns {
            return None;
        }
        // Reject a hit in the gap after this column's own cell width —
        // the same dead-zone rule `RowLayout::row_at` applies vertically.
        if x - (col as f64 * stride) > self.cell_size {
            return None;
        }

        let row = (y / stride) as usize;
        if y - (row as f64 * stride) > self.cell_size {
            return None;
        }

        let index = row * columns + col;
        if index >= visible_cells {
            return None;
        }
        Some(index)
    }
}

/// The tone-variant strip a long press (or its keyboard equivalent, Tab
/// — see `crate::popup_app`) opens: five cells, one per
/// [`hyprforge_emoji::Tone`] in [`hyprforge_emoji::TONES`] order.
///
/// Deliberately **not** positioned relative to whichever grid cell
/// opened it — see this module's own doc for why the invariant this
/// crate cares about is "drawn, hit-tested and fit agree", and anchoring
/// the strip to a cell whose position depends on scroll state would make
/// this layout a function of the model's own scroll offset, not just the
/// theme. Instead it is a fixed bar directly under the header, at the
/// same place regardless of which cell opened it — this popup already
/// makes an out-of-flow overlay work as a `Stack` layer over the grid
/// (see `view::view`), so nothing about the grid's own geometry moves or
/// needs to know the strip exists.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ToneStrip {
    pub top: f64,
    pub left: f64,
    pub cell_size: f64,
    pub spacing: f64,
}

impl ToneStrip {
    /// Slightly smaller than a grid cell: this bar sits *over* the
    /// grid's own first row rather than pushing it down (see this
    /// struct's own doc), and a little breathing room around it reads
    /// more like a popover than a second, identical row of the grid
    /// underneath.
    ///
    /// Takes `font_size` (unused today, beyond what `grid` already
    /// derived from it) so the signature has somewhere to grow if a
    /// future tweak ever needs the font size directly, without every
    /// call site changing again.
    pub fn for_font_size(_font_size: f32, grid: &GridLayout) -> ToneStrip {
        ToneStrip {
            top: grid.padding,
            left: grid.padding,
            cell_size: grid.cell_size * 0.9,
            spacing: grid.spacing,
        }
    }

    /// Total width of the five-cell strip, including the gaps between
    /// them but not a trailing one — mirrors `GridLayout::columns`'s own
    /// "no trailing gap" accounting.
    pub fn width(&self) -> f64 {
        5.0 * self.cell_size + 4.0 * self.spacing
    }

    pub fn height(&self) -> f64 {
        self.cell_size
    }

    /// Which of the five tone cells (0 = [`hyprforge_emoji::Tone::Light`]
    /// .. 4 = [`hyprforge_emoji::Tone::Dark`]) a position lands on, or
    /// `None` if it is outside the strip's own rectangle (including the
    /// gaps between cells) — the caller decides what "outside" means
    /// (closing the strip without picking anything, in
    /// `crate::popup_app`).
    pub fn tone_at(&self, position: (f64, f64)) -> Option<usize> {
        let x = position.0 - self.left;
        let y = position.1 - self.top;
        if x < 0.0 || y < 0.0 || y > self.height() {
            return None;
        }
        let stride = self.cell_size + self.spacing;
        let index = (x / stride) as usize;
        if index >= 5 {
            return None;
        }
        if x - (index as f64 * stride) > self.cell_size {
            return None;
        }
        Some(index)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A popup width the tests measure against. The grid is centred in
    /// it, so "where the first column starts" is `left_margin`, not
    /// `padding` — these tests used to assume the two were the same and
    /// caught the change the moment centring landed, which is what they
    /// are for.
    const TEST_WIDTH: f64 = 360.0;

    fn left(layout: &GridLayout) -> f64 {
        layout.left_margin(TEST_WIDTH, 4)
    }

    // --- GridLayout::columns / rows_that_fit: the "how many fit" half of
    // the drawn/hit-tested/fit invariant.

    #[test]
    fn a_wider_popup_fits_more_columns() {
        let layout = GridLayout::for_font_size(15.0);
        assert!(layout.columns(600.0) > layout.columns(300.0));
    }

    #[test]
    fn a_popup_with_no_room_still_reports_at_least_one_column_or_row() {
        let layout = GridLayout::for_font_size(15.0);
        assert_eq!(layout.columns(0.0), 1, "never zero columns, or nothing can ever be drawn");
        assert_eq!(layout.rows_that_fit(0.0), 0, "but zero rows is a legitimate 'nothing fits yet'");
    }

    #[test]
    fn columns_and_rows_describe_a_grid_that_actually_fits_inside_the_popup() {
        let layout = GridLayout::for_font_size(15.0);
        let width = 320.0;
        let height = 420.0;
        let columns = layout.columns(width);
        let rows = layout.rows_that_fit(height);

        let stride = layout.cell_size + layout.spacing;
        let grid_width = columns as f64 * stride - layout.spacing;
        assert!(
            grid_width <= width - layout.padding * 2.0 + 0.001,
            "{columns} columns ({grid_width}px) must fit inside {width}px"
        );
        let grid_height = rows as f64 * stride - layout.spacing;
        assert!(
            grid_height <= height - layout.padding * 2.0 - layout.header_height + 0.001,
            "{rows} rows ({grid_height}px) must fit under the header"
        );

        // And one more of either would not have fit — otherwise this is
        // leaving room on the table, the same "one more would still fit"
        // check `hyprforge-clipmenu`'s own `fit_tests` module runs for
        // `RowLayout::rows_that_fit`.
        let one_more_column_width = (columns + 1) as f64 * stride - layout.spacing;
        assert!(one_more_column_width > width - layout.padding * 2.0);
        let one_more_row_height = (rows + 1) as f64 * stride - layout.spacing;
        assert!(one_more_row_height > height - layout.padding * 2.0 - layout.header_height);
    }

    // --- GridLayout::cell_at: the "which cell" half.

    /// The grid is centred, so the space left over from whole columns
    /// is split rather than pooled on the right — and the hit-test has
    /// to measure from the same edge the drawing starts at, or every
    /// click lands half a gap out.
    #[test]
    fn the_leftover_width_is_split_evenly_and_the_hit_test_agrees() {
        let layout = GridLayout::for_font_size(15.0);
        let columns = layout.columns(TEST_WIDTH);
        let stride = layout.cell_size + layout.spacing;
        let grid_width = columns as f64 * stride - layout.spacing;

        // Measured from the popup's own edges, both sides: the gap to
        // the left of the first column and the gap to the right of the
        // last. Padding is part of both, which is the whole point —
        // what the eye sees is the total gap, not the slack on top of
        // the padding.
        let left_edge = layout.left_margin(TEST_WIDTH, columns);
        let right_gap = TEST_WIDTH - (left_edge + grid_width);
        assert!(
            (left_edge - right_gap).abs() < 0.001,
            "gap left of the grid is {left_edge}, right is {right_gap}"
        );
        assert!(left_edge >= layout.padding, "the grid must never sit inside the padding");

        // Just inside the first cell hits it; just outside does not.
        let top = layout.padding + layout.header_height;
        assert_eq!(layout.cell_at((left_edge + 0.01, top), TEST_WIDTH, columns, 20), Some(0));
        assert_eq!(layout.cell_at((left_edge - 0.01, top), TEST_WIDTH, columns, 20), None);
    }

    #[test]
    fn a_pointer_over_the_header_hits_no_cell() {
        let layout = GridLayout::for_font_size(15.0);
        assert_eq!(layout.cell_at((left(&layout), 0.0), TEST_WIDTH, 4, 20), None);
    }

    #[test]
    fn the_first_cell_starts_right_after_the_header() {
        let layout = GridLayout::for_font_size(15.0);
        let top = layout.padding + layout.header_height;
        assert_eq!(layout.cell_at((left(&layout), top), TEST_WIDTH, 4, 20), Some(0));
        assert_eq!(
            layout.cell_at((left(&layout) + layout.cell_size - 0.01, top + layout.cell_size - 0.01), TEST_WIDTH, 4, 20),
            Some(0),
            "must still be cell 0 right up to its own far edge"
        );
    }

    #[test]
    fn cells_are_found_in_row_major_order() {
        let layout = GridLayout::for_font_size(15.0);
        let top = layout.padding + layout.header_height;
        let stride = layout.cell_size + layout.spacing;
        let columns = 4;
        for row in 0..3usize {
            for col in 0..columns {
                let x = left(&layout) + col as f64 * stride + layout.cell_size / 2.0;
                let y = top + row as f64 * stride + layout.cell_size / 2.0;
                assert_eq!(layout.cell_at((x, y), TEST_WIDTH, columns, 12), Some(row * columns + col));
            }
        }
    }

    #[test]
    fn a_pointer_in_the_gap_between_two_columns_hits_nothing() {
        let layout = GridLayout::for_font_size(15.0);
        let top = layout.padding + layout.header_height;
        let gap_x = left(&layout) + layout.cell_size + layout.spacing / 2.0;
        assert_eq!(layout.cell_at((gap_x, top + layout.cell_size / 2.0), TEST_WIDTH, 4, 20), None);
    }

    #[test]
    fn a_pointer_past_the_last_column_hits_nothing() {
        let layout = GridLayout::for_font_size(15.0);
        let top = layout.padding + layout.header_height;
        let stride = layout.cell_size + layout.spacing;
        let past_last_column = left(&layout) + 4.0 * stride + 1.0;
        assert_eq!(layout.cell_at((past_last_column, top), TEST_WIDTH, 4, 20), None);
    }

    #[test]
    fn a_pointer_past_the_last_built_cell_hits_nothing_even_inside_a_column_that_exists() {
        let layout = GridLayout::for_font_size(15.0);
        let top = layout.padding + layout.header_height;
        // Row 2, column 0 would be index 8 with 4 columns — but only 8
        // cells (indices 0..8) are actually built, so this must be `None`
        // rather than resolving to a cell nothing drew.
        let stride = layout.cell_size + layout.spacing;
        let y = top + 2.0 * stride + layout.cell_size / 2.0;
        assert_eq!(layout.cell_at((left(&layout), y), TEST_WIDTH, 4, 8), None);
    }

    #[test]
    fn zero_columns_or_zero_visible_cells_hits_nothing_rather_than_dividing_by_zero() {
        let layout = GridLayout::for_font_size(15.0);
        let top = layout.padding + layout.header_height;
        assert_eq!(layout.cell_at((left(&layout), top), TEST_WIDTH, 0, 20), None);
        assert_eq!(layout.cell_at((left(&layout), top), TEST_WIDTH, 4, 0), None);
    }

    // --- ToneStrip: the five-cell overlay a long press opens.

    #[test]
    fn the_five_tone_cells_are_found_in_order() {
        let grid = GridLayout::for_font_size(15.0);
        let strip = ToneStrip::for_font_size(15.0, &grid);
        let stride = strip.cell_size + strip.spacing;
        for index in 0..5usize {
            let x = strip.left + index as f64 * stride + strip.cell_size / 2.0;
            let y = strip.top + strip.cell_size / 2.0;
            assert_eq!(strip.tone_at((x, y)), Some(index));
        }
    }

    #[test]
    fn a_point_in_the_gap_between_tone_cells_hits_nothing() {
        let grid = GridLayout::for_font_size(15.0);
        let strip = ToneStrip::for_font_size(15.0, &grid);
        let gap_x = strip.left + strip.cell_size + strip.spacing / 2.0;
        assert_eq!(strip.tone_at((gap_x, strip.top + strip.cell_size / 2.0)), None);
    }

    #[test]
    fn a_point_outside_the_strips_own_rectangle_hits_nothing() {
        let grid = GridLayout::for_font_size(15.0);
        let strip = ToneStrip::for_font_size(15.0, &grid);
        assert_eq!(strip.tone_at((strip.left - 1.0, strip.top)), None);
        assert_eq!(strip.tone_at((strip.left, strip.top - 1.0)), None);
        assert_eq!(strip.tone_at((strip.left, strip.top + strip.height() + 1.0)), None);
        assert_eq!(strip.tone_at((strip.left + strip.width() + 1.0, strip.top)), None);
    }
}
