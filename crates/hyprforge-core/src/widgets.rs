use crate::theme::{spacing, surface, FontScale, TEXT_DIM};
use iced::widget::{button, column, container, row, text, text::IntoFragment, Text};
use iced::{Background, Border, Color, Element, Length, Theme};

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

/// Dim, secondary-emphasis text — timestamps, hints, IDs. Distinct from
/// `scaled_text` alone so "this is metadata, not content" is visible at a
/// glance rather than relying on size difference only.
pub fn meta_text<'a>(content: impl IntoFragment<'a>, base_size: f32, scale: FontScale) -> Text<'a> {
    scaled_text(content, base_size, scale).color(TEXT_DIM)
}

/// The raised-card look used for every section and dialog — a real step
/// up from the root background plus a hairline border, not an auto-derived
/// shade a few percent off the background (which reads as "no elevation
/// at all" on a real monitor).
fn card_style(_theme: &Theme) -> container::Style {
    container::Style {
        background: Some(Background::Color(surface::CARD)),
        border: Border {
            radius: 10.0.into(),
            width: 1.0,
            color: surface::CARD_BORDER,
        },
        ..container::Style::default()
    }
}

/// A titled group of content — the standard way every Hyprforge settings
/// screen breaks itself into blocks (vision pillar #2). Sized to its
/// content; callers that need scrolling should cap a `max_height` on the
/// `content` they pass in rather than have the card stretch to fill
/// whatever space happens to be available.
pub fn section<'a, Message: 'a>(
    title: &'a str,
    scale: FontScale,
    content: impl Into<Element<'a, Message>>,
) -> Element<'a, Message> {
    column![
        meta_text(title.to_uppercase(), 12.0, scale),
        container(content.into())
            .padding(spacing::MD)
            .width(Length::Fill)
            .style(card_style)
    ]
    .spacing(spacing::SM)
    .into()
}

/// A thin separator between list rows inside a card — the visual grammar
/// used everywhere Hyprforge shows a list of things (profiles, rules) so
/// rows read as distinct items rather than one unbroken block of text.
pub fn divider<'a, Message: 'a>() -> Element<'a, Message> {
    container(column![])
        .width(Length::Fill)
        .height(Length::Fixed(1.0))
        .style(|_theme: &Theme| container::Style {
            background: Some(Background::Color(surface::CARD_BORDER)),
            ..container::Style::default()
        })
        .into()
}

/// Widest a settings control grows before it stops tracking the row.
///
/// Without a cap, a text input in an 880px content column stretches to
/// ~590px — far wider than any value typed into it, which reads as an
/// unfinished form. Windows and macOS both hold settings controls to
/// roughly this width regardless of window size.
const CONTROL_MAX_WIDTH: f32 = 340.0;

/// A label paired with its control, aligned into a settings-style row.
pub fn row_field<'a, Message: 'a>(
    label: &'a str,
    control: impl Into<Element<'a, Message>>,
) -> Element<'a, Message> {
    row![
        text(label).width(Length::FillPortion(2)),
        container(control.into())
            .max_width(CONTROL_MAX_WIDTH)
            .width(Length::FillPortion(3)),
    ]
    .spacing(spacing::MD)
    .align_y(iced::Alignment::Center)
    .into()
}

fn primary_style(theme: &Theme, status: button::Status) -> button::Style {
    let palette = theme.extended_palette();
    let base = button::Style {
        background: Some(Background::Color(palette.primary.base.color)),
        text_color: palette.primary.base.text,
        border: Border {
            radius: 8.0.into(),
            ..Border::default()
        },
        ..button::Style::default()
    };
    match status {
        button::Status::Hovered => button::Style {
            background: Some(Background::Color(palette.primary.strong.color)),
            ..base
        },
        button::Status::Disabled => button::Style {
            background: base.background.map(|b| match b {
                Background::Color(c) => Background::Color(Color { a: 0.4, ..c }),
                other => other,
            }),
            text_color: Color {
                a: 0.4,
                ..base.text_color
            },
            ..base
        },
        _ => base,
    }
}

