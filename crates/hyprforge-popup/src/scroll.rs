//! Turning one scroll-wheel axis event into whole rows/cells to move a
//! selection by.
//!
//! Pure and popup-content-agnostic — a list moving one row per notch and
//! a grid moving one cell per notch both want the same "how many
//! notches" arithmetic, so it lives here rather than in
//! `hyprforge-clipmenu` alone.

/// Tries three ways to answer "how many notches", in order of how
/// reliable each is on a current compositor: `value120` is what a modern
/// wl_pointer sends (120 per logical step — see the field's own doc),
/// `discrete` is the older "one integer per click" event some
/// compositors still send instead, and the sign of `absolute` (raw
/// scrolled pixels) is the fallback for a touchpad's continuous scroll,
/// which reports neither — moving exactly one row per axis frame there,
/// since there is no such thing as half a row to select.
pub fn scroll_rows(discrete: i32, value120: i32, absolute: f64) -> i32 {
    if value120 != 0 {
        match value120 / 120 {
            0 if value120 > 0 => 1,
            0 => -1,
            steps => steps,
        }
    } else if discrete != 0 {
        discrete
    } else if absolute > 0.0 {
        1
    } else if absolute < 0.0 {
        -1
    } else {
        0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_high_resolution_wheel_step_of_exactly_120_moves_one_row() {
        assert_eq!(scroll_rows(0, 120, 0.0), 1);
        assert_eq!(scroll_rows(0, -120, 0.0), -1);
    }

    #[test]
    fn a_fractional_high_resolution_step_still_moves_at_least_one_row() {
        // A touchpad or a fine-grained wheel can report a `value120`
        // smaller than one logical step; it must still count as a
        // scroll, not be truncated away to nothing.
        assert_eq!(scroll_rows(0, 40, 0.0), 1);
        assert_eq!(scroll_rows(0, -40, 0.0), -1);
    }

    #[test]
    fn a_legacy_discrete_step_is_used_when_no_value120_is_reported() {
        assert_eq!(scroll_rows(2, 0, 0.0), 2);
        assert_eq!(scroll_rows(-3, 0, 0.0), -3);
    }

    #[test]
    fn a_touchpads_continuous_scroll_falls_back_to_one_row_per_frame() {
        assert_eq!(scroll_rows(0, 0, 5.0), 1);
        assert_eq!(scroll_rows(0, 0, -5.0), -1);
        assert_eq!(scroll_rows(0, 0, 0.0), 0, "no motion at all must move nothing");
    }

    #[test]
    fn value120_wins_over_discrete_when_a_compositor_reports_both() {
        assert_eq!(scroll_rows(1, 240, 0.0), 2);
    }
}
