//! What the grid shows and where the selection is — plain data and pure
//! functions over it, the same split `hyprforge-clipmenu::model` uses and
//! CLAUDE.md asks every D-Bus-backed module to use for the same reason:
//! everything here is testable with no compositor, no daemon, and no
//! renderer.
//!
//! # Scrolling, adapted from rows to a grid
//!
//! [`Model::sync_scroll`] is the grid version of exactly the same piece
//! in `hyprforge-clipmenu::model` — moved by the *minimum* amount needed
//! to keep the selection on screen, never recentred, for the identical
//! reason that crate's own doc gives: recentring on every hover would
//! make "the cell under the pointer is already visible" scroll the grid
//! out from under the cursor. The clamping and scroll-into-view
//! arithmetic itself lives in `hyprforge_popup::scrollbar`
//! (`clamp_offset`, `scroll_into_view`) rather than being reimplemented
//! here a second time — the same shared pixel math
//! `hyprforge-clipmenu::model` now uses. The one adaptation: a clipboard
//! history scrolls in units of one row per entry, but this grid's
//! selection is a single linear index into `filtered`, and what actually
//! scrolls is *rows* of cells (`columns` cells at a time) — so every
//! offset computation here converts the linear `selected` index to a row
//! (`selected / columns`) before handing it to the shared row-windowing
//! arithmetic, then converts the resulting row range back to a cell
//! range for [`Model::visible_range`].

use hyprforge_emoji::{Emoji, Tone};

/// The grid's live state: the query, what it matched, where the
/// selection and the visible window are, and whether the tone-variant
/// strip is open.
pub struct Model {
    query: String,
    filtered: Vec<Emoji>,
    /// Linear index into `filtered` — row-major, so
    /// `selected / columns` is the row and `selected % columns` the
    /// column, exactly the order [`crate::geometry::GridLayout::cell_at`]
    /// returns.
    selected: usize,
    /// How tall (in logical pixels) the popup's own scrollable content
    /// area is, below the header — the viewport a continuous
    /// [`Model::scroll_offset`] scrolls within. Set once from
    /// `geometry::GridLayout` via [`Model::set_grid`].
    viewport_height: f64,
    /// One grid row's own stride — a cell's side length plus the spacing
    /// after it (`geometry::GridLayout::cell_size` + `spacing`), needed
    /// here to convert a row index to pixels for this module's own
    /// scroll-into-view arithmetic.
    row_stride: f64,
    /// The gap between two rows — `geometry::GridLayout::spacing` — kept
    /// separate from [`Model::row_stride`] because content height is
    /// `rows * row_stride - spacing` (no trailing gap after the last
    /// row), the same accounting `geometry::GridLayout::rows_that_fit`
    /// and `columns` already use.
    spacing: f64,
    /// How far, in pixels, the visible window has scrolled down into the
    /// grid's own stacked rows — **state**, not a value derived fresh
    /// from `selected` on every call, for the same reason
    /// `hyprforge-clipmenu::model`'s own `scroll_offset` field doc gives:
    /// a hover only ever targets a cell already built into the visible
    /// window, so recomputing this fresh from `selected` on every call
    /// would make "the cell already under the pointer is already
    /// visible" a re-centre instead of a no-op.
    scroll_offset: f64,
    columns: usize,
    /// The user's chosen default tone, `None` for neutral — what
    /// [`Model::display_char`] shows for every tone-capable entry, and
    /// what a tone pick through the overlay updates (see
    /// `crate::popup_app`'s `Action::ChooseTone`).
    tone: Option<Tone>,
    /// `Some(cursor)` while the tone-variant strip is open, `cursor`
    /// being which of the five cells (0..5, in
    /// [`hyprforge_emoji::TONES`] order) is currently highlighted. The
    /// strip is anchored at whichever cell was selected when it opened —
    /// see [`Model::open_tone_overlay`] — but nothing here needs to
    /// remember *which* cell that was beyond `selected` already being
    /// it: opening the overlay never itself moves the grid selection.
    tone_overlay: Option<usize>,
    paste_target: Option<String>,
}

