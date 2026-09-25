//! Text fields that sit recessed into a bar: search, a path, a filter.

use crate::density;
use crate::theme::{self, spacing, surface, FontScale};
use iced::widget::{container, row, text_input, Container};
use iced::Length;

/// The look every inset field shares: recessed into the bar, outlined.
///
/// One function rather than identical closures, so Files' path bar, its
/// search field and Settings' search cannot drift apart — they are
/// siblings by construction, not by someone remembering to keep them
/// matched. Also the look a dropdown's closed state copies, so a picker
/// beside a text field reads as the same kind of control.
pub fn inset_field_style(_t: &iced::Theme) -> container::Style {
    container::Style {
        background: Some(iced::Background::Color(surface::card())),
        border: iced::Border {
            radius: density::inner_radius().into(),
            width: 1.0,
            color: surface::card_border(),
        },
        ..container::Style::default()
    }
}

/// A free-standing text input's look — a setting's value, a filter —
/// matching the inset fields and the dropdowns beside it, rather than
/// iced's stock input with its bright outline.
///
/// Focus outlines in the dim text colour, like an open dropdown, not the
/// accent: typing into a field is not a selection.
pub fn inset_input_style(t: &iced::Theme, status: text_input::Status) -> text_input::Style {
    let border_color = match status {
        text_input::Status::Focused { .. } | text_input::Status::Hovered => theme::text_dim(),
        _ => surface::card_border(),
    };
    text_input::Style {
        background: iced::Background::Color(surface::card()),
        border: iced::Border {
            radius: density::inner_radius().into(),
            width: 1.0,
            color: border_color,
        },
        icon: theme::text_dim(),
        placeholder: theme::text_dim(),
        value: theme::text(),
        selection: t.extended_palette().primary.weak.color,
    }
}

/// The magnifier ring's diameter at 100% scale.
const SEARCH_RING_BASE: f32 = 9.0;

/// A search field: a drawn magnifier ring and a transparent input inside
/// an inset container, one field tall.
///
/// Returns the container so the caller decides the width — Files holds
/// its search to a fixed width so the path bar flexes, where Settings'
/// header search is the one thing that does.
///
/// `placeholder` should name the scope — "Search crates", "Search any
/// setting or config key" — so what will be searched is stated before
/// anything is typed, rather than discovered afterwards.
///
/// The magnifier is a sibling widget rather than the input's own `icon`,
/// because iced's `text_input::Icon` takes its glyph from a `Font` and
/// this suite ships none — the theme's font is whatever the desktop
/// chose, and it may have no magnifier at all. A drawn ring always
/// renders.
pub fn search_field<'a, Message: Clone + 'a>(
    placeholder: &str,
    value: &str,
    on_input: impl Fn(String) -> Message + 'a,
    on_submit: Option<Message>,
    id: Option<iced::widget::Id>,
    scale: FontScale,
) -> Container<'a, Message> {
    let ring_side = scale.apply(SEARCH_RING_BASE);
    let ring = container(iced::widget::Space::new())
        .width(Length::Fixed(ring_side))
        .height(Length::Fixed(ring_side))
        .style(move |_t: &iced::Theme| container::Style {
            border: iced::Border {
                // Half the side: a circle, not a rounded square.
                radius: (ring_side / 2.0).into(),
                width: 1.5,
                color: theme::text_dim(),
            },
            ..container::Style::default()
        });

    let mut input = text_input(placeholder, value)
        .on_input(on_input)
        .size(scale.apply(density::META_TEXT_BASE))
        .style(|t: &iced::Theme, _status| text_input::Style {
            // Transparent: the bordered container around this row is the
            // field. An input drawing its own background inside it would
            // show as a box within a box.
            background: iced::Background::Color(iced::Color::TRANSPARENT),
            border: iced::Border::default(),
            icon: theme::text_dim(),
            placeholder: theme::text_dim(),
            value: theme::text(),
            selection: t.extended_palette().primary.weak.color,
        })
        .width(Length::Fill);
    if let Some(id) = id {
        input = input.id(id);
    }
    if let Some(submit) = on_submit {
        input = input.on_submit(submit);
    }

    container(row![ring, input].spacing(spacing::XS).align_y(iced::Alignment::Center))
        .height(Length::Fixed(density::field_height(scale)))
        .center_y(Length::Fixed(density::field_height(scale)))
        .padding([0, spacing::SM as u16])
        .style(inset_field_style)
}
