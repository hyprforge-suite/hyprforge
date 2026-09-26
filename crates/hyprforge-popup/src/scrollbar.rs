//! A scrollbar and the pixel-offset arithmetic behind continuous
//! scrolling — shared by `hyprforge-clipmenu` (rows) and
//! `hyprforge-emojimenu` (grid cells) because none of it cares what the
//! content actually is. A scrollbar only ever needs three numbers —
//! how tall the content is, how tall the viewport showing it is, and
//! where in it the viewport currently sits — to decide a thumb's size
//! and position, and a pointer position plus a drag delta to move it.
//!
//! # Why this, not `iced_widget::scrollable`
//!
//! `hyprforge-popup::popup::Popup` builds one
//! `iced_runtime::user_interface::UserInterface` per frame with
//! `UserInterface::build`/`draw` and never calls `ui.update()` — see
//! that module's own doc. iced's widgets never see an event here, so a
//! `scrollable` would draw a scrollbar that never responds to the wheel
//! or a drag. This crate owns the hit-testing itself instead, the same
//! way it already owns everything else a `PopupApp` hit-tests.
//!
//! # The invariant, twice as hard now
//!
//! Two popups now compute "which row/cell is under the pointer" and
//! "where is the thumb" independently, each against its own geometry
//! (`hyprforge-clipmenu::geometry::Layout`,
//! `hyprforge-emojimenu::geometry::Layout`). What is shared here —
//! [`clamp_offset`], [`scroll_into_view`], and [`Scrollbar`] itself — is
//! exactly the piece that is the same shape in both: pixel arithmetic
//! over content height, viewport height and offset, with no idea what a
//! "row" or a "cell" is. Each consumer's own view and hit-test both read
//! the *same* `Model::scroll_offset()` (or equivalent) before doing
//! their own content-shaped math on top of it — that is what keeps the
//! thing drawn and the thing hit-tested from disagreeing about where the
//! scroll took them, the same discipline each popup's `Layout` already
//! apply to positions before scrolling existed at all.

/// Clamps a content-scroll `offset` so the viewport never shows space
/// above the top of the content or below its bottom — a short list must
/// not be scrollable into empty space.
///
/// `content_height` shorter than `viewport_height` clamps to `0.0`
/// unconditionally (nothing to scroll), which is also what makes a
/// scrollbar hide itself: see [`Scrollbar::is_needed`].
pub fn clamp_offset(offset: f64, content_height: f64, viewport_height: f64) -> f64 {
    let max_offset = (content_height - viewport_height).max(0.0);
    offset.clamp(0.0, max_offset)
}

/// Moves `offset` the *minimum* amount needed to bring the span
/// `[item_top, item_bottom)` inside the visible window
/// `[offset, offset + viewport_height)` — never recentres.
///
/// This is the pixel-offset version of the rule both popups' own
/// `scroll_start` used to implement in row/cell-index space (see
/// `hyprforge-clipmenu::model`'s doc on the auto-scroll bug this
/// protects against): a hover or a click only ever lands on an item
/// already built into the visible window, so calling this from a hover
/// path is always a no-op — it is keyboard movement, which can select an
/// item outside the window, that actually needs either branch to fire.
/// The result still passes through [`clamp_offset`] by the caller, since
/// this alone does not know `content_height`.
pub fn scroll_into_view(offset: f64, item_top: f64, item_bottom: f64, viewport_height: f64) -> f64 {
    if item_top < offset {
        item_top
    } else if item_bottom > offset + viewport_height {
        item_bottom - viewport_height
    } else {
        offset
    }
}

/// A vertical track and thumb, drawn and hit-tested from the same three
/// geometry fields plus whatever content height and offset a caller's
/// own model tracks — the fixed half of the calculation; `content_height`
/// and `offset` are the live half, passed to every method rather than
/// stored, so nothing here can go stale relative to a model that just
/// scrolled.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Scrollbar {
    /// Left edge of the track, in the popup's own logical-pixel space —
    /// the same space a `PointerEvent::position` reports and a view
    /// draws into.
    pub track_x: f64,
    /// Top edge of the track — normally wherever a consumer's own
    /// content area starts (below the header, inside the padding).
    pub track_top: f64,
    /// Height of the track — the viewport height, i.e. how much content
    /// height is visible at once.
    pub track_height: f64,
    /// Thickness of the bar.
    pub width: f64,
}

impl Scrollbar {
    /// A thin bar — wide enough to grab, narrow enough to stay out of
    /// the way of the content it sits beside.
    pub const WIDTH: f64 = 6.0;
    /// A thumb never shrinks below this, however long the content is —
    /// otherwise a very long history's thumb would be a sliver too small
    /// to see or drag reliably.
    pub const MIN_THUMB_HEIGHT: f64 = 24.0;

