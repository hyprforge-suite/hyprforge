//! What a keypress or a click *means* for the emoji picker, and the
//! [`EmojiApp`] that plugs that into `hyprforge-popup`'s generic
//! layer-shell machinery — the grid's counterpart to
//! `hyprforge-clipmenu::surface::ClipApp`.
//!
//! # Keys
//!
//! Tab and Shift+Tab step through the Emoji, Kaomoji and Symbols tabs,
//! the same keys the clipboard popup steps its filter tabs with — one
//! grammar for the suite's two pointer popups. That moved the keyboard
//! route to skin tones off Tab, where it used to be: it is Shift+Enter
//! now, on a cell that has tones. Holding the pointer down on one is the
//! mouse's way; the footer names both.
//!
//! While the tone picker is open it is modal: Left and Right move along
//! it, Enter pastes the highlighted tone, Shift+Enter pastes it *and*
//! makes it the default, and Escape or Tab puts it away. The ✋ button
//! beside the search opens the same picker to set the default without
//! pasting anything.
//!
//! # Remembering
//!
//! A pick is counted towards "Frequently used", and a tone made default
//! is kept. [`dispatch_action`] only changes the in-memory
//! [`config::Prefs`] — so every rule here is testable without touching a
//! file — and [`EmojiApp`] writes it out afterwards, unless the file was
//! unreadable when the picker started (see `crate::config`'s module doc
//! for why saving over it would be the wrong thing to do).

use crate::chooser::Chooser;
use crate::config::{self, Prefs};
use crate::geometry::{Hit, Layout, Picker};
use crate::model::{self, Item, Model, Tab, ToneMode};
use hyprforge_look::Theme;
use hyprforge_popup::{Keysym, Modifiers};
use iced_runtime::core::Element;
use std::convert::Infallible;
use std::time::Duration;

/// However the popup ended, as far as this crate's own logic is
/// concerned.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ChoiceOutcome {
    /// Something was put on the clipboard successfully.
    Chosen,
    /// Escape with nothing left to clear, or the [`Chooser`] failed and
    /// there was nothing more useful to do than close.
    Cancelled,
}

pub use hyprforge_clipboard::Shortcut;

/// How long the pointer must stay down on a tone-capable cell before it
/// counts as holding it rather than clicking it.
///
/// A judgement call rather than a measurement — roughly where Android and
/// iOS land for the same gesture: short enough that opening the tones
/// does not feel like a wait bolted onto a tap, long enough that an
/// ordinary, slightly unsteady click never opens them by accident.
pub const LONG_PRESS_DURATION: Duration = Duration::from_millis(450);

/// The emoji picker's own [`hyprforge_popup::PopupApp`].
pub struct EmojiApp<C: Chooser> {
    model: Model,
    chooser: C,
    shortcut: Shortcut,
    prefs: Prefs,
    /// Whether `prefs` may be written back. `false` when the file existed
    /// and could not be read: see this module's doc.
    writable: bool,
    /// The pointer's y as of the last scrollbar-drag event, `None` when
    /// no drag is in progress.
    drag_last_y: Option<f64>,
}

impl<C: Chooser> EmojiApp<C> {
    pub fn new(model: Model, chooser: C, shortcut: Shortcut, prefs: Prefs, writable: bool) -> EmojiApp<C> {
        EmojiApp { model, chooser, shortcut, prefs, writable, drag_last_y: None }
    }

    fn act(&mut self, action: Action) -> Option<ChoiceOutcome> {
        let before = self.prefs.clone();
        let outcome = dispatch_action(&mut self.model, &self.chooser, &mut self.prefs, action);
        self.persist(&before);
        outcome
    }

    /// Writes the remembered state out if an action changed it — and only
    /// if the file was readable when the picker started.
    fn persist(&self, before: &Prefs) {
        if self.prefs != *before && self.writable {
            if let Err(e) = config::save(&self.prefs) {
                eprintln!("couldn't remember this pick: {e}");
            }
        }
    }

    /// The tone picker's rectangle while it is open — anchored to the
    /// selected cell, or under the ✋ button — from the same numbers the
    /// view draws it with.
    fn picker(&self, layout: &Layout) -> Option<Picker> {
        let picker = self.model.tone_picker()?;
        Some(picker_rect(&self.model, layout, picker.mode))
    }

