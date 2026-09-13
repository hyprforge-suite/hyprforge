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
    /// The index (into the filtered list) of the first row
    /// [`Model::visible_range`] builds — **state**, not a value derived
    /// fresh from `selected` on every call.
    ///
    /// This used to *not* exist: `visible_range` recomputed a window
    /// centred on `selected` every time it was called, which reads fine
    /// for keyboard movement but is exactly wrong for the mouse —
    /// hovering a row calls `Model::select` on it (see
    /// `surface::pointer_move`), and a window that recentres on whatever
    /// was just selected puts a *different* row under a pointer that
    /// never moved, which is indistinguishable from the list scrolling
    /// on its own. Keeping the window's start as state and only ever
    /// nudging it just far enough to keep the selection inside (see
    /// [`scroll_start`]) is what makes "the row already under the
    /// pointer is already visible" a no-op instead of a re-centre.
    window_start: usize,
    /// A short label for where a chosen entry will be pasted — "Ghostty",
    /// say — shown in the header before anything is chosen. Cosmetic
    /// only: nothing here reads this to decide *how* to paste; that
    /// decision (`target::paste_shortcut`) is made once in `main.rs` from
    /// the same focused-window lookup this label came from, and handed to
    /// `ClipMenu::run` separately. `None` when `main.rs` could not work
    /// out what had focus (an empty desktop, or `hyprctl` not answering)
    /// — the header falls back to its plain "Type to filter" text rather
    /// than naming a window that was never found.
    paste_target: Option<String>,
}

/// [`Model::window`]'s value until [`Model::set_window`] is called.
/// Every test in this crate that does not care about window sizing
/// leaves it at this default; `main.rs` always overrides it with the
/// popup's real row count before showing anything.
const DEFAULT_WINDOW: usize = 24;

impl Model {
    pub fn new(history: HistoryState) -> Model {
        Model {
            history,
            filter: String::new(),
            selected: 0,
            pin_notice: None,
            window: DEFAULT_WINDOW,
            window_start: 0,
            paste_target: None,
        }
    }

    /// Sets how many rows [`Model::visible_range`] builds around the
    /// selection — see [`Model::window`]'s own doc for why this is a
    /// field set after construction rather than a constant. Clamped to
    /// at least one: a window of zero would make `visible_range` unable
    /// to show anything even when the filtered list is non-empty, which
    /// is a worse failure than showing one row more than a degenerate
    /// height technically fit.
    ///
    /// Re-syncs [`Model::window_start`] afterward: a window that just
    /// shrank could otherwise leave the selection outside it until the
    /// next unrelated action nudged things back into range.
    pub fn set_window(&mut self, window: usize) {
        self.window = window.max(1);
        self.sync_window();
    }

    /// The label [`Model::set_paste_target`] set, if any — see that
    /// field's own doc.
    pub fn paste_target(&self) -> Option<&str> {
        self.paste_target.as_deref()
    }

    pub fn set_paste_target(&mut self, target: Option<String>) {
        self.paste_target = target;
    }

    /// Re-clamps [`Model::window_start`] so the current selection is
    /// inside it — see [`scroll_start`]'s own doc for the rule. Called
    /// after anything that can move `selected` or shrink/grow the
    /// filtered list out from under it: typing, backspace, an explicit
    /// selection, or a changed window size.
    fn sync_window(&mut self) {
        let len = self.filtered().len();
        self.window_start = scroll_start(len, self.window_start, self.selected, self.window);
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
        self.sync_window();
    }

