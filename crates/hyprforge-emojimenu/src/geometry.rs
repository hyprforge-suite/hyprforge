//! Where everything in the emoji picker is, and what a pointer position
//! lands on.
//!
//! The popup builds a fresh iced `UserInterface` every frame and throws
//! it away (see `hyprforge_popup::popup::Popup::draw`), so there is no
//! live layout tree to ask "what's under the cursor". This module has to
//! already agree with `view.rs` about where everything is — and it does
//! so by being the only place any of it is decided. `view.rs` sizes every
//! region from [`Layout`]; [`Layout::hit`] measures with the same
//! numbers; the grid's lines come from the model's
//! [`hyprforge_popup::Stack`]. CLAUDE.md's rule that the thing drawn and
//! the thing hit-tested must never be two different numbers is the whole
//! contract.
//!
//! # The shape, top to bottom
//!
//! ```text
//! ┌────────────────────────────────────┐
//! │ [ search                    ] [✋▾]│
//! │ [ Emoji | Kaomoji | Symbols        ]│
//! │ SMILEYS & EMOTION        (sticky)  │
//! │ 😀 😃 😄 😁 😆 🥹 😅 😂 🤣          │  grid: 9 × 34px, or 2 pills
//! │ …                                  │
//! ├────────────────────────────────────┤
//! │ 😂  Face with tears of joy  Enter  │  footer
//! └────────────────────────────────────┘
//! ```
//!
//! Pure arithmetic — no `hyprctl`, Wayland or iced — so an off-by-one at
//! an edge, or a hit in the gap between two cells, is something a unit
//! test can pin.

use crate::model::{GridGeometry, Line, Tab, TONE_CELLS};
use hyprforge_popup::kit::{Rect, Tabs};
use hyprforge_popup::Stack;

/// The popup's width, fixed: the design's 360.
pub const POPUP_WIDTH: f64 = 360.0;

/// The design's figures at the 13px body size it was drawn at. The
/// layout grows past them when the theme's font would not fit, and never
/// shrinks below them.
mod design {
    pub const PADDING: f64 = 10.0;
    pub const SEARCH: f64 = 30.0;
    pub const TONE_BUTTON: f64 = 44.0;
    pub const GAP: f64 = 8.0;
    pub const TABS: f64 = 26.0;
    pub const HEADER: f64 = 26.0;
    pub const CELL: f64 = 34.0;
    pub const CELL_GAP: f64 = 2.0;
    pub const PILL: f64 = 38.0;
    pub const PILL_GAP: f64 = 6.0;
    pub const GRID: f64 = 236.0;
    pub const FOOTER: f64 = 48.0;
    pub const PICKER_PADDING: f64 = 5.0;
}

/// See the module doc.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Layout {
    pub width: f64,
    pub padding: f64,
    pub search: Rect,
    /// The ✋ button that sets the default tone.
    pub tone_button: Rect,
    pub tabs: Tabs,
    /// The scrolling grid's rectangle.
    pub grid: Rect,
    /// A section label's line.
    pub header_height: f64,
    /// An emoji or symbol cell's side.
    pub cell: f64,
    /// A kaomoji pill's height.
    pub pill: f64,
    pub footer_top: f64,
    pub footer_height: f64,
    pub height: f64,
}

impl Layout {
    /// The one place the picker's geometry is derived — from the theme's
    /// font size alone.
    pub fn for_font_size(font_size: f32) -> Layout {
        let fs = font_size as f64;
        let line = |size: f64| size * 1.2;
        let width = POPUP_WIDTH;
        let padding = design::PADDING;

        let search_height = design::SEARCH.max(line(fs) + 12.0);
        let tone_button = Rect {
            x: width - padding - design::TONE_BUTTON,
            y: padding,
            width: design::TONE_BUTTON,
            height: search_height,
        };
        let search = Rect { x: padding, y: padding, width: tone_button.x - design::GAP - padding, height: search_height };
        let tabs = Tabs {
            x: padding,
            y: search.bottom() + design::GAP,
            width: width - padding * 2.0,
            height: design::TABS.max(line(fs * 0.96) + 10.0),
            count: Tab::ALL.len(),
        };
        let cell = design::CELL.max(fs * 2.3).round();
        let grow = cell / design::CELL;
        let grid = Rect {
            x: padding,
            y: tabs.y + tabs.height + design::GAP,
            width: width - padding * 2.0,
            height: (design::GRID * grow).round(),
        };
        let footer_top = grid.bottom() + 1.0;
        let footer_height = design::FOOTER.max(line(fs) + line(fs * 0.85) + 18.0);
        Layout {
            width,
            padding,
            search,
            tone_button,
            tabs,
            grid,
            header_height: design::HEADER.max(line(fs * 0.81) + 13.0),
            cell,
            pill: design::PILL.max(line(fs * 1.08) + 18.0),
            footer_top,
            footer_height,
            height: footer_top + footer_height,
        }
    }

