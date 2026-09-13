//! The popup's widget tree, built fresh each frame from a [`Model`] — the
//! grid counterpart to `hyprforge-clipmenu::view`.
//!
//! Every colour comes from [`hyprforge_look::Theme`] — CLAUDE.md is
//! explicit that no app may define its own colour constant. Like
//! `hyprforge-clipmenu`'s own `view`, this never routes through iced's
//! widget-level click handling at all (every `Element` here is
//! `Infallible`-messaged): `hyprforge-popup::popup::Popup` draws with
//! `mouse::Cursor::Unavailable` and this crate's own
//! `crate::geometry::GridLayout`/`ToneStrip` resolve every click and
//! hover directly from raw pointer coordinates (see `crate::popup_app`).
//! What this module draws and what those hit-test against are required
//! to describe the same pixels — the numbers below (`grid.cell_size`,
//! `grid.spacing`, `ToneStrip::top`/`left`) are exactly the ones that
//! module reads, never a second, independently-chosen set.

use crate::geometry::{GridLayout, ToneStrip};
use crate::model::Model;
use hyprforge_emoji::Emoji;
use hyprforge_look::Theme;
use iced_runtime::core::alignment::{Horizontal, Vertical};
use iced_runtime::core::text::Wrapping;
use iced_runtime::core::text::LineHeight;
use iced_runtime::core::{Element, Font, Length, Padding};
use iced_widget::{column, container, row, text, Space, Stack};

fn to_iced(c: hyprforge_look::Color) -> iced_runtime::core::Color {
    iced_runtime::core::Color::from_rgba8(c.r, c.g, c.b, c.a as f32 / 255.0)
}


/// The font emoji cells are drawn in, named explicitly rather than left
/// to the theme's UI font.
///
/// Without this, roughly half the grid renders as monochrome glyphs
/// instead of emoji — and it is not a data problem: the table carries
/// `U+FE0F` on all 365 entries that need it, and the 60 bare
/// single-codepoint entries are `Emoji_Presentation=Yes` characters
/// that do not. The cause is font selection. Plenty of ordinary UI
/// fonts contain their own black-and-white glyphs for ✔ ☺ ⚙ and the
/// like, so a renderer asked for the theme's font finds a glyph there,
/// is satisfied, and never falls back to the colour emoji font sitting
/// right beside it.
///
/// `noto-fonts-emoji` is a hard dependency of this package for exactly
/// this reason — see `packaging/arch/PKGBUILD`. If it is missing, cells
/// render as tofu, which is the honest failure rather than a silent
/// half-monochrome grid.
const EMOJI_FONT: Font = Font::with_name("Noto Color Emoji");

/// How much of a cell the glyph fills.
///
/// Was 0.55, which left an emoji looking lost in its own cell. Emoji are
/// square and have no descenders to leave room for, so they can take far
/// more of the box than a line of text would.
const GLYPH_FILL: f64 = 0.78;