/// [`Model::viewport_height`]/[`Model::row_stride`]'s values until
/// [`Model::set_grid`] is called — big enough that ordinary filtering and
/// selection tests never have to think about scrolling at all;
/// `main.rs` always overrides these with the popup's real, theme-derived
/// geometry before showing anything.
const DEFAULT_VIEWPORT_HEIGHT: f64 = 1000.0;
const DEFAULT_ROW_STRIDE: f64 = 44.0;
const DEFAULT_SPACING: f64 = 4.0;

impl Model {
    /// `tone` is the picker's persisted default, read once at startup —
    /// see `crate::config`.
    pub fn new(tone: Option<Tone>) -> Model {
        Model {
            query: String::new(),
            filtered: hyprforge_emoji::search("", hyprforge_emoji::all()),
            selected: 0,
            viewport_height: DEFAULT_VIEWPORT_HEIGHT,
            row_stride: DEFAULT_ROW_STRIDE,
            spacing: DEFAULT_SPACING,
            scroll_offset: 0.0,
            columns: 1,
            tone,
            tone_overlay: None,
            paste_target: None,
        }
    }

    pub fn set_paste_target(&mut self, target: Option<String>) {
        self.paste_target = target;
    }

    pub fn paste_target(&self) -> Option<&str> {
        self.paste_target.as_deref()
    }

    /// Sets the grid's shape — how many columns wide the popup's own
    /// fixed size has room for, how tall its scrollable content area is,
    /// and one row's own stride and spacing — all four read from
    /// `crate::geometry::GridLayout`, the same "derive it, never
    /// hardcode it" discipline `hyprforge-clipmenu::main`'s own
    /// regression test pins for its row viewport (a window that could
    /// silently disagree with what actually fits).
    pub fn set_grid(&mut self, columns: usize, viewport_height: f64, row_stride: f64, spacing: f64) {
        self.columns = columns.max(1);
        self.viewport_height = viewport_height.max(0.0);
        self.row_stride = row_stride.max(1.0);
        self.spacing = spacing.max(0.0);
        self.sync_scroll();
    }

    pub fn columns(&self) -> usize {
        self.columns
    }

    pub fn viewport_height(&self) -> f64 {
        self.viewport_height
    }

    /// One grid row's own stride — the pixel unit
    /// `popup_app::EmojiApp::pointer_scroll` moves the view by per wheel
    /// notch, mirroring `hyprforge-clipmenu::model::Model::row_stride`.
    pub fn row_stride(&self) -> f64 {
        self.row_stride
    }

    fn total_rows(&self) -> usize {
        total_rows(self.filtered.len(), self.columns)
    }

    /// The total height, in pixels, of every filtered row of cells
    /// stacked with its own spacing — the "content height" half of the
    /// scrollbar and offset-clamping arithmetic in
    /// [`hyprforge_popup::scrollbar`].
    pub fn content_height(&self) -> f64 {
        let rows = self.total_rows();
        if rows == 0 {
            return 0.0;
        }
        rows as f64 * self.row_stride - self.spacing
    }

    /// How far the view has scrolled, in pixels — what
    /// [`hyprforge_popup::Scrollbar`] positions its thumb from, and what
    /// `view.rs` shifts the rendered rows up by (via
    /// [`Model::scroll_remainder`]).
    pub fn scroll_offset(&self) -> f64 {
        self.scroll_offset
    }

    /// Scrolls by `delta` pixels (negative is up) — the wheel's own
    /// path, and also what a scrollbar-thumb drag applies through (see
    /// `hyprforge-clipmenu::model::Model::scroll_by`'s identical doc for
    /// why a drag uses this rather than a second, absolute-offset
    /// setter). No longer moves the *selection* the way a wheel notch
    /// used to — scrolling moves the view.
    pub fn scroll_by(&mut self, delta: f64) {
        self.scroll_offset = hyprforge_popup::clamp_offset(self.scroll_offset + delta, self.content_height(), self.viewport_height);
    }

    /// The row index of the first row actually built into a widget right
    /// now — the single source both this module's own
    /// [`Model::visible_range`] and `view.rs`'s rendering read.
    fn first_visible_row(&self) -> usize {
        if self.row_stride <= 0.0 {
            return 0;
        }
        (self.scroll_offset / self.row_stride).floor().max(0.0) as usize
    }

