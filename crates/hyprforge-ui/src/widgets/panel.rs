//! The pieces of a docked side panel: tabs across its top, and the
//! label-and-value lines a panel about one thing is mostly made of.
//!
//! Written for Files' Properties inspector (mockup `1g`), and here rather
//! than in Files because neither is about files: Settings' pages describe
//! a monitor or a battery with the same "Where · Created · Owner" lines,
//! and any panel with more than one view of its subject wants the same
//! tabs. See `DESIGN-SYSTEM.md`'s rule about the second user arriving
//! with a copy of its own.

use super::{meta_text, scaled_text};
use crate::density;
use crate::theme::{self, spacing, FontScale};
use iced::widget::{button, column, container, row, text::IntoFragment, Row, Space};
use iced::{Background, Element, Length};

/// How wide a fact's label column is at 100% — room for "Accessed" and
/// "Permissions" in the dim metadata size, so every value in a panel
/// starts at one edge and the eye can run down them.
const FACT_LABEL_BASE: f32 = 86.0;

/// The underline under the selected tab.
const TAB_UNDERLINE: f32 = 2.0;

/// Tabs across the top of a panel: each a word, the selected one in the
/// body colour over an accent underline, the rest dim.
///
/// The underline is the accent because a tab *is* a selection — which of
/// the panel's views you picked — and that is the one thing the accent
/// means. Hover only lifts the text to the body colour; it never borrows
/// the underline, for the reason `selectable_row_style` gives.
///
/// Every tab keeps a transparent underline of the same height, so
/// switching tabs moves nothing: the row is the same height whichever
/// one is lit.
pub fn panel_tabs<'a, T, Message>(
    tabs: &[(T, &'a str)],
    selected: &T,
    on_select: impl Fn(T) -> Message,
    scale: FontScale,
) -> Element<'a, Message>
where
    T: Clone + PartialEq,
    Message: Clone + 'a,
{
    let mut strip = Row::new().spacing(spacing::MD).align_y(iced::Alignment::End);
    for (tab, label) in tabs {
        let lit = tab == selected;
        let word = button(scaled_text(*label, density::META_TEXT_BASE, scale))
            .padding([2.0, 0.0])
            .on_press(on_select(tab.clone()))
            .style(move |_t: &iced::Theme, status| button::Style {
                background: None,
                text_color: if lit || matches!(status, button::Status::Hovered) {
                    theme::text()
                } else {
                    theme::text_dim()
                },
                ..button::Style::default()
            });
        let underline = container(Space::new())
            .width(Length::Fill)
            .height(Length::Fixed(TAB_UNDERLINE))
            .style(move |t: &iced::Theme| iced::widget::container::Style {
                background: lit.then(|| Background::Color(t.extended_palette().primary.base.color)),
                ..iced::widget::container::Style::default()
            });
        strip = strip.push(column![word, underline].width(Length::Shrink));
    }
    strip.into()
}

/// One fact about the thing a panel describes: a dim label on the left,
/// at a fixed width so every value in the panel lines up, and the value
/// beside it — `Where   ~/projects/hyprsuite/src`.
///
/// The value is an element rather than a string so a caller can set a
/// path or a mode in the mono font (`config_line`), add a chip, or put a
/// button beside it. A long value wraps under itself, never back under
/// the label.
pub fn fact_row<'a, Message: 'a>(
    label: impl IntoFragment<'a>,
    value: impl Into<Element<'a, Message>>,
    scale: FontScale,
) -> Element<'a, Message> {
    row![
        container(meta_text(label, density::META_TEXT_BASE, scale))
            .width(Length::Fixed(scale.apply(FACT_LABEL_BASE))),
        container(value.into()).width(Length::Fill),
    ]
    .spacing(spacing::SM)
    .align_y(iced::Alignment::Start)
    .into()
}

/// [`fact_row`] for the common case of a plain value, wrapped by word —
/// or by glyph, for a name with no spaces to break at.
pub fn fact<'a, Message: 'a>(
    label: impl IntoFragment<'a>,
    value: impl IntoFragment<'a>,
    scale: FontScale,
) -> Element<'a, Message> {
    fact_row(
        label,
        scaled_text(value, density::META_TEXT_BASE, scale)
            .color(theme::text())
            .wrapping(iced::widget::text::Wrapping::WordOrGlyph),
        scale,
    )
}

/// One figure in a grid of them: a dim label, the figure large, and a
/// dim line saying what it is of — `Temperature / 74 °C / package`. For
/// a page that states a handful of facts about one thing at a glance
/// ([`fact`] is the same fact as a line, for a panel to read down).
/// `warn` draws the figure in the warning colour, for the one that is
/// likely why someone is looking.
///
/// No card around it, and the label in sentence case: a grid of boxed,
/// spaced-capital tiles is a dashboard template, and it was read as one
/// (Processes' Performance page, 2026-10-18). The grid's own spacing is
/// what groups them.
pub fn stat_tile<'a, Message: 'a>(
    label: &str,
    value: impl IntoFragment<'a>,
    note: Option<String>,
    warn: bool,
    scale: FontScale,
) -> Element<'a, Message> {
    let mut body = column![
        meta_text(label.to_string(), density::META_TEXT_BASE, scale),
        scaled_text(value, STAT_VALUE_BASE, scale)
            .font(theme::mono_font())
            .color(if warn { theme::warning() } else { theme::text() }),
    ]
    .spacing(2.0);
    if let Some(note) = note {
        body = body.push(super::clamped_text(note, 1).size(scale.apply(density::META_TEXT_BASE)).color(theme::text_dim()));
    }
    container(body).width(Length::Fill).padding([spacing::XS, 0.0]).into()
}

/// A [`stat_tile`]'s figure at 100%: large enough to read across a
/// desk, a step under a page's own headline figure.
const STAT_VALUE_BASE: f32 = 18.0;
