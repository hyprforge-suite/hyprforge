//! What the picker shows and where the selection is — plain data and
//! pure functions over it, testable with no compositor, no daemon and no
//! renderer.
//!
//! # Sections, lines and one flat selection
//!
//! Each tab is a list of [`Section`]s — "Frequently used", "Smileys &
//! Emotion", … or, while searching, a single "24 results" — and every
//! section is a label followed by rows of cells. The selection is one
//! index into the flat run of every section's items, so Left and Right
//! simply step through it, across section boundaries, the way reading
//! does. Up and Down move by *row*: to the same column of the row above
//! or below, clamped to that row's last cell when it is shorter — which a
//! section's last row usually is.
//!
//! Where each label and row sits comes from one
//! [`hyprforge_popup::Stack`] ([`Model::stack`]), the same one `view.rs`
//! draws from and `geometry::Layout::hit` measures against — see
//! `hyprforge_popup::stack` for why a list of mixed heights needs exactly
//! one answer to "where is line `n`".
//!
//! # Scrolling is state
//!
//! [`Model::scroll_offset`] is kept, not recomputed from the selection,
//! and only nudged far enough to keep the selection in view. A hover
//! selects the cell under the pointer; a view that recentred on every
//! selection would put a different cell under a pointer that never
//! moved.

use crate::extras::{self, Extra};
use hyprforge_emoji::{Emoji, Tone};
use hyprforge_popup::Stack;

/// The three tabs across the top.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Tab {
    Emoji,
    Kaomoji,
    Symbols,
}

impl Tab {
    pub const ALL: [Tab; 3] = [Tab::Emoji, Tab::Kaomoji, Tab::Symbols];

    pub fn label(self) -> &'static str {
        match self {
            Tab::Emoji => "Emoji",
            Tab::Kaomoji => "Kaomoji",
            Tab::Symbols => "Symbols",
        }
    }

    pub fn placeholder(self) -> &'static str {
        match self {
            Tab::Emoji => "Search emoji",
            Tab::Kaomoji => "Search kaomoji",
            Tab::Symbols => "Search symbols",
        }
    }

    pub fn index(self) -> usize {
        Tab::ALL.iter().position(|&t| t == self).unwrap_or(0)
    }

    /// The next tab along, wrapping — Tab and Shift+Tab.
    pub fn next(self, forward: bool) -> Tab {
        let n = Tab::ALL.len();
        let i = self.index();
        Tab::ALL[if forward { (i + 1) % n } else { (i + n - 1) % n }]
    }
}

/// One thing that can be picked.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Item {
    Emoji(Emoji),
    Extra(&'static Extra),
}

/// A labelled run of items: `items[start..start + len]`.
#[derive(Debug, Clone, PartialEq)]
pub struct Section {
    pub title: String,
    /// A note at the label's right — "best match first".
    pub note: Option<String>,
    pub start: usize,
    pub len: usize,
}

/// One line of the scrolling area.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Line {
    /// A section's label, by index into [`Model::sections`].
    Header(usize),
    /// A row of cells: items `start..end`, in section `section`.
    Cells { section: usize, start: usize, end: usize },
}

/// A tab's grid shape, from `geometry::Layout` — never guessed here.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct GridGeometry {
    pub columns: usize,
    pub header_height: f64,
    pub cell_height: f64,
    pub spacing: f64,
    pub viewport_height: f64,
}

/// Plausible figures until `main.rs` sets the real ones: nine columns,
/// and tall enough that a filtering test never thinks about scrolling.
const DEFAULT_GEOMETRY: GridGeometry =
    GridGeometry { columns: 9, header_height: 26.0, cell_height: 34.0, spacing: 2.0, viewport_height: 1000.0 };

