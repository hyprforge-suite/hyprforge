//! The popup's widget tree, built fresh each frame from a [`Model`].
//!
//! Renders two ways from one description, the same reason
//! `hyprforge_authui::screen::view` does: nothing here is Wayland-
//! specific, so the same tree could sit in an ordinary iced window for
//! testing if that were ever useful. Every colour comes from
//! [`hyprforge_look::Theme`] — CLAUDE.md is explicit that no app may
//! define its own colour constant.

use crate::model::{HistoryState, Model};
use crate::thumbnail;
use hyprforge_clipboard::{Content, Entry};
use hyprforge_look::Theme;
use iced_runtime::core::{Element, Length, Padding};
use iced_widget::{column, container, row, text, Space};

fn to_iced(c: hyprforge_look::Color) -> iced_runtime::core::Color {
    iced_runtime::core::Color::from_rgba8(c.r, c.g, c.b, c.a as f32 / 255.0)
}

/// Side length of an image row's thumbnail, in logical pixels. Small
/// enough that a row stays a single line's height; the full image is
/// never needed here, only a preview of what was copied.
const THUMBNAIL_SIZE: f32 = 32.0;

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

    let header: Element<'a, Message, iced_widget::Theme, Renderer> = if model.filter_text().is_empty()
    {
        text("Type to filter").size(theme.font_size * 0.85).color(dim_color).into()
    } else {
        text(model.filter_text().to_string())
            .size(theme.font_size * 0.85)
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
                    .map(|(index, entry)| entry_row(entry, index == selected, theme, thumbnails))
                    .collect::<Vec<_>>();
                column(rows).spacing(2).into()
            }
        }
    };

    container(
        column![header, Space::new().height(6), body]
            .spacing(0)
            .width(Length::Fill),
    )
    .width(Length::Fill)
    .height(Length::Fill)
    .padding(Padding::from(10))
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
    thumbnails: &mut thumbnail::Cache,
) -> Element<'a, Message, iced_widget::Theme, Renderer>
where
    Message: 'a,
    Renderer: iced_runtime::core::text::Renderer<Font = iced_runtime::core::Font>
        + iced_runtime::core::image::Renderer<Handle = iced_runtime::core::image::Handle>
        + 'a,
{
    let text_color = if selected { to_iced(theme.surfaces.text) } else { to_iced(theme.surfaces.text_dim) };

    let preview = entry.content.preview(96);
    let label: Element<'a, Message, iced_widget::Theme, Renderer> =
        text(preview).size(theme.font_size).color(text_color).into();

    let content: Element<'a, Message, iced_widget::Theme, Renderer> = match &entry.content {
        // Only entries actually built into a row (see
        // `Model::visible_range`) ever reach `thumbnails.get`, so a
        // history of hundreds of images costs nothing until scrolled
        // to.
        Content::Image { .. } => match thumbnails.get(entry) {
            Some(handle) => row![
                iced_widget::image(handle)
                    .width(Length::Fixed(THUMBNAIL_SIZE))
                    .height(Length::Fixed(THUMBNAIL_SIZE)),
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
        .padding(Padding::from(6))
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
