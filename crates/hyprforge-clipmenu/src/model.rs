//! What is on screen, kept apart from how it is drawn or how it got
//! there.
//!
//! Everything here is plain data and pure functions over it — no
//! Wayland, no iced, no `hyprctl`. That is what makes filtering,
//! selection movement and "which rows are on screen right now" testable
//! directly, the same split `hyprforge-bluetooth`'s and
//! `hyprforge-network`'s D-Bus-backed modules use for their own models.

use hyprforge_clipboard::{Entry, EntryId, HistoryError};

/// How the history read from disk turned out.
///
/// Kept distinct from an *empty* history — CLAUDE.md is explicit that
/// "this file could not be read" must never collapse into "there is
/// nothing configured". `hyprforge_clipboard::HistoryError::Unreadable`
/// means the index exists and will not parse; that is worth telling the
/// user about, on a screen that shows nothing else, rather than quietly
/// rendering the same blank list a first run would show.
pub enum HistoryState {
    /// Read cleanly. An empty `Vec` here is the sorted history genuinely
    /// having nothing in it — a still-distinct case the view has to
    /// handle by saying so, not by showing a blank box.
    Loaded(Vec<Entry>),
    /// The index exists and could not be parsed. Carries the message so
    /// the popup can show *why*, not just that something went wrong.
    Unreadable(String),
}

impl HistoryState {
    pub fn from_result(result: Result<hyprforge_clipboard::History, HistoryError>) -> HistoryState {
        match result {
            Ok(history) => HistoryState::Loaded(history.entries().to_vec()),
            Err(err) => HistoryState::Unreadable(err.to_string()),
        }
    }
}

/// The popup's state: what it knows, what the user has typed, and which
/// row is selected.
pub struct Model {
    history: HistoryState,
    filter: String,
    /// Index into the *filtered* list, not the full history. Always
    /// within bounds of a non-empty filtered list; meaningless (and
    /// never read) when the filtered list is empty.
    selected: usize,
    /// The message from the most recent pin/unpin attempt that did not
    /// succeed — shown in the header in place of the filter text until
    /// the next action of any kind (see `surface::dispatch_action`).
    /// Never carries clipboard content, only an id (a hash) and the
    /// daemon's own reason, or a note that there was nobody to ask.
    ///
    /// Deliberately not "sticky": CLAUDE.md is explicit that a failed
    /// connection must never be cached as a reason to stop trying, and
    /// a notice that lingered after the user had moved on would start to
    /// look exactly like that — this field is cleared the moment
    /// anything else happens, not just on the next successful pin.
    pin_notice: Option<String>,
    /// How many rows either side of the selection are worth building a
    /// real widget for. Everything else in the filtered list is real
    /// data the user can scroll to, but nothing is decoded or laid out
    /// for it until it is.
    ///
    /// This used to be a hardcoded 24 regardless of the popup's own
    /// height: at a 44px row inside a 420px popup that laid out 1104px
    /// of rows in a surface a quarter that tall, so the drawn rows could
    /// not possibly be where a pointer's hit-test computed them — the
    /// popup's own doc on `geometry::RowLayout::rows_that_fit` tells the
    /// rest of that story. The fix is that this crate has exactly one
    /// number for "how many rows", derived once in `main.rs` from
    /// `RowLayout::rows_that_fit(POPUP_HEIGHT)` and threaded in here via
    /// [`Model::set_window`] — not read back from the popup's actual
    /// size, and not reinvented as a second constant, either of which
    /// could drift from what `rows_that_fit` says fits.
    window: usize,
}

/// [`Model::window`]'s value until [`Model::set_window`] is called.
/// Every test in this crate that does not care about window sizing
/// leaves it at this default; `main.rs` always overrides it with the
/// popup's real row count before showing anything.
const DEFAULT_WINDOW: usize = 24;

impl Model {
    pub fn new(history: HistoryState) -> Model {
        Model { history, filter: String::new(), selected: 0, pin_notice: None, window: DEFAULT_WINDOW }
    }

    /// Sets how many rows [`Model::visible_range`] builds around the
    /// selection — see [`Model::window`]'s own doc for why this is a
    /// field set after construction rather than a constant. Clamped to
    /// at least one: a window of zero would make `visible_range` unable
    /// to show anything even when the filtered list is non-empty, which
    /// is a worse failure than showing one row more than a degenerate
    /// height technically fit.
    pub fn set_window(&mut self, window: usize) {
        self.window = window.max(1);
    }