/// What the six-cell tone picker is for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ToneMode {
    /// Opened on a grid cell by holding it (or Shift+Enter): picking a
    /// tone pastes the selected emoji in that tone.
    Paste,
    /// Opened from the ✋ button beside the search: picking a tone makes
    /// it the default every cell is shown in, and pastes nothing.
    Default,
}

/// The picker's state while it is open. `cursor` is 0 for the neutral,
/// untoned glyph and 1..=5 for [`hyprforge_emoji::TONES`] in order.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TonePicker {
    pub mode: ToneMode,
    pub cursor: usize,
}

/// How many cells the tone picker has: the neutral glyph and five tones.
pub const TONE_CELLS: usize = 6;

/// How many rows "Frequently used" fills.
const FREQUENT_ROWS: usize = 2;

pub struct Model {
    tab: Tab,
    query: String,
    items: Vec<Item>,
    sections: Vec<Section>,
    selected: usize,
    scroll_offset: f64,
    geometry: [GridGeometry; 3],
    /// The default tone, `None` for neutral.
    tone: Option<Tone>,
    /// Most-picked first, already resolved from the saved glyphs.
    frequent: Vec<Emoji>,
    tone_picker: Option<TonePicker>,
    paste_target: Option<String>,
}

impl Model {
    /// `tone` and `frequent` are what `crate::config` remembered.
    pub fn new(tone: Option<Tone>, frequent: &[(String, u32)]) -> Model {
        let all = hyprforge_emoji::all();
        let frequent = frequent.iter().filter_map(|(glyph, _)| all.iter().find(|e| e.emoji == glyph).copied()).collect();
        let mut model = Model {
            tab: Tab::Emoji,
            query: String::new(),
            items: Vec::new(),
            sections: Vec::new(),
            selected: 0,
            scroll_offset: 0.0,
            geometry: [DEFAULT_GEOMETRY; 3],
            tone,
            frequent,
            tone_picker: None,
            paste_target: None,
        };
        model.rebuild();
        model
    }

    pub fn set_paste_target(&mut self, target: Option<String>) {
        self.paste_target = target;
    }

    pub fn paste_target(&self) -> Option<&str> {
        self.paste_target.as_deref()
    }

    /// Sets `tab`'s grid shape from `geometry::Layout`.
    pub fn set_geometry(&mut self, tab: Tab, geometry: GridGeometry) {
        self.geometry[tab.index()] = GridGeometry {
            columns: geometry.columns.max(1),
            header_height: geometry.header_height.max(1.0),
            cell_height: geometry.cell_height.max(1.0),
            spacing: geometry.spacing.max(0.0),
            viewport_height: geometry.viewport_height.max(0.0),
        };
        if tab == self.tab {
            self.rebuild();
        }
    }

    fn grid(&self) -> GridGeometry {
        self.geometry[self.tab.index()]
    }

    pub fn tab(&self) -> Tab {
        self.tab
    }

    pub fn filter_text(&self) -> &str {
        &self.query
    }

    pub fn items(&self) -> &[Item] {
        &self.items
    }

    pub fn sections(&self) -> &[Section] {
        &self.sections
    }

    pub fn default_tone(&self) -> Option<Tone> {
        self.tone
    }

    // --- building what is shown