pub fn view<'a, Message, Renderer>(
    model: &'a Model,
    theme: &'a Theme,
    popup_width: f64,
) -> Element<'a, Message, iced_widget::Theme, Renderer>
where
    Message: 'a,
    Renderer: iced_runtime::core::text::Renderer<Font = iced_runtime::core::Font> + 'a,
{
    let grid = GridLayout::for_font_size(theme.font_size);
    let text_color = to_iced(theme.surfaces.text);
    let dim_color = to_iced(theme.surfaces.text_dim);
    let header_text_height = grid.header_height - GridLayout::HEADER_GAP;

    let root_background = theme.surfaces.root;
    let popup_border = theme.accent;
    let popup_radius = corner_radius(theme);

    let header_inner: Element<'a, Message, iced_widget::Theme, Renderer> = if model.filter_text().is_empty() {
        let placeholder = match model.paste_target() {
            Some(target) => format!("Type to filter — pasting into {target}"),
            None => "Type to filter".to_string(),
        };
        text(placeholder).size(theme.font_size).wrapping(Wrapping::None).color(dim_color).into()
    } else {
        text(model.filter_text().to_string()).size(theme.font_size).wrapping(Wrapping::None).color(text_color).into()
    };
    let field_background = theme.surfaces.card;
    let field_border = theme.accent;
    let field_radius = corner_radius(theme).min((header_text_height / 2.0) as f32);
    let header: Element<'a, Message, iced_widget::Theme, Renderer> = container(header_inner)
        .width(Length::Fill)
        .height(Length::Fixed(header_text_height as f32))
        .padding(Padding { top: 0.0, right: 8.0, bottom: 0.0, left: 8.0 })
        .align_y(Vertical::Center)
        .style(move |_: &iced_widget::Theme| container::Style {
            background: Some(to_iced(field_background).into()),
            border: iced_runtime::core::Border { radius: field_radius.into(), width: 1.0, color: to_iced(field_border) },
            ..Default::default()
        })
        .into();

    let filtered = model.filtered();
    let selected = model.selected_index();
    let columns = model.columns().max(1);

    let body: Element<'a, Message, iced_widget::Theme, Renderer> = if filtered.is_empty() {
        message(
            if model.filter_text().is_empty() { "No emoji to show" } else { "No matches" },
            dim_color,
            theme,
        )
    } else {
        let range = model.visible_range();
        let window = &filtered[range.clone()];

        let mut rows_vec: Vec<Element<'a, Message, iced_widget::Theme, Renderer>> = Vec::new();
        let mut current_row: Vec<Element<'a, Message, iced_widget::Theme, Renderer>> = Vec::new();
        for (offset, entry) in window.iter().enumerate() {
            let index = range.start + offset;
            current_row.push(grid_cell(model, entry, index == selected, theme, &grid));
            if current_row.len() == columns {
                rows_vec.push(row(std::mem::take(&mut current_row)).spacing(grid.spacing as f32).into());
            }
        }
        if !current_row.is_empty() {
            rows_vec.push(row(current_row).spacing(grid.spacing as f32).into());
        }
        // Shifted right by whatever `left_margin` says, so the grid sits
        // centred rather than pushed against the left edge with the
        // leftover width pooled on the right. The outer container
        // already applies `PADDING`, so only the extra slack is added
        // here — and it is the *same* number `GridLayout::cell_at`
        // measures from, which is what keeps a click landing on the cell
        // under the pointer.
        let indent = (grid.left_margin(popup_width, columns) - GridLayout::PADDING).max(0.0);
        container(column(rows_vec).spacing(grid.spacing as f32))
            .padding(Padding { top: 0.0, right: 0.0, bottom: 0.0, left: indent as f32 })
            .into()
    };

    let content: Element<'a, Message, iced_widget::Theme, Renderer> = container(
        column![header, Space::new().height(GridLayout::HEADER_GAP as f32), body]
            .spacing(0)
            .width(Length::Fill),
    )
    .width(Length::Fill)
    .height(Length::Fill)
    .padding(Padding::from(GridLayout::PADDING as f32))
    .style(move |_: &iced_widget::Theme| container::Style {
        background: Some(to_iced(root_background).into()),
        border: iced_runtime::core::Border { radius: popup_radius.into(), width: 1.0, color: to_iced(popup_border) },
        ..Default::default()
    })
    .into();

    match model.tone_overlay() {
        Some(cursor) => {
            let strip = ToneStrip::for_font_size(theme.font_size, &grid);
            let overlay = tone_overlay(model, cursor, theme, &strip);
            Stack::with_children([content, overlay]).width(Length::Fill).height(Length::Fill).into()
        }
        None => content,
    }
}

/// One grid cell: a fixed-size square, the same fixed-size discipline
/// `hyprforge-clipmenu::view::entry_row`'s doc explains — a size the
/// renderer measured out could disagree with what
/// `crate::geometry::GridLayout::cell_at` was told to expect, so nothing
/// here is left to shrink or grow around its own glyph.
fn grid_cell<'a, Message, Renderer>(
    model: &Model,
    entry: &Emoji,
    selected: bool,
    theme: &Theme,
    grid: &GridLayout,
) -> Element<'a, Message, iced_widget::Theme, Renderer>
where
    Message: 'a,
    Renderer: iced_runtime::core::text::Renderer<Font = iced_runtime::core::Font> + 'a,
{
    let glyph = model.display_char(entry).to_string();
    let glyph_size = (grid.cell_size * GLYPH_FILL) as f32;
    // `LineHeight::Absolute` so the text box is exactly the glyph and
    // not a line box with leading above and below it: the container
    // centres whatever it is given, and centring a box that is taller
    // than its own glyph puts the glyph high in the cell rather than in
    // the middle of it.
    let label = text(glyph)
        .font(EMOJI_FONT)
        .size(glyph_size)
        .line_height(LineHeight::Absolute(glyph_size.into()))
        .align_x(Horizontal::Center)
        .align_y(Vertical::Center)
        .wrapping(Wrapping::None);

    let border_color = to_iced(theme.accent);
    let cell_background = to_iced(if selected { theme.surfaces.row } else { theme.surfaces.card });
    let border_width = if selected { 1.5 } else { 0.0 };
    let radius = corner_radius(theme).min(grid.cell_size as f32 / 2.0);

    container(label)
        .width(Length::Fixed(grid.cell_size as f32))
        .height(Length::Fixed(grid.cell_size as f32))
        .align_x(Horizontal::Center)
        .align_y(Vertical::Center)
        .clip(true)
        .style(move |_: &iced_widget::Theme| container::Style {
            background: Some(cell_background.into()),
            border: iced_runtime::core::Border { radius: radius.into(), width: border_width, color: border_color },
            ..Default::default()
        })
        .into()
}

