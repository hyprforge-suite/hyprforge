//! Small inline marks: a chip, a keycap, a config line.

use super::Tint;
use crate::density;
use crate::theme::{self, surface, FontScale};
use iced::widget::{container, text, text::IntoFragment, Text};
use iced::{Background, Border, Element};

/// How strongly a chip's own colour tints its fill and its outline.
///
/// Faint on purpose: the text carries the colour, and the fill only
/// separates the chip from the row it sits in. A chip at full strength
/// would outshout the label it annotates.
const CHIP_FILL_ALPHA: f32 = 0.10;
const CHIP_BORDER_ALPHA: f32 = 0.45;

/// A short fact about the row it sits in — `WPA3`, `saved`,
/// `captive portal`, `suite default` — in the mono font, in `tint`.
///
/// Mono because a chip is usually something the system said rather than
/// something written for a person, and setting it in the same font as
/// config lines says so.
pub fn chip<'a, Message: 'a>(
    label: impl IntoFragment<'a>,
    tint: Tint,
    scale: FontScale,
) -> Element<'a, Message> {
    let color = tint.iced();
    container(
        text(label)
            .font(theme::mono_font())
            .size(scale.apply(density::META_TEXT_BASE * 0.9))
            .color(color)
            // Never wrapped: a chip broken over two lines stops reading
            // as one mark and doubles the height of the row it sits in.
            .wrapping(iced::widget::text::Wrapping::None),
    )
    .padding([1.0, scale.apply(6.0)])
    .style(move |_t: &iced::Theme| container::Style {
        background: Some(Background::Color(iced::Color { a: CHIP_FILL_ALPHA, ..color })),
        border: Border {
            radius: density::nested_radius().into(),
            width: 1.0,
            color: iced::Color { a: CHIP_BORDER_ALPHA, ..color },
        },
        ..container::Style::default()
    })
    .into()
}

/// One key of a chord, as a cap: `SUPER`, `SHIFT`, `⏎`.
///
/// A chord is drawn as a row of these rather than as `SUPER + SHIFT + S`
/// in a line of text, because the caps are what make a key binding
/// scannable down a table — the eye finds the shape before reading the
/// word.
pub fn keycap<'a, Message: 'a>(label: impl IntoFragment<'a>, scale: FontScale) -> Element<'a, Message> {
    container(
        text(label)
            .font(iced::Font { weight: iced::font::Weight::Semibold, ..theme::mono_font() })
            .size(scale.apply(density::META_TEXT_BASE * 0.85))
            .color(theme::text()),
    )
    .padding([1.0, scale.apply(6.0)])
    .style(|_t: &iced::Theme| container::Style {
        background: Some(Background::Color(surface::sidebar())),
        border: Border {
            radius: density::nested_radius().into(),
            width: 1.0,
            color: surface::card_border(),
        },
        ..container::Style::default()
    })
    .into()
}

/// Text shown exactly as it is written in a config file —
/// `input:follow_mouse = 1`, `monitor=DP-1,2560x1440@165`.
///
/// Mono and dim: it is there to be checked, not read, and it must be
/// possible to tell `l` from `1` in it.
pub fn config_line<'a>(content: impl IntoFragment<'a>, scale: FontScale) -> Text<'a> {
    text(content)
        .font(theme::mono_font())
        .size(scale.apply(density::META_TEXT_BASE * 0.9))
        .color(theme::text_dim())
}
