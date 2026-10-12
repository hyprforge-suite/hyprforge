//! The divider between two panes side by side, as something to drag.
//!
//! The line is [`super::vertical_divider`]'s hairline; what this adds is
//! a hit area around it wide enough to grab, the resize cursor that says
//! it can be, and the arithmetic for where a drag leaves it — kept here,
//! pure, so every two-pane window splits by the same rule.

use iced::widget::{container, mouse_area};
use iced::{mouse, Element, Length};

/// How wide the grabbable strip is. The line stays one pixel; a strip
/// that narrow is a target nobody can hit, and one much wider eats into
/// both panes.
pub const SPLIT_HANDLE_WIDTH: f32 = 7.0;

/// The divider, draggable: `on_press` starts a drag (the host follows the
/// pointer until the button is let go, and asks [`split_ratio`] where it
/// is), `on_reset` is a double click, which puts the halves back even.
pub fn split_handle<'a, Message: Clone + 'a>(on_press: Message, on_reset: Message) -> Element<'a, Message> {
    let strip = container(super::vertical_divider())
        .width(Length::Fixed(SPLIT_HANDLE_WIDTH))
        .height(Length::Fill)
        .center_x(Length::Fixed(SPLIT_HANDLE_WIDTH));
    mouse_area(strip)
        .interaction(mouse::Interaction::ResizingHorizontally)
        .on_press(on_press)
        .on_double_click(on_reset)
        .into()
}

/// Where a divider dragged to `x` in a `width`-wide area leaves it, as the
/// left pane's share — never so far that either pane is narrower than
/// `min_side`. An area too narrow for two of those splits evenly, the
/// one answer that favours neither.
pub fn split_ratio(x: f32, width: f32, min_side: f32) -> f32 {
    if !(x.is_finite() && width.is_finite()) || width <= 2.0 * min_side {
        return 0.5;
    }
    let edge = min_side / width;
    (x / width).clamp(edge, 1.0 - edge)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_drag_follows_the_pointer_but_never_squeezes_a_pane_below_its_minimum() {
        assert_eq!(split_ratio(300.0, 1000.0, 200.0), 0.3);
        assert_eq!(split_ratio(50.0, 1000.0, 200.0), 0.2, "left kept at 200");
        assert_eq!(split_ratio(990.0, 1000.0, 200.0), 0.8, "right kept at 200");
    }

    #[test]
    fn an_area_too_narrow_for_two_minimums_splits_evenly() {
        assert_eq!(split_ratio(100.0, 300.0, 200.0), 0.5);
        assert_eq!(split_ratio(f32::NAN, 1000.0, 200.0), 0.5);
    }
}