    /// The entries matching the current filter, newest-first /
    /// pinned-first exactly as the store ordered them — filtering only
    /// removes rows, it never reorders what is left.
    ///
    /// Matching is case-insensitive substring matching against the same
    /// one-line preview the row renders, so what the user sees is what
    /// they can search for. An empty filter matches everything, which is
    /// also the state the popup opens in.
    pub fn filtered(&self) -> Vec<&Entry> {
        let HistoryState::Loaded(entries) = &self.history else {
            return Vec::new();
        };
        if self.filter.is_empty() {
            return entries.iter().collect();
        }
        let needle = self.filter.to_lowercase();
        entries
            .iter()
            .filter(|e| e.content.preview(usize::MAX).to_lowercase().contains(&needle))
            .collect()
    }

    pub fn history(&self) -> &HistoryState {
        &self.history
    }

    pub fn filter_text(&self) -> &str {
        &self.filter
    }

    /// Appends to the filter, as a keystroke does. Resets the selection
    /// to the top of the new (generally shorter, and always different)
    /// filtered list — keeping an index into a list that just changed
    /// underneath it would either select the wrong row or nothing.
    pub fn type_char(&mut self, c: char) {
        self.filter.push(c);
        self.selected = 0;
    }

    pub fn backspace(&mut self) {
        self.filter.pop();
        self.selected = 0;
    }

    /// Moves the selection by `delta` rows (negative is up), clamped to
    /// the ends of the filtered list rather than wrapping — the
    /// behaviour the task asks for by name, and the one a Windows user
    /// actually expects: Up at the top does nothing, it does not jump to
    /// the bottom.
    pub fn move_selection(&mut self, delta: i32) {
        let len = self.filtered().len();
        if len == 0 {
            self.selected = 0;
            return;
        }
        let current = self.selected.min(len - 1) as i32;
        self.selected = (current + delta).clamp(0, len as i32 - 1) as usize;
    }

    pub fn selected_index(&self) -> usize {
        self.selected
    }

    /// Sets the selection to exactly this row of the filtered list —
    /// what the pointer entering or moving over a row means, as opposed
    /// to [`Model::move_selection`]'s relative Up/Down. Out-of-range is
    /// clamped rather than ignored: a hit-test racing a list that just
    /// got shorter (the user typed a filter character between the
    /// pointer motion and this call landing) should still select
    /// *something* sane rather than silently do nothing.
    pub fn select(&mut self, index: usize) {
        let len = self.filtered().len();
        if len == 0 {
            self.selected = 0;
            return;
        }
        self.selected = index.min(len - 1);
    }

    pub fn selected_entry(&self) -> Option<Entry> {
        self.filtered().get(self.selected).map(|e| (*e).clone())
    }

    /// The range of the filtered list worth building real widgets for
    /// right now: a window of [`Model::window`] rows centred on the
    /// selection, clamped to the list's own bounds.
    ///
    /// This is the piece that keeps an image entry from being decoded
    /// (see `thumbnail.rs`) until it is actually one of the rows a
    /// person could be looking at — a history of 500 entries must not
    /// mean 500 decode attempts on every keystroke.
    pub fn visible_range(&self) -> std::ops::Range<usize> {
        let len = self.filtered().len();
        visible_range(len, self.selected, self.window)
    }

    /// Reflects a pin/unpin the daemon has already confirmed, in this
    /// popup's own copy of the history — this never writes anything to
    /// disk and never talks to the daemon itself; that already happened
    /// by the time `surface::dispatch_action` calls this. A no-op if
    /// `id` is not in the current history (it was removed or the popup
    /// is showing stale state some other way) rather than a panic.
    ///
    /// Re-sorts with exactly the same rule
    /// `hyprforge_clipboard::store::History::sort` uses — pinned first,
    /// then newest first, ties broken by id — so the row lands where a
    /// fresh load would put it next time the popup opens, instead of
    /// drifting from what the daemon's own file now says. Then keeps
    /// the *same entry* selected across the reorder: the id, not the
    /// index, is what the user cared about, and pinning something is
    /// supposed to move it, not lose the selection.
    pub fn set_entry_pinned(&mut self, id: &EntryId, pinned: bool) {
        let HistoryState::Loaded(entries) = &mut self.history else { return };
        let Some(entry) = entries.iter_mut().find(|e| &e.id == id) else { return };
        entry.pinned = pinned;
        sort_entries(entries);
        if let Some(new_index) = self.filtered().iter().position(|e| &e.id == id) {
            self.selected = new_index;
        }
    }

