//! What a keypress or a click *means* for the emoji picker, and the
//! [`EmojiApp`] that plugs that into `hyprforge-popup`'s generic
//! layer-shell machinery — the grid's counterpart to
//! `hyprforge-clipmenu::surface::ClipApp`. Named `popup_app` rather than
//! `surface` only so nothing suggests this crate's module means the same
//! thing `hyprforge-clipmenu::surface` does beyond "the `PopupApp`
//! implementation lives here" — the role is identical, the content is
//! not.
//!
//! # The keyboard equivalent of a long press
//!
//! A keyboard user cannot hold the pointer down, so
//! [`LONG_PRESS_DURATION`]'s whole mechanism needs a keyboard equivalent
//! — the task asks this to be a deliberate choice, not an omission.
//! **Tab** is it: [`dispatch_key`] recognises it as "open the tone strip
//! for whatever is currently selected", the same thing a long press does
//! for whatever is currently under the pointer. Tab means nothing else
//! in this popup — there is exactly one field to type into and no second
//! control to move focus between, unlike a form with several fields
//! where Tab's ordinary job (move focus forward) would collide with
//! this — and it costs nothing new to recognise: `dispatch_key` already
//! matches specific keysyms ahead of the "everything else is typed
//! text" fallback, the same way `hyprforge-clipmenu` reserves F2 for
//! pinning. If the currently selected cell has no tone variants, Tab is
//! a no-op — the same answer a long press gives for the identical case
//! (see [`hyprforge_popup::PopupApp::pointer_long_press`]'s own doc).

use crate::chooser::Chooser;
use crate::geometry::{GridLayout, ToneStrip};
use crate::model::Model;
use hyprforge_look::Theme;
use hyprforge_popup::Keysym;
use iced_runtime::core::Element;
use std::convert::Infallible;
use std::time::Duration;

/// However the popup ended, as far as this crate's own logic is
/// concerned.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ChoiceOutcome {
    /// An emoji was put on the clipboard successfully.
    Chosen,
    /// Escape closed the popup with nothing left to clear first, or the
    /// [`Chooser`] failed and there was nothing more useful to do than
    /// close.
    Cancelled,
}

pub use hyprforge_clipboard::Shortcut;

/// How long the pointer must stay down over a tone-capable cell before
/// it counts as a long press rather than an ordinary click.
///
/// Chosen for the same reason Android and iOS both land in roughly this
/// neighbourhood for their own long-press gestures: short enough that
/// opening the tone strip does not feel like a separate, deliberate
/// wait bolted onto a tap, long enough that an ordinary quick click —
/// even a slightly unsteady one — never opens it by accident. Unlike
/// `hyprforge_popup::popup::FOCUS_RELEASE_TIMEOUT`, there is no
/// measurement behind this exact figure; it is a UX judgement call, not
/// a bound on how long a syscall or a compositor round trip is allowed
/// to take, which is why the task asks for it to be named and explained
/// rather than left as a bare literal.
pub const LONG_PRESS_DURATION: Duration = Duration::from_millis(450);

/// The emoji picker's own [`hyprforge_popup::PopupApp`]: a live [`Model`]
/// and the [`Chooser`] seam that reaches the clipboard.
pub struct EmojiApp<C: Chooser> {
    model: Model,
    chooser: C,
    shortcut: Shortcut,
    /// The popup's own width, needed because the grid is centred in it:
    /// `GridLayout::left_margin` decides where the first column starts,
    /// and the hit-test has to measure from the same place the drawing
    /// does. See that function's doc for why this is not an `align_x`.
    width: f64,
}

impl<C: Chooser> EmojiApp<C> {
    pub fn new(model: Model, chooser: C, shortcut: Shortcut, width: f64) -> EmojiApp<C> {
        EmojiApp { model, chooser, shortcut, width }
    }
}