    pub fn new(track_x: f64, track_top: f64, track_height: f64) -> Scrollbar {
        Scrollbar { track_x, track_top, track_height: track_height.max(0.0), width: Self::WIDTH }
    }

    /// Whether there is anything to scroll at all. A scrollbar that
    /// cannot scroll is noise — the task's own words — so every other
    /// method here is meaningless (and callers should not draw or
    /// hit-test a thumb) when this is `false`.
    pub fn is_needed(&self, content_height: f64) -> bool {
        content_height > self.track_height + 0.5
    }

    /// The furthest `offset` can go before the last of the content
    /// reaches the bottom of the viewport — the same bound
    /// [`clamp_offset`] enforces, exposed here too since the thumb's own
    /// position is a fraction of it.
    pub fn max_offset(&self, content_height: f64) -> f64 {
        (content_height - self.track_height).max(0.0)
    }

    /// The thumb's height: proportional to how much of the content the
    /// viewport actually shows, floored at [`Self::MIN_THUMB_HEIGHT`]
    /// (or the track's own height, if that is somehow shorter) and
    /// capped at the track's height.
    pub fn thumb_height(&self, content_height: f64) -> f64 {
        if content_height <= 0.0 || !self.is_needed(content_height) {
            return self.track_height;
        }
        let ratio = self.track_height / content_height;
        let floor = Self::MIN_THUMB_HEIGHT.min(self.track_height);
        (self.track_height * ratio).clamp(floor, self.track_height)
    }

    /// Where the thumb's own top edge sits, given how far scrolled
    /// `offset` is — linear in how far through the scrollable range
    /// `offset` is, the same fraction whether that range is a handful of
    /// pixels or thousands of them.
    pub fn thumb_top(&self, content_height: f64, offset: f64) -> f64 {
        let thumb_height = self.thumb_height(content_height);
        let max_thumb_top = (self.track_height - thumb_height).max(0.0);
        let max_offset = self.max_offset(content_height);
        let ratio = if max_offset > 0.0 { offset.clamp(0.0, max_offset) / max_offset } else { 0.0 };
        self.track_top + ratio * max_thumb_top
    }

    /// Whether `position` lands on the thumb — the same rectangle
    /// [`Self::thumb_top`]/[`Self::thumb_height`] describe, read by both
    /// whatever draws it and whatever decides a press should start a
    /// drag, so the two can never disagree about where the thumb is.
    pub fn hit_thumb(&self, position: (f64, f64), content_height: f64, offset: f64) -> bool {
        if !self.is_needed(content_height) {
            return false;
        }
        let top = self.thumb_top(content_height, offset);
        let height = self.thumb_height(content_height);
        position.0 >= self.track_x
            && position.0 <= self.track_x + self.width
            && position.1 >= top
            && position.1 <= top + height
    }