    /// How many columns `tab` lays out: as many cells as fit for emoji
    /// and symbols, two pills for kaomoji — they are words, not glyphs.
    pub fn columns(&self, tab: Tab) -> usize {
        match tab {
            Tab::Kaomoji => 2,
            Tab::Emoji | Tab::Symbols => (((self.grid.width + design::CELL_GAP) / (self.cell + design::CELL_GAP)).floor() as usize).max(1),
        }
    }

    /// A cell's width for `tab`.
    pub fn cell_width(&self, tab: Tab) -> f64 {
        match tab {
            Tab::Kaomoji => (self.grid.width - design::PILL_GAP) / 2.0,
            Tab::Emoji | Tab::Symbols => self.cell,
        }
    }

    pub fn cell_height(&self, tab: Tab) -> f64 {
        match tab {
            Tab::Kaomoji => self.pill,
            Tab::Emoji | Tab::Symbols => self.cell,
        }
    }

    /// The gap between two rows (and between a label and its first row).
    pub fn row_gap(&self, tab: Tab) -> f64 {
        match tab {
            Tab::Kaomoji => design::PILL_GAP,
            Tab::Emoji | Tab::Symbols => design::CELL_GAP,
        }
    }

    /// The gap between two cells in a row: whatever is left once the
    /// columns are placed, spread evenly between them, so the grid spans
    /// the full width edge to edge — the design's `space-between`.
    pub fn column_gap(&self, tab: Tab) -> f64 {
        let columns = self.columns(tab);
        if columns < 2 {
            return 0.0;
        }
        ((self.grid.width - columns as f64 * self.cell_width(tab)) / (columns - 1) as f64).max(0.0)
    }

    /// `tab`'s shape, for `Model::set_geometry`.
    pub fn grid_geometry(&self, tab: Tab) -> GridGeometry {
        GridGeometry {
            columns: self.columns(tab),
            header_height: self.header_height,
            cell_height: self.cell_height(tab),
            spacing: self.row_gap(tab),
            viewport_height: self.grid.height,
        }
    }

    /// Where item `index` is drawn right now, or `None` if it is in no
    /// line. What the tone picker anchors to.
    pub fn cell_rect(&self, tab: Tab, lines: &[Line], stack: &Stack, offset: f64, index: usize) -> Option<Rect> {
        let line = lines.iter().position(|l| matches!(*l, Line::Cells { start, end, .. } if (start..end).contains(&index)))?;
        let Line::Cells { start, .. } = lines[line] else { return None };
        let column = (index - start) as f64;
        Some(Rect {
            x: self.grid.x + column * (self.cell_width(tab) + self.column_gap(tab)),
            y: self.grid.y + stack.top(line) - offset,
            width: self.cell_width(tab),
            height: stack.height(line),
        })
    }

    /// What `position` lands on. `sticky` is whether a section label is
    /// pinned over the top of the grid right now — a point on it is on
    /// the label, not on the cells scrolling underneath.
    pub fn hit(&self, position: (f64, f64), tab: Tab, lines: &[Line], stack: &Stack, offset: f64, sticky: bool) -> Option<Hit> {
        if let Some(index) = self.tabs.tab_at(position) {
            return Some(Hit::Tab(index));
        }
        if self.tone_button.contains(position) {
            return Some(Hit::ToneButton);
        }
        if !self.grid.contains(position) {
            return None;
        }
        let y = position.1 - self.grid.y;
        if sticky && y < self.header_height {
            return None;
        }
        let Line::Cells { start, end, .. } = *lines.get(stack.line_at(y + offset)?)? else { return None };
        let x = position.0 - self.grid.x;
        let stride = self.cell_width(tab) + self.column_gap(tab);
        let column = (x / stride).floor();
        if column < 0.0 || x - column * stride > self.cell_width(tab) {
            return None;
        }
        let index = start + column as usize;
        (index < end).then_some(Hit::Cell(index))
    }

