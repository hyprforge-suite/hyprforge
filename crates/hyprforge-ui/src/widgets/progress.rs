//! How far along something is, and the card a short list of such things
//! floats on.
//!
//! Files drew the first of both — a copy's bar, and the transfers
//! popover hung from its window chrome — and neither is about files: a
//! download in Media, a firmware flash in Settings, anything that takes
//! long enough to watch draws the same line, and any small panel that
//! opens from a button and closes on a click elsewhere wants the same
//! surface.

use crate::density;
use crate::theme::{self, surface, FontScale};
use iced::widget::{container, progress_bar, Container};
use iced::{Background, Border, Element, Length};

/// How thick a progress line is at 100%.
const PROGRESS_GIRTH_BASE: f32 = 4.0;

/// A thin bar across its width, filled to `fraction` (0 to 1).
///
/// The fill is the foreground text colour on the row step, and never
/// the accent. iced's own bar takes the palette's primary, which in
/// this suite *is* the accent — and the accent means "selected", so a
/// running copy drawn in it read as a highlighted row beside the
/// selected one it sat under. A bar that means something else (a
/// failure) is a different widget, not a different colour of this one.
///
/// Callers pass a fraction only when there is an honest one. An
/// indeterminate state is said in words, not drawn as a bar that fills
/// at a made-up rate.
pub fn progress_line<'a, Message: 'a>(fraction: f32, scale: FontScale) -> Element<'a, Message> {
    progress_bar(0.0..=1.0, fraction.clamp(0.0, 1.0))
        .girth(scale.apply(PROGRESS_GIRTH_BASE))
        .length(Length::Fill)
        .style(|_t: &iced::Theme| progress_line_style())
        .into()
}

/// [`progress_line`]'s look, on its own so a test can hold it to the
/// rule above.
pub fn progress_line_style() -> progress_bar::Style {
    progress_bar::Style {
        background: Background::Color(surface::row()),
        bar: Background::Color(theme::text()),
        border: Border { radius: (PROGRESS_GIRTH_BASE / 2.0).into(), ..Border::default() },
    }
}

/// The surface a popover is drawn on: a small panel that opens from a
/// control and closes on a click anywhere else.
///
/// The context menu's surface, on purpose — `sidebar`, a one-pixel
/// `card_border` outline, a card's corner — because a popover is a
/// floating list with more in each row, and two floating things that
/// look different read as two different kinds of thing. The caller
/// wraps it in `iced::widget::opaque` where it stacks it, so a click on
/// the padding does not fall through to the window below.
pub fn popover_card<'a, Message: 'a>(content: impl Into<Element<'a, Message>>) -> Container<'a, Message> {
    container(content).style(|_t: &iced::Theme| container::Style {
        background: Some(Background::Color(surface::sidebar())),
        border: Border {
            radius: density::card_radius().into(),
            width: 1.0,
            color: surface::card_border(),
        },
        ..container::Style::default()
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The reason this widget exists rather than iced's default bar:
    /// the default fills with the palette's primary, which is the
    /// accent, and the accent means selected.
    #[test]
    fn a_progress_line_never_fills_with_the_accent() {
        let style = progress_line_style();
        let accent = crate::color::to_iced(theme::active().accent);
        assert_ne!(style.bar, Background::Color(accent));
        assert_eq!(style.bar, Background::Color(theme::text()));
        assert_ne!(style.bar, style.background, "a full bar has to look full");
    }
}