    /// Turns a pointer's vertical motion (`delta_y`, in the same
    /// logical-pixel space) while dragging the thumb into the matching
    /// change in scroll `offset`.
    ///
    /// The thumb travels `track_height - thumb_height` pixels while the
    /// content scrolls `content_height - track_height` pixels, so one
    /// pixel of thumb motion is worth `max_offset / max_thumb_top` pixels
    /// of content offset — the ratio a full-length scrollbar's thumb
    /// would need to cover in one drag from top to bottom. `0.0` (no
    /// motion) whenever there is nothing to scroll or the thumb has no
    /// room to move at all, rather than a division by zero.
    pub fn drag_delta_to_offset_delta(&self, delta_y: f64, content_height: f64) -> f64 {
        if !self.is_needed(content_height) {
            return 0.0;
        }
        let thumb_height = self.thumb_height(content_height);
        let max_thumb_top = self.track_height - thumb_height;
        if max_thumb_top <= 0.0 {
            return 0.0;
        }
        let max_offset = self.max_offset(content_height);
        delta_y * (max_offset / max_thumb_top)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // --- `clamp_offset`

    #[test]
    fn a_short_list_clamps_to_the_top_no_matter_what_is_asked_for() {
        assert_eq!(clamp_offset(500.0, 100.0, 400.0), 0.0, "content shorter than the viewport cannot scroll at all");
    }

    #[test]
    fn offset_never_goes_negative() {
        assert_eq!(clamp_offset(-50.0, 1000.0, 400.0), 0.0);
    }

    #[test]
    fn offset_never_runs_past_the_point_where_the_last_content_reaches_the_bottom() {
        let clamped = clamp_offset(10_000.0, 1000.0, 400.0);
        assert_eq!(clamped, 600.0, "the max offset is content_height - viewport_height");
    }

    // --- `scroll_into_view`

    #[test]
    fn an_item_already_inside_the_window_does_not_move_the_offset() {
        assert_eq!(scroll_into_view(100.0, 120.0, 150.0, 400.0), 100.0);
    }

    #[test]
    fn an_item_above_the_window_pulls_the_offset_up_to_meet_its_own_top() {
        assert_eq!(scroll_into_view(200.0, 50.0, 90.0, 400.0), 50.0);
    }

    #[test]
    fn an_item_below_the_window_pushes_the_offset_down_by_exactly_the_overshoot() {
        // Window [0, 400); item [450, 500) is 100px past the bottom edge.
        assert_eq!(scroll_into_view(0.0, 450.0, 500.0, 400.0), 100.0);
    }

    // --- `Scrollbar::is_needed` / hiding when content fits

    #[test]
    fn a_scrollbar_is_not_needed_when_all_the_content_already_fits() {
        let bar = Scrollbar::new(300.0, 20.0, 400.0);
        assert!(!bar.is_needed(400.0));
        assert!(!bar.is_needed(200.0), "less content than the viewport is still nothing to scroll");
    }

    #[test]
    fn a_scrollbar_is_needed_once_content_overflows_the_viewport() {
        let bar = Scrollbar::new(300.0, 20.0, 400.0);
        assert!(bar.is_needed(401.0));
    }

    // --- thumb size and position

    #[test]
    fn the_thumb_shrinks_as_content_grows_but_never_below_the_floor() {
        let bar = Scrollbar::new(300.0, 20.0, 400.0);
        let modest = bar.thumb_height(800.0);
        let huge = bar.thumb_height(100_000.0);
        assert!(huge < modest, "far more content must make a smaller thumb");
        assert!(huge >= Scrollbar::MIN_THUMB_HEIGHT - 0.001, "the thumb must never shrink below the floor");
    }

    #[test]
    fn the_thumb_fills_the_whole_track_when_nothing_needs_scrolling() {
        let bar = Scrollbar::new(300.0, 20.0, 400.0);
        assert_eq!(bar.thumb_height(300.0), 400.0);
    }

    #[test]
    fn the_thumb_sits_at_the_top_at_zero_offset_and_the_bottom_at_the_max_offset() {
        let bar = Scrollbar::new(300.0, 20.0, 400.0);
        let content_height = 2000.0;
        assert_eq!(bar.thumb_top(content_height, 0.0), bar.track_top);
        let max_offset = bar.max_offset(content_height);
        let bottom = bar.thumb_top(content_height, max_offset);
        let expected_bottom = bar.track_top + bar.track_height - bar.thumb_height(content_height);
        assert!((bottom - expected_bottom).abs() < 0.001, "got {bottom}, expected {expected_bottom}");
    }

    #[test]
    fn hit_thumb_is_false_everywhere_when_the_scrollbar_is_not_needed() {
        let bar = Scrollbar::new(300.0, 20.0, 400.0);
        assert!(!bar.hit_thumb((301.0, 25.0), 400.0, 0.0), "nothing to grab when everything already fits");
    }

    #[test]
    fn hit_thumb_is_true_only_inside_the_thumbs_own_rectangle() {
        let bar = Scrollbar::new(300.0, 20.0, 400.0);
        let content_height = 2000.0;
        let top = bar.thumb_top(content_height, 0.0);
        let height = bar.thumb_height(content_height);
        assert!(bar.hit_thumb((303.0, top + height / 2.0), content_height, 0.0));
        assert!(!bar.hit_thumb((303.0, top - 1.0), content_height, 0.0), "just above the thumb misses");
        assert!(!bar.hit_thumb((303.0, top + height + 1.0), content_height, 0.0), "just below the thumb misses");
        assert!(!bar.hit_thumb((bar.track_x - 1.0, top), content_height, 0.0), "left of the track misses");
        assert!(!bar.hit_thumb((bar.track_x + bar.width + 1.0, top), content_height, 0.0), "right of the track misses");
    }

    // --- dragging

    #[test]
    fn dragging_the_thumb_from_top_to_bottom_scrolls_from_zero_to_the_max_offset() {
        let bar = Scrollbar::new(300.0, 20.0, 400.0);
        let content_height = 2000.0;
        let max_thumb_travel = bar.track_height - bar.thumb_height(content_height);
        let delta = bar.drag_delta_to_offset_delta(max_thumb_travel, content_height);
        assert!((delta - bar.max_offset(content_height)).abs() < 0.01, "got {delta}");
    }

    #[test]
    fn a_drag_when_nothing_needs_scrolling_moves_the_offset_by_nothing() {
        let bar = Scrollbar::new(300.0, 20.0, 400.0);
        assert_eq!(bar.drag_delta_to_offset_delta(50.0, 300.0), 0.0);
    }

    #[test]
    fn dragging_up_and_down_are_mirror_images() {
        let bar = Scrollbar::new(300.0, 20.0, 400.0);
        let content_height = 2000.0;
        let down = bar.drag_delta_to_offset_delta(10.0, content_height);
        let up = bar.drag_delta_to_offset_delta(-10.0, content_height);
        assert!((down + up).abs() < 0.001);
    }
}
