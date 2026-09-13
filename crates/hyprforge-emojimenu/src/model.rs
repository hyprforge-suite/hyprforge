//! What the grid shows and where the selection is — plain data and pure
//! functions over it, the same split `hyprforge-clipmenu::model` uses and
//! CLAUDE.md asks every D-Bus-backed module to use for the same reason:
//! everything here is testable with no compositor, no daemon, and no
//! renderer.
//!
//! # Scrolling, adapted from rows to a grid
//!
//! [`Model::sync_window`], [`clamp_window`] and [`scroll_start`] are the
//! grid version of exactly the same three pieces in
//! `hyprforge-clipmenu::model` — moved by the *minimum* amount needed to
//! keep the selection on screen, never recentred, for the identical
//! reason that crate's own doc gives: recentring on every hover would
//! make "the cell under the pointer is already visible" scroll the grid
//! out from under the cursor. The one adaptation: a clipboard history
//! scrolls in units of one row per entry, but this grid's selection is a
//! single linear index into `filtered`, and what actually scrolls is
//! *rows* of cells (`columns` cells at a time) — so every window
//! computation here converts the linear `selected` index to a row
//! (`selected / columns`) before handing it to the same row-windowing
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
    /// First visible *row* (not cell) — kept as state and only ever
    /// nudged by [`Model::sync_window`], never recomputed fresh from
    /// `selected`, for the reason this module's own doc gives.
    window_start_row: usize,
    columns: usize,
    rows_window: usize,
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

impl Model {
    /// `tone` is the picker's persisted default, read once at startup —
    /// see `crate::config`.
    pub fn new(tone: Option<Tone>) -> Model {
        Model {
            query: String::new(),
            filtered: hyprforge_emoji::search("", hyprforge_emoji::all()),
            selected: 0,
            window_start_row: 0,
            columns: 1,
            rows_window: 1,
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

    /// Sets the grid's shape — how many columns wide and how many rows
    /// tall the popup's own fixed size actually has room for. Read once
    /// from [`crate::geometry::GridLayout`] before the popup opens, the
    /// same way `hyprforge-clipmenu::main` derives `Model::set_window`
    /// from `RowLayout::rows_that_fit` — see that crate's own regression
    /// test for the bug this pattern exists to prevent (a hardcoded
    /// window that could silently disagree with what actually fits).
    pub fn set_grid(&mut self, columns: usize, rows_visible: usize) {
        self.columns = columns.max(1);
        self.rows_window = rows_visible.max(1);
        self.sync_window();
    }

    pub fn columns(&self) -> usize {
        self.columns
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
        self.window_start_row = 0;
        self.sync_window();
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
        self.sync_window();
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
        self.sync_window();
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

    fn total_rows(&self) -> usize {
        total_rows(self.filtered.len(), self.columns)
    }

    fn sync_window(&mut self) {
        let rows = self.total_rows();
        let selected_row = self.selected / self.columns;
        self.window_start_row = scroll_start(rows, self.window_start_row, selected_row, self.rows_window);
    }

    /// The row range worth building real grid widgets for right now —
    /// the row-space version of
    /// `hyprforge-clipmenu::model::Model::visible_range`.
    fn visible_row_range(&self) -> std::ops::Range<usize> {
        clamp_window(self.total_rows(), self.window_start_row, self.rows_window)
    }

    /// The range of `filtered` worth building cells for — every complete
    /// visible row's worth of cells, clamped to the list's own length so
    /// a ragged last row never reads past the end.
    pub fn visible_range(&self) -> std::ops::Range<usize> {
        let rows = self.visible_row_range();
        let start = rows.start * self.columns;
        let end = (rows.end * self.columns).min(self.filtered.len());
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

/// Identical to `hyprforge-clipmenu::model`'s free function of the same
/// name, operating on rows of a grid instead of rows of a list — see
/// that module's own doc for the reasoning; duplicated rather than
/// shared because there is nothing Hyprland- or clipboard-specific in
/// either copy to share *from*, and each crate's own `Model` is the only
/// caller.
fn clamp_window(len: usize, start: usize, window: usize) -> std::ops::Range<usize> {
    if len == 0 {
        return 0..0;
    }
    let end = (start + window).min(len);
    let start = end.saturating_sub(window);
    start..end
}

/// Identical in spirit to `hyprforge-clipmenu::model::scroll_start` — see
/// that function's own doc for the full reasoning (moving the window by
/// the minimum amount needed rather than recentring on every hover).
fn scroll_start(len: usize, start: usize, selected: usize, window: usize) -> usize {
    if len == 0 {
        return 0;
    }
    let selected = selected.min(len - 1);
    let mut start = start;
    if selected < start {
        start = selected;
    } else if selected >= start + window {
        start = selected + 1 - window;
    }
    clamp_window(len, start, window).start
}

fn tone_index(tone: Tone) -> usize {
    hyprforge_emoji::TONES.iter().position(|&t| t == tone).unwrap_or(2)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn model_with_grid(columns: usize, rows: usize) -> Model {
        let mut model = Model::new(None);
        model.set_grid(columns, rows);
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
        let mut model = model_with_grid(4, 2); // 2 rows visible = 8 cells
        model.select(5); // still inside rows 0..2
        let before = model.visible_range();
        model.select(6);
        assert_eq!(model.visible_range(), before, "a hover inside the window must not move it");
    }

    #[test]
    fn moving_past_the_bottom_of_the_window_scrolls_by_exactly_one_row() {
        let mut model = model_with_grid(4, 2); // rows_window = 2
        model.select(7); // row 1, the window's own last visible row
        assert_eq!(model.visible_range(), 0..8);
        model.move_down(); // row 2 — one past the window
        assert_eq!(model.visible_range(), 4..12, "must scroll down by exactly one row");
    }

    #[test]
    fn moving_above_the_top_of_the_window_scrolls_up_to_meet_it() {
        let mut model = model_with_grid(4, 2);
        model.select(9);
        assert!(model.visible_range().start > 0);
        model.select(0);
        assert_eq!(model.visible_range(), 0..8, "must scroll all the way back to the top");
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
        model.set_grid(6, 5);
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