    fn hit(&self, layout: &Layout, position: (f64, f64)) -> Option<Hit> {
        let offset = self.model.scroll_offset();
        layout.hit(position, self.model.tab(), &self.model.lines(), &self.model.stack(), offset, self.model.sticky_section().is_some())
    }
}

/// Where the tone picker is drawn for `mode` — shared by the view and the
/// hit-test so both use one rectangle.
pub fn picker_rect(model: &Model, layout: &Layout, mode: ToneMode) -> Picker {
    match mode {
        ToneMode::Default => layout.picker(layout.tone_button, true),
        ToneMode::Paste => {
            let anchor = layout
                .cell_rect(model.tab(), &model.lines(), &model.stack(), model.scroll_offset(), model.selected_index())
                .unwrap_or(hyprforge_popup::kit::Rect { x: layout.grid.x, y: layout.grid.y, width: layout.cell, height: layout.cell });
            layout.picker(anchor, false)
        }
    }
}

impl<C: Chooser + 'static> hyprforge_popup::PopupApp for EmojiApp<C> {
    type Outcome = ChoiceOutcome;

    fn view<'a>(&'a mut self, theme: &'a Theme, _now: u64, _width: f64) -> Element<'a, Infallible, iced_widget::Theme, iced_tiny_skia::Renderer> {
        crate::view::view(&self.model, theme)
    }

    fn rows_that_fit(&self, theme: &Theme, _height: f64) -> usize {
        let layout = Layout::for_font_size(theme.font_size);
        (layout.grid.height / (layout.cell + layout.row_gap(Tab::Emoji))).floor() as usize
    }

    /// While the tone picker is open it is the only thing that responds —
    /// it is modal, the same rule [`Self::pointer_click`] applies.
    fn pointer_move(&mut self, theme: &Theme, position: (f64, f64)) -> bool {
        let layout = Layout::for_font_size(theme.font_size);
        if let Some(picker) = self.picker(&layout) {
            return match picker.cell_at(position) {
                Some(cell) => {
                    self.act(Action::HoverTone(cell));
                    true
                }
                None => false,
            };
        }
        match self.hit(&layout, position) {
            Some(Hit::Cell(index)) => {
                self.act(Action::Select(index));
                true
            }
            Some(_) => true,
            None => false,
        }
    }

    /// Re-hit-tests at the click rather than trusting the last hover: a
    /// click landing between a hover and a redraw must still be honest
    /// about what it is over.
    fn pointer_click(&mut self, theme: &Theme, _width: f64, position: (f64, f64)) -> Option<ChoiceOutcome> {
        let layout = Layout::for_font_size(theme.font_size);
        if let Some(picker) = self.picker(&layout) {
            // Outside the picker puts it away without acting on whatever
            // is underneath: it is modal while it is open.
            return match picker.cell_at(position) {
                Some(cell) => self.act(Action::PickTone { cell, remember: false }),
                None => self.act(Action::CloseTonePicker),
            };
        }
        match self.hit(&layout, position)? {
            Hit::Tab(index) => self.act(Action::SetTab(Tab::ALL[index.min(Tab::ALL.len() - 1)])),
            Hit::ToneButton => self.act(Action::OpenDefaultPicker),
            Hit::Cell(index) => {
                self.act(Action::Select(index));
                self.act(Action::Choose)
            }
        }
    }

    /// Scrolls the view, not the selection — one row per wheel notch.
    fn pointer_scroll(&mut self, rows: i32) {
        if self.model.tone_picker().is_none() {
            let stride = self.model.row_stride();
            self.model.scroll_by(rows as f64 * stride);
        }
    }

    fn key(&mut self, keysym: Keysym, utf8: Option<String>, modifiers: Modifiers) -> Option<ChoiceOutcome> {
        let before = self.prefs.clone();
        let outcome = dispatch_key(&mut self.model, &self.chooser, &mut self.prefs, keysym, utf8, modifiers);
        self.persist(&before);
        outcome
    }

    fn long_press_duration(&self) -> Option<Duration> {
        Some(LONG_PRESS_DURATION)
    }

    /// Re-hit-tests at the press's own position, for the same reason
    /// [`Self::pointer_click`] does.
    fn pointer_long_press(&mut self, theme: &Theme, position: (f64, f64)) -> bool {
        let layout = Layout::for_font_size(theme.font_size);
        if self.model.tone_picker().is_some() {
            return false;
        }
        let Some(Hit::Cell(index)) = self.hit(&layout, position) else { return false };
        self.model.select(index);
        self.model.open_tone_picker()
    }

    fn pointer_drag_start(&mut self, theme: &Theme, position: (f64, f64)) -> bool {
        if self.model.tone_picker().is_some() {
            return false;
        }
        let bar = Layout::for_font_size(theme.font_size).scrollbar();
        if bar.hit_thumb(position, self.model.stack().content_height(), self.model.scroll_offset()) {
            self.drag_last_y = Some(position.1);
            true
        } else {
            false
        }
    }

    fn pointer_drag_move(&mut self, theme: &Theme, position: (f64, f64)) {
        let Some(last_y) = self.drag_last_y else { return };
        self.drag_last_y = Some(position.1);
        let bar = Layout::for_font_size(theme.font_size).scrollbar();
        let delta = bar.drag_delta_to_offset_delta(position.1 - last_y, self.model.stack().content_height());
        self.model.scroll_by(delta);
    }

    fn pointer_drag_end(&mut self) {
        self.drag_last_y = None;
    }

    fn needs_finish(outcome: ChoiceOutcome) -> bool {
        outcome == ChoiceOutcome::Chosen
    }

    /// Runs only after `hyprforge-popup` has torn this popup's surface
    /// down and proven the compositor processed that — see
    /// `hyprforge_popup::PopupApp::finish`. Choosing only put the text on
    /// the clipboard; synthesizing the paste is this method's job alone.
    fn finish(&mut self, outcome: ChoiceOutcome) {
        if outcome == ChoiceOutcome::Chosen {
            self.chooser.finish_paste(self.shortcut);
        }
    }
}