    /// How many pixels of the first built row are already scrolled past
    /// — the amount `view.rs` shifts the rendered rows up by so a
    /// partially-visible row at the top reads as partially visible
    /// rather than snapping to a row boundary.
    /// [`crate::geometry::GridLayout::cell_at`] reads the exact same
    /// number before hit-testing, which is what keeps drawn and
    /// hit-tested cells from disagreeing under a scroll offset.
    pub fn scroll_remainder(&self) -> f64 {
        self.scroll_offset - self.first_visible_row() as f64 * self.row_stride
    }

    pub fn filter_text(&self) -> &str {
        &self.query
    }

    pub fn filtered(&self) -> &[Emoji] {
        &self.filtered
    }

    /// The character to show (and, if chosen, to copy) for `entry` —
    /// the user's default tone if one is set and `entry` supports tones,
    /// its own plain glyph otherwise. The one place this crate ever
    /// calls [`Emoji::tone`], so `view::view` and `Model::chosen_text`
    /// can never disagree about which glyph a cell is currently showing.
    pub fn display_char(&self, entry: &Emoji) -> &'static str {
        match self.tone {
            Some(tone) => entry.tone(tone),
            None => entry.emoji,
        }
    }

    /// Appends `c` to the filter and re-runs the search — any typing
    /// also dismisses the tone-variant strip if one was open, the same
    /// way `hyprforge-clipmenu`'s Escape-then-close and F2-pin actions
    /// each clear whatever transient state does not apply to the new
    /// action (see that crate's `Model::pin_notice` doc).
    pub fn type_char(&mut self, c: char) {
        self.tone_overlay = None;
        self.query.push(c);
        self.refilter();
    }

    /// Removes the last *character*, never a byte — an emoji query can
    /// itself contain one (pasting an emoji into the search field to
    /// find its siblings is a reasonable thing to try), and `String::pop`
    /// already operates on whole `char`s, so this never risks slicing
    /// through the middle of a multi-byte one. CLAUDE.md's rule about
    /// never slicing an emoji mid-sequence is about *content* — the
    /// grid's `&'static str`s from `hyprforge_emoji` — but the discipline
    /// is the same discipline, applied here to the query a person typed.
    pub fn backspace(&mut self) {
        self.tone_overlay = None;
        self.query.pop();
        self.refilter();
    }

    /// Clears the filter outright — the first press of Escape, per
    /// `crate::popup_app::dispatch_key`'s two-stage rule.
    pub fn clear_filter(&mut self) {
        self.tone_overlay = None;
        self.query.clear();
        self.refilter();
    }

    fn refilter(&mut self) {
        self.filtered = hyprforge_emoji::search(&self.query, hyprforge_emoji::all());
        self.selected = 0;
        self.scroll_offset = 0.0;
        self.sync_scroll();
    }

    pub fn selected_index(&self) -> usize {
        self.selected
    }

    pub fn selected_entry(&self) -> Option<Emoji> {
        self.filtered.get(self.selected).copied()
    }

    /// Sets the selection to exactly this cell — a hover, or a click
    /// that already resolved to a cell via
    /// [`crate::geometry::GridLayout::cell_at`]. Clamped rather than
    /// ignored out of range, the same reasoning
    /// `hyprforge-clipmenu::model::Model::select`'s own doc gives: a hit
    /// racing a list that just got shorter should still select something
    /// sane.
    pub fn select(&mut self, index: usize) {
        if self.filtered.is_empty() {
            self.selected = 0;
            return;
        }
        self.selected = index.min(self.filtered.len() - 1);
        self.sync_scroll();
    }

    /// Moves the linear selection by `delta` cells (negative is back),
    /// clamped to the ends of the filtered list rather than wrapping —
    /// the general form [`Self::move_left`]/[`Self::move_right`]/
    /// [`Self::move_up`]/[`Self::move_down`] are all expressed in terms
    /// of, and what `crate::popup_app::Action::Move` (the wheel, and the
    /// arrow keys alike) dispatches through directly.
    pub fn move_by(&mut self, delta: i32) {
        let len = self.filtered.len();
        if len == 0 {
            self.selected = 0;
            return;
        }
        let current = self.selected.min(len - 1) as i32;
        self.selected = (current + delta).clamp(0, len as i32 - 1) as usize;
        self.sync_scroll();
    }

    /// Left/Right move the linear selection by one — along a row, the
    /// same as a clipboard list's Up/Down move by one entry.
    pub fn move_left(&mut self) {
        self.move_by(-1);
    }

    pub fn move_right(&mut self) {
        self.move_by(1);
    }

    /// Up/Down move by a whole row (`columns` cells) — the one real
    /// difference from a one-dimensional list's Up/Down, per the task's
    /// own framing. Clamped to the list's own ends rather than wrapping,
    /// same as every other direction here: landing past the last row
    /// lands on the last cell that exists, not on nothing.
    pub fn move_up(&mut self) {
        self.move_by(-(self.columns as i32));
    }

    pub fn move_down(&mut self) {
        self.move_by(self.columns as i32);
    }

    /// Moves [`Model::scroll_offset`] the minimum amount needed to bring
    /// the current selection's own row back inside the viewport — see
    /// [`hyprforge_popup::scroll_into_view`]'s own doc for the rule.
    /// Called after anything that can move `selected` or shrink/grow the
    /// filtered list out from under it.
    fn sync_scroll(&mut self) {
        if self.filtered.is_empty() {
            self.scroll_offset = 0.0;
            return;
        }
        let selected_row = self.selected / self.columns;
        let item_top = selected_row as f64 * self.row_stride;
        let item_bottom = item_top + self.row_stride - self.spacing;
        let offset = hyprforge_popup::scroll_into_view(self.scroll_offset, item_top, item_bottom, self.viewport_height);
        self.scroll_offset = hyprforge_popup::clamp_offset(offset, self.content_height(), self.viewport_height);
    }

    /// The range of `filtered` worth building cells for right now — every
    /// complete visible row's worth of cells, from
    /// [`Model::first_visible_row`] through enough further rows to cover
    /// the viewport (plus a buffer for a partially-visible row at each
    /// edge under the worst-case [`Model::scroll_remainder`]), clamped to
    /// the filtered list's own length so a ragged last row never reads
    /// past the end. `view.rs` builds exactly this range and shifts it up
    /// by `scroll_remainder` — the same range this module's own doc says
    /// must never drift from what is drawn.
    pub fn visible_range(&self) -> std::ops::Range<usize> {
        let len = self.filtered.len();
        if len == 0 {
            return 0..0;
        }
        let total = total_rows(len, self.columns);
        let start_row = self.first_visible_row().min(total.saturating_sub(1));
        let rows_needed = if self.row_stride <= 0.0 { 1 } else { (self.viewport_height / self.row_stride).ceil() as usize + 2 };
        let end_row = (start_row + rows_needed.max(1)).min(total);
        let start = start_row * self.columns;
        let end = (end_row * self.columns).min(len);
        start..end
    }

    // --- The tone-variant strip.

    /// `Some(cursor)` while the strip is open.
    pub fn tone_overlay(&self) -> Option<usize> {
        self.tone_overlay
    }

    /// Opens the strip for the currently selected cell, if — and only
    /// if — it actually has tone variants; a long-press (or Tab) over a
    /// plain emoji is a no-op, matching
    /// `hyprforge_popup::PopupApp::pointer_long_press`'s contract that
    /// "nothing changed" is a legitimate answer. The cursor starts on
    /// whichever tone is already the default (or the middle one,
    /// [`hyprforge_emoji::Tone::Medium`], if none is set yet) — starting
    /// on the tone already in use reads as "here's what you have,
    /// change it if you like" rather than resetting the highlight to one
    /// end every time.
    ///
    /// Returns whether anything changed, for
    /// [`hyprforge_popup::PopupApp::pointer_long_press`]'s own return
    /// value.
    pub fn open_tone_overlay(&mut self) -> bool {
        let Some(entry) = self.selected_entry() else { return false };
        if !entry.supports_tones() {
            return false;
        }
        self.tone_overlay = Some(tone_index(self.tone.unwrap_or(hyprforge_emoji::Tone::Medium)));
        true
    }

    pub fn close_tone_overlay(&mut self) {
        self.tone_overlay = None;
    }

    /// Moves the strip's own highlighted cell left/right — a no-op if
    /// the strip is not open.
    pub fn move_tone_cursor(&mut self, delta: i32) {
        let Some(cursor) = self.tone_overlay else { return };
        self.tone_overlay = Some((cursor as i32 + delta).clamp(0, 4) as usize);
    }

    /// Sets the strip's cursor directly — a hover over one of its five
    /// cells.
    pub fn hover_tone_cursor(&mut self, index: usize) {
        if self.tone_overlay.is_some() {
            self.tone_overlay = Some(index.min(4));
        }
    }

    /// The tone the strip's cursor currently highlights, if the strip is
    /// open.
    pub fn tone_cursor_value(&self) -> Option<Tone> {
        self.tone_overlay.map(|cursor| hyprforge_emoji::TONES[cursor.min(4)])
    }

    /// Sets the persisted default tone — a pick made through the strip
    /// (see `crate::popup_app::Action::ChooseTone`) updates it, per the
    /// task's own framing that picking a tone chooses it as the new
    /// default for next time, the same convention Windows and macOS use.
    pub fn set_default_tone(&mut self, tone: Tone) {
        self.tone = Some(tone);
    }
}

