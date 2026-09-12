//! Where the popup lands, kept as pure functions over plain numbers.
//!
//! Everything here is deliberately free of `hyprctl`, Wayland or iced —
//! it only knows about points and sizes. That is what makes it testable
//! without a compositor: the piece most likely to be wrong (an
//! off-by-one at an edge, a sign flipped for the bottom-right corner) is
//! exactly the piece a unit test can pin directly, without a monitor to
//! click on.
//!
//! Units are logical pixels throughout — `hyprctl cursorpos` and the
//! `x`/`y` fields of `hyprctl monitors -j` are already logical, and
//! `width`/`height` there are physical and must be divided by `scale`
//! before they reach anything in this module. That division happens in
//! `main.rs`, at the boundary where the JSON is read.

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

/// Clamps a popup's top-left corner so the whole popup stays on screen.
///
/// `cursor` and `monitor` are both in the *same* monitor-local logical
/// space (the origin has already been subtracted — see `main.rs`). The
/// popup is anchored top-left in the layer-shell sense, so this returns
/// the margins to hand the compositor directly.
///
/// The rule is: put the top-left corner at the cursor when there is
/// room; otherwise slide the corner back just far enough that the
/// popup's far edge lands on the monitor's edge instead of past it.
/// Never slides past 0 in either axis — a popup taller or wider than the
/// whole monitor is clamped to the top-left corner rather than pushed
/// negative, which would put it off the *opposite* edge instead.
pub fn clamp_popup(cursor: Point, popup: Size, monitor: Size) -> Point {
    let max_x = (monitor.width - popup.width).max(0.0);
    let max_y = (monitor.height - popup.height).max(0.0);
    Point {
        x: cursor.x.clamp(0.0, max_x),
        y: cursor.y.clamp(0.0, max_y),
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
        // Cursor near the right edge: the popup's right edge must not
        // pass the monitor's right edge.
        let cursor = Point { x: 1550.0, y: 312.0 };
        let placed = clamp_popup(cursor, POPUP, MONITOR);
        assert_eq!(placed.x, MONITOR.width - POPUP.width);
        assert!(placed.x + POPUP.width <= MONITOR.width);
    }

    #[test]
    fn the_popup_flips_up_off_the_bottom_edge() {
        let cursor = Point { x: 753.0, y: 980.0 };
        let placed = clamp_popup(cursor, POPUP, MONITOR);
        assert_eq!(placed.y, MONITOR.height - POPUP.height);
        assert!(placed.y + POPUP.height <= MONITOR.height);
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
}