    /// Recomputes the tab's sections from the query, and goes back to the
    /// top — an index into the old list would select the wrong thing.
    fn rebuild(&mut self) {
        self.items.clear();
        self.sections.clear();
        let query = self.query.trim().to_lowercase();
        match self.tab {
            Tab::Emoji if query.is_empty() => {
                let shown = (FREQUENT_ROWS * self.grid().columns).min(self.frequent.len());
                if shown > 0 {
                    let frequent: Vec<Item> = self.frequent[..shown].iter().copied().map(Item::Emoji).collect();
                    self.push_section("Frequently used", None, frequent);
                }
                for group in hyprforge_emoji::groups() {
                    let items: Vec<Item> =
                        hyprforge_emoji::all().iter().filter(|e| e.group == *group).copied().map(Item::Emoji).collect();
                    self.push_section(group, None, items);
                }
            }
            Tab::Emoji => {
                let found: Vec<Item> = hyprforge_emoji::search(&query, hyprforge_emoji::all()).into_iter().map(Item::Emoji).collect();
                self.push_results(found);
            }
            Tab::Kaomoji | Tab::Symbols => {
                let table = if self.tab == Tab::Kaomoji { extras::KAOMOJI } else { extras::SYMBOLS };
                if query.is_empty() {
                    for (title, entries) in table {
                        self.push_section(title, None, entries.iter().map(Item::Extra).collect());
                    }
                } else {
                    let found = table
                        .iter()
                        .flat_map(|(_, entries)| entries.iter())
                        .filter(|x| x.name.contains(&query) || x.text.contains(self.query.trim()))
                        .map(Item::Extra)
                        .collect();
                    self.push_results(found);
                }
            }
        }
        self.selected = 0;
        self.scroll_offset = 0.0;
        self.tone_picker = None;
    }

    fn push_results(&mut self, found: Vec<Item>) {
        let title = match found.len() {
            1 => "1 result".to_string(),
            n => format!("{n} results"),
        };
        let note = (found.len() > 1).then(|| "best match first".to_string());
        self.push_section(&title, note, found);
    }

    /// Adds a section — never an empty one: Unicode's "Component" group
    /// has nothing in the grid, and a label over nothing labels nothing.
    fn push_section(&mut self, title: &str, note: Option<String>, items: Vec<Item>) {
        if items.is_empty() {
            return;
        }
        self.sections.push(Section { title: title.to_string(), note, start: self.items.len(), len: items.len() });
        self.items.extend(items);
    }

    /// Every line: each section's label, then its items in rows of the
    /// tab's column count.
    pub fn lines(&self) -> Vec<Line> {
        let columns = self.grid().columns;
        let mut lines = Vec::new();
        for (index, section) in self.sections.iter().enumerate() {
            lines.push(Line::Header(index));
            let end = section.start + section.len;
            let mut start = section.start;
            while start < end {
                let row_end = (start + columns).min(end);
                lines.push(Line::Cells { section: index, start, end: row_end });
                start = row_end;
            }
        }
        lines
    }

    /// Where every line sits — the one [`Stack`] both drawing and
    /// hit-testing read.
    pub fn stack(&self) -> Stack {
        let g = self.grid();
        Stack::new(
            self.lines().iter().map(|line| match line {
                Line::Header(_) => g.header_height,
                Line::Cells { .. } => g.cell_height,
            }),
            g.spacing,
        )
    }

    /// The section whose label belongs pinned at the top of the viewport:
    /// the one owning the first line below where that label is drawn.
    /// `None` at the very top, where the real label is already there.
    pub fn sticky_section(&self) -> Option<usize> {
        if self.scroll_offset <= 0.0 {
            return None;
        }
        let stack = self.stack();
        let lines = self.lines();
        let y = self.scroll_offset + self.grid().header_height;
        let line = stack.visible(y, 0.0).start.min(lines.len().saturating_sub(1));
        lines.get(line).map(|l| match *l {
            Line::Header(s) | Line::Cells { section: s, .. } => s,
        })
    }

    // --- typing and tabs

    pub fn type_char(&mut self, c: char) {
        self.query.push(c);
        self.rebuild();
    }

    /// Removes the last *character*, never a byte — an emoji pasted into
    /// the search to find its siblings is several bytes.
    pub fn backspace(&mut self) {
        self.query.pop();
        self.rebuild();
    }

    pub fn clear_filter(&mut self) {
        self.query.clear();
        self.rebuild();
    }

    /// Switches tab, keeping what was typed: "heart" is as good a search
    /// in Symbols as it was in Emoji.
    pub fn set_tab(&mut self, tab: Tab) {
        if tab != self.tab {
            self.tab = tab;
            self.rebuild();
        }
    }

