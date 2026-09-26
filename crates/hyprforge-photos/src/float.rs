//! Opening one picture as a floating window, sized to it.
//!
//! A picture opened on its own — double-clicked in Files, handed over by
//! `xdg-open` — is a glance, not a session in the library, and a tiled
//! viewer that takes half the screen and reflows every other window for
//! a glance is the wrong weight. So the viewer asks the compositor to
//! float it, sizes itself to the picture and centres itself.
//!
//! # Asked of Hyprland, after the window exists
//!
//! Wayland gives a client no way to ask to float: `xdg_toplevel` has no
//! such request, and "floating" is the compositor's own idea. So this is
//! three `hyprctl dispatch` calls against the window's own pid, made once
//! the window has mapped — `window.float`, `window.resize`, `window.center`,
//! each checked live against Hyprland 0.56 (`action = "set"` floats; the
//! window floats first at the size of the screen, which is why the resize
//! follows). Off Hyprland, or with `hyprctl` missing, none of it runs and
//! the window is simply tiled: a failure here is a logged line, never a
//! message, because a viewer that shows the picture has done its job.
//!
//! Every call is bounded by `hyprforge_process::TIMEOUT`, because
//! `hyprctl` can stop answering like any other program.
//!
//! # The size is arithmetic
//!
//! [`floating_size`] is pure: the picture's size, the monitor's usable
//! area and the chrome around the picture in, a window size out. That is
//! where the decisions are — never past 1:1, never bigger than most of the
//! screen, never smaller than the window's own minimum — and a test can
//! reach it without a compositor.

use std::process::Command;
use std::time::Duration;

/// The most of the monitor's usable area a floating viewer takes.
const MOST_OF_WIDTH: f32 = 0.8;
const MOST_OF_HEIGHT: f32 = 0.85;
/// The window's own minimum, which `main.rs` also gives iced.
pub const MIN_SIZE: (u32, u32) = (640, 420);

/// A monitor's usable area, logical pixels: its size divided by its
/// scale, less what bars reserve.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Usable {
    pub width: f32,
    pub height: f32,
}

/// The window size that shows `picture` (its presented size, in image
/// pixels) at 1:1 if it fits, or scaled down to fit most of `usable`,
/// with `chrome` (the header, status bar and filmstrip around it,
/// logical pixels) added. `None` for the picture — it could not be
/// measured — is a comfortable 3:2 window rather than a guess at it.
pub fn floating_size(picture: Option<(u32, u32)>, usable: Usable, chrome: (f32, f32)) -> (u32, u32) {
    let most = (usable.width * MOST_OF_WIDTH, usable.height * MOST_OF_HEIGHT);
    let room = ((most.0 - chrome.0).max(1.0), (most.1 - chrome.1).max(1.0));
    let (w, h) = match picture.filter(|(w, h)| *w > 0 && *h > 0) {
        Some((pw, ph)) => {
            let (pw, ph) = (pw as f32, ph as f32);
            // Never past 1:1 — a small picture shown at its size is sharp;
            // blown up to fill a window it is a blur.
            let scale = (room.0 / pw).min(room.1 / ph).min(1.0);
            (pw * scale + chrome.0, ph * scale + chrome.1)
        }
        None => {
            let w = room.0.min(room.1 * 1.5);
            (w + chrome.0, w / 1.5 + chrome.1)
        }
    };
    (
        (w.round() as u32).clamp(MIN_SIZE.0, most.0.max(MIN_SIZE.0 as f32) as u32),
        (h.round() as u32).clamp(MIN_SIZE.1, most.1.max(MIN_SIZE.1 as f32) as u32),
    )
}

/// Whether there is a Hyprland to ask. The same test `hyprctl` makes for
/// itself.
pub fn on_hyprland() -> bool {
    std::env::var_os("HYPRLAND_INSTANCE_SIGNATURE").is_some()
}

/// The focused monitor's usable area, from `hyprctl monitors -j`.
pub fn focused_monitor() -> Result<Usable, String> {
    let out = hyprctl(&["-j", "monitors"])?;
    let monitors: Vec<serde_json::Value> =
        serde_json::from_slice(&out).map_err(|e| format!("hyprctl monitors: {e}"))?;
    let monitor = monitors
        .iter()
        .find(|m| m["focused"].as_bool() == Some(true))
        .or(monitors.first())
        .ok_or("hyprctl reported no monitors")?;
    Ok(usable_of(monitor))
}

/// A monitor object's usable logical area. `reserved` is left, top,
/// right, bottom, in logical pixels already.
fn usable_of(monitor: &serde_json::Value) -> Usable {
    let scale = monitor["scale"].as_f64().filter(|s| *s > 0.0).unwrap_or(1.0) as f32;
    let (w, h) = (monitor["width"].as_f64().unwrap_or(1280.0) as f32, monitor["height"].as_f64().unwrap_or(800.0) as f32);
    // A monitor turned sideways reports its mode, not what you see.
    let transform = monitor["transform"].as_u64().unwrap_or(0);
    let (w, h) = if transform % 2 == 1 { (h, w) } else { (w, h) };
    let reserved: Vec<f32> = monitor["reserved"]
        .as_array()
        .map(|r| r.iter().map(|v| v.as_f64().unwrap_or(0.0) as f32).collect())
        .unwrap_or_default();
    let r = |i: usize| reserved.get(i).copied().unwrap_or(0.0);
    Usable { width: (w / scale - r(0) - r(2)).max(1.0), height: (h / scale - r(1) - r(3)).max(1.0) }
}

