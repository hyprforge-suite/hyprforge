use crate::theme::{spacing, FontScale};
use iced::widget::{button, column, container, row, text, text::IntoFragment, Text};
use iced::{Element, Length, Theme};

/// `text(content).size(scale.apply(base_size))` — the one place every piece
/// of Hyprforge UI text should go through, so `FontScale` (vision pillar
/// #7: accessibility built into the shared layer, not bolted on per-app)
/// actually reaches the screen instead of being a struct nobody calls.
pub fn scaled_text<'a>(
    content: impl IntoFragment<'a>,
    base_size: f32,
    scale: FontScale,
) -> Text<'a> {
    text(content).size(scale.apply(base_size))
}

/// A titled group of content — the standard way every Hyprforge settings
/// screen breaks itself into blocks (vision pillar #2).
pub fn section<'a, Message: 'a>(
    title: &'a str,
    scale: FontScale,
    content: impl Into<Element<'a, Message>>,
) -> Element<'a, Message> {
    column![
        scaled_text(title, 16.0, scale),
        container(content.into())
            .padding(spacing::MD)
            .style(container::rounded_box)
    ]
    .spacing(spacing::SM)
    .into()
}

/// A label paired with its control, aligned into a settings-style row.
pub fn row_field<'a, Message: 'a>(
    label: &'a str,
    control: impl Into<Element<'a, Message>>,
) -> Element<'a, Message> {
    row![
        text(label).width(Length::FillPortion(1)),
        container(control.into()).width(Length::FillPortion(2)),
    ]
    .spacing(spacing::MD)
    .align_y(iced::Alignment::Center)
    .into()
}

/// A button styled for destructive/irreversible actions.
pub fn danger_button<'a, Message: Clone + 'a>(
    label: &'a str,
    on_press: Message,
) -> Element<'a, Message> {
    button(text(label))
        .on_press(on_press)
        .style(button::danger)
        .into()
}

/// The standard "this is about to touch a file you own" confirmation dialog
/// (vision pillar #4: nothing requires faith that a change worked — show the
/// exact diff before writing).
pub fn confirm_dialog<'a, Message: Clone + 'a>(
    title: &'a str,
    body: &'a str,
    diff_text: &'a str,
    on_confirm: Message,
    on_cancel: Message,
) -> Element<'a, Message> {
    let diff = container(text(diff_text).font(iced::Font::MONOSPACE).size(13))
        .padding(spacing::SM)
        .width(Length::Fill)
        .style(container::rounded_box);

    container(
        column![
            text(title).size(18),
            text(body),
            diff,
            row![
                button("Cancel").on_press(on_cancel).style(button::secondary),
                button("Confirm").on_press(on_confirm).style(button::primary),
            ]
            .spacing(spacing::SM),
        ]
        .spacing(spacing::MD)
        .padding(spacing::LG)
        .max_width(560.0),
    )
    .style(|theme: &Theme| {
        let palette = theme.extended_palette();
        container::Style {
            background: Some(iced::Background::Color(palette.background.base.color)),
            border: iced::Border {
                radius: 12.0.into(),
                width: 1.0,
                color: palette.background.strong.color,
            },
            ..Default::default()
        }
    })
    .into()
}
