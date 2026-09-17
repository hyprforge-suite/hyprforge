//! Telling a double click from two single clicks.
//!
//! This is not `mouse_area::on_double_click`, and the reason is worth
//! writing down because the obvious wiring looks right and does nothing.
//! A row is an `iced::widget::button` — that is what gives it hover
//! styling, selection styling and a focusable hit area — and `button`
//! calls `shell.capture_event()` the moment it sees a left press
//! (`iced_widget-0.14.2/src/button.rs`, in `update`). A `mouse_area`
//! wrapped *around* the button is a parent, so it is offered the event
//! after the button has already captured it, and its `on_double_click`
//! never fires. Putting the `mouse_area` inside the button instead just
//! moves the problem: whichever of the two captures the press, the other
//! one stops working, so the row would double-click but no longer
//! select.
//!
//! So the timing is measured where the clicks arrive rather than asked
//! of a widget. The host already does this for modifier keys — a click
//! in iced carries no modifier state either — and it is the same
//! principle: the side that observes the world reports what it saw, and
//! the model stays a function of the message it is given.
//!
//! The clock is a parameter throughout, so the rule is testable against
//! fixed instants rather than against how fast a test machine happens to
//! run.

use std::time::{Duration, Instant};

/// How close together two presses must be to count as one double click.
///
/// 400ms, matching the GTK and Qt defaults — this app sits beside
/// applications using both, and a file manager whose double click is
/// fussier than the rest of the desktop reads as broken rather than as
/// deliberate.
pub const DOUBLE_CLICK_WINDOW: Duration = Duration::from_millis(400);

/// What a press turned out to be.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Click {
    Single,
    Double,
}

/// Remembers the last press, so the next one can be compared against it.
#[derive(Debug, Clone, Copy, Default)]
pub struct ClickTracker {
    last: Option<(usize, Instant)>,
}

impl ClickTracker {
    pub fn new() -> Self {
        Self::default()
    }

    /// Records a press on `index` at `now`, and says what it was.
    ///
    /// A press counts as a double click only when it lands on the *same
    /// row* as the one before it and inside [`DOUBLE_CLICK_WINDOW`].
    /// Requiring the same row matters: clicking one file and then
    /// quickly clicking another is two single clicks and must not open
    /// the second, which is precisely the case where opening the wrong
    /// thing would be most surprising.
    ///
    /// A double click also *ends* the sequence — the state is cleared —
    /// so a third rapid press starts over rather than opening the row
    /// again. Otherwise holding a click down on a trackpad's tap-to-click
    /// could open the same file three times.
    pub fn press(&mut self, index: usize, now: Instant) -> Click {
        let is_double = matches!(
            self.last,
            Some((last_index, at))
                if last_index == index && now.duration_since(at) <= DOUBLE_CLICK_WINDOW
        );
        if is_double {
            self.last = None;
            Click::Double
        } else {
            self.last = Some((index, now));
            Click::Single
        }
    }

    /// Forgets the last press.
    ///
    /// Called when something happened that makes the previous click no
    /// longer the one immediately before this one — a navigation, or a
    /// modifier-held click, which is a selection gesture rather than the
    /// first half of an open.
    pub fn reset(&mut self) {
        self.last = None;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn at(base: Instant, millis: u64) -> Instant {
        base + Duration::from_millis(millis)
    }

    #[test]
    fn two_quick_presses_on_the_same_row_are_a_double_click() {
        let base = Instant::now();
        let mut tracker = ClickTracker::new();
        assert_eq!(tracker.press(3, base), Click::Single);
        assert_eq!(tracker.press(3, at(base, 120)), Click::Double);
    }

    #[test]
    fn two_slow_presses_are_two_single_clicks() {
        let base = Instant::now();
        let mut tracker = ClickTracker::new();
        assert_eq!(tracker.press(3, base), Click::Single);
        assert_eq!(tracker.press(3, at(base, 900)), Click::Single);
    }

    /// The case where getting it wrong opens something nobody asked for.
    #[test]
    fn two_quick_presses_on_different_rows_never_open_anything() {
        let base = Instant::now();
        let mut tracker = ClickTracker::new();
        assert_eq!(tracker.press(3, base), Click::Single);
        assert_eq!(tracker.press(4, at(base, 50)), Click::Single);
    }

    /// A third rapid press starts a new sequence rather than opening the
    /// row a second time.
    #[test]
    fn a_third_rapid_press_does_not_open_the_row_again() {
        let base = Instant::now();
        let mut tracker = ClickTracker::new();
        assert_eq!(tracker.press(1, base), Click::Single);
        assert_eq!(tracker.press(1, at(base, 100)), Click::Double);
        assert_eq!(tracker.press(1, at(base, 200)), Click::Single);
        assert_eq!(tracker.press(1, at(base, 300)), Click::Double);
    }

    #[test]
    fn exactly_at_the_window_still_counts_and_past_it_does_not() {
        let base = Instant::now();
        let mut tracker = ClickTracker::new();
        tracker.press(0, base);
        assert_eq!(tracker.press(0, base + DOUBLE_CLICK_WINDOW), Click::Double);

        tracker.press(0, base);
        assert_eq!(
            tracker.press(0, base + DOUBLE_CLICK_WINDOW + Duration::from_millis(1)),
            Click::Single
        );
    }

    #[test]
    fn a_reset_makes_the_next_press_a_first_press_again() {
        let base = Instant::now();
        let mut tracker = ClickTracker::new();
        tracker.press(2, base);
        tracker.reset();
        assert_eq!(tracker.press(2, at(base, 50)), Click::Single);
    }
}
