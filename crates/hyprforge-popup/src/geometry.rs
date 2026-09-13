//! Where a popup lands, kept as pure functions over plain numbers.
//!
//! Everything here is deliberately free of `hyprctl`, Wayland or iced —
//! it only knows about points and sizes. That is what makes it testable
//! without a compositor: the piece most likely to be wrong (an
//! off-by-one at an edge, a sign flipped for the bottom-right corner) is
//! exactly the piece a unit test can pin directly, without a monitor to
//! click on. Moved here unchanged from `hyprforge-clipmenu::geometry`,
//! which is where it was originally written and tested — see this
//! crate's own module doc for why this half of that file moved and
//! `RowLayout` did not.
//!
//! Units are logical pixels throughout — `hyprctl cursorpos` and the
//! `x`/`y` fields of `hyprctl monitors -j` are already logical, and
//! `width`/`height` there are physical and must be divided by `scale`
//! before they reach anything in this module. That division happens in
//! [`crate::placement::monitors`], at the boundary where the JSON is
//! read.

/// A point in logical compositor space (or, once translated, in a
/// monitor's own logical space).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Point {
    pub x: f64,
    pub y: f64,
}

/// A size in logical pixels.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Size {
    pub width: f64,
    pub height: f64,
}

/// One monitor's logical rectangle, in the same global space `hyprctl
/// cursorpos` reports.
#[derive(Debug, Clone, PartialEq)]
pub struct Monitor {
    pub name: String,
    pub origin: Point,
    pub size: Size,
}

impl Monitor {
    /// Whether `global` falls inside this monitor's rectangle.
    fn contains(&self, global: Point) -> bool {
        global.x >= self.origin.x
            && global.x < self.origin.x + self.size.width
            && global.y >= self.origin.y
            && global.y < self.origin.y + self.size.height
    }
}

/// The monitor the cursor is currently over, if any.
///
/// `None` when the reported position lands outside every monitor
/// `hyprctl` described — a coordinate read at the wrong moment (a
/// monitor unplugged between the two `hyprctl` calls) rather than
/// anything to guess at. The caller falls back to the first monitor
/// rather than refusing to show the popup at all.
pub fn monitor_at(monitors: &[Monitor], global: Point) -> Option<&Monitor> {
    monitors.iter().find(|m| m.contains(global))
}

/// Places a popup relative to the cursor the way a context menu does:
/// prefer opening down-and-right of the cursor, flip to the opposite
/// side on whichever axis does not have room, and clamp into the
/// monitor only as a last resort when neither direction fits.
///
/// `cursor` and `monitor` are both in the *same* monitor-local logical
/// space (the origin has already been subtracted — see
/// [`crate::placement::place`]). The popup is anchored top-left in the
/// layer-shell sense, so this returns the margins to hand the
/// compositor directly.
///
/// The previous rule here was a plain clamp: slide the popup's corner
/// back just far enough that its far edge landed on the monitor's edge.
/// That kept the whole popup on screen, but near the bottom or right
/// edge it drew the popup *over* the cursor, with the pointer somewhere
/// in the middle of it — exactly what a context menu does not do; a
/// context menu flips to the other side of the point that opened it
/// instead. [`flip_axis`] is that rule, applied to each axis
/// independently.
pub fn clamp_popup(cursor: Point, popup: Size, monitor: Size) -> Point {
    Point {
        x: flip_axis(cursor.x, popup.width, monitor.width),
        y: flip_axis(cursor.y, popup.height, monitor.height),
    }
}