    /// The tone picker, anchored to `anchor`: above it if there is room
    /// inside the grid, below it otherwise, and never past the popup's
    /// sides. From the ✋ button (`below_button`), it opens under the
    /// button, right-aligned to it.
    pub fn picker(&self, anchor: Rect, below_button: bool) -> Picker {
        let cell = self.cell;
        let gap = design::CELL_GAP;
        let pad = design::PICKER_PADDING;
        let width = TONE_CELLS as f64 * cell + (TONE_CELLS - 1) as f64 * gap + pad * 2.0;
        let height = cell + pad * 2.0;
        let (x, y) = if below_button {
            (anchor.right() - width, anchor.bottom() + 4.0)
        } else {
            let above = anchor.y - height - 4.0;
            let y = if above >= self.grid.y { above } else { anchor.bottom() + 4.0 };
            (anchor.x - pad, y)
        };
        let x = x.clamp(self.padding, self.width - self.padding - width);
        Picker { rect: Rect { x, y, width, height }, cell, gap, padding: pad }
    }

    /// The grid's scrollbar — the one place its track is computed, read
    /// by both the drawing and the thumb drag. It sits in the popup's
    /// right padding, clear of the last column.
    pub fn scrollbar(&self) -> hyprforge_popup::Scrollbar {
        let track_x = self.width - (self.padding + hyprforge_popup::Scrollbar::WIDTH) / 2.0;
        hyprforge_popup::Scrollbar::new(track_x, self.grid.y, self.grid.height)
    }
}

/// What a pointer position resolved to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Hit {
    Tab(usize),
    ToneButton,
    /// An item, by index into `Model::items`.
    Cell(usize),
}

/// The six-cell tone picker's rectangle and cells.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Picker {
    pub rect: Rect,
    pub cell: f64,
    pub gap: f64,
    pub padding: f64,
}

