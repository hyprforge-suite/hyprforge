//! A card over the dimmed window: the shape a modal glance takes.
//!
//! Files' Quick Look was the first written here; Files' own dialogs drew
//! the same dim layer in the app before it, and a lightbox in Media or a
//! sheet in Settings would draw it again. Nothing about it is about
//! files — it is "this, over everything, until you look away".

use crate::theme::surface;
use iced::widget::{container, mouse_area, opaque, stack, Space};
use iced::{Background, Color, Element, Length};

/// How much of the window behind a scrim hides. Enough that the card is
/// plainly what the keyboard and pointer are on, little enough that the
/// window still reads as there.
pub const SCRIM_ALPHA: f32 = 0.6;

/// The dim layer's colour: the window's own root step, see-through —
/// never an invented black, which would be a colour constant no theme
/// can reach, and never the accent, which means selected.
pub fn scrim_color() -> Color {
    Color { a: SCRIM_ALPHA, ..surface::root() }
}

/// `content`, centred over a dim layer that fills whatever it is stacked
/// over.
///
/// A press on the dim layer — either button — sends `on_dismiss`; with
/// `None` the layer only swallows the press, for a question the window
/// is waiting on. Both layers are `opaque`, so nothing beneath is
/// clickable or scrollable while a scrim is up: iced's `stack` gives the
/// layers under one a levitated cursor exactly when the one above
/// reports an interaction, and `opaque` always does.
///
/// The caller sizes `content`. The scrim only centres it — a card that
/// wants a margin from the window's edges gives itself one.
pub fn scrim<'a, Message: Clone + 'a>(
    content: impl Into<Element<'a, Message>>,
    on_dismiss: Option<Message>,
) -> Element<'a, Message> {
    let dim = container(Space::new().width(Length::Fill).height(Length::Fill))
        .width(Length::Fill)
        .height(Length::Fill)
        .style(|_t: &iced::Theme| container::Style {
            background: Some(Background::Color(scrim_color())),
            ..container::Style::default()
        });
    let away: Element<'a, Message> = match on_dismiss {
        Some(message) => opaque(mouse_area(dim).on_press(message.clone()).on_right_press(message)),
        None => opaque(dim),
    };
    let placed = container(opaque(content))
        .center_x(Length::Fill)
        .center_y(Length::Fill);
    stack![away, placed].width(Length::Fill).height(Length::Fill).into()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The dim layer is the window's own surface, see-through — the rule
    /// that no app adds a colour of its own, held for the one layer that
    /// would be easiest to fill with plain black.
    #[test]
    fn a_scrim_dims_with_the_windows_own_root_colour() {
        let dim = scrim_color();
        let root = surface::root();
        assert_eq!((dim.r, dim.g, dim.b), (root.r, root.g, root.b));
        assert!(dim.a > 0.0 && dim.a < 1.0, "see-through, not a wall: {}", dim.a);
    }
}
