//! The popup's widget tree, built fresh each frame from a [`Model`].
//!
//! Renders two ways from one description, the same reason
//! `hyprforge_authui::screen::view` does: nothing here is Wayland-
//! specific, so the same tree could sit in an ordinary iced window for
//! testing if that were ever useful. Every colour comes from
//! [`hyprforge_look::Theme`] — CLAUDE.md is explicit that no app may
//! define its own colour constant.

use crate::geometry::RowLayout;
use crate::model::{HistoryState, Model};
use crate::thumbnail;
use hyprforge_clipboard::{Content, Entry};
use hyprforge_look::Theme;
use iced_runtime::core::text::Wrapping;
use iced_runtime::core::{Element, Length, Padding};
use iced_widget::{column, container, row, text, Space};

fn to_iced(c: hyprforge_look::Color) -> iced_runtime::core::Color {
    iced_runtime::core::Color::from_rgba8(c.r, c.g, c.b, c.a as f32 / 255.0)
}

pub fn view<'a, Message, Renderer>(
    model: &'a Model,
    theme: &'a Theme,
    thumbnails: &mut thumbnail::Cache,
) -> Element<'a, Message, iced_widget::Theme, Renderer>
where
    Message: 'a,
    Renderer: iced_runtime::core::text::Renderer<Font = iced_runtime::core::Font>
        + iced_runtime::core::image::Renderer<Handle = iced_runtime::core::image::Handle>
        + 'a,
{
    let text_color = to_iced(theme.surfaces.text);
    let dim_color = to_iced(theme.surfaces.text_dim);

    // Both fixed to the theme's font size alone — see
    // `geometry::RowLayout`'s doc comment for why the hit-test in
    // `surface.rs` has to agree with these two numbers exactly, and why
    // that means they are never left to iced's own text metrics to
    // decide.
    let layout = RowLayout::for_font_size(theme.font_size);
    let header_text_height = layout.header_height - RowLayout::HEADER_GAP;

    let header: Element<'a, Message, iced_widget::Theme, Renderer> = if model.filter_text().is_empty()
    {
        text("Type to filter")
            .size(theme.font_size * 0.85)
            .height(Length::Fixed(header_text_height as f32))
            .wrapping(Wrapping::None)
            .color(dim_color)
            .into()
    } else {
        text(model.filter_text().to_string())
            .size(theme.font_size * 0.85)
            .height(Length::Fixed(header_text_height as f32))
            .wrapping(Wrapping::None)
            .color(text_color)
            .into()
    };

    let body: Element<'a, Message, iced_widget::Theme, Renderer> = match model.history() {
        // Never collapse "could not be read" into "there is nothing
        // configured" — CLAUDE.md's rule, and the reason this is a
        // distinct branch instead of falling into the empty-list one
        // below.
        HistoryState::Unreadable(reason) => message(
            &format!("Couldn't read the clipboard history: {reason}"),
            to_iced(theme.error),
            theme,
        ),
        HistoryState::Loaded(_) => {
            let filtered = model.filtered();
            if filtered.is_empty() {
                let msg = if model.filter_text().is_empty() {
                    "No clipboard history yet"
                } else {
                    "No matches"
                };
                message(msg, dim_color, theme)
            } else {
                let range = model.visible_range();
                let selected = model.selected_index();
                let window = &filtered[range.clone()];
                let rows = range
                    .zip(window.iter())
                    .map(|(index, entry)| entry_row(entry, index == selected, theme, &layout, thumbnails))
                    .collect::<Vec<_>>();
                column(rows).spacing(RowLayout::ROW_SPACING as f32).into()
            }
        }
    };

    container(
        column![header, Space::new().height(RowLayout::HEADER_GAP as f32), body]
            .spacing(0)
            .width(Length::Fill),
    )
    .width(Length::Fill)
    .height(Length::Fill)
    .padding(Padding::from(RowLayout::PADDING as f32))
    .style(move |_: &iced_widget::Theme| container::Style {
        background: Some(to_iced(theme.surfaces.root).into()),
        ..Default::default()
    })
    .into()
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