impl Picker {
    /// The picker cell under `position` (0 is neutral), or `None` outside
    /// the picker or in a gap.
    pub fn cell_at(&self, position: (f64, f64)) -> Option<usize> {
        let x = position.0 - self.rect.x - self.padding;
        let y = position.1 - self.rect.y - self.padding;
        if x < 0.0 || y < 0.0 || y > self.cell {
            return None;
        }
        let stride = self.cell + self.gap;
        let index = (x / stride) as usize;
        (index < TONE_CELLS && x - index as f64 * stride <= self.cell).then_some(index)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::Model;

    fn layout() -> Layout {
        Layout::for_font_size(13.0)
    }

    #[test]
    fn at_the_designs_own_font_size_the_layout_is_the_designs() {
        let l = layout();
        assert_eq!(l.search.height, 30.0);
        assert_eq!(l.tabs.height, 26.0);
        assert_eq!(l.cell, 34.0);
        assert_eq!(l.columns(Tab::Emoji), 9);
        assert_eq!(l.grid.height, 236.0);
        assert_eq!(l.header_height, 26.0);
        assert_eq!(l.grid.y, 82.0, "10 + 30 + 8 + 26 + 8");
    }

    #[test]
    fn the_columns_span_the_grid_edge_to_edge() {
        for tab in Tab::ALL {
            let l = layout();
            let n = l.columns(tab) as f64;
            let span = n * l.cell_width(tab) + (n - 1.0) * l.column_gap(tab);
            assert!((span - l.grid.width).abs() < 1e-9, "{tab:?} spans {span} of {}", l.grid.width);
        }
    }

    #[test]
    fn a_bigger_font_gets_bigger_cells_and_never_overflows_the_popup() {
        for fs in [9.0, 13.0, 15.0, 22.0, 30.0] {
            let l = Layout::for_font_size(fs);
            assert!(l.cell >= fs as f64 * 2.0);
            assert!(l.columns(Tab::Emoji) as f64 * l.cell <= l.grid.width);
            assert!(l.search.right() < l.tone_button.x);
            assert!(l.grid.y >= l.tabs.y + l.tabs.height);
            assert!(l.footer_top > l.grid.bottom());
        }
    }

    /// Every cell of the first rows is hit in its middle, unscrolled and
    /// scrolled — the drawn/hit-tested contract, through the model's own
    /// lines and stack.
    #[test]
    fn every_visible_cell_is_hit_where_it_is_drawn() {
        let l = layout();
        for tab in Tab::ALL {
            let mut model = Model::new(None, &[]);
            model.set_tab(tab);
            model.set_geometry(tab, l.grid_geometry(tab));
            for offset in [0.0, 17.0] {
                model.scroll_by(offset - model.scroll_offset());
                let (lines, stack) = (model.lines(), model.stack());
                let sticky = model.scroll_offset() > 0.0;
                for index in 0..6.min(model.items().len()) {
                    let r = l.cell_rect(tab, &lines, &stack, model.scroll_offset(), index).unwrap();
                    let middle = (r.x + r.width / 2.0, r.y + r.height / 2.0);
                    if !l.grid.contains(middle) || (sticky && middle.1 - l.grid.y < l.header_height) {
                        continue;
                    }
                    assert_eq!(l.hit(middle, tab, &lines, &stack, model.scroll_offset(), sticky), Some(Hit::Cell(index)), "{tab:?} at {offset}");
                }
            }
        }
    }

    #[test]
    fn a_gap_a_label_and_the_sticky_band_hit_no_cell() {
        let l = layout();
        let mut model = Model::new(None, &[]);
        model.set_geometry(Tab::Emoji, l.grid_geometry(Tab::Emoji));
        let (lines, stack) = (model.lines(), model.stack());
        let label = (l.grid.x + 20.0, l.grid.y + l.header_height / 2.0);
        assert_eq!(l.hit(label, Tab::Emoji, &lines, &stack, 0.0, false), None, "a section label is not a cell");
        let first = l.cell_rect(Tab::Emoji, &lines, &stack, 0.0, 0).unwrap();
        let gap = (first.right() + l.column_gap(Tab::Emoji) / 2.0, first.y + first.height / 2.0);
        assert_eq!(l.hit(gap, Tab::Emoji, &lines, &stack, 0.0, false), None);
        let band = (l.grid.x + 5.0, l.grid.y + 3.0);
        assert_eq!(l.hit(band, Tab::Emoji, &lines, &stack, 60.0, true), None, "the pinned label covers what scrolls under it");
    }

    #[test]
    fn the_tabs_and_the_tone_button_are_hit() {
        let l = layout();
        let stack = Stack::new([], 0.0);
        let middle = |r: Rect| (r.x + r.width / 2.0, r.y + r.height / 2.0);
        assert_eq!(l.hit(middle(l.tone_button), Tab::Emoji, &[], &stack, 0.0, false), Some(Hit::ToneButton));
        assert_eq!(l.hit((l.tabs.x + 30.0, l.tabs.y + 10.0), Tab::Emoji, &[], &stack, 0.0, false), Some(Hit::Tab(0)));
    }

    #[test]
    fn the_picker_opens_above_a_cell_when_there_is_room_and_below_when_not() {
        let l = layout();
        let high = Rect { x: 100.0, y: l.grid.y + 2.0, width: l.cell, height: l.cell };
        let low = Rect { y: l.grid.y + 150.0, ..high };
        assert!(l.picker(high, false).rect.y > high.bottom(), "no room above the top row");
        assert!(l.picker(low, false).rect.bottom() < low.y);
    }

    #[test]
    fn the_picker_never_leaves_the_popup_sideways() {
        let l = layout();
        for x in [-50.0, 0.0, 300.0, 500.0] {
            let p = l.picker(Rect { x, y: l.grid.y + 100.0, width: l.cell, height: l.cell }, false);
            assert!(p.rect.x >= l.padding && p.rect.right() <= l.width - l.padding + 1e-9);
        }
        let from_button = l.picker(l.tone_button, true);
        assert!(from_button.rect.y > l.tone_button.bottom());
    }

    #[test]
    fn every_picker_cell_is_hit_in_its_middle_and_its_gaps_are_not() {
        let l = layout();
        let p = l.picker(Rect { x: 100.0, y: l.grid.y + 100.0, width: l.cell, height: l.cell }, false);
        for i in 0..TONE_CELLS {
            let x = p.rect.x + p.padding + i as f64 * (p.cell + p.gap) + p.cell / 2.0;
            assert_eq!(p.cell_at((x, p.rect.y + p.padding + p.cell / 2.0)), Some(i));
        }
        let gap = p.rect.x + p.padding + p.cell + p.gap / 2.0;
        assert_eq!(p.cell_at((gap, p.rect.y + p.padding + 5.0)), None);
        assert_eq!(p.cell_at((p.rect.x - 1.0, p.rect.y + 10.0)), None);
    }

    #[test]
    fn the_scrollbar_sits_in_the_right_padding() {
        let l = layout();
        let bar = l.scrollbar();
        assert!(bar.track_x >= l.grid.right());
        assert!(bar.track_x + bar.width <= l.width);
    }
}
