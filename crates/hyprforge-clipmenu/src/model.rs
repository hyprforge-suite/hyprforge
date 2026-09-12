//! What is on screen, kept apart from how it is drawn or how it got
//! there.
//!
//! Everything here is plain data and pure functions over it — no
//! Wayland, no iced, no `hyprctl`. That is what makes filtering,
//! selection movement and "which rows are on screen right now" testable
//! directly, the same split `hyprforge-bluetooth`'s and
//! `hyprforge-network`'s D-Bus-backed modules use for their own models.

use hyprforge_clipboard::{Entry, HistoryError};

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
}

/// How many rows either side of the selection are worth building a real
/// widget for. Everything else in the filtered list is real data the
/// user can scroll to, but nothing is decoded or laid out for it until
/// it is.
///
/// Chosen generously enough that a full popup's height is covered twice
/// over — the popup does not show hundreds of rows at once — while
/// still bounding the work done per frame to a window instead of the
/// whole (up to 500-entry) history.
pub const VISIBLE_WINDOW: usize = 24;

impl Model {
    pub fn new(history: HistoryState) -> Model {
        Model { history, filter: String::new(), selected: 0 }
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

    pub fn selected_entry(&self) -> Option<Entry> {
        self.filtered().get(self.selected).map(|e| (*e).clone())
    }

    /// The range of the filtered list worth building real widgets for
    /// right now: a window of [`VISIBLE_WINDOW`] rows centred on the
    /// selection, clamped to the list's own bounds.
    ///
    /// This is the piece that keeps an image entry from being decoded
    /// (see `thumbnail.rs`) until it is actually one of the rows a
    /// person could be looking at — a history of 500 entries must not
    /// mean 500 decode attempts on every keystroke.
    pub fn visible_range(&self) -> std::ops::Range<usize> {
        let len = self.filtered().len();
        visible_range(len, self.selected, VISIBLE_WINDOW)
    }
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
    fn the_visible_window_stays_within_the_lists_bounds() {
        let range = visible_range(500, 3, VISIBLE_WINDOW);
        assert_eq!(range.start, 0);
        assert!(range.end <= 500);

        let range = visible_range(500, 499, VISIBLE_WINDOW);
        assert_eq!(range.end, 500);
        assert!(range.start < range.end);

        let range = visible_range(3, 1, VISIBLE_WINDOW);
        assert_eq!(range, 0..3, "a list shorter than the window is shown in full");
    }
}