    // --- selection

    pub fn selected_index(&self) -> usize {
        self.selected
    }

    pub fn selected_item(&self) -> Option<Item> {
        self.items.get(self.selected).copied()
    }

    /// Selects exactly this item — a hover, or a click. Clamped, so a hit
    /// racing a list that just got shorter still selects something sane.
    pub fn select(&mut self, index: usize) {
        if self.items.is_empty() {
            self.selected = 0;
            return;
        }
        self.selected = index.min(self.items.len() - 1);
        self.sync_scroll();
    }

    /// Left and Right: one item along, across section boundaries, clamped
    /// at the ends rather than wrapping.
    pub fn move_by(&mut self, delta: i32) {
        if self.items.is_empty() {
            return;
        }
        let last = self.items.len() as i32 - 1;
        self.selected = (self.selected as i32 + delta).clamp(0, last) as usize;
        self.sync_scroll();
    }

    /// Up and Down: the same column of the row above or below, skipping
    /// labels; clamped to that row's last cell when it is shorter. At the
    /// first or last row, nothing moves.
    pub fn move_row(&mut self, down: bool) {
        let rows: Vec<(usize, usize)> =
            self.lines().into_iter().filter_map(|l| if let Line::Cells { start, end, .. } = l { Some((start, end)) } else { None }).collect();
        let Some(here) = rows.iter().position(|&(s, e)| (s..e).contains(&self.selected)) else { return };
        let column = self.selected - rows[here].0;
        let target = if down { here + 1 } else { match here.checked_sub(1) { Some(t) => t, None => return } };
        let Some(&(start, end)) = rows.get(target) else { return };
        self.selected = (start + column).min(end - 1);
        self.sync_scroll();
    }

    /// Nudges the view the minimum needed to show the selected row — and
    /// its section's label too, when it is the section's first row.
    fn sync_scroll(&mut self) {
        let lines = self.lines();
        let Some(line) = lines.iter().position(|l| matches!(*l, Line::Cells { start, end, .. } if (start..end).contains(&self.selected)))
        else {
            self.scroll_offset = 0.0;
            return;
        };
        let first = if line > 0 && matches!(lines[line - 1], Line::Header(_)) { line - 1 } else { line };
        self.scroll_offset = self.stack().reveal(first, line, self.scroll_offset, self.grid().viewport_height);
    }

    pub fn scroll_offset(&self) -> f64 {
        self.scroll_offset
    }

    /// Scrolls the view by `delta` pixels — the wheel and a scrollbar
    /// drag. Moves the view, never the selection.
    pub fn scroll_by(&mut self, delta: f64) {
        let content = self.stack().content_height();
        self.scroll_offset = hyprforge_popup::clamp_offset(self.scroll_offset + delta, content, self.grid().viewport_height);
    }

    /// One wheel notch: a row of cells.
    pub fn row_stride(&self) -> f64 {
        self.grid().cell_height + self.grid().spacing
    }

    // --- what an item looks like, and what picking it pastes