fn secondary_style(_theme: &Theme, status: button::Status) -> button::Style {
    let base = button::Style {
        background: Some(Background::Color(surface::ROW)),
        text_color: crate::theme::TEXT,
        border: Border {
            radius: 8.0.into(),
            width: 1.0,
            color: surface::CARD_BORDER,
        },
        ..button::Style::default()
    };
    match status {
        button::Status::Hovered => button::Style {
            background: Some(Background::Color(surface::CARD_BORDER)),
            ..base
        },
        button::Status::Disabled => button::Style {
            text_color: Color {
                a: 0.4,
                ..base.text_color
            },
            ..base
        },
        _ => base,
    }
}

fn danger_style(theme: &Theme, status: button::Status) -> button::Style {
    let palette = theme.extended_palette();
    let base = button::Style {
        background: Some(Background::Color(surface::ROW)),
        text_color: palette.danger.base.color,
        border: Border {
            radius: 8.0.into(),
            width: 1.0,
            color: palette.danger.base.color,
        },
        ..button::Style::default()
    };
    match status {
        button::Status::Hovered => button::Style {
            background: Some(Background::Color(palette.danger.base.color)),
            text_color: palette.danger.base.text,
            ..base
        },
        _ => base,
    }
}

/// The default action button — filled with the accent color. Use for the
/// one primary action in a row or dialog (Apply, Save, Confirm).
pub fn primary_button<'a, Message: Clone + 'a>(
    label: impl IntoFragment<'a>,
) -> button::Button<'a, Message> {
    button(text(label)).style(primary_style)
}

/// A quiet, outlined button for secondary actions (Rename, Cancel, Edit) —
/// visible without competing with the primary action in the same row.
pub fn secondary_button<'a, Message: Clone + 'a>(
    label: impl IntoFragment<'a>,
) -> button::Button<'a, Message> {
    button(text(label)).style(secondary_style)
}

/// A button styled for destructive/irreversible actions. Reserve for
/// things that actually destroy data (Delete) — a plain Cancel/Close is
/// `secondary_button`, not this.
pub fn danger_button<'a, Message: Clone + 'a>(
    label: &'a str,
    on_press: Message,
) -> Element<'a, Message> {
    button(text(label))
        .on_press(on_press)
        .style(danger_style)
        .into()
}

/// A three-way "unset / true / false" selector, for the settings that have
/// a real difference between "don't care" and "explicitly false" — a window
/// -rule matcher of `floating = false` selects tiled windows, which a plain
/// checkbox can't express separately from not matching on it at all.
pub fn tri_state<'a, Message: Clone + 'a>(
    value: Option<bool>,
    on_select: impl Fn(Option<bool>) -> Message,
) -> Element<'a, Message> {
    let option = |label: &'a str, choice: Option<bool>| {
        let button = if value == choice {
            primary_button(label)
        } else {
            secondary_button(label)
        };
        button.on_press(on_select(choice))
    };
    row![
        option("Any", None),
        option("Yes", Some(true)),
        option("No", Some(false)),
    ]
    .spacing(spacing::XS)
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
        .style(|_theme: &Theme| container::Style {
            background: Some(Background::Color(surface::ROW)),
            border: Border {
                radius: 6.0.into(),
                width: 1.0,
                color: surface::CARD_BORDER,
            },
            ..container::Style::default()
        });

    container(
        column![
            text(title).size(18),
            text(body),
            diff,
            row![
                secondary_button("Cancel").on_press(on_cancel),
                primary_button("Confirm").on_press(on_confirm),
            ]
            .spacing(spacing::SM),
        ]
        .spacing(spacing::MD)
        .padding(spacing::LG)
        .max_width(560.0),
    )
    .style(card_style)
    .into()
}
