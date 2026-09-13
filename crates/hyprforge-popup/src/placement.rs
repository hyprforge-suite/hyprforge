//! Reading Hyprland's own idea of where the monitors and the cursor are,
//! and turning that into where a popup should open.
//!
//! `hyprctl` usage is Hyprland-specific knowledge, and this crate is a
//! leaf `hyprforge-clipmenu`'s CLAUDE.md-mandated layering rules say a
//! new popup should be able to depend on without pulling in
//! `hyprforge-core`, an async runtime, or D-Bus. It stays here rather
//! than with each consumer for the same reason [`Placement`] itself
//! lives in this crate: every layer-shell popup needs to answer "which
//! monitor, and where on it" the same way, and getting that wrong once
//! per consumer is exactly the drift CLAUDE.md's "one place to fix a
//! shared thing" exists to prevent. `crate::singleton` and
//! `crate::popup` are the same kind of call: infrastructure every popup
//! needs, not something specific to a clipboard history.

use crate::geometry::{Monitor, Point, Size};
use crate::popup::Placement;
use hyprforge_process::{output, TIMEOUT};
use std::process::Command;

/// `hyprctl cursorpos -j`, in logical global coordinates.
///
/// `None` covers every way this can fail to answer: not installed, no
/// compositor, a malformed reply. All of them mean "don't know where the
/// cursor is", and the caller falls back to the first monitor's origin
/// rather than guessing at a position.
pub fn cursor_position() -> Option<Point> {
    let result = output(Command::new("hyprctl").args(["cursorpos", "-j"]), TIMEOUT).ok()?;
    let value: serde_json::Value = serde_json::from_slice(&result.stdout).ok()?;
    Some(Point { x: value.get("x")?.as_f64()?, y: value.get("y")?.as_f64()? })
}

/// `hyprctl monitors -j`, converted to logical rectangles.
///
/// `width`/`height` there are **physical** pixels; dividing by `scale`
/// is what makes them comparable to `x`/`y` and to `cursorpos`, which are
/// already logical. Verified on this machine: a 2560x1600 monitor at
/// scale 1.6 reports a logical size of 1600x1000, which is the space
/// `cursorpos` and a popup's placement both live in.
///
/// An empty list (no `hyprctl`, no compositor, unparsable JSON) is
/// reported as such rather than guessed at; the caller treats it as
/// nowhere to show the popup.
pub fn monitors() -> Vec<Monitor> {
    let Ok(result) = output(Command::new("hyprctl").args(["monitors", "-j"]), TIMEOUT) else {
        return Vec::new();
    };
    let Ok(value) = serde_json::from_slice::<serde_json::Value>(&result.stdout) else {
        return Vec::new();
    };
    value
        .as_array()
        .map(|list| {
            list.iter()
                .filter_map(|m| {
                    let name = m.get("name")?.as_str()?.to_string();
                    let x = m.get("x")?.as_f64()?;
                    let y = m.get("y")?.as_f64()?;
                    let width = m.get("width")?.as_f64()?;
                    let height = m.get("height")?.as_f64()?;
                    // A missing or zero scale would divide by zero;
                    // falling back to 1.0 treats the monitor as
                    // unscaled rather than producing an infinite size.
                    let scale = m.get("scale").and_then(|s| s.as_f64()).filter(|s| *s > 0.0).unwrap_or(1.0);
                    Some(Monitor {
                        name,
                        origin: Point { x, y },
                        size: Size { width: width / scale, height: height / scale },
                    })
                })
                .collect()
        })
        .unwrap_or_default()
}

/// Where a popup of `popup_size` should open, given `monitors` and the
/// cursor's last-known global position.
///
/// Finds the monitor the cursor is over (falling back to the first
/// monitor when there is no cursor position, or when the cursor landed
/// on none of them), translates into that monitor's own local space —
/// the cursor position `hyprctl` reports is global, but the layer-shell
/// margins a popup sets are relative to the output it is anchored to —
/// and hands the translated point to [`crate::geometry::clamp_popup`],
/// which only ever sees one monitor's rectangle at a time.
pub fn place(monitors: &[Monitor], cursor: Option<Point>, popup_size: Size) -> Option<Placement> {
    let monitor = match cursor {
        Some(cursor) => crate::geometry::monitor_at(monitors, cursor).or_else(|| monitors.first()),
        None => monitors.first(),
    }?;
    let local_cursor = match cursor {
        Some(cursor) => Point { x: cursor.x - monitor.origin.x, y: cursor.y - monitor.origin.y },
        // No cursor position at all: open in the monitor's corner
        // rather than not at all.
        None => Point { x: 0.0, y: 0.0 },
    };
    let placed = crate::geometry::clamp_popup(local_cursor, popup_size, monitor.size);
    Some(Placement {
        output_name: monitor.name.clone(),
        margin_top: placed.y.round() as i32,
        margin_left: placed.x.round() as i32,
        width: popup_size.width.round() as u32,
        height: popup_size.height.round() as u32,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn monitor(name: &str, x: f64, y: f64, width: f64, height: f64) -> Monitor {
        Monitor { name: name.into(), origin: Point { x, y }, size: Size { width, height } }
    }

    const POPUP: Size = Size { width: 360.0, height: 420.0 };

    #[test]
    fn the_popup_is_placed_on_the_monitor_the_cursor_is_over() {
        let monitors = vec![monitor("eDP-2", 0.0, 0.0, 1600.0, 1000.0), monitor("DP-3", 1600.0, 0.0, 1920.0, 1080.0)];
        let placed = place(&monitors, Some(Point { x: 1700.0, y: 50.0 }), POPUP).unwrap();
        assert_eq!(placed.output_name, "DP-3");
        // 100 into the second monitor, not 1700 into the first.
        assert_eq!(placed.margin_left, 100);
        assert_eq!(placed.margin_top, 50);
    }

    #[test]
    fn a_cursor_position_that_could_not_be_read_falls_back_to_the_first_monitor() {
        let monitors = vec![monitor("eDP-2", 0.0, 0.0, 1600.0, 1000.0)];
        let placed = place(&monitors, None, POPUP).unwrap();
        assert_eq!(placed.output_name, "eDP-2");
        assert_eq!((placed.margin_left, placed.margin_top), (0, 0));
    }

    #[test]
    fn placement_still_clamps_to_the_monitor_the_cursor_landed_on() {
        let monitors = vec![monitor("eDP-2", 0.0, 0.0, 1600.0, 1000.0)];
        let placed = place(&monitors, Some(Point { x: 1590.0, y: 990.0 }), POPUP).unwrap();
        assert!(placed.margin_left as f64 + placed.width as f64 <= 1600.0);
        assert!(placed.margin_top as f64 + placed.height as f64 <= 1000.0);
    }
}
