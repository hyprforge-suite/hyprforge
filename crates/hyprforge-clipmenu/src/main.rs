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
//!
//! The layer-shell surface, the event loop, pointer/keyboard handling,
//! placement and the single-instance lock all live in `hyprforge-popup`
//! now — see that crate's own module doc for why, and `surface::ClipApp`
//! for the seam this binary plugs a clipboard history into it through.

mod chooser;
mod geometry;
mod model;
mod pinner;
mod surface;
mod target;
mod thumbnail;
mod view;

use geometry::RowLayout;
use hyprforge_popup::geometry::Size;
use model::{HistoryState, Model};
use surface::{ChoiceOutcome, ClipApp};

/// The popup's fixed size in logical pixels. Not configurable yet —
/// there is nowhere for a setting like this to live until the Settings
/// app grows a clipboard tab, and a fixed size is a perfectly ordinary
/// thing for a Windows-style clipboard popup to have.
const POPUP_WIDTH: f64 = 360.0;
const POPUP_HEIGHT: f64 = 420.0;

/// The name this popup's single-instance lock is filed under — see
/// `hyprforge_popup::singleton`'s own doc for why a name, not a shared
/// lock, and why an `flock` rather than a name match or a PID file.
const LOCK_NAME: &str = "hyprforge-clipmenu.lock";

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

fn main() -> std::process::ExitCode {
    // Only one popup at a time: a keybind pressed twice while one is
    // already open must leave the first alone and exit quietly, not
    // start a second process.
    let lock_path = hyprforge_popup::singleton::lock_path(LOCK_NAME);
    let _lock = match hyprforge_popup::singleton::acquire(&lock_path) {
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

    // Read *before* this popup's own layer surface ever exists: once it
    // takes `KeyboardInteractivity::Exclusive`, the focused window *is*
    // this popup, and asking `hyprctl` afterward would only ever answer
    // that. See `target::focused_window`'s own doc.
    let focused = target::focused_window();
    let paste_shortcut =
        focused.as_ref().map(|w| target::paste_shortcut(&w.class)).unwrap_or(hyprforge_clipboard::Shortcut::CtrlV);
    let paste_target_label = focused.as_ref().map(|w| target::display_name(&w.class));

    let monitors = hyprforge_popup::monitors();
    if monitors.is_empty() {
        eprintln!("couldn't read any monitors from hyprctl — is Hyprland running?");
        return std::process::ExitCode::FAILURE;
    }
    let popup_size = Size { width: POPUP_WIDTH, height: POPUP_HEIGHT };
    let Some(placement) = hyprforge_popup::place(&monitors, hyprforge_popup::cursor_position(), popup_size) else {
        eprintln!("couldn't work out where to place the popup");
        return std::process::ExitCode::FAILURE;
    };

    let history = HistoryState::from_result(hyprforge_clipboard::History::load());
    let mut model = Model::new(history);
    model.set_paste_target(paste_target_label);

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

    let connection = match hyprforge_popup::Connection::connect_to_env() {
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

    let app = ClipApp::new(model, chooser, pinner::Wired, paste_shortcut);

    match hyprforge_popup::Popup::run(connection, placement, app, theme) {
        Ok(hyprforge_popup::Outcome::App(ChoiceOutcome::Chosen | ChoiceOutcome::Cancelled)) => {
            std::process::ExitCode::SUCCESS
        }
        Ok(hyprforge_popup::Outcome::Closed | hyprforge_popup::Outcome::Disconnected) => {
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