impl<C: Chooser + 'static> hyprforge_popup::PopupApp for EmojiApp<C> {
    type Outcome = ChoiceOutcome;

    fn view<'a>(&'a mut self, theme: &'a Theme, _now: u64, width: f64) -> Element<'a, Infallible, iced_widget::Theme, iced_tiny_skia::Renderer> {
        crate::view::view(&self.model, theme, width)
    }

    fn rows_that_fit(&self, theme: &Theme, height: f64) -> usize {
        GridLayout::for_font_size(theme.font_size).rows_that_fit(height)
    }

    /// While the tone strip is open it is the only thing hoverable — the
    /// grid underneath is fixed in place and modal to the strip, the
    /// same "click elsewhere dismisses it" rule [`Self::pointer_click`]
    /// applies to a click.
    fn pointer_move(&mut self, theme: &Theme, position: (f64, f64)) -> bool {
        if self.model.tone_overlay().is_some() {
            let strip = ToneStrip::for_font_size(theme.font_size, &GridLayout::for_font_size(theme.font_size));
            return match strip.tone_at(position) {
                Some(index) => {
                    dispatch_action(&mut self.model, &self.chooser, Action::HoverTone(index));
                    true
                }
                None => false,
            };
        }

        let grid = GridLayout::for_font_size(theme.font_size);
        let range = self.model.visible_range();
        let columns = self.model.columns();
        match grid.cell_at(position, self.width, columns, range.len()) {
            Some(local) => {
                dispatch_action(&mut self.model, &self.chooser, Action::Select(range.start + local));
                true
            }
            None => false,
        }
    }

    /// Re-hit-tests at the click position rather than trusting the last
    /// hover — the same discipline
    /// `hyprforge-clipmenu::surface::ClipApp::pointer_click`'s own doc
    /// explains, for the identical reason: a click landing between a
    /// hover event and a redraw must still be honest about what it is
    /// actually over.
    fn pointer_click(&mut self, theme: &Theme, _width: f64, position: (f64, f64)) -> Option<ChoiceOutcome> {
        let grid = GridLayout::for_font_size(theme.font_size);

        if self.model.tone_overlay().is_some() {
            let strip = ToneStrip::for_font_size(theme.font_size, &grid);
            return match strip.tone_at(position) {
                Some(index) => dispatch_action(
                    &mut self.model,
                    &self.chooser,
                    Action::ChooseTone(hyprforge_emoji::TONES[index]),
                ),
                // Anywhere outside the strip's own rectangle dismisses
                // it without picking anything, and without also acting
                // on whatever grid cell happens to be underneath — the
                // strip is modal while it is open.
                None => dispatch_action(&mut self.model, &self.chooser, Action::CloseToneOverlay),
            };
        }

        let range = self.model.visible_range();
        let columns = self.model.columns();
        let local = grid.cell_at(position, self.width, columns, range.len())?;
        dispatch_action(&mut self.model, &self.chooser, Action::Select(range.start + local));
        dispatch_action(&mut self.model, &self.chooser, Action::Choose)
    }

    /// Moved through the same [`Action::Move`] the arrow keys use, one
    /// whole grid row (`columns` cells) per notch — a grid scrolls in
    /// rows, the same unit `Model::move_up`/`move_down` already work in,
    /// rather than panning the view independently of the selection; see
    /// `Model`'s own module doc for why the two are the same thing here.
    fn pointer_scroll(&mut self, rows: i32) {
        if self.model.tone_overlay().is_some() {
            return;
        }
        let columns = self.model.columns() as i32;
        dispatch_action(&mut self.model, &self.chooser, Action::Move(rows * columns));
    }

    fn key(&mut self, keysym: Keysym, utf8: Option<String>) -> Option<ChoiceOutcome> {
        dispatch_key(&mut self.model, &self.chooser, keysym, utf8)
    }

    fn long_press_duration(&self) -> Option<Duration> {
        Some(LONG_PRESS_DURATION)
    }

    /// Re-hit-tests at `position` (the press's own location) rather than
    /// trusting the current selection, for the same reason
    /// [`Self::pointer_click`] does.
    fn pointer_long_press(&mut self, theme: &Theme, position: (f64, f64)) -> bool {
        let grid = GridLayout::for_font_size(theme.font_size);
        let range = self.model.visible_range();
        let columns = self.model.columns();
        let Some(local) = grid.cell_at(position, self.width, columns, range.len()) else { return false };
        self.model.select(range.start + local);
        self.model.open_tone_overlay()
    }

    fn needs_finish(outcome: ChoiceOutcome) -> bool {
        outcome == ChoiceOutcome::Chosen
    }

    /// Runs only after `hyprforge-popup` has torn this popup's own
    /// surface down and proven the compositor processed that — see
    /// `hyprforge_popup::PopupApp::finish`'s own doc. `Action::Choose`/
    /// `Action::ChooseTone` (see [`dispatch_action`]) only put the emoji
    /// on the clipboard; synthesizing the paste is this method's job
    /// alone.
    fn finish(&mut self, outcome: ChoiceOutcome) {
        if outcome == ChoiceOutcome::Chosen {
            self.chooser.finish_paste(self.shortcut);
        }
    }
}