    pub fn backspace(&mut self) {
        self.filter.pop();
        self.selected = 0;
        self.sync_window();
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
        self.sync_window();
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
        // The whole point of `window_start` being persisted state rather
        // than derived from `selected`: a hover that lands on a row
        // already inside the current window (which every hover does,
        // since a hover only ever targets a row `view.rs` actually drew)
        // leaves the window exactly where it was. See `scroll_start`'s
        // own doc.
        self.sync_window();
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
        clamp_window(len, self.window_start, self.window)
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
            self.sync_window();
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

/// Turns a window's `start` into the `start..end` range actually shown,
/// for a list of length `len` and a window of `window` rows.
///
/// Deliberately ignorant of `selected` — see [`scroll_start`] for the
/// function that decides where `start` itself should be, and this
/// function's own call site (`Model::visible_range`) for why the two are
/// split apart. All this does is keep a given `start` valid: never
/// running past `len`, and — the one adjustment this makes to `start`
/// itself — pulled back down so the window stays full-sized wherever the
/// list is long enough to fill it, rather than shrinking once `start`
/// alone would run it past the end. That shrink-at-the-end behaviour is
/// the one thing carried over unchanged from this crate's original
/// (`selected`-centred) windowing rule.
fn clamp_window(len: usize, start: usize, window: usize) -> std::ops::Range<usize> {
    if len == 0 {
        return 0..0;
    }
    let end = (start + window).min(len);
    let start = end.saturating_sub(window);
    start..end
}

/// Where the window should start so that `selected` (into a list of
/// length `len`) falls inside a window of `window` rows beginning at
/// `start` — moved by the *minimum* amount needed, never recentred.
///
/// This is the fix for the popup's own auto-scrolling bug: the previous
/// rule recomputed `start` fresh from `selected` on every call, centring
/// the window under whatever was selected — which included the pointer
/// merely hovering a row (`Model::select`, called from
/// `surface::pointer_move`), so hovering a row that was already on
/// screen still recentred the list *under the cursor*, landing a
/// different row there and reading as the list scrolling on its own.
/// Keeping `start` as state and only nudging it here — never otherwise —
/// is what makes "the row under the pointer is already visible" a true
/// no-op: `selected` already satisfies `start <= selected < start +
/// window`, so neither branch below fires and `start` is returned
/// unchanged.
///
/// The two branches are keyboard movement's two edges: `selected` above
/// the window pulls `start` down to meet it exactly (a scroll up by
/// exactly the overshoot, one row at a time when `move_selection` is
/// called one row at a time); `selected` at or past the window's far end
/// pushes `start` up to meet it the same way. [`clamp_window`] (called by
/// every reader of the resulting `start`, including this function's own
/// final clamp) is what then keeps the window from running past the
/// list's own end or shrinking there.
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

    // --- `clamp_window`: keeping a given `start` valid, independent of
    // any selection at all.

    #[test]
    fn clamp_window_stays_within_the_lists_bounds() {
        let range = clamp_window(500, 3, DEFAULT_WINDOW);
        assert_eq!(range.start, 3);
        assert!(range.end <= 500);

        let range = clamp_window(500, 499, DEFAULT_WINDOW);
        assert_eq!(range.end, 500);
        assert!(range.start < range.end, "must still be full-sized, not shrunk to one row");

        let range = clamp_window(3, 1, DEFAULT_WINDOW);
        assert_eq!(range, 0..3, "a list shorter than the window is shown in full");
    }

    #[test]
    fn clamp_window_pulls_start_back_so_the_window_stays_full_sized_at_the_end() {
        // `start` alone would run the window past the list's end; the
        // window must still show `window` rows, sliding `start` back
        // rather than shrinking.
        let range = clamp_window(10, 8, 5);
        assert_eq!(range, 5..10);
    }

    // --- `scroll_start`: the fix for the auto-scrolling bug. Moves
    // `start` by the minimum needed to keep `selected` inside the
    // window — never recentres.

    #[test]
    fn scroll_start_leaves_start_untouched_when_selected_is_already_inside_the_window() {
        assert_eq!(scroll_start(10, 2, 3, 4), 2, "selected 3 is already within [2, 6)");
        assert_eq!(scroll_start(10, 2, 2, 4), 2, "selected at the window's own top edge is still inside it");
        assert_eq!(scroll_start(10, 2, 5, 4), 2, "selected at the window's own bottom edge is still inside it");
    }

    #[test]
    fn scroll_start_moves_down_by_exactly_the_overshoot_past_the_bottom_edge() {
        // Window [2, 6); selecting row 6 is one past the edge, so start
        // must move to exactly 3 — a one-row scroll, not a recentre.
        assert_eq!(scroll_start(10, 2, 6, 4), 3);
    }

    #[test]
    fn scroll_start_moves_up_to_meet_a_selection_above_the_window() {
        // Window [4, 8); selecting row 2 is above it, so start must jump
        // exactly to 2, not merely far enough to include it plus slack.
        assert_eq!(scroll_start(10, 4, 2, 4), 2);
    }

    #[test]
    fn scroll_start_never_runs_the_window_past_the_lists_own_end() {
        assert_eq!(scroll_start(10, 0, 9, 4), 6, "window must end exactly at len, not run past it");
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

    // --- The auto-scroll regression, exercised through `Model` itself
    // rather than the bare `scroll_start`/`clamp_window` functions above —
    // these are the properties the owner reported by name.

    /// The bug: hovering a row that is already on screen re-centred the
    /// window under it, since the old rule derived the window fresh from
    /// `selected` on every call. A hover must be a true no-op when the
    /// row it lands on is already visible.
    #[test]
    fn hovering_a_row_that_is_already_visible_does_not_move_the_window() {
        let mut model = model_with(&["a", "b", "c", "d", "e"]);
        model.set_window(3);
        model.move_selection(2); // selected = 2, still inside the initial window [0, 3)
        let before = model.visible_range();
        model.select(1); // hover row 1 — already visible
        assert_eq!(model.visible_range(), before, "hovering an already-visible row must not scroll");
    }

    #[test]
    fn moving_past_the_bottom_edge_scrolls_the_window_by_exactly_one_row() {
        let mut model = model_with(&["a", "b", "c", "d", "e"]);
        model.set_window(3);
        model.move_selection(2); // selected = 2, the window's own last visible row
        assert_eq!(model.visible_range(), 0..3);
        model.move_selection(1); // selected = 3, one past the window
        assert_eq!(model.visible_range(), 1..4, "the window must slide down by exactly one row");
    }

    #[test]
    fn moving_past_the_top_edge_scrolls_the_window_by_exactly_one_row() {
        let mut model = model_with(&["a", "b", "c", "d", "e"]);
        model.set_window(3);
        model.select(4); // jump to the end; window becomes [2, 5)
        assert_eq!(model.visible_range(), 2..5);
        model.move_selection(-2); // selected = 2, the window's own top edge — not past it yet
        assert_eq!(model.visible_range(), 2..5, "the window's own top edge is still inside it");
        model.move_selection(-1); // selected = 1, now past the top edge
        assert_eq!(model.visible_range(), 1..4, "the window must slide up by exactly one row");
    }

    #[test]
    fn changing_the_filter_keeps_the_window_valid_for_the_new_shorter_list() {
        let mut model = model_with(&["aa", "ab", "cc", "dd", "ee"]);
        model.set_window(3);
        model.select(4); // jump to the end of the full 5-item list; window becomes [2, 5)
        assert_eq!(model.visible_range(), 2..5);
        model.type_char('a'); // filters down to ["aa", "ab"] — 2 rows
        assert_eq!(model.filtered().len(), 2);
        assert_eq!(
            model.visible_range(),
            0..2,
            "a filtered-down list shorter than the window must be shown in full, from the start"
        );
    }

    // --- `paste_target`: purely cosmetic label state, never consulted
    // for anything but the header text `view.rs` shows.

    #[test]
    fn a_fresh_model_has_no_paste_target() {
        let model = model_with(&["a"]);
        assert_eq!(model.paste_target(), None);
    }

    #[test]
    fn a_paste_target_can_be_set_and_read_back() {
        let mut model = model_with(&["a"]);
        model.set_paste_target(Some("Ghostty".to_string()));
        assert_eq!(model.paste_target(), Some("Ghostty"));
    }
}