/// What the user asked for, whether a key or the pointer produced it.
#[derive(Debug, Clone, Copy, PartialEq)]
enum Action {
    MoveLeft,
    MoveRight,
    MoveUp,
    MoveDown,
    /// Select exactly this item — a hover, or a click.
    Select(usize),
    /// Paste what the selected cell shows — Enter, or a click.
    Choose,
    /// Open the tone picker on the selected cell — Shift+Enter, or a
    /// hold. Chooses instead when the cell has no tones, so Shift+Enter
    /// is never a dead key.
    OpenTonePicker,
    /// Open the tone picker to set the default — the ✋ button.
    OpenDefaultPicker,
    CloseTonePicker,
    MoveTone(i32),
    HoverTone(usize),
    /// Pick picker cell `cell` (0 is neutral). In paste mode this pastes
    /// that tone, and `remember` also makes it the default; in default
    /// mode it only sets the default.
    PickTone { cell: usize, remember: bool },
    SetTab(Tab),
    StepTab(bool),
    /// The first Escape with a search typed — clears it rather than
    /// closing the popup.
    ClearFilter,
    /// Escape with nothing left to clear.
    Cancel,
    Backspace,
    Type(char),
}

/// The one place any [`Action`] takes effect — testable end to end
/// against a mock [`Chooser`] and an in-memory [`Prefs`], with no
/// Wayland connection and no file.
fn dispatch_action<C: Chooser>(model: &mut Model, chooser: &C, prefs: &mut Prefs, action: Action) -> Option<ChoiceOutcome> {
    match action {
        Action::Cancel => Some(ChoiceOutcome::Cancelled),
        Action::Choose => {
            let item = model.selected_item()?;
            paste(chooser, prefs, item, model.display(item))
        }
        Action::OpenTonePicker => {
            if model.open_tone_picker() {
                None
            } else {
                dispatch_action(model, chooser, prefs, Action::Choose)
            }
        }
        Action::OpenDefaultPicker => {
            model.open_default_picker();
            None
        }
        Action::PickTone { cell, remember } => {
            let picker = model.tone_picker()?;
            let tone = model::cell_tone(cell);
            match picker.mode {
                ToneMode::Default => {
                    model.set_default_tone(tone);
                    prefs.tone = tone;
                    model.close_tone_picker();
                    None
                }
                ToneMode::Paste => {
                    let glyph = model.tone_glyphs()?[cell.min(model::TONE_CELLS - 1)];
                    let item = model.selected_item()?;
                    if remember {
                        model.set_default_tone(tone);
                        prefs.tone = tone;
                    }
                    model.close_tone_picker();
                    paste(chooser, prefs, item, glyph)
                }
            }
        }
        Action::CloseTonePicker => {
            model.close_tone_picker();
            None
        }
        Action::MoveTone(delta) => {
            model.move_tone_cursor(delta);
            None
        }
        Action::HoverTone(cell) => {
            model.hover_tone_cursor(cell);
            None
        }
        Action::MoveLeft => {
            model.move_by(-1);
            None
        }
        Action::MoveRight => {
            model.move_by(1);
            None
        }
        Action::MoveUp => {
            model.move_row(false);
            None
        }
        Action::MoveDown => {
            model.move_row(true);
            None
        }
        Action::Select(index) => {
            model.select(index);
            None
        }
        Action::SetTab(tab) => {
            model.set_tab(tab);
            None
        }
        Action::StepTab(forward) => {
            model.set_tab(model.tab().next(forward));
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

/// Puts `text` on the clipboard and, for an emoji, counts the pick —
/// against its neutral glyph, so every tone of one emoji counts together.
/// A pick that never reached the clipboard is not counted.
fn paste<C: Chooser>(chooser: &C, prefs: &mut Prefs, item: Item, text: &str) -> Option<ChoiceOutcome> {
    match chooser.set_clipboard(text) {
        Ok(()) => {
            if let Item::Emoji(emoji) = item {
                prefs.count(emoji.emoji);
            }
            Some(ChoiceOutcome::Chosen)
        }
        Err(message) => {
            eprintln!("couldn't put the chosen text on the clipboard: {message}");
            Some(ChoiceOutcome::Cancelled)
        }
    }
}

/// The keystroke rules. Nothing typed is ever logged or inspected beyond
/// being appended to the search. Chords are recognised before typing, and
/// a held Ctrl, Alt or Super never types into the search at all.
fn dispatch_key<C: Chooser>(
    model: &mut Model,
    chooser: &C,
    prefs: &mut Prefs,
    keysym: Keysym,
    utf8: Option<String>,
    modifiers: Modifiers,
) -> Option<ChoiceOutcome> {
    if let Some(picker) = model.tone_picker() {
        let action = match keysym {
            Keysym::Escape | Keysym::Tab | Keysym::ISO_Left_Tab => Action::CloseTonePicker,
            Keysym::Return | Keysym::KP_Enter => Action::PickTone { cell: picker.cursor, remember: modifiers.shift },
            Keysym::Left => Action::MoveTone(-1),
            Keysym::Right => Action::MoveTone(1),
            // Up/Down and typing do nothing while the picker is open:
            // typing would both dismiss it and start a search in one
            // keystroke, which is more surprising than asking for Escape.
            _ => return None,
        };
        return dispatch_action(model, chooser, prefs, action);
    }

    let action = match keysym {
        Keysym::Escape if model.filter_text().is_empty() => Action::Cancel,
        Keysym::Escape => Action::ClearFilter,
        Keysym::Return | Keysym::KP_Enter if modifiers.shift => Action::OpenTonePicker,
        Keysym::Return | Keysym::KP_Enter => Action::Choose,
        Keysym::Left => Action::MoveLeft,
        Keysym::Right => Action::MoveRight,
        Keysym::Up => Action::MoveUp,
        Keysym::Down => Action::MoveDown,
        Keysym::BackSpace => Action::Backspace,
        Keysym::Tab => Action::StepTab(!modifiers.shift),
        Keysym::ISO_Left_Tab => Action::StepTab(false),
        _ if modifiers.ctrl || modifiers.alt || modifiers.logo => return None,
        _ => {
            if let Some(text) = utf8 {
                for c in text.chars().filter(|c| !c.is_control()) {
                    dispatch_action(model, chooser, prefs, Action::Type(c));
                }
            }
            return None;
        }
    };
    dispatch_action(model, chooser, prefs, action)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::chooser::mock::MockChooser;
    use hyprforge_emoji::Tone;

    const NONE: Modifiers = Modifiers { ctrl: false, alt: false, shift: false, caps_lock: false, logo: false, num_lock: false };
    const SHIFT: Modifiers = Modifiers { shift: true, ..NONE };
    const CTRL: Modifiers = Modifiers { ctrl: true, ..NONE };

    struct Rig {
        model: Model,
        chooser: MockChooser,
        prefs: Prefs,
    }

    impl Rig {
        fn new() -> Rig {
            Rig { model: Model::new(None, &[]), chooser: MockChooser::succeeding(), prefs: Prefs::default() }
        }

        fn key(&mut self, keysym: Keysym, modifiers: Modifiers) -> Option<ChoiceOutcome> {
            dispatch_key(&mut self.model, &self.chooser, &mut self.prefs, keysym, None, modifiers)
        }

        fn typed(&mut self, text: &str) {
            for c in text.chars() {
                dispatch_key(&mut self.model, &self.chooser, &mut self.prefs, Keysym::NoSymbol, Some(c.to_string()), NONE);
            }
        }

        fn act(&mut self, action: Action) -> Option<ChoiceOutcome> {
            dispatch_action(&mut self.model, &self.chooser, &mut self.prefs, action)
        }

        fn select(&mut self, name: &str) {
            let i = self.model.items().iter().position(|i| matches!(i, Item::Emoji(e) if e.name == name)).unwrap();
            self.model.select(i);
        }

        fn pasted(&self) -> Vec<String> {
            self.chooser.calls.borrow().clone()
        }
    }

    #[test]
    fn enter_pastes_what_the_cell_shows_and_ends_the_popup() {
        let mut rig = Rig::new();
        rig.select("fire");
        assert_eq!(rig.key(Keysym::Return, NONE), Some(ChoiceOutcome::Chosen));
        assert_eq!(rig.pasted(), vec!["🔥".to_string()]);
    }

    /// Choosing only sets the clipboard. A paste synthesized now would go
    /// to this popup, which still holds the keyboard.
    #[test]
    fn choosing_sets_the_clipboard_without_synthesizing_the_paste_yet() {
        let mut rig = Rig::new();
        rig.key(Keysym::Return, NONE);
        assert_eq!(rig.chooser.log.borrow().as_slice(), &["set_clipboard"]);
    }

    #[test]
    fn a_pick_is_counted_against_the_emojis_neutral_glyph() {
        let mut rig = Rig::new();
        rig.select("thumbs up");
        rig.key(Keysym::Return, SHIFT);
        rig.key(Keysym::Right, NONE);
        rig.key(Keysym::Return, NONE);
        assert_eq!(rig.prefs.frequent, vec![("👍".to_string(), 1)]);
    }

    #[test]
    fn a_pick_that_never_reached_the_clipboard_is_not_counted() {
        let mut rig = Rig { chooser: MockChooser::failing("no seat"), ..Rig::new() };
        assert_eq!(rig.key(Keysym::Return, NONE), Some(ChoiceOutcome::Cancelled));
        assert!(rig.prefs.frequent.is_empty());
    }

    #[test]
    fn escape_clears_the_search_first_and_closes_second() {
        let mut rig = Rig::new();
        rig.typed("fi");
        assert_eq!(rig.key(Keysym::Escape, NONE), None);
        assert_eq!(rig.model.filter_text(), "");
        assert_eq!(rig.key(Keysym::Escape, NONE), Some(ChoiceOutcome::Cancelled));
    }

    #[test]
    fn a_chord_never_types_into_the_search() {
        let mut rig = Rig::new();
        dispatch_key(&mut rig.model, &rig.chooser, &mut rig.prefs, Keysym::a, Some("a".into()), CTRL);
        assert_eq!(rig.model.filter_text(), "");
    }

    #[test]
    fn tab_and_shift_tab_step_through_the_tabs() {
        let mut rig = Rig::new();
        rig.key(Keysym::Tab, NONE);
        assert_eq!(rig.model.tab(), Tab::Kaomoji);
        rig.key(Keysym::ISO_Left_Tab, SHIFT);
        assert_eq!(rig.model.tab(), Tab::Emoji);
    }

    #[test]
    fn a_kaomoji_pastes_as_written_and_is_not_counted_as_an_emoji() {
        let mut rig = Rig::new();
        rig.act(Action::SetTab(Tab::Kaomoji));
        rig.typed("shrug");
        rig.key(Keysym::Return, NONE);
        assert_eq!(rig.pasted(), vec!["¯\\_(ツ)_/¯".to_string()]);
        assert!(rig.prefs.frequent.is_empty());
    }

    // --- tones

    #[test]
    fn shift_enter_on_a_plain_emoji_just_pastes_it() {
        let mut rig = Rig::new();
        rig.select("fire");
        assert_eq!(rig.key(Keysym::Return, SHIFT), Some(ChoiceOutcome::Chosen));
    }

    #[test]
    fn enter_in_the_picker_pastes_that_tone_without_changing_the_default() {
        let mut rig = Rig::new();
        rig.select("thumbs up");
        assert_eq!(rig.key(Keysym::Return, SHIFT), None, "Shift+Enter on a toned emoji opens the picker");
        rig.act(Action::HoverTone(5));
        assert_eq!(rig.key(Keysym::Return, NONE), Some(ChoiceOutcome::Chosen));
        let Some(Item::Emoji(thumbs)) = rig.model.selected_item() else { unreachable!() };
        assert_eq!(rig.pasted(), vec![thumbs.tone(Tone::Dark).to_string()]);
        assert_eq!(rig.prefs.tone, None);
        assert_eq!(rig.model.default_tone(), None);
    }

    #[test]
    fn shift_enter_in_the_picker_pastes_that_tone_and_makes_it_the_default() {
        let mut rig = Rig::new();
        rig.select("thumbs up");
        rig.key(Keysym::Return, SHIFT);
        rig.act(Action::HoverTone(3));
        rig.key(Keysym::Return, SHIFT);
        assert_eq!(rig.prefs.tone, Some(Tone::Medium));
        assert_eq!(rig.model.default_tone(), Some(Tone::Medium));
    }

    #[test]
    fn the_default_picker_sets_the_default_and_pastes_nothing() {
        let mut rig = Rig::new();
        rig.act(Action::OpenDefaultPicker);
        assert_eq!(rig.act(Action::PickTone { cell: 2, remember: false }), None, "setting a default does not close the popup");
        assert!(rig.pasted().is_empty());
        assert_eq!(rig.prefs.tone, Some(Tone::MediumLight));
        assert_eq!(rig.model.tone_picker(), None);
    }

    #[test]
    fn picking_neutral_in_the_default_picker_clears_the_default() {
        let mut rig = Rig { model: Model::new(Some(Tone::Dark), &[]), prefs: Prefs { tone: Some(Tone::Dark), ..Prefs::default() }, ..Rig::new() };
        rig.act(Action::OpenDefaultPicker);
        rig.act(Action::PickTone { cell: 0, remember: false });
        assert_eq!(rig.prefs.tone, None);
    }

    #[test]
    fn escape_puts_the_picker_away_without_pasting_or_closing() {
        let mut rig = Rig::new();
        rig.select("thumbs up");
        rig.key(Keysym::Return, SHIFT);
        assert_eq!(rig.key(Keysym::Escape, NONE), None);
        assert_eq!(rig.model.tone_picker(), None);
        assert!(rig.pasted().is_empty());
    }

    #[test]
    fn typing_while_the_picker_is_open_does_nothing() {
        let mut rig = Rig::new();
        rig.select("thumbs up");
        rig.key(Keysym::Return, SHIFT);
        rig.typed("x");
        assert_eq!(rig.model.filter_text(), "");
        assert!(rig.model.tone_picker().is_some());
    }

    #[test]
    fn with_a_default_tone_enter_pastes_the_toned_glyph() {
        let mut rig = Rig { model: Model::new(Some(Tone::Light), &[]), ..Rig::new() };
        rig.select("waving hand");
        rig.key(Keysym::Return, NONE);
        let Some(Item::Emoji(hand)) = rig.model.selected_item() else { unreachable!() };
        assert_eq!(rig.pasted(), vec![hand.tone(Tone::Light).to_string()]);
    }
}