/// What the user asked the popup to do, independent of whether a key or
/// a pointer produced it — the same seam
/// `hyprforge-clipmenu::surface::Action` is, extended with the two
/// grid-specific and tone-specific actions this picker needs.
#[derive(Debug, Clone, Copy, PartialEq)]
enum Action {
    /// Move the linear selection by this many cells (negative is back) —
    /// what the wheel dispatches through, a whole row (`columns` cells)
    /// at a time. The arrow keys go through
    /// [`Action::MoveLeft`]/[`Action::MoveRight`]/[`Action::MoveUp`]/
    /// [`Action::MoveDown`] instead, so each direction's own meaning
    /// (one cell along a row versus one whole row) lives once, in
    /// [`Model`] itself, rather than being recomputed here too.
    Move(i32),
    MoveLeft,
    MoveRight,
    MoveUp,
    MoveDown,
    /// Select exactly this cell of the filtered grid — a hover, or a
    /// click that already resolved to a cell.
    Select(usize),
    /// Insert whichever glyph the selected cell is currently displaying
    /// (the user's default tone, or the plain glyph) — Enter, or a
    /// click.
    Choose,
    /// Open the tone strip for the selected cell — Tab. A no-op if that
    /// cell has no tone variants.
    OpenToneOverlay,
    /// Close the tone strip without picking anything — Escape while it
    /// is open, or a click outside its own rectangle.
    CloseToneOverlay,
    /// Move the strip's own highlighted cell — Left/Right while it is
    /// open.
    MoveTone(i32),
    /// Hover highlights a specific tone cell.
    HoverTone(usize),
    /// Pick the strip's currently highlighted tone (or, from a click,
    /// whichever cell was hit directly) — Enter while the strip is
    /// open, or a click on one of its cells. Sets the picked tone as the
    /// new persisted default, per the task's own framing.
    ChooseTone(hyprforge_emoji::Tone),
    /// The first Escape with a filter (or the strip) open — clears it
    /// rather than closing the popup.
    ClearFilter,
    /// The second Escape, with nothing left to clear.
    Cancel,
    Backspace,
    Type(char),
}

/// The one place any [`Action`] takes effect on a [`Model`] and a
/// [`Chooser`] — the same split
/// `hyprforge-clipmenu::surface::dispatch_action` uses, and for the same
/// reason: every rule about what a selection, a click or a keystroke
/// *means* is testable with no Wayland connection at all.
fn dispatch_action<C: Chooser>(model: &mut Model, chooser: &C, action: Action) -> Option<ChoiceOutcome> {
    match action {
        Action::Cancel => Some(ChoiceOutcome::Cancelled),
        Action::Choose => {
            let entry = model.selected_entry()?;
            let text = model.display_char(&entry);
            match chooser.set_clipboard(text) {
                Ok(()) => Some(ChoiceOutcome::Chosen),
                Err(message) => {
                    eprintln!("couldn't put the chosen emoji on the clipboard: {message}");
                    Some(ChoiceOutcome::Cancelled)
                }
            }
        }
        Action::ChooseTone(tone) => {
            let entry = model.selected_entry()?;
            let text = entry.tone(tone);
            match chooser.set_clipboard(text) {
                Ok(()) => {
                    // The pick itself is also the new default for next
                    // time — see `Model::set_default_tone`'s own doc for
                    // why this crate treats "picked a tone" and "chose
                    // it as the default" as the same act rather than
                    // requiring a separate settings screen for the
                    // second.
                    model.set_default_tone(tone);
                    if let Err(e) = crate::config::save(Some(tone)) {
                        eprintln!("couldn't remember this as the default skin tone: {e}");
                    }
                    model.close_tone_overlay();
                    Some(ChoiceOutcome::Chosen)
                }
                Err(message) => {
                    eprintln!("couldn't put the chosen emoji on the clipboard: {message}");
                    Some(ChoiceOutcome::Cancelled)
                }
            }
        }
        Action::Move(delta) => {
            model.move_by(delta);
            None
        }
        Action::MoveLeft => {
            model.move_left();
            None
        }
        Action::MoveRight => {
            model.move_right();
            None
        }
        Action::MoveUp => {
            model.move_up();
            None
        }
        Action::MoveDown => {
            model.move_down();
            None
        }
        Action::Select(index) => {
            model.select(index);
            None
        }
        Action::OpenToneOverlay => {
            model.open_tone_overlay();
            None
        }
        Action::CloseToneOverlay => {
            model.close_tone_overlay();
            None
        }
        Action::MoveTone(delta) => {
            model.move_tone_cursor(delta);
            None
        }
        Action::HoverTone(index) => {
            model.hover_tone_cursor(index);
            None
        }
        Action::ClearFilter => {
            model.clear_filter();
            None
        }
        Action::Backspace => {
            model.backspace();
            None
        }
        Action::Type(c) => {
            model.type_char(c);
            None
        }
    }
}