/// The five-cell skin-tone strip a long press (or Tab) opens, laid out
/// as an out-of-flow overlay via `Stack` — see this module's own doc for
/// why positioning it is purely a matter of matching
/// `crate::geometry::ToneStrip`'s numbers, not iced's own layout
/// deciding where it goes.
fn tone_overlay<'a, Message, Renderer>(
    model: &Model,
    cursor: usize,
    theme: &Theme,
    strip: &ToneStrip,
) -> Element<'a, Message, iced_widget::Theme, Renderer>
where
    Message: 'a,
    Renderer: iced_runtime::core::text::Renderer<Font = iced_runtime::core::Font> + 'a,
{
    // The overlay only ever opens for the currently selected entry (see
    // `Model::open_tone_overlay`), and the selection never moves while it
    // is up (`crate::popup_app::dispatch_key` ignores Up/Down and
    // typing, and a click closes the overlay before it would ever touch
    // the grid's own selection) — so this is always the same entry the
    // strip opened on.
    let Some(entry) = model.selected_entry() else {
        return Space::new().into();
    };
    let Some(variants) = entry.tone_variants() else {
        return Space::new().into();
    };

    let glyph_size = (strip.cell_size * 0.55) as f32;
    let border_color = to_iced(theme.accent);
    let radius = corner_radius(theme).min(strip.cell_size as f32 / 2.0);

    let cells = variants.into_iter().enumerate().map(|(index, (_, glyph))| {
        let highlighted = index == cursor;
        let background = to_iced(if highlighted { theme.surfaces.row } else { theme.surfaces.card });
        let border_width = if highlighted { 1.5 } else { 0.0 };
        let cell: Element<'a, Message, iced_widget::Theme, Renderer> = container(
            text(glyph)
                .font(EMOJI_FONT)
                .size(glyph_size)
                .line_height(LineHeight::Absolute(glyph_size.into()))
                .align_x(Horizontal::Center)
                .align_y(Vertical::Center)
                .wrapping(Wrapping::None),
        )
            .width(Length::Fixed(strip.cell_size as f32))
            .height(Length::Fixed(strip.cell_size as f32))
            .align_x(Horizontal::Center)
            .align_y(Vertical::Center)
            .style(move |_: &iced_widget::Theme| container::Style {
                background: Some(background.into()),
                border: iced_runtime::core::Border { radius: radius.into(), width: border_width, color: border_color },
                ..Default::default()
            })
            .into();
        cell
    });

    let strip_background = theme.surfaces.card;
    let strip_border = theme.accent;
    let strip_radius = corner_radius(theme);
    // `width` set explicitly to `strip.width()` rather than left to
    // shrink around the five cells' own measured size — the same
    // discipline `grid_cell` follows for an individual cell, so this
    // container's drawn rectangle can never drift from the width
    // `ToneStrip::tone_at` hit-tests against.
    let bar: Element<'a, Message, iced_widget::Theme, Renderer> = container(row(cells).spacing(strip.spacing as f32))
        .width(Length::Fixed(strip.width() as f32))
        .padding(Padding::from(4.0))
        .style(move |_: &iced_widget::Theme| container::Style {
            background: Some(to_iced(strip_background).into()),
            border: iced_runtime::core::Border { radius: strip_radius.into(), width: 1.0, color: to_iced(strip_border) },
            ..Default::default()
        })
        .into();

    container(bar)
        .padding(Padding { top: strip.top as f32, left: strip.left as f32, right: 0.0, bottom: 0.0 })
        .into()
}

/// The theme's corner radius, bounded so it cannot describe a shape the
/// renderer refuses to build — identical to
/// `hyprforge-clipmenu::view::corner_radius`'s own reasoning: the
/// geometry ends in `tiny_skia` path builders that return `None` for a
/// degenerate radius, and iced unwraps that.
fn corner_radius(theme: &Theme) -> f32 {
    const MAX_ROUNDING: u32 = 64;
    theme.rounding.min(MAX_ROUNDING) as f32
}

fn message<'a, Message, Renderer>(
    text_value: &str,
    color: iced_runtime::core::Color,
    theme: &Theme,
) -> Element<'a, Message, iced_widget::Theme, Renderer>
where
    Message: 'a,
    Renderer: iced_runtime::core::text::Renderer<Font = iced_runtime::core::Font> + 'a,
{
    container(text(text_value.to_string()).size(theme.font_size).color(color))
        .width(Length::Fill)
        .padding(Padding::from(12))
        .into()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_corner_radius_comes_from_the_theme_rather_than_a_constant() {
        let theme = Theme { rounding: 12, ..Theme::default() };
        assert_eq!(corner_radius(&theme), 12.0);
    }

    #[test]
    fn an_absurd_rounding_is_bounded_rather_than_handed_to_the_renderer() {
        let theme = Theme { rounding: u32::MAX, ..Theme::default() };
        let radius = corner_radius(&theme);
        assert!(radius.is_finite());
        assert!(radius <= 64.0, "got {radius}");
    }
}