    /// The reason the last pin/unpin attempt did not succeed, if any —
    /// see the field's own doc for why this is cleared so aggressively.
    pub fn pin_notice(&self) -> Option<&str> {
        self.pin_notice.as_deref()
    }

    pub fn set_pin_notice(&mut self, notice: Option<String>) {
        self.pin_notice = notice;
    }
}

/// The same ordering `History::sort` applies on the daemon side, kept as
/// a free function so `set_entry_pinned` can call it without punching a
/// hole in `HistoryState` for the sort itself. Duplicated rather than
/// shared because there is nothing to share *from*: `History::sort` is
/// a private method on a type this crate does not construct (it only
/// ever reads a `Vec<Entry>` out of one) — sharing it would mean making
/// it public API of `hyprforge-clipboard` for exactly one caller outside
/// the daemon.
fn sort_entries(entries: &mut [Entry]) {
    entries.sort_by(|a, b| {
        b.pinned
            .cmp(&a.pinned)
            .then(b.copied_at.cmp(&a.copied_at))
            .then(a.id.cmp(&b.id))
    });
}

/// The windowing rule above, pulled out so it can be pinned without a
/// `Model` around it.
fn visible_range(len: usize, selected: usize, window: usize) -> std::ops::Range<usize> {
    if len == 0 {
        return 0..0;
    }
    let selected = selected.min(len - 1);
    let half = window / 2;
    let start = selected.saturating_sub(half);
    let end = (start + window).min(len);
    // If `end` hit the list's own end before using the whole window,
    // pull `start` back down so the window is still full-sized where
    // there is enough list to fill it — otherwise scrolling to the
    // bottom would shrink the window instead of just sliding it.
    let start = end.saturating_sub(window).min(start);
    start..end
}

#[cfg(test)]
mod tests {
    use super::*;
    use hyprforge_clipboard::{Content, EntryId};

    fn text_entry(s: &str) -> Entry {
        let content = Content::Text(s.to_string());
        Entry { id: EntryId::of(&content), content, copied_at: 0, pinned: false }
    }

    fn model_with(texts: &[&str]) -> Model {
        Model::new(HistoryState::Loaded(texts.iter().map(|s| text_entry(s)).collect()))
    }

    #[test]
    fn an_empty_filter_shows_everything() {
        let model = model_with(&["alpha", "beta", "gamma"]);
        assert_eq!(model.filtered().len(), 3);
    }

    #[test]
    fn typing_filters_to_what_was_typed() {
        let mut model = model_with(&["alpha", "beta", "gamma"]);
        for c in "et".chars() {
            model.type_char(c);
        }
        let filtered = model.filtered();
        assert_eq!(filtered.len(), 1);
        assert_eq!(filtered[0].content, Content::Text("beta".into()));
    }

    #[test]
    fn filtering_is_case_insensitive() {
        let mut model = model_with(&["Alpha", "beta"]);
        model.type_char('A');
        model.type_char('L');
        assert_eq!(model.filtered().len(), 1);
    }

    #[test]
    fn backspace_widens_the_filter_back_out() {
        let mut model = model_with(&["alpha", "beta"]);
        model.type_char('z'); // matches nothing
        assert!(model.filtered().is_empty());
        model.backspace();
        assert_eq!(model.filtered().len(), 2);
    }

    #[test]
    fn moving_down_advances_the_selection() {
        let mut model = model_with(&["a", "b", "c"]);
        model.move_selection(1);
        assert_eq!(model.selected_index(), 1);
    }

    #[test]
    fn the_selection_stops_at_the_bottom_rather_than_wrapping() {
        let mut model = model_with(&["a", "b"]);
        model.move_selection(1);
        model.move_selection(1);
        model.move_selection(1);
        assert_eq!(model.selected_index(), 1, "must stop at the last row, not wrap to 0");
    }

    #[test]
    fn the_selection_stops_at_the_top_rather_than_wrapping() {
        let mut model = model_with(&["a", "b"]);
        model.move_selection(-1);
        model.move_selection(-1);
        assert_eq!(model.selected_index(), 0, "must stop at the first row, not wrap to the end");
    }

    #[test]
    fn selecting_a_row_directly_lands_on_that_row() {
        let mut model = model_with(&["a", "b", "c"]);
        model.select(2);
        assert_eq!(model.selected_index(), 2);
    }

    #[test]
    fn selecting_past_the_end_of_the_list_clamps_to_the_last_row() {
        let mut model = model_with(&["a", "b"]);
        model.select(50);
        assert_eq!(model.selected_index(), 1);
    }

