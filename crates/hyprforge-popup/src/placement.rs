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
                    // `reserved` is `[left, top, right, bottom]`, not the
                    // more guessable-sounding `[top, bottom, left,
                    // right]` — confirmed against Hyprland's own
                    // `src/ipc/s1/Commands.cpp`, which builds the array
                    // as `left, top, right, bottom`, and against this
                    // workspace's own live output: a waybar reserving 50
                    // logical pixels at the top reports `reserved: [0,
                    // 50, 0, 0]`, which is only consistent with index
                    // `1` being `top`. Already logical — see
                    // `Monitor::reserved_top`'s own doc — so no `/scale`
                    // here, unlike `width`/`height` above. A monitor with
                    // no `reserved` array at all (an older `hyprctl`, or
                    // a malformed reply) reserves nothing rather than
                    // failing the whole monitor.
                    let reserved_top = m
                        .get("reserved")
                        .and_then(|r| r.as_array())
                        .and_then(|r| r.get(1))
                        .and_then(|v| v.as_f64())
                        .unwrap_or(0.0);
                    Some(Monitor {
                        name,
                        origin: Point { x, y },
                        size: Size { width: width / scale, height: height / scale },
                        reserved_top,
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

/// Where the tray's right-click menu should open: anchored on the Y axis
/// to the monitor's own reserved area (so it lands in the same place
/// every time, regardless of where on the icon the click landed) and on
/// the X axis to the click itself (so it still visibly belongs to the
/// icon that opened it).
///
/// `click` is the icon's own position, in the same global logical space
/// [`monitors`] and [`cursor_position`] both use — confirmed against
/// waybar's own tray module (`Item::handleClick` in `src/modules/sni/item.cpp`),
/// which sends `ContextMenu` a `GdkEventButton`'s `x_root`/`y_root` plus
/// the bar's own logical global offset, never anything scaled by the
/// output's buffer scale. That is why this crate's own `sni.rs` no
/// longer needs to guess: the click coordinates it is handed were never
/// physical pixels in the first place.
///
/// Reuses [`crate::geometry::clamp_popup`] rather than a second flip
/// rule: the anchor point handed to it is `(click's local X, this
/// monitor's own reserved top + menu_y_offset)` instead of a raw cursor
/// position, but the "prefer forward, flip if it doesn't fit, clamp only
/// as a last resort" rule is exactly the same rule a fixed anchor point
/// needs too — a menu tall enough to run off the bottom of the screen
/// must still end up fully on screen.
pub fn place_below_bar(monitors: &[Monitor], click: Point, menu_y_offset: i32, popup_size: Size) -> Option<Placement> {
    let monitor = crate::geometry::monitor_at(monitors, click).or_else(|| monitors.first())?;
    let anchor = Point {
        x: click.x - monitor.origin.x,
        y: monitor.reserved_top + menu_y_offset as f64,
    };
    let placed = crate::geometry::clamp_popup(anchor, popup_size, monitor.size);
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
        Monitor { name: name.into(), origin: Point { x, y }, size: Size { width, height }, reserved_top: 0.0 }
    }

    fn monitor_with_bar(name: &str, x: f64, y: f64, width: f64, height: f64, reserved_top: f64) -> Monitor {
        Monitor { name: name.into(), origin: Point { x, y }, size: Size { width, height }, reserved_top }
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

    // --- `place_below_bar`: the tray menu's own placement, anchored to
    // the bar rather than to the pointer.

    const MENU: Size = Size { width: 220.0, height: 180.0 };

    #[test]
    fn the_menu_y_is_the_reserved_area_plus_the_offset_regardless_of_where_the_icon_was_clicked() {
        let monitors = vec![monitor_with_bar("eDP-2", 0.0, 0.0, 1600.0, 1000.0, 50.0)];
        // Two clicks at very different Y positions on the icon itself —
        // both must land at exactly the same Y, which is the whole
        // point of anchoring to the bar instead of to the pointer.
        let low = place_below_bar(&monitors, Point { x: 800.0, y: 12.0 }, 32, MENU).unwrap();
        let high = place_below_bar(&monitors, Point { x: 800.0, y: 48.0 }, 32, MENU).unwrap();
        assert_eq!(low.margin_top, 82, "50 (reserved) + 32 (offset)");
        assert_eq!(high.margin_top, 82, "must not move with the click's own Y");
    }

    #[test]
    fn the_menu_x_still_follows_the_icon_that_was_clicked() {
        let monitors = vec![monitor_with_bar("eDP-2", 0.0, 0.0, 1600.0, 1000.0, 50.0)];
        let left_icon = place_below_bar(&monitors, Point { x: 100.0, y: 20.0 }, 32, MENU).unwrap();
        let right_icon = place_below_bar(&monitors, Point { x: 900.0, y: 20.0 }, 32, MENU).unwrap();
        assert_eq!(left_icon.margin_left, 100);
        assert_eq!(right_icon.margin_left, 900);
    }

    #[test]
    fn the_click_x_is_translated_into_the_monitor_the_icon_is_actually_on() {
        let monitors = vec![
            monitor_with_bar("eDP-2", 0.0, 0.0, 1600.0, 1000.0, 50.0),
            monitor_with_bar("DP-3", 1600.0, 0.0, 1920.0, 1080.0, 0.0),
        ];
        let placed = place_below_bar(&monitors, Point { x: 1700.0, y: 20.0 }, 32, MENU).unwrap();
        assert_eq!(placed.output_name, "DP-3");
        assert_eq!(placed.margin_left, 100, "1700 minus the second monitor's own 1600 origin");
        assert_eq!(placed.margin_top, 32, "this monitor's own bar reserves nothing");
    }

    /// The one case a fixed anchor still needs the flip-then-clamp rule
    /// for: a menu tall enough (or a bar low enough) that the anchor
    /// point plus the popup's height would run off the bottom of the
    /// screen.
    #[test]
    fn a_menu_that_would_run_off_the_bottom_still_ends_up_fully_on_screen() {
        let monitors = vec![monitor_with_bar("eDP-2", 0.0, 0.0, 1600.0, 1000.0, 900.0)];
        let placed = place_below_bar(&monitors, Point { x: 800.0, y: 905.0 }, 32, MENU).unwrap();
        assert!(placed.margin_top as f64 + MENU.height <= 1000.0, "must not run off the bottom");
    }

    #[test]
    fn a_monitor_reporting_no_reserved_area_at_all_opens_the_menu_at_just_the_offset() {
        let monitors = vec![monitor("eDP-2", 0.0, 0.0, 1600.0, 1000.0)];
        let placed = place_below_bar(&monitors, Point { x: 800.0, y: 20.0 }, 32, MENU).unwrap();
        assert_eq!(placed.margin_top, 32);
    }
}