fn entry_row<'a, Message, Renderer>(
    entry: &Entry,
    selected: bool,
    theme: &Theme,
    layout: &RowLayout,
    thumbnails: &mut thumbnail::Cache,
) -> Element<'a, Message, iced_widget::Theme, Renderer>
where
    Message: 'a,
    Renderer: iced_runtime::core::text::Renderer<Font = iced_runtime::core::Font>
        + iced_runtime::core::image::Renderer<Handle = iced_runtime::core::image::Handle>
        + 'a,
{
    let text_color = if selected { to_iced(theme.surfaces.text) } else { to_iced(theme.surfaces.text_dim) };

    // `preview` already collapses whitespace and truncates on a
    // character boundary (`Content::preview`'s own doc comment) — that
    // handles a multi-line copy or an absurdly long single line in terms
    // of *characters*. `Wrapping::None` below is what stops the row
    // itself from growing: without it, iced wraps at word boundaries
    // regardless of how few characters got through, and a row full of
    // hyphen-free base64 or a URL would still lay out as several tall
    // lines instead of the one-line preview a clipboard history needs.
    let preview = entry.content.preview(96);
    let label: Element<'a, Message, iced_widget::Theme, Renderer> = text(preview)
        .size(theme.font_size)
        .wrapping(Wrapping::None)
        .color(text_color)
        .into();

    let content: Element<'a, Message, iced_widget::Theme, Renderer> = match &entry.content {
        // Only entries actually built into a row (see
        // `Model::visible_range`) ever reach `thumbnails.get`, so a
        // history of hundreds of images costs nothing until scrolled
        // to.
        Content::Image { .. } => match thumbnails.get(entry) {
            Some(handle) => row![
                iced_widget::image(handle)
                    .width(Length::Fixed(RowLayout::THUMBNAIL_SIZE as f32))
                    .height(Length::Fixed(RowLayout::THUMBNAIL_SIZE as f32)),
                Space::new().width(8),
                label,
            ]
            .into(),
            // Over the decode cap, or not a decodable image at all —
            // the row still shows the text preview rather than nothing.
            None => label,
        },
        Content::Text(_) => label,
    };

    // Copied out of `theme` as plain `Color`s rather than captured by
    // reference: the style closure below has to be `'static`-ish (bound
    // by `'a`, same as the `Element` it ends up in), and `theme` itself
    // only ever borrows for the length of one `view` call.
    let row_background = to_iced(if selected { theme.surfaces.row } else { theme.surfaces.card });
    let border_color = to_iced(theme.accent);
    let border_width = if selected { 1.5 } else { 0.0 };

    container(content)
        .width(Length::Fill)
        // Fixed to `layout.row_height`, the exact number
        // `geometry::RowLayout::row_at` hit-tests against — not left to
        // shrink around whatever the label or thumbnail measure out to.
        // A row that grew or shrank with its content would still overflow
        // visually (an unusually tall glyph, a thumbnail bigger than
        // `THUMBNAIL_SIZE` somehow) *and* would silently invalidate the
        // hit-test's arithmetic at the same time.
        .height(Length::Fixed(layout.row_height as f32))
        .align_y(iced_runtime::core::alignment::Vertical::Center)
        .padding(Padding::from(RowLayout::ROW_PADDING as f32))
        // Belt and braces alongside `Wrapping::None`: a preview that is
        // still wider than the row (a long run of characters with no
        // word breaks at all, wider per-character than average) is
        // clipped at the row's own edge instead of overdrawing into the
        // padding or the row below it.
        .clip(true)
        .style(move |_: &iced_widget::Theme| container::Style {
            background: Some(row_background.into()),
            border: iced_runtime::core::Border {
                radius: 4.0.into(),
                width: border_width,
                color: border_color,
            },
            ..Default::default()
        })
        .into()
}