/// The keystroke rules: which [`Action`] each key produces, routed
/// through whether the tone strip is currently open — the strip is
/// modal, so the same physical keys mean something different while it
/// is up (see this module's own doc on Tab).
///
/// Nothing typed here is ever logged or inspected beyond being appended
/// to the filter, the same discipline
/// `hyprforge-clipmenu::surface::dispatch_key`'s doc states — an emoji
/// search is not a password, but the rule is followed regardless of what
/// the field means.
fn dispatch_key<C: Chooser>(model: &mut Model, chooser: &C, keysym: Keysym, utf8: Option<String>) -> Option<ChoiceOutcome> {
    if model.tone_overlay().is_some() {
        return match keysym {
            Keysym::Escape | Keysym::Tab => dispatch_action(model, chooser, Action::CloseToneOverlay),
            Keysym::Return | Keysym::KP_Enter => {
                let tone = model.tone_cursor_value().unwrap_or(hyprforge_emoji::Tone::Medium);
                dispatch_action(model, chooser, Action::ChooseTone(tone))
            }
            Keysym::Left => dispatch_action(model, chooser, Action::MoveTone(-1)),
            Keysym::Right => dispatch_action(model, chooser, Action::MoveTone(1)),
            // Up/Down and typing are ignored while the strip is open —
            // there is nothing above or below a one-dimensional strip to
            // move to, and typing while a modal picker is open would
            // both dismiss it and start a new search in the same
            // keystroke, which is more surprising than simply requiring
            // Escape or Tab first.
            _ => None,
        };
    }

    match keysym {
        Keysym::Escape => {
            // Two-stage, per the task: clear the filter first, only
            // close on a second press with nothing left to clear.
            if model.filter_text().is_empty() {
                dispatch_action(model, chooser, Action::Cancel)
            } else {
                dispatch_action(model, chooser, Action::ClearFilter)
            }
        }
        Keysym::Return | Keysym::KP_Enter => dispatch_action(model, chooser, Action::Choose),
        Keysym::Left => dispatch_action(model, chooser, Action::MoveLeft),
        Keysym::Right => dispatch_action(model, chooser, Action::MoveRight),
        Keysym::Up => dispatch_action(model, chooser, Action::MoveUp),
        Keysym::Down => dispatch_action(model, chooser, Action::MoveDown),
        Keysym::BackSpace => dispatch_action(model, chooser, Action::Backspace),
        // The keyboard equivalent of a long press — see this module's
        // own doc comment.
        Keysym::Tab => dispatch_action(model, chooser, Action::OpenToneOverlay),
        _ => {
            if let Some(text) = utf8 {
                for c in text.chars().filter(|c| !c.is_control()) {
                    dispatch_action(model, chooser, Action::Type(c));
                }
            }
            None
        }
    }
}