/// One axis of [`clamp_popup`]'s flip-then-clamp rule: prefer the
/// forward direction (right, or down) from `point`, flip to the
/// backward direction when the forward one does not fit the whole
/// `size`, and clamp into `[0, extent - size]` only when neither
/// direction has room for it — the same "never push past the opposite
/// edge" guarantee the old plain clamp had, kept as the fallback rather
/// than the whole rule.
fn flip_axis(point: f64, size: f64, extent: f64) -> f64 {
    if point + size <= extent {
        point
    } else if point - size >= 0.0 {
        point - size
    } else {
        point.clamp(0.0, (extent - size).max(0.0))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const POPUP: Size = Size { width: 320.0, height: 400.0 };
    const MONITOR: Size = Size { width: 1600.0, height: 1000.0 };

    #[test]
    fn the_popup_lands_exactly_at_the_cursor_when_there_is_room() {
        let cursor = Point { x: 753.0, y: 312.0 };
        assert_eq!(clamp_popup(cursor, POPUP, MONITOR), cursor);
    }

    #[test]
    fn the_popup_flips_left_off_the_right_edge() {
        // No room to the right of the cursor: the popup opens to the
        // left instead, so its right edge lands exactly at the cursor —
        // not slid along the screen edge with the cursor buried in the
        // middle of it, which is what a plain clamp used to draw here.
        let cursor = Point { x: 1550.0, y: 312.0 };
        let placed = clamp_popup(cursor, POPUP, MONITOR);
        assert_eq!(placed.x, cursor.x - POPUP.width, "must flip left, not slide to the edge");
        assert_eq!(placed.x + POPUP.width, cursor.x, "the popup's right edge must land at the cursor");
        assert!(placed.x + POPUP.width <= MONITOR.width);
    }

    #[test]
    fn the_popup_flips_up_off_the_bottom_edge() {
        // Same rule on the other axis: no room below the cursor, so the
        // popup opens upward with its bottom edge at the cursor.
        let cursor = Point { x: 753.0, y: 980.0 };
        let placed = clamp_popup(cursor, POPUP, MONITOR);
        assert_eq!(placed.y, cursor.y - POPUP.height, "must flip up, not slide to the edge");
        assert_eq!(placed.y + POPUP.height, cursor.y, "the popup's bottom edge must land at the cursor");
        assert!(placed.y + POPUP.height <= MONITOR.height);
    }

    /// Both axes flip at once in a corner — the case a context menu
    /// handles by opening up-and-left, which is exactly what a user
    /// expects and exactly what a plain clamp got wrong (it would have
    /// drawn the popup over the cursor in both directions at once).
    #[test]
    fn the_popup_flips_both_axes_in_the_bottom_right_corner() {
        // The real machine this was found on: a 2560x1600 monitor at
        // scale 1.6 reports a 1600x1000 logical size — the exact
        // `MONITOR` this test module already uses.
        let cursor = Point { x: 1550.0, y: 980.0 };
        let placed = clamp_popup(cursor, POPUP, MONITOR);
        assert_eq!(placed.x, cursor.x - POPUP.width);
        assert_eq!(placed.y, cursor.y - POPUP.height);
        assert!(placed.x + POPUP.width <= MONITOR.width);
        assert!(placed.y + POPUP.height <= MONITOR.height);
    }

    /// A popup wider than the monitor cannot fit either direction on
    /// that axis alone, while the other axis still flips normally — the
    /// mixed case a monitor-sized-only test (using a popup bigger on
    /// both axes) cannot exercise.
    #[test]
    fn an_axis_with_room_neither_way_clamps_while_the_other_still_flips() {
        let too_wide = Size { width: MONITOR.width + 100.0, height: POPUP.height };
        let cursor = Point { x: 900.0, y: 980.0 };
        let placed = clamp_popup(cursor, too_wide, MONITOR);
        assert_eq!(placed.x, 0.0, "neither direction fits, so the wide axis clamps");
        assert_eq!(placed.y, cursor.y - POPUP.height, "the other axis still flips normally");
    }

    #[test]
    fn the_popup_stays_fully_on_screen_in_every_corner() {
        for cursor in [
            Point { x: 0.0, y: 0.0 },
            Point { x: MONITOR.width, y: 0.0 },
            Point { x: 0.0, y: MONITOR.height },
            Point { x: MONITOR.width, y: MONITOR.height },
        ] {
            let placed = clamp_popup(cursor, POPUP, MONITOR);
            assert!(placed.x >= 0.0, "{placed:?} left the left edge");
            assert!(placed.y >= 0.0, "{placed:?} left the top edge");
            assert!(placed.x + POPUP.width <= MONITOR.width, "{placed:?} ran off the right");
            assert!(placed.y + POPUP.height <= MONITOR.height, "{placed:?} ran off the bottom");
        }
    }

    /// A popup bigger than the monitor cannot be fully on screen either
    /// way; it must clamp to the top-left corner rather than go negative
    /// and run off the *other* edge instead.
    #[test]
    fn a_popup_larger_than_the_monitor_clamps_to_the_top_left_rather_than_going_negative() {
        let huge = Size { width: MONITOR.width * 2.0, height: MONITOR.height * 2.0 };
        let placed = clamp_popup(Point { x: 500.0, y: 500.0 }, huge, MONITOR);
        assert_eq!(placed, Point { x: 0.0, y: 0.0 });
    }

    #[test]
    fn the_monitor_under_the_cursor_is_found_among_several() {
        let left = Monitor {
            name: "eDP-2".into(),
            origin: Point { x: 0.0, y: 0.0 },
            size: Size { width: 1600.0, height: 1000.0 },
        };
        let right = Monitor {
            name: "DP-3".into(),
            origin: Point { x: 1600.0, y: 0.0 },
            size: Size { width: 1920.0, height: 1080.0 },
        };
        let monitors = vec![left.clone(), right.clone()];

        assert_eq!(monitor_at(&monitors, Point { x: 753.0, y: 312.0 }), Some(&left));
        assert_eq!(monitor_at(&monitors, Point { x: 1700.0, y: 50.0 }), Some(&right));
    }

    /// A coordinate that lands outside every monitor is reported as
    /// such, not guessed at — the caller decides the fallback.
    #[test]
    fn a_cursor_outside_every_monitor_finds_none() {
        let monitors = vec![Monitor {
            name: "eDP-2".into(),
            origin: Point { x: 0.0, y: 0.0 },
            size: Size { width: 1600.0, height: 1000.0 },
        }];
        assert_eq!(monitor_at(&monitors, Point { x: 5000.0, y: 5000.0 }), None);
    }

    #[test]
    fn flip_axis_prefers_the_forward_direction_when_it_fits() {
        assert_eq!(flip_axis(100.0, 50.0, 1000.0), 100.0);
    }

    #[test]
    fn flip_axis_flips_backward_when_the_forward_direction_does_not_fit() {
        // 980 + 50 = 1030 > 1000, so it must flip: 980 - 50 = 930.
        assert_eq!(flip_axis(980.0, 50.0, 1000.0), 930.0);
    }

    #[test]
    fn flip_axis_clamps_only_when_neither_direction_fits() {
        // A 2000-wide span can never fit in a 1000-wide extent either
        // way; this is the fallback, not the first choice.
        assert_eq!(flip_axis(500.0, 2000.0, 1000.0), 0.0);
    }
}