    #[test]
    fn choosing_hands_back_the_selected_entry() {
        let mut model = model_with(&["a", "b", "c"]);
        model.move_selection(1);
        assert_eq!(model.selected_entry().unwrap().content, Content::Text("b".into()));
    }

    #[test]
    fn an_empty_history_shows_a_message_rather_than_a_blank_list() {
        let model = Model::new(HistoryState::Loaded(Vec::new()));
        assert!(model.filtered().is_empty());
        assert!(matches!(model.history(), HistoryState::Loaded(entries) if entries.is_empty()));
    }

    #[test]
    fn an_unreadable_history_is_distinct_from_an_empty_one() {
        let model = Model::new(HistoryState::Unreadable("bad toml".into()));
        assert!(model.filtered().is_empty());
        assert!(matches!(model.history(), HistoryState::Unreadable(_)));
    }

    #[test]
    fn pinning_an_entry_moves_it_above_unpinned_ones() {
        let mut model = model_with(&["a", "b", "c"]);
        let b_id = model.filtered()[1].id.clone();
        model.set_entry_pinned(&b_id, true);
        assert_eq!(model.filtered()[0].id, b_id, "the pinned entry must sort first");
    }

    #[test]
    fn unpinning_lets_an_entry_fall_back_out_of_the_pinned_group() {
        let mut model = model_with(&["a", "b", "c"]);
        let b_id = model.filtered()[1].id.clone();
        model.set_entry_pinned(&b_id, true);
        model.set_entry_pinned(&b_id, false);
        assert!(
            model.filtered().iter().all(|e| !e.pinned),
            "nothing should still be pinned"
        );
    }

    #[test]
    fn pinning_the_selected_entry_keeps_it_selected_after_it_moves() {
        let mut model = model_with(&["a", "b", "c"]);
        model.select(2); // "c"
        let c_id = model.filtered()[2].id.clone();
        model.set_entry_pinned(&c_id, true);
        assert_eq!(
            model.filtered()[model.selected_index()].id,
            c_id,
            "the same entry must still be selected even though its row moved"
        );
    }

    #[test]
    fn pinning_an_id_that_is_not_in_the_history_does_nothing() {
        let mut model = model_with(&["a"]);
        let bogus = EntryId::from_raw("not-a-real-id".to_string());
        model.set_entry_pinned(&bogus, true);
        assert!(model.filtered().iter().all(|e| !e.pinned));
    }

    #[test]
    fn a_fresh_model_has_no_pin_notice() {
        let model = model_with(&["a"]);
        assert_eq!(model.pin_notice(), None);
    }

    #[test]
    fn a_pin_notice_can_be_set_and_cleared() {
        let mut model = model_with(&["a"]);
        model.set_pin_notice(Some("hyprforge-clipd isn't running".to_string()));
        assert_eq!(model.pin_notice(), Some("hyprforge-clipd isn't running"));
        model.set_pin_notice(None);
        assert_eq!(model.pin_notice(), None);
    }

    #[test]
    fn the_visible_window_stays_within_the_lists_bounds() {
        let range = visible_range(500, 3, DEFAULT_WINDOW);
        assert_eq!(range.start, 0);
        assert!(range.end <= 500);

        let range = visible_range(500, 499, DEFAULT_WINDOW);
        assert_eq!(range.end, 500);
        assert!(range.start < range.end);

        let range = visible_range(3, 1, DEFAULT_WINDOW);
        assert_eq!(range, 0..3, "a list shorter than the window is shown in full");
    }

    /// The property `main.rs` depends on: telling the model a new window
    /// size actually changes how many rows `visible_range` builds,
    /// rather than the field being write-only.
    #[test]
    fn set_window_changes_how_many_rows_are_built() {
        let mut model = model_with(&["a", "b", "c", "d", "e"]);
        model.set_window(2);
        assert_eq!(model.visible_range().len(), 2);
        model.set_window(4);
        assert_eq!(model.visible_range().len(), 4);
    }

    /// A window of zero would leave `visible_range` unable to show
    /// anything even for a non-empty list — `set_window` floors it at
    /// one rather than letting a degenerate popup height (or a future
    /// caller's bug) do that.
    #[test]
    fn set_window_floors_at_one_rather_than_showing_nothing() {
        let mut model = model_with(&["a", "b"]);
        model.set_window(0);
        assert_eq!(model.visible_range().len(), 1);
    }
}