/// Floats this process's window, gives it `size` and centres it.
///
/// Waits, bounded, for the window to appear in `hyprctl clients` first:
/// iced reports its window before Hyprland has mapped it, and a dispatch
/// aimed at a pid with no window yet does nothing and says "ok".
pub fn float_self(size: (u32, u32)) -> Result<(), String> {
    let window = format!("pid:{}", std::process::id());
    wait_for_own_window(Duration::from_secs(3))?;
    dispatch(&format!("hl.dsp.window.float({{ window = \"{window}\", action = \"set\" }})"))?;
    resize_self(size)
}

/// Gives this process's (already floating) window `size`, and centres it.
pub fn resize_self(size: (u32, u32)) -> Result<(), String> {
    let window = format!("pid:{}", std::process::id());
    dispatch(&format!("hl.dsp.window.resize({{ window = \"{window}\", x = {}, y = {} }})", size.0, size.1))?;
    dispatch(&format!("hl.dsp.window.center({{ window = \"{window}\" }})"))
}

fn wait_for_own_window(limit: Duration) -> Result<(), String> {
    let pid = std::process::id() as u64;
    let start = std::time::Instant::now();
    loop {
        let out = hyprctl(&["-j", "clients"])?;
        let clients: Vec<serde_json::Value> = serde_json::from_slice(&out).unwrap_or_default();
        if clients.iter().any(|c| c["pid"].as_u64() == Some(pid)) {
            return Ok(());
        }
        if start.elapsed() > limit {
            return Err("the window never appeared in hyprctl clients".to_string());
        }
        std::thread::sleep(Duration::from_millis(50));
    }
}

fn dispatch(call: &str) -> Result<(), String> {
    let out = hyprctl(&["dispatch", call])?;
    let said = String::from_utf8_lossy(&out);
    if said.trim() == "ok" {
        Ok(())
    } else {
        Err(format!("hyprctl dispatch {call}: {}", said.trim()))
    }
}

fn hyprctl(args: &[&str]) -> Result<Vec<u8>, String> {
    let out = hyprforge_process::output(Command::new("hyprctl").args(args), hyprforge_process::TIMEOUT)
        .map_err(|e| format!("hyprctl {}: {e}", args.join(" ")))?;
    if !out.status.success() {
        return Err(format!("hyprctl {} failed: {}", args.join(" "), String::from_utf8_lossy(&out.stderr).trim()));
    }
    Ok(out.stdout)
}

#[cfg(test)]
mod tests {
    use super::*;

    const LAPTOP: Usable = Usable { width: 1600.0, height: 970.0 };
    const CHROME: (f32, f32) = (0.0, 160.0);

    /// A photograph from a camera is far bigger than the screen: the
    /// window is most of the screen, with the picture's own shape.
    #[test]
    fn a_big_photo_gets_most_of_the_screen_in_its_own_shape() {
        let (w, h) = floating_size(Some((6000, 4000)), LAPTOP, CHROME);
        assert!(w as f32 <= 1600.0 * MOST_OF_WIDTH + 0.5 && h as f32 <= 970.0 * MOST_OF_HEIGHT + 0.5, "{w}x{h}");
        let picture_h = h as f32 - CHROME.1;
        assert!((w as f32 / picture_h - 1.5).abs() < 0.01, "{w}x{h} is not 3:2 around the picture");
    }

    /// A small picture opens at its size, not blown up into a blur.
    #[test]
    fn a_small_picture_is_never_shown_past_its_own_size() {
        let (w, h) = floating_size(Some((800, 500)), LAPTOP, CHROME);
        assert_eq!((w, h), (800, 660));
    }

    /// A tiny icon still gets a window the controls fit in.
    #[test]
    fn a_tiny_picture_still_gets_the_minimum_window() {
        assert_eq!(floating_size(Some((32, 32)), LAPTOP, CHROME), MIN_SIZE);
    }

    #[test]
    fn a_portrait_photo_opens_tall_and_narrow() {
        let (w, h) = floating_size(Some((3024, 4032)), LAPTOP, CHROME);
        assert!(h > w, "{w}x{h}");
    }

    #[test]
    fn a_picture_that_could_not_be_measured_gets_a_calm_default() {
        let (w, h) = floating_size(None, LAPTOP, CHROME);
        assert!(w > h && w >= MIN_SIZE.0 && h >= MIN_SIZE.1, "{w}x{h}");
    }

    /// The monitor's usable area is logical and leaves out the bar —
    /// otherwise a 1.6-scale laptop would be offered a window bigger than
    /// its screen.
    #[test]
    fn the_usable_area_is_logical_and_leaves_out_the_bar() {
        let m = serde_json::json!({
            "width": 2560, "height": 1600, "scale": 1.6, "transform": 0,
            "reserved": [0, 30, 0, 0]
        });
        assert_eq!(usable_of(&m), Usable { width: 1600.0, height: 970.0 });
        let sideways = serde_json::json!({ "width": 2560, "height": 1440, "scale": 1.0, "transform": 1, "reserved": [0, 0, 0, 0] });
        assert_eq!(usable_of(&sideways), Usable { width: 1440.0, height: 2560.0 });
    }
}