    /// What a cell shows, and what picking it pastes: an emoji in the
    /// default tone when it has tones, its plain glyph otherwise; a
    /// kaomoji or symbol as written. The one place this is decided, so
    /// the cell and the paste can never disagree.
    pub fn display(&self, item: Item) -> &'static str {
        match (item, self.tone) {
            (Item::Emoji(e), Some(tone)) => e.tone(tone),
            (Item::Emoji(e), None) => e.emoji,
            (Item::Extra(x), _) => x.text,
        }
    }

    /// The footer's name for what is selected: CLDR's name with a capital,
    /// and the tone it is shown in — "Thumbs up: medium skin tone".
    pub fn describe(&self, item: Item, tone: Option<Tone>) -> String {
        match item {
            Item::Emoji(e) => {
                let name = capitalise(e.name);
                match tone.filter(|_| e.supports_tones()) {
                    Some(t) => format!("{name}: {} skin tone", crate::config::tone_label(Some(t))),
                    None => name,
                }
            }
            Item::Extra(x) => capitalise(x.name),
        }
    }

    // --- the tone picker

    pub fn tone_picker(&self) -> Option<TonePicker> {
        self.tone_picker
    }

    /// Opens the picker on the selected cell — only when it has tones:
    /// holding a plain emoji is a no-op, which is a legitimate answer for
    /// `PopupApp::pointer_long_press`. Returns whether it opened. The
    /// cursor starts on the default tone: "here is what you have".
    pub fn open_tone_picker(&mut self) -> bool {
        match self.selected_item() {
            Some(Item::Emoji(e)) if e.supports_tones() => {
                self.tone_picker = Some(TonePicker { mode: ToneMode::Paste, cursor: tone_cell(self.tone) });
                true
            }
            _ => false,
        }
    }

    /// Opens the picker from the ✋ button, to set the default.
    pub fn open_default_picker(&mut self) {
        self.tone_picker = Some(TonePicker { mode: ToneMode::Default, cursor: tone_cell(self.tone) });
    }

    pub fn close_tone_picker(&mut self) {
        self.tone_picker = None;
    }

    pub fn move_tone_cursor(&mut self, delta: i32) {
        if let Some(picker) = &mut self.tone_picker {
            picker.cursor = (picker.cursor as i32 + delta).clamp(0, TONE_CELLS as i32 - 1) as usize;
        }
    }

    pub fn hover_tone_cursor(&mut self, cell: usize) {
        if let Some(picker) = &mut self.tone_picker {
            picker.cursor = cell.min(TONE_CELLS - 1);
        }
    }

    /// The emoji the picker is showing tones of: the selected cell in
    /// paste mode, a raised hand for the default — the same stand-in the
    /// ✋ button itself is drawn with.
    pub fn tone_subject(&self) -> Option<Emoji> {
        match self.tone_picker?.mode {
            ToneMode::Paste => match self.selected_item()? {
                Item::Emoji(e) => Some(e),
                Item::Extra(_) => None,
            },
            ToneMode::Default => raised_hand(),
        }
    }

    /// The six glyphs the picker shows: neutral, then the five tones.
    pub fn tone_glyphs(&self) -> Option<[&'static str; TONE_CELLS]> {
        let emoji = self.tone_subject()?;
        let variants = emoji.tone_variants()?;
        let mut glyphs = [emoji.emoji; TONE_CELLS];
        for (i, (_, glyph)) in variants.iter().enumerate() {
            glyphs[i + 1] = glyph;
        }
        Some(glyphs)
    }

    pub fn set_default_tone(&mut self, tone: Option<Tone>) {
        self.tone = tone;
    }

    /// The glyph the ✋ button shows: a raised hand in the default tone.
    pub fn default_tone_glyph(&self) -> &'static str {
        match raised_hand() {
            Some(hand) => match self.tone {
                Some(t) => hand.tone(t),
                None => hand.emoji,
            },
            None => "✋",
        }
    }
}

/// The picker cell for `tone`: 0 for neutral, 1..=5 for the tones.
pub fn tone_cell(tone: Option<Tone>) -> usize {
    match tone {
        None => 0,
        Some(t) => 1 + hyprforge_emoji::TONES.iter().position(|&x| x == t).unwrap_or(2),
    }
}

/// The tone a picker cell stands for — the inverse of [`tone_cell`].
pub fn cell_tone(cell: usize) -> Option<Tone> {
    cell.checked_sub(1).and_then(|i| hyprforge_emoji::TONES.get(i).copied())
}

fn raised_hand() -> Option<Emoji> {
    hyprforge_emoji::all().iter().find(|e| e.name == "raised hand").copied()
}

