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
mod geometry;
mod model;
mod popup_app;
mod target;
mod view;

use geometry::GridLayout;
use hyprforge_popup::geometry::Size;
use model::Model;
use popup_app::{ChoiceOutcome, EmojiApp};

/// The popup's fixed size in logical pixels — a plain, roughly-square
/// picker, wide enough for a comfortable number of columns and tall
/// enough to show several rows without a monitor's own screen height
/// mattering.
const POPUP_WIDTH: f64 = 360.0;
const POPUP_HEIGHT: f64 = 420.0;

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
    let popup_size = Size { width: POPUP_WIDTH, height: POPUP_HEIGHT };
    let Some(placement) = hyprforge_popup::place(&monitors, hyprforge_popup::cursor_position(), popup_size) else {
        eprintln!("couldn't work out where to place the popup");
        return std::process::ExitCode::FAILURE;
    };

    // A missing config file is first-run and means the neutral tone —
    // never an error. A file that exists but will not parse is a
    // problem worth a word on stderr, but still not a reason to refuse
    // to open — see `config::ToneSetting`'s own doc for why the two
    // must never read the same to a caller.
    let default_tone = match config::load() {
        config::ToneSetting::Neutral => None,
        config::ToneSetting::Tone(tone) => Some(tone),
        config::ToneSetting::Unreadable(reason) => {
            eprintln!("couldn't read the saved default skin tone ({reason}) — using the neutral tone");
            None
        }
    };

    let mut theme = hyprforge_appearance::look::resolve();
    theme.font_size = theme.drawable_font_size();

    let mut model = Model::new(default_tone);
    model.set_paste_target(paste_target_label);

    // The grid's own shape, derived from the popup's fixed size at this
    // theme's font size — the same discipline
    // `hyprforge-clipmenu::main`'s own regression test pins for its row
    // window: the number of cells the model is allowed to build has to
    // be derived from what actually fits, never a separate hardcoded
    // number that can silently drift out of step with it.
    let grid = GridLayout::for_font_size(theme.font_size);
    model.set_grid(grid.columns(POPUP_WIDTH), grid.viewport_height(POPUP_HEIGHT), grid.row_stride(), grid.spacing);

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

    let app = EmojiApp::new(model, chooser, paste_shortcut, POPUP_WIDTH);

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
    /// row window, adapted to a grid: the number of cells `Model` is
    /// allowed to show has to be derived from the popup's actual fixed
    /// size, not a separate hardcoded number.
    #[test]
    fn the_models_grid_is_derived_from_the_popups_actual_size() {
        let grid = GridLayout::for_font_size(15.0);
        let columns = grid.columns(POPUP_WIDTH);
        let rows = grid.rows_that_fit(POPUP_HEIGHT);

        let mut model = Model::new(None);
        model.set_grid(columns, grid.viewport_height(POPUP_HEIGHT), grid.row_stride(), grid.spacing);

        // At least as many cells as the whole rows that fully fit — a
        // pixel viewport can now show one further partially-visible row
        // too (see `Model::visible_range`'s own doc), so this is no
        // longer required to be an exact upper bound.
        let visible = model.visible_range();
        assert!(visible.len() >= rows.saturating_sub(1) * columns);

        // Every whole row `rows_that_fit` claims fits must actually fit
        // inside the popup.
        let stride = grid.cell_size + grid.spacing;
        let last_row_bottom = grid.padding + grid.header_height + (rows as f64 - 1.0) * stride + grid.cell_size;
        assert!(last_row_bottom <= POPUP_HEIGHT - grid.padding);
        let last_column_right = grid.padding + (columns as f64) * stride - grid.spacing;
        assert!(last_column_right <= POPUP_WIDTH - grid.padding);
    }

    #[test]
    fn a_non_finite_font_size_falls_back_rather_than_panicking_the_renderer() {
        let theme = hyprforge_look::Theme { font_size: f32::NAN, ..hyprforge_look::Theme::default() };
        assert_eq!(theme.drawable_font_size(), 15.0);
        let theme = hyprforge_look::Theme { font_size: 0.0, ..hyprforge_look::Theme::default() };
        assert!(theme.drawable_font_size() >= 6.0);
    }
}
