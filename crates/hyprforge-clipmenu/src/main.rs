//! A clipboard history popup that appears where the mouse is, shows one
//! item once, and exits.
//!
//! This is a per-invocation program, launched by a keybind, not a
//! long-running daemon. Two reasons, not one:
//!
//! - A popup that lives forever is a window manager's problem: staying
//!   out of the way when unfocused, reappearing on the right output
//!   when the cursor has moved, surviving a monitor being unplugged.
//!   None of that is this program's job, and a daemon would have to
//!   solve all of it just to sit idle between invocations.
//! - A short-lived process cannot leak a stuck layer surface. It takes
//!   `KeyboardInteractivity::Exclusive` — CLAUDE.md is explicit that a
//!   keybind's compositor belongs to whoever is standing at the
//!   keyboard — and the one guarantee that has to hold no matter how
//!   this exits (chosen, cancelled, killed, panicked) is that the
//!   surface it made goes away with the process. A daemon that misjudged
//!   its own state and left the popup up, holding exclusive keyboard
//!   focus, would be a stuck keyboard on a compositor with no window
//!   manager left to blame.
//!
//! `hyprforge-clipboard` owns the history and its own daemon watches the
//! compositor's clipboard to fill it; this only ever reads it (see
//! `crates/hyprforge-clipboard/src/store.rs`) and, on Enter, writes the
//! chosen entry back out through `chooser::Chooser` — see `chooser::Wired`
//! for where that plugs into `hyprforge-clipboard`'s write side.

mod chooser;
mod geometry;
mod model;
mod pinner;
mod singleton;
mod surface;
mod thumbnail;
mod view;

use geometry::{Monitor, Point, RowLayout, Size};
use hyprforge_process::{output, TIMEOUT};
use model::{HistoryState, Model};
use std::process::Command;
use surface::{ClipMenu, Outcome, Placement};

/// The popup's fixed size in logical pixels. Not configurable yet —
/// there is nowhere for a setting like this to live until the Settings
/// app grows a clipboard tab, and a fixed size is a perfectly ordinary
/// thing for a Windows-style clipboard popup to have.
const POPUP_WIDTH: f64 = 360.0;
const POPUP_HEIGHT: f64 = 420.0;

