//! A countdown drawn as a ring: how much of a fixed time is left, with
//! the number of seconds in the middle.

use crate::theme::{self, surface};
use iced::widget::canvas;
use iced::{mouse, Element, Length, Point, Radians, Rectangle, Renderer, Theme};

/// A ring of side `side` with `left` of `total` still to go, the rest
/// drawn empty, and `left` written in its centre.
///
/// The ring answers what the number alone cannot: whether nine seconds
/// is most of the time or the last of it. The filled part runs clockwise
/// from twelve o'clock and shrinks as time passes, the way every
/// countdown people already know does.
///
/// A `total` of zero draws an empty ring rather than dividing by it.
pub fn countdown_ring<'a, Message: 'a>(left: u32, total: u32, side: f32) -> Element<'a, Message> {
    canvas(Ring { left, total })
        .width(Length::Fixed(side))
        .height(Length::Fixed(side))
        .into()
}

/// The fraction of the ring still filled, clamped to the ring.
pub fn remaining_fraction(left: u32, total: u32) -> f32 {
    match total {
        0 => 0.0,
        t => (left as f32 / t as f32).clamp(0.0, 1.0),
    }
}

struct Ring {
    left: u32,
    total: u32,
}

impl<Message> canvas::Program<Message, Theme, Renderer> for Ring {
    type State = ();

    fn draw(
        &self,
        _state: &Self::State,
        renderer: &Renderer,
        theme: &Theme,
        bounds: Rectangle,
        _cursor: mouse::Cursor,
    ) -> Vec<canvas::Geometry> {
        use std::f32::consts::PI;
        let mut frame = canvas::Frame::new(renderer, bounds.size());
        let side = bounds.width.min(bounds.height);
        let width = (side * 0.08).max(2.0);
        let centre = Point::new(bounds.width / 2.0, bounds.height / 2.0);
        let radius = side / 2.0 - width;

        let track = canvas::Path::circle(centre, radius);
        frame.stroke(
            &track,
            canvas::Stroke::default().with_width(width).with_color(surface::card_border()),
        );

        let fraction = remaining_fraction(self.left, self.total);
        if fraction > 0.0 {
            let start = -PI / 2.0;
            let arc = canvas::Path::new(|b| {
                b.arc(canvas::path::Arc {
                    center: centre,
                    radius,
                    start_angle: Radians(start),
                    end_angle: Radians(start + 2.0 * PI * fraction),
                });
            });
            frame.stroke(
                &arc,
                canvas::Stroke {
                    style: canvas::Style::Solid(theme.extended_palette().primary.base.color),
                    width,
                    line_cap: canvas::LineCap::Round,
                    ..canvas::Stroke::default()
                },
            );
        }

        frame.fill_text(canvas::Text {
            content: self.left.to_string(),
            position: centre,
            color: theme::text(),
            size: (side * 0.36).into(),
            align_x: iced::widget::text::Alignment::Center,
            align_y: iced::alignment::Vertical::Center,
            ..canvas::Text::default()
        });

        vec![frame.into_geometry()]
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_ring_empties_as_the_countdown_runs() {
        assert_eq!(remaining_fraction(10, 10), 1.0);
        assert_eq!(remaining_fraction(5, 10), 0.5);
        assert_eq!(remaining_fraction(0, 10), 0.0);
    }

    /// A countdown whose length never arrived must still draw, not
    /// divide by zero, and a tick that outran its total is a full ring,
    /// not more than one.
    #[test]
    fn a_ring_with_no_total_or_too_much_left_stays_a_ring() {
        assert_eq!(remaining_fraction(3, 0), 0.0);
        assert_eq!(remaining_fraction(12, 10), 1.0);
    }
}
