//! A list of answers under a field: where a typed path could go, which
//! setting a search means.
//!
//! The panel only — what goes in it, and in what order, is the caller's,
//! and so is where it hangs (see [`super::anchored`]). One look for
//! every such list, so Files' path bar and Settings' search palette read
//! as the same kind of control rather than two that happen to be similar.

use crate::density;
use crate::theme::{self, spacing, surface, FontScale};
use crate::widgets::{meta_text, scaled_text, selectable_row_style};
use iced::widget::{button, column, container, opaque, row};
use iced::{Element, Length};

/// One row: what it is, and an optional dim note at its right edge.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Suggestion {
    pub label: String,
    pub hint: Option<String>,
}

/// The panel. `heading` is a dim line above the rows — Files puts the
/// resolution there, `~/pr/hy → ~/projects/hyprsuite`. `empty` is what it
/// says when there are no rows; `None` draws nothing at all instead,
/// which is right while the first answer is still being worked out.
///
/// Opaque: a click on the panel's own padding must not fall through to
/// whatever listing is underneath it.
pub fn suggestions<'a, Message: Clone + 'a>(
    heading: Option<String>,
    rows: &[Suggestion],
    highlighted: Option<usize>,
    on_choose: impl Fn(usize) -> Message,
    empty: Option<String>,
    mono: bool,
    scale: FontScale,
) -> Option<Element<'a, Message>> {
    if rows.is_empty() && empty.is_none() {
        return None;
    }
    let font = if mono { theme::mono_font() } else { iced::Font::DEFAULT };
    let mut list = column![].width(Length::Fill).spacing(1);
    if let Some(heading) = heading {
        list = list.push(
            container(meta_text(heading, density::META_TEXT_BASE, scale).font(font))
                .padding([spacing::XS as u16, spacing::SM as u16]),
        );
    }
    if rows.is_empty() {
        if let Some(empty) = empty {
            list = list.push(
                container(meta_text(empty, density::META_TEXT_BASE, scale))
                    .padding([spacing::XS as u16, spacing::SM as u16]),
            );
        }
    }
    for (index, item) in rows.iter().enumerate() {
        let selected = highlighted == Some(index);
        let mut content = row![scaled_text(item.label.clone(), density::META_TEXT_BASE, scale)
            .font(font)
            .width(Length::Fill)]
        // Full height, or `align_y` has nothing to centre in: the row
        // shrinks to its text and the button draws it at the top of a
        // 28px row. Seen in the first screenshot.
        .height(Length::Fill)
        .align_y(iced::Alignment::Center)
        .spacing(spacing::MD);
        if let Some(hint) = &item.hint {
            content = content.push(meta_text(hint.clone(), density::META_TEXT_BASE, scale));
        }
        list = list.push(
            button(content)
                .on_press(on_choose(index))
                .width(Length::Fill)
                .height(Length::Fixed(density::row_height(scale)))
                .padding([0, spacing::SM as u16])
                .style(move |t: &iced::Theme, status| selectable_row_style(t, status, selected)),
        );
    }
    // The context menu's surface: a step down from the listing it covers,
    // outlined, at the inner radius — it is the same kind of thing, a
    // short list floating over the window.
    let panel = container(list)
        .padding(spacing::XS)
        .width(Length::Fill)
        .style(|_t: &iced::Theme| container::Style {
            background: Some(iced::Background::Color(surface::sidebar())),
            border: iced::Border {
                color: surface::card_border(),
                width: 1.0,
                radius: density::inner_radius().into(),
            },
            ..container::Style::default()
        });
    Some(opaque(panel))
}
