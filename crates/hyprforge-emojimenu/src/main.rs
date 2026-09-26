//! An emoji picker popup that appears where the mouse is, shows a grid
//! filtered by whatever is typed, and exits once something is picked (or
//! cancelled).
//!
//! Built the same way as `hyprforge-clipmenu`: a short-lived,
//! per-invocation process bound to a keybind (`Meta+.`, in the owner's
//! own config), not a daemon — see that crate's own module doc for the
//! two reasons this shape matters (a window manager's own concerns about
//! staying out of the way, and never leaking
//! `KeyboardInteractivity::Exclusive` stuck open). Everything Wayland/
//! iced-shaped lives in `hyprforge-popup`; everything specific to *this*
//! popup — the grid, the search filter, skin tones — lives in this
//! crate's own modules, plugged into `hyprforge-popup::PopupApp` through
//! `popup_app::EmojiApp`.

mod chooser;
mod config;
mod extras;
mod geometry;
mod model;
mod popup_app;
mod target;
mod view;

use geometry::Layout;
use hyprforge_popup::geometry::Size;
use model::{Model, Tab};
use popup_app::{ChoiceOutcome, EmojiApp};

/// Hands every tab its grid shape from `layout` — the same `Layout` the
/// view draws and the hit-test measures, never a second set of numbers.
fn apply_layout(model: &mut Model, layout: &Layout) {
    for tab in Tab::ALL {
        model.set_geometry(tab, layout.grid_geometry(tab));
    }
}

/// The name this popup's single-instance lock is filed under — see
/// `hyprforge_popup::singleton`'s own doc for why a name rather than a
/// shared lock: opening the emoji picker must not be refused because the
/// clipboard popup happens to be open, or vice versa, so each popup gets
/// its own name.
const LOCK_NAME: &str = "hyprforge-emojimenu.lock";


fn main() -> std::process::ExitCode {
    // Only one instance at a time — a keybind pressed twice while one is
    // already open must leave the first alone.
    let lock_path = hyprforge_popup::singleton::lock_path(LOCK_NAME);
    let _lock = match hyprforge_popup::singleton::acquire(&lock_path) {
        Ok(Some(lock)) => Some(lock),
        Ok(None) => return std::process::ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("couldn't set up the single-instance lock ({e}) — continuing anyway");
            None
        }
    };

    // Read *before* this popup's own layer surface exists and steals
    // keyboard focus for itself — see `target::focused_window`'s own
    // doc, and `hyprforge-clipmenu::main`'s identical ordering.
    let focused = target::focused_window();
    let paste_shortcut =
        focused.as_ref().map(|w| target::paste_shortcut(&w.class)).unwrap_or(hyprforge_clipboard::Shortcut::CtrlV);
    let paste_target_label = focused.as_ref().map(|w| target::display_name(&w.class));

    let monitors = hyprforge_popup::monitors();
    if monitors.is_empty() {
        eprintln!("couldn't read any monitors from hyprctl — is Hyprland running?");
        return std::process::ExitCode::FAILURE;
    }
    // Resolved before placing: the popup's height follows the theme's
    // font (see `geometry::Layout::for_font_size`), and placement needs
    // the real size to keep the whole popup on screen.
    let mut theme = hyprforge_appearance::look::resolve();
    theme.font_size = theme.drawable_font_size();
    let layout = Layout::for_font_size(theme.font_size);

    let popup_size = Size { width: layout.width, height: layout.height };
    let Some(placement) = hyprforge_popup::place(&monitors, hyprforge_popup::cursor_position(), popup_size) else {
        eprintln!("couldn't work out where to place the popup");
        return std::process::ExitCode::FAILURE;
    };

    // A missing file is first run — defaults, nothing said. A file that
    // exists and will not parse is worth a word on stderr but not a
    // reason to refuse to open; and it is never saved over, because the
    // defaults this picker falls back to would replace whatever the user
    // had with nothing. See `config`'s module doc.
    let (prefs, writable) = match config::load() {
        config::Stored::Fresh => (config::Prefs::default(), true),
        config::Stored::Found(prefs) => (prefs, true),
        config::Stored::Unreadable(reason) => {
            eprintln!("couldn't read the emoji picker's saved settings ({reason}) — using defaults, and not saving over the file");
            (config::Prefs::default(), false)
        }
    };

    let mut model = Model::new(prefs.tone, &prefs.frequent);
    model.set_paste_target(paste_target_label);
    apply_layout(&mut model, &layout);

    let connection = match hyprforge_popup::Connection::connect_to_env() {
        Ok(connection) => connection,
        Err(e) => {
            eprintln!("couldn't connect to the compositor: {e}");
            return std::process::ExitCode::FAILURE;
        }
    };

    let chooser = match chooser::Wired::connect() {
        Ok(chooser) => chooser,
        Err(e) => {
            eprintln!("couldn't set up pasting: {e}");
            return std::process::ExitCode::FAILURE;
        }
    };

    let app = EmojiApp::new(model, chooser, paste_shortcut, prefs, writable);

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

    /// The regression test `hyprforge-clipmenu::main` pins for its own
    /// list, adapted to a grid: the model's rows have to be as many as
    /// the layout's grid really shows — derived, wired exactly the way
    /// `main` wires it — not a separate number that can drift.
    #[test]
    fn the_models_grid_is_derived_from_the_popups_actual_layout() {
        let layout = Layout::for_font_size(15.0);
        let mut model = Model::new(None, &[]);
        apply_layout(&mut model, &layout);

        let first_row = model.lines().iter().find_map(|l| match *l {
            model::Line::Cells { start, end, .. } => Some(end - start),
            _ => None,
        });
        assert_eq!(first_row, Some(layout.columns(Tab::Emoji)), "a row is as wide as the layout's columns");

        let fit = (layout.grid.height / (layout.cell + layout.row_gap(Tab::Emoji))).floor() as usize;
        let built = model.stack().visible(0.0, layout.grid.height).len();
        assert!(built >= fit && built <= fit + 2, "{built} lines built for about {fit} that fit");
    }

    #[test]
    fn a_non_finite_font_size_falls_back_rather_than_panicking_the_renderer() {
        let theme = hyprforge_look::Theme { font_size: f32::NAN, ..hyprforge_look::Theme::default() };
        assert_eq!(theme.drawable_font_size(), 15.0);
        let theme = hyprforge_look::Theme { font_size: 0.0, ..hyprforge_look::Theme::default() };
        assert!(theme.drawable_font_size() >= 6.0);
    }
}