/// `hyprctl cursorpos -j`, in logical global coordinates.
///
/// `None` covers every way this can fail to answer: not installed, no
/// compositor, a malformed reply. All of them mean "don't know where the
/// cursor is", and the caller falls back to the first monitor's origin
/// rather than guessing at a position.
fn cursor_position() -> Option<Point> {
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
/// `cursorpos` and this popup's placement both live in.
///
/// An empty list (no `hyprctl`, no compositor, unparsable JSON) is
/// reported as such rather than guessed at; `main` treats it as nowhere
/// to show the popup.
fn monitors() -> Vec<Monitor> {
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

/// A font size the renderer will not choke on.
///
/// `iced_tiny_skia`/`cosmic-text` assert a non-zero line height, so a
/// font size of `0.0` panics on the first frame — the same hazard
/// `hyprforge_authui::screen::renderable` guards the lock screen's theme
/// against.
///
/// Deliberately not a call to `renderable` itself: that lives in
/// `hyprforge-authui`, and a clipboard popup has no business depending
/// on the authentication screen to clamp a number. `view`'s
/// `corner_radius` bounds the rounding the same way and says so. If a
/// third host ever needs these bounds, they belong beside `Theme` in
/// `hyprforge-look` rather than being copied a third time.
fn sane_font_size(theme: &hyprforge_look::Theme) -> f32 {
    let size = theme.font_size;
    if size.is_finite() { size.clamp(6.0, 48.0) } else { 15.0 }
}

fn placement(monitors: &[Monitor], cursor: Option<Point>) -> Option<Placement> {
    let monitor = match cursor {
        Some(cursor) => geometry::monitor_at(monitors, cursor).or_else(|| monitors.first()),
        None => monitors.first(),
    }?;
    // The cursor position `hyprctl` reports is global; the layer-shell
    // margins this popup sets are relative to the output it is anchored
    // to. Translating into the monitor's own local space is what makes
    // `clamp_popup` (which only ever sees one monitor's rectangle at a
    // time) correct for any monitor, not just the one at the origin.
    let local_cursor = match cursor {
        Some(cursor) => {
            Point { x: cursor.x - monitor.origin.x, y: cursor.y - monitor.origin.y }
        }
        // No cursor position at all: open in the monitor's corner
        // rather than not at all.
        None => Point { x: 0.0, y: 0.0 },
    };
    let popup = Size { width: POPUP_WIDTH, height: POPUP_HEIGHT };
    let placed = geometry::clamp_popup(local_cursor, popup, monitor.size);
    Some(Placement {
        output_name: monitor.name.clone(),
        margin_top: placed.y.round() as i32,
        margin_left: placed.x.round() as i32,
        width: popup.width.round() as u32,
        height: popup.height.round() as u32,
    })
}

fn main() -> std::process::ExitCode {
    // Only one popup at a time: a keybind pressed twice while one is
    // already open must leave the first alone and exit quietly, not
    // start a second process — see `singleton`'s own doc for why this is
    // an flock rather than a PID file or a process-name match.
    let lock_path = singleton::lock_path();
    let _lock = match singleton::acquire(&lock_path) {
        Ok(Some(lock)) => Some(lock),
        // Someone already has it: this is a keybind pressed twice, not
        // an error — silent and successful, exactly as if this process
        // had never run.
        Ok(None) => return std::process::ExitCode::SUCCESS,
        // Couldn't even check — never let a broken lock lock out every
        // future popup; proceed without one.
        Err(e) => {
            eprintln!("couldn't set up the single-instance lock ({e}) — continuing anyway");
            None
        }
    };

    let monitors = monitors();
    if monitors.is_empty() {
        eprintln!("couldn't read any monitors from hyprctl — is Hyprland running?");
        return std::process::ExitCode::FAILURE;
    }
    let Some(placement) = placement(&monitors, cursor_position()) else {
        eprintln!("couldn't work out where to place the popup");
        return std::process::ExitCode::FAILURE;
    };

    let history = HistoryState::from_result(hyprforge_clipboard::History::load());
    let mut model = Model::new(history);

    let mut theme = hyprforge_appearance::look::resolve();
    theme.font_size = sane_font_size(&theme);

    // How many rows the fixed-height popup actually has room for — the
    // fix for the bug that motivated this: `Model` used to build a
    // hardcoded 24 rows regardless of `POPUP_HEIGHT`, which laid out
    // more than twice the popup's own height in rows, so nothing a
    // pointer touched was where the drawn rows actually were. Deriving
    // it from `RowLayout::rows_that_fit` — the exact inverse of the hit
    // test — is what keeps rows drawn, rows hit-tested and rows that
    // physically fit from ever being three different numbers again.
    model.set_window(RowLayout::for_font_size(theme.font_size).rows_that_fit(POPUP_HEIGHT));

    let connection = match wayland_client::Connection::connect_to_env() {
        Ok(connection) => connection,
        Err(e) => {
            eprintln!("couldn't connect to the compositor: {e}");
            return std::process::ExitCode::FAILURE;
        }
    };

    // **The seam**: `chooser::Wired` is where `hyprforge-clipboard`'s
    // write-side traits (`ClipboardWriter`, `PasteSynthesizer`) plug in —
    // see its doc comment. Every test in this crate instead drives
    // `chooser::mock::MockChooser`, so nothing here depends on a real
    // compositor to be checked.
    let chooser = match chooser::Wired::connect() {
        Ok(chooser) => chooser,
        Err(e) => {
            eprintln!("couldn't set up pasting: {e}");
            return std::process::ExitCode::FAILURE;
        }
    };

    match ClipMenu::run(connection, placement, model, chooser, pinner::Wired, theme) {
        Ok(Outcome::Chosen | Outcome::Cancelled) => std::process::ExitCode::SUCCESS,
        Ok(Outcome::Closed | Outcome::Disconnected) => {
            eprintln!("the popup closed unexpectedly");
            std::process::ExitCode::FAILURE
        }
        Err(e) => {
            eprintln!("{e}");
            std::process::ExitCode::FAILURE
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn monitor(name: &str, x: f64, y: f64, width: f64, height: f64) -> Monitor {
        Monitor { name: name.into(), origin: Point { x, y }, size: Size { width, height } }
    }

    #[test]
    fn the_popup_is_placed_on_the_monitor_the_cursor_is_over() {
        let monitors = vec![monitor("eDP-2", 0.0, 0.0, 1600.0, 1000.0), monitor("DP-3", 1600.0, 0.0, 1920.0, 1080.0)];
        let placed = placement(&monitors, Some(Point { x: 1700.0, y: 50.0 })).unwrap();
        assert_eq!(placed.output_name, "DP-3");
        // 100 into the second monitor, not 1700 into the first.
        assert_eq!(placed.margin_left, 100);
        assert_eq!(placed.margin_top, 50);
    }

    #[test]
    fn a_cursor_position_that_could_not_be_read_falls_back_to_the_first_monitor() {
        let monitors = vec![monitor("eDP-2", 0.0, 0.0, 1600.0, 1000.0)];
        let placed = placement(&monitors, None).unwrap();
        assert_eq!(placed.output_name, "eDP-2");
        assert_eq!((placed.margin_left, placed.margin_top), (0, 0));
    }

    #[test]
    fn placement_still_clamps_to_the_monitor_the_cursor_landed_on() {
        let monitors = vec![monitor("eDP-2", 0.0, 0.0, 1600.0, 1000.0)];
        let placed = placement(&monitors, Some(Point { x: 1590.0, y: 990.0 })).unwrap();
        assert!(placed.margin_left as f64 + placed.width as f64 <= 1600.0);
        assert!(placed.margin_top as f64 + placed.height as f64 <= 1000.0);
    }

    /// The regression test for the bug that started all of this: the
    /// model's row window has to be *derived* from the popup's own fixed
    /// height, not a separate hardcoded number that can drift out of
    /// step with it. A history far longer than the window still has to
    /// build only as many rows as physically fit — `rows_that_fit` and
    /// `set_window` are exercised exactly the way `main` wires them
    /// together, against a history long enough that the old hardcoded
    /// 24 would have shown a different number than this does.
    #[test]
    fn the_models_row_window_is_derived_from_the_popups_actual_height() {
        let layout = RowLayout::for_font_size(15.0);
        let expected_rows = layout.rows_that_fit(POPUP_HEIGHT);

        let history = model::HistoryState::Loaded(
            (0..200).map(|i| test_entry(&format!("entry {i}"))).collect(),
        );
        let mut model = Model::new(history);
        model.set_window(layout.rows_that_fit(POPUP_HEIGHT));

        assert_eq!(model.visible_range().len(), expected_rows);
        // And every one of those rows has to actually fit: the same
        // check `geometry`'s own `fit_tests` module pins for
        // `rows_that_fit` in isolation, repeated here end to end through
        // `Model`.
        let stride = layout.row_height + layout.row_spacing;
        let last_row_bottom =
            layout.padding + layout.header_height + (expected_rows as f64 - 1.0) * stride + layout.row_height;
        assert!(last_row_bottom <= POPUP_HEIGHT - layout.padding);
    }

    fn test_entry(text: &str) -> hyprforge_clipboard::Entry {
        let content = hyprforge_clipboard::Content::Text(text.to_string());
        hyprforge_clipboard::Entry {
            id: hyprforge_clipboard::EntryId::of(&content),
            content,
            copied_at: 0,
            pinned: false,
        }
    }

    #[test]
    fn a_non_finite_font_size_falls_back_rather_than_panicking_the_renderer() {
        let theme = hyprforge_look::Theme { font_size: f32::NAN, ..hyprforge_look::Theme::default() };
        assert_eq!(sane_font_size(&theme), 15.0);
        let theme = hyprforge_look::Theme { font_size: 0.0, ..hyprforge_look::Theme::default() };
        assert!(sane_font_size(&theme) >= 6.0);
    }
}