fn capitalise(name: &str) -> String {
    let mut chars = name.chars();
    match chars.next() {
        Some(first) => first.to_uppercase().chain(chars).collect(),
        None => String::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn model() -> Model {
        Model::new(None, &[])
    }

    fn position_of(model: &Model, name: &str) -> usize {
        model.items().iter().position(|i| matches!(i, Item::Emoji(e) if e.name == name)).unwrap()
    }

    // --- sections

    #[test]
    fn with_no_history_the_emoji_tab_is_one_section_per_group_with_nothing_empty() {
        let model = model();
        assert!(model.sections().iter().all(|s| s.len > 0), "no label over nothing");
        assert!(!model.sections().iter().any(|s| s.title == "Component"));
        assert_eq!(model.sections()[0].title, "Smileys & Emotion");
        assert_eq!(model.items().len(), hyprforge_emoji::EMOJI_COUNT);
    }

    #[test]
    fn frequently_used_comes_first_most_picked_first_and_fills_two_rows_at_most() {
        let frequent: Vec<(String, u32)> = hyprforge_emoji::all().iter().take(40).map(|e| (e.emoji.to_string(), 1)).collect();
        let model = Model::new(None, &frequent);
        assert_eq!(model.sections()[0].title, "Frequently used");
        assert_eq!(model.sections()[0].len, 2 * 9);
    }

    #[test]
    fn a_remembered_glyph_that_is_no_longer_an_emoji_is_skipped_not_shown_blank() {
        let model = Model::new(None, &[("not an emoji".into(), 9), ("🔥".into(), 3)]);
        assert_eq!(model.sections()[0].len, 1);
        assert_eq!(model.selected_item(), Some(Item::Emoji(*hyprforge_emoji::all().iter().find(|e| e.emoji == "🔥").unwrap())));
    }

    #[test]
    fn searching_is_one_section_of_results_best_match_first() {
        let mut model = model();
        for c in "fire".chars() {
            model.type_char(c);
        }
        assert_eq!(model.sections().len(), 1);
        assert!(model.sections()[0].title.ends_with("results"));
        assert_eq!(model.sections()[0].note.as_deref(), Some("best match first"));
        assert!(matches!(model.items()[0], Item::Emoji(e) if e.name == "fire"));
    }

    #[test]
    fn a_search_with_one_answer_says_result_not_results() {
        let mut model = model();
        model.set_tab(Tab::Kaomoji);
        for c in "shrug".chars() {
            model.type_char(c);
        }
        assert_eq!(model.sections()[0].title, "1 result");
        assert_eq!(model.sections()[0].note, None);
    }

    #[test]
    fn the_kaomoji_and_symbols_tabs_show_their_own_tables() {
        let mut model = model();
        model.set_tab(Tab::Kaomoji);
        assert_eq!(model.sections()[0].title, "Happy");
        model.set_tab(Tab::Symbols);
        assert_eq!(model.sections()[0].title, "Arrows");
        assert!(matches!(model.items()[0], Item::Extra(x) if x.text == "→"));
    }

    #[test]
    fn switching_tab_keeps_the_search() {
        let mut model = model();
        for c in "arrow".chars() {
            model.type_char(c);
        }
        model.set_tab(Tab::Symbols);
        assert_eq!(model.filter_text(), "arrow");
        assert!(model.items().iter().all(|i| matches!(i, Item::Extra(x) if x.name.contains("arrow"))));
    }

    #[test]
    fn tab_steps_through_the_tabs_and_wraps() {
        assert_eq!(Tab::Emoji.next(true), Tab::Kaomoji);
        assert_eq!(Tab::Symbols.next(true), Tab::Emoji);
        assert_eq!(Tab::Emoji.next(false), Tab::Symbols);
    }

    #[test]
    fn backspace_never_panics_on_a_multi_byte_character_in_the_query() {
        let mut model = model();
        model.type_char('🔥');
        model.backspace();
        assert_eq!(model.filter_text(), "");
    }

    // --- lines

    #[test]
    fn every_section_is_a_label_then_rows_of_at_most_the_column_count() {
        let model = model();
        let lines = model.lines();
        assert!(matches!(lines[0], Line::Header(0)));
        for line in &lines {
            if let Line::Cells { start, end, .. } = *line {
                assert!(end > start && end - start <= 9);
            }
        }
        let cells: usize = lines.iter().map(|l| if let Line::Cells { start, end, .. } = *l { end - start } else { 0 }).sum();
        assert_eq!(cells, model.items().len(), "every item is in exactly one row");
    }

    #[test]
    fn the_stack_has_one_line_per_line_at_its_own_height() {
        let model = model();
        let stack = model.stack();
        assert_eq!(stack.len(), model.lines().len());
        assert_eq!(stack.height(0), DEFAULT_GEOMETRY.header_height);
        assert_eq!(stack.height(1), DEFAULT_GEOMETRY.cell_height);
    }

    // --- movement

    #[test]
    fn left_and_right_step_along_and_clamp_at_the_ends() {
        let mut model = model();
        model.move_by(-1);
        assert_eq!(model.selected_index(), 0);
        model.move_by(2);
        assert_eq!(model.selected_index(), 2);
        model.select(usize::MAX);
        model.move_by(1);
        assert_eq!(model.selected_index(), model.items().len() - 1);
    }

    #[test]
    fn up_and_down_keep_the_column() {
        let mut model = model();
        model.select(3);
        model.move_row(true);
        assert_eq!(model.selected_index(), 12);
        model.move_row(false);
        assert_eq!(model.selected_index(), 3);
    }

    /// A section's last row is usually short. Moving down into it from a
    /// column past its end lands on its last cell, not on nothing and not
    /// in the next section.
    #[test]
    fn moving_into_a_shorter_row_lands_on_its_last_cell() {
        let mut model = model();
        let first = model.sections().iter().find(|s| s.len % 9 != 0 && s.len > 9).cloned().expect("some section has a ragged last row");
        let last_full_row_start = first.start + (first.len / 9 - 1) * 9;
        model.select(last_full_row_start + 8);
        model.move_row(true);
        assert_eq!(model.selected_index(), first.start + first.len - 1);
    }

    #[test]
    fn down_from_a_sections_last_row_crosses_its_neighbours_label() {
        let mut model = model();
        let first_len = model.sections()[0].len;
        model.select(first_len - 1);
        model.move_row(true);
        assert!(model.selected_index() >= model.sections()[1].start);
    }

    // --- scrolling

    #[test]
    fn hovering_a_visible_cell_does_not_scroll() {
        let mut model = model();
        model.set_geometry(Tab::Emoji, GridGeometry { viewport_height: 150.0, ..DEFAULT_GEOMETRY });
        model.select(12);
        let before = model.scroll_offset();
        model.select(2);
        assert_eq!(model.scroll_offset(), before);
    }

    #[test]
    fn moving_below_the_viewport_scrolls_just_far_enough() {
        let mut model = model();
        model.set_geometry(Tab::Emoji, GridGeometry { viewport_height: 100.0, ..DEFAULT_GEOMETRY });
        for _ in 0..5 {
            model.move_row(true);
        }
        let stack = model.stack();
        let offset = model.scroll_offset();
        assert!(offset > 0.0);
        let line = model.lines().iter().position(|l| matches!(*l, Line::Cells { start, end, .. } if (start..end).contains(&model.selected_index()))).unwrap();
        assert!((stack.top(line) + stack.height(line) - (offset + 100.0)).abs() < 1e-9, "the row sits exactly at the bottom");
    }

    #[test]
    fn at_the_top_there_is_no_sticky_label_and_once_scrolled_there_is() {
        let mut model = model();
        model.set_geometry(Tab::Emoji, GridGeometry { viewport_height: 100.0, ..DEFAULT_GEOMETRY });
        assert_eq!(model.sticky_section(), None);
        model.scroll_by(200.0);
        assert_eq!(model.sticky_section(), Some(0));
    }

    #[test]
    fn scrolling_clamps_at_both_ends() {
        let mut model = model();
        model.set_geometry(Tab::Emoji, GridGeometry { viewport_height: 100.0, ..DEFAULT_GEOMETRY });
        model.scroll_by(-50.0);
        assert_eq!(model.scroll_offset(), 0.0);
        model.scroll_by(1e9);
        assert_eq!(model.scroll_offset(), model.stack().content_height() - 100.0);
    }

    // --- display

    #[test]
    fn a_default_tone_shows_on_every_tone_capable_emoji_and_nothing_else() {
        let model = Model::new(Some(Tone::Dark), &[]);
        let hand = model.items()[position_of(&model, "waving hand")];
        let fire = model.items()[position_of(&model, "fire")];
        let Item::Emoji(h) = hand else { unreachable!() };
        let Item::Emoji(f) = fire else { unreachable!() };
        assert_eq!(model.display(hand), h.tone(Tone::Dark));
        assert_eq!(model.display(fire), f.emoji);
    }

    #[test]
    fn the_footer_names_the_tone_it_is_showing() {
        let model = model();
        let thumbs = model.items()[position_of(&model, "thumbs up")];
        assert_eq!(model.describe(thumbs, Some(Tone::Medium)), "Thumbs up: medium skin tone");
        assert_eq!(model.describe(thumbs, None), "Thumbs up");
        let fire = model.items()[position_of(&model, "fire")];
        assert_eq!(model.describe(fire, Some(Tone::Medium)), "Fire", "no tone is claimed for an emoji that has none");
    }

    // --- tone picker

    #[test]
    fn holding_a_plain_emoji_opens_nothing() {
        let mut model = model();
        model.select(position_of(&model, "fire"));
        assert!(!model.open_tone_picker());
        assert_eq!(model.tone_picker(), None);
    }

    #[test]
    fn holding_a_tone_capable_emoji_offers_its_neutral_glyph_and_five_tones() {
        let mut model = model();
        model.select(position_of(&model, "thumbs up"));
        assert!(model.open_tone_picker());
        let glyphs = model.tone_glyphs().unwrap();
        let Some(Item::Emoji(thumbs)) = model.selected_item() else { unreachable!() };
        assert_eq!(glyphs[0], thumbs.emoji);
        assert_eq!(glyphs[3], thumbs.tone(Tone::Medium));
    }

    #[test]
    fn the_picker_starts_on_the_default_tone_and_clamps_its_cursor() {
        let mut model = Model::new(Some(Tone::Light), &[]);
        model.open_default_picker();
        assert_eq!(model.tone_picker().unwrap().cursor, 1);
        model.move_tone_cursor(-10);
        assert_eq!(model.tone_picker().unwrap().cursor, 0);
        model.move_tone_cursor(10);
        assert_eq!(model.tone_picker().unwrap().cursor, TONE_CELLS - 1);
    }

    #[test]
    fn a_picker_cell_and_its_tone_map_both_ways() {
        assert_eq!(cell_tone(0), None);
        for tone in hyprforge_emoji::TONES {
            assert_eq!(cell_tone(tone_cell(Some(tone))), Some(tone));
        }
        assert_eq!(cell_tone(99), None);
    }

    #[test]
    fn typing_closes_the_picker() {
        let mut model = model();
        model.open_default_picker();
        model.type_char('a');
        assert_eq!(model.tone_picker(), None);
    }

    #[test]
    fn the_default_tone_button_shows_a_hand_in_the_default_tone() {
        let mut model = model();
        let neutral = model.default_tone_glyph();
        model.set_default_tone(Some(Tone::Dark));
        assert_ne!(model.default_tone_glyph(), neutral);
    }
}