fn total_rows(len: usize, columns: usize) -> usize {
    if len == 0 || columns == 0 {
        return 0;
    }
    len.div_ceil(columns)
}

fn tone_index(tone: Tone) -> usize {
    hyprforge_emoji::TONES.iter().position(|&t| t == tone).unwrap_or(2)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Matches [`DEFAULT_ROW_STRIDE`]/[`DEFAULT_SPACING`] so a test's own
    /// arithmetic (a scroll amount, a row's own pixel span) lines up with
    /// what `model_with_grid` actually configures.
    const STRIDE: f64 = DEFAULT_ROW_STRIDE;
    const SPACING: f64 = DEFAULT_SPACING;

    /// `rows` is how many whole rows the viewport is sized to show
    /// exactly — the pixel equivalent of the old row-count `rows_window`,
    /// kept as the same two-argument shape every non-scrolling test here
    /// already calls.
    fn model_with_grid(columns: usize, rows: usize) -> Model {
        let mut model = Model::new(None);
        let viewport_height = rows as f64 * STRIDE - SPACING;
        model.set_grid(columns, viewport_height, STRIDE, SPACING);
        model
    }

    // --- filtering

    #[test]
    fn an_empty_query_shows_everything() {
        let model = model_with_grid(6, 5);
        assert_eq!(model.filtered().len(), hyprforge_emoji::EMOJI_COUNT);
    }

    #[test]
    fn typing_filters_and_resets_the_selection() {
        let mut model = model_with_grid(6, 5);
        model.select(3);
        for c in "fire".chars() {
            model.type_char(c);
        }
        assert!(model.filtered().iter().any(|e| e.name == "fire"));
        assert_eq!(model.selected_index(), 0, "a new search resets the selection");
    }

    #[test]
    fn backspace_never_panics_on_a_multi_byte_character_in_the_query() {
        let mut model = model_with_grid(6, 5);
        model.type_char('🔥');
        model.backspace();
        assert_eq!(model.filter_text(), "");
    }

    #[test]
    fn clear_filter_empties_the_query_and_shows_everything_again() {
        let mut model = model_with_grid(6, 5);
        model.type_char('f');
        model.type_char('i');
        model.clear_filter();
        assert_eq!(model.filter_text(), "");
        assert_eq!(model.filtered().len(), hyprforge_emoji::EMOJI_COUNT);
    }

    #[test]
    fn typing_closes_an_open_tone_overlay() {
        let mut model = model_with_grid(6, 5);
        while !model.selected_entry().unwrap().supports_tones() {
            model.move_right();
        }
        assert!(model.open_tone_overlay());
        model.type_char('a');
        assert_eq!(model.tone_overlay(), None);
    }

    // --- 2D movement — the one real difference from a list.

    #[test]
    fn left_and_right_move_one_cell_along_a_row() {
        let mut model = model_with_grid(4, 5);
        model.move_right();
        assert_eq!(model.selected_index(), 1);
        model.move_right();
        assert_eq!(model.selected_index(), 2);
        model.move_left();
        assert_eq!(model.selected_index(), 1);
    }

    #[test]
    fn up_and_down_move_by_a_whole_row() {
        let mut model = model_with_grid(4, 5);
        model.select(1); // row 0, col 1
        model.move_down();
        assert_eq!(model.selected_index(), 5, "row 1, same column");
        model.move_down();
        assert_eq!(model.selected_index(), 9);
        model.move_up();
        assert_eq!(model.selected_index(), 5);
    }

    #[test]
    fn movement_clamps_at_the_ends_rather_than_wrapping() {
        let mut model = model_with_grid(4, 5);
        model.move_left();
        assert_eq!(model.selected_index(), 0, "left of the first cell stays put");
        model.move_up();
        assert_eq!(model.selected_index(), 0, "up from the first row stays put");

        let last = model.filtered().len() - 1;
        model.select(last);
        model.move_right();
        assert_eq!(model.selected_index(), last, "right of the last cell stays put");
        model.move_down();
        assert_eq!(model.selected_index(), last, "down from the last row stays put");
    }

    // --- scrolling — the grid analogue of `hyprforge-clipmenu`'s own
    // auto-scroll regression tests.

    #[test]
    fn hovering_an_already_visible_cell_does_not_scroll() {
        let mut model = model_with_grid(4, 3); // 3 rows visible
        model.select(9); // row 2, still inside the initial viewport
        let before = model.scroll_offset();
        model.select(1); // hover row 0 — already visible
        assert_eq!(model.scroll_offset(), before, "a hover inside the viewport must not move it");
    }

    #[test]
    fn moving_past_the_bottom_of_the_viewport_scrolls_by_exactly_one_rows_worth_of_pixels() {
        let mut model = model_with_grid(4, 2); // viewport shows exactly 2 rows
        model.select(7); // row 1, the viewport's own last visible row
        assert_eq!(model.scroll_offset(), 0.0);
        model.move_down(); // row 2 — one row past the viewport
        assert_eq!(model.scroll_offset(), STRIDE, "must scroll down by exactly one row's stride");
    }

    #[test]
    fn moving_above_the_top_of_the_viewport_scrolls_up_to_meet_it() {
        let mut model = model_with_grid(4, 2);
        model.select(9); // scrolls down first
        assert!(model.scroll_offset() > 0.0);
        model.select(0);
        assert_eq!(model.scroll_offset(), 0.0, "must scroll all the way back to the top");
    }

    #[test]
    fn the_visible_range_never_runs_past_the_filtered_lists_own_length() {
        let mut model = model_with_grid(4, 5); // room for far more than exists after a narrow filter
        for c in "waving hand".chars() {
            model.type_char(c);
        }
        let len = model.filtered().len();
        assert!(len < 4 * 5, "the filtered list must be small enough to exercise the clamp");
        model.select(len.saturating_sub(1));
        assert!(model.visible_range().end <= len);
    }

    // --- The rest of the pixel-offset arithmetic `Model` itself owns —
    // the grid analogue of `hyprforge-clipmenu::model`'s own tests for
    // the same properties.

    #[test]
    fn set_grid_changes_how_many_cells_are_built() {
        let mut model = Model::new(None);
        model.set_grid(4, 50.0, STRIDE, SPACING);
        let small = model.visible_range().len();
        model.set_grid(4, 500.0, STRIDE, SPACING);
        let big = model.visible_range().len();
        assert!(big > small, "a taller viewport must build more cells ({small} vs {big})");
    }

    #[test]
    fn a_zero_height_viewport_still_builds_at_least_one_row_of_cells() {
        let mut model = Model::new(None);
        model.set_grid(4, 0.0, STRIDE, SPACING);
        assert!(!model.visible_range().is_empty());
    }

    #[test]
    fn scrolling_up_past_the_top_clamps_at_zero() {
        let mut model = model_with_grid(4, 3);
        model.scroll_by(-500.0);
        assert_eq!(model.scroll_offset(), 0.0);
    }

    #[test]
    fn scrolling_down_clamps_at_the_point_the_last_row_reaches_the_bottom() {
        let mut model = model_with_grid(4, 3);
        model.scroll_by(1_000_000.0);
        assert_eq!(model.scroll_offset(), model.content_height() - model.viewport_height());
    }

    #[test]
    fn a_short_filtered_list_that_already_fits_cannot_be_scrolled_at_all() {
        let mut model = model_with_grid(4, 20); // a viewport tall enough for everything
        for c in "fire".chars() {
            model.type_char(c);
        }
        model.scroll_by(500.0);
        assert_eq!(model.scroll_offset(), 0.0, "content shorter than the viewport must not scroll at all");
    }

    #[test]
    fn scroll_remainder_is_how_far_the_offset_sits_into_the_first_built_row() {
        let mut model = model_with_grid(4, 3);
        model.scroll_by(50.0); // one whole stride (44) plus 6px into the next row
        assert_eq!(model.scroll_remainder(), 6.0);
    }

    // --- tone display and the default-tone overlay.

    #[test]
    fn with_no_default_tone_the_grid_shows_the_plain_neutral_glyph() {
        let model = Model::new(None);
        let waving_hand = model.filtered().iter().find(|e| e.name == "waving hand").unwrap();
        assert_eq!(model.display_char(waving_hand), waving_hand.emoji);
    }

    #[test]
    fn a_default_tone_is_shown_for_every_tone_capable_entry() {
        let model = Model::new(Some(Tone::Dark));
        let waving_hand = model.filtered().iter().find(|e| e.name == "waving hand").unwrap();
        assert_eq!(model.display_char(waving_hand), waving_hand.tone(Tone::Dark));
    }

    #[test]
    fn a_default_tone_does_not_affect_an_entry_with_no_tone_variants() {
        let model = Model::new(Some(Tone::Dark));
        let fire = model.filtered().iter().find(|e| e.name == "fire").unwrap();
        assert_eq!(model.display_char(fire), fire.emoji);
    }

    #[test]
    fn opening_the_tone_overlay_on_a_tone_incapable_entry_does_nothing() {
        let mut model = model_with_grid(6, 5);
        while model.selected_entry().unwrap().supports_tones() {
            model.move_right();
        }
        assert!(!model.open_tone_overlay());
        assert_eq!(model.tone_overlay(), None);
    }

    #[test]
    fn opening_the_tone_overlay_on_a_tone_capable_entry_opens_it() {
        let mut model = model_with_grid(6, 5);
        while !model.selected_entry().unwrap().supports_tones() {
            model.move_right();
        }
        assert!(model.open_tone_overlay());
        assert!(model.tone_overlay().is_some());
    }

    #[test]
    fn the_overlay_starts_on_the_currently_saved_default_tone() {
        let mut model = Model::new(Some(Tone::Light));
        model.set_grid(6, 5.0 * STRIDE - SPACING, STRIDE, SPACING);
        while !model.selected_entry().unwrap().supports_tones() {
            model.move_right();
        }
        model.open_tone_overlay();
        assert_eq!(model.tone_cursor_value(), Some(Tone::Light));
    }

    #[test]
    fn with_no_default_tone_the_overlay_starts_on_medium() {
        let mut model = model_with_grid(6, 5);
        while !model.selected_entry().unwrap().supports_tones() {
            model.move_right();
        }
        model.open_tone_overlay();
        assert_eq!(model.tone_cursor_value(), Some(Tone::Medium));
    }

    #[test]
    fn the_tone_cursor_moves_left_and_right_and_clamps_at_the_ends() {
        let mut model = model_with_grid(6, 5);
        while !model.selected_entry().unwrap().supports_tones() {
            model.move_right();
        }
        model.open_tone_overlay();
        model.move_tone_cursor(-10);
        assert_eq!(model.tone_cursor_value(), Some(Tone::Light));
        model.move_tone_cursor(10);
        assert_eq!(model.tone_cursor_value(), Some(Tone::Dark));
    }

    #[test]
    fn moving_the_tone_cursor_while_the_overlay_is_closed_does_nothing() {
        let mut model = model_with_grid(6, 5);
        model.move_tone_cursor(1);
        assert_eq!(model.tone_overlay(), None);
    }

    #[test]
    fn setting_the_default_tone_changes_what_the_grid_displays() {
        let mut model = model_with_grid(6, 5);
        model.set_default_tone(Tone::MediumLight);
        let waving_hand = model.filtered().iter().find(|e| e.name == "waving hand").unwrap();
        assert_eq!(model.display_char(waving_hand), waving_hand.tone(Tone::MediumLight));
    }
}
