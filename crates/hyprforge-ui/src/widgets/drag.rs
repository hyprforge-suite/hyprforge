//! What a drag looks like: the card that travels under the pointer, and
//! the place it is about to land.
//!
//! Files' was the first written here. Nothing about either is about
//! files: Media dragging a picture into an album, or Settings dragging
//! a rule up its list, would want a card under the pointer and a target
//! that says "here" in the same way.
//!
//! Two rules from the catalogue shape them:
//!
//! - **Purple means selected.** A drop target is not a selection, so it
//!   is drawn in the foreground colour — an outline crisp enough to tell
//!   from a hover, which has none, and a fill. What is being *carried*
//!   is the selection, so the card's count badge is the one place the
//!   accent appears.
//! - **Progress is not selection.** A target that will open if the drag
//!   rests on it (a spring-loaded folder) fills from the left as the
//!   wait runs out, in the foreground colour, so the opening is
//!   announced rather than sprung.

use crate::theme::{self, spacing, surface, FontScale};
use crate::widgets::scaled_text;
use iced::widget::{container, row, stack, Space};
use iced::{gradient, Background, Border, Color, Element, Length, Padding, Radians, Shadow, Vector};

/// How a drop target is drawn.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub enum DropLook {
    /// Nothing is held over it.
    #[default]
    Idle,
    /// A drag is over it. `opening` is how much of the wait before it
    /// opens has passed, `0.0` to `1.0` — `0.0` for one that does not
    /// open, or has not started to.
    Over { opening: f32 },
}

/// How opaque the target's fill is at its strongest — the part of the
/// wait that has passed.
const OPENING_ALPHA: f32 = 0.16;

/// The style of a container that something can be dropped on.
pub fn drop_target_style(look: DropLook) -> container::Style {
    let radius = crate::density::inner_radius();
    let DropLook::Over { opening } = look else {
        return container::Style {
            border: Border { radius: radius.into(), ..Border::default() },
            ..container::Style::default()
        };
    };
    let opening = opening.clamp(0.0, 1.0);
    let filled = Color { a: OPENING_ALPHA, ..theme::text() };
    let background = if opening <= 0.0 {
        Background::Color(surface::row())
    } else {
        // Two stops at the same offset make a hard edge: the part of the
        // wait that has passed, and the part still to come.
        Background::Gradient(
            gradient::Linear::new(Radians(std::f32::consts::FRAC_PI_2))
                .add_stop(0.0, filled)
                .add_stop(opening, filled)
                .add_stop(opening, surface::row())
                .add_stop(1.0, surface::row())
                .into(),
        )
    };
    container::Style {
        background: Some(background),
        border: Border { color: Color { a: 0.85, ..theme::text() }, width: 1.5, radius: radius.into() },
        ..container::Style::default()
    }
}

/// What the card under the pointer says.
#[derive(Debug, Clone, PartialEq)]
pub struct DragCard {
    /// The first thing carried, by name.
    pub label: String,
    /// How many things are carried. More than one stacks the card and
    /// shows the count.
    pub count: usize,
    /// Letting go copies rather than moves — Ctrl is held. A `+` says so,
    /// the mark every desktop uses for it.
    pub copying: bool,
}

/// The card under the pointer, `lift` of the way picked up: `0.0` lying
/// flat and invisible, `1.0` held. Picking up and letting go both pass
/// through the values between, which is the whole of the animation —
/// the caller eases `lift` over time and redraws.
pub fn drag_card<'a, Message: 'a>(card: &DragCard, lift: f32, scale: FontScale) -> Element<'a, Message> {
    let lift = lift.clamp(0.0, 1.0);
    let fade = move |c: Color| Color { a: c.a * lift, ..c };
    let radius = crate::density::inner_radius();
    let face = move |shadow: bool| {
        move |_: &iced::Theme| container::Style {
            background: Some(Background::Color(fade(surface::sidebar()))),
            text_color: Some(fade(theme::text())),
            border: Border { color: fade(surface::card_border()), width: 1.0, radius: radius.into() },
            // The shadow grows as the card rises: what makes it read as
            // picked up rather than slid along. The root step, darkened
            // by its own alpha — never an invented black.
            shadow: if shadow {
                Shadow {
                    color: fade(Color { a: 0.55, ..surface::root() }),
                    offset: Vector::new(0.0, 2.0 + 6.0 * lift),
                    blur_radius: 4.0 + 14.0 * lift,
                }
            } else {
                Shadow::default()
            },
            ..container::Style::default()
        }
    };
    let label = scaled_text(card.label.clone(), 13.0, scale).wrapping(iced::widget::text::Wrapping::None);
    let mut front = row![label].spacing(spacing::SM).align_y(iced::Alignment::Center);
    if card.copying {
        front = front.push(badge("+".to_string(), fade(theme::success()), scale));
    }
    if card.count > 1 {
        front = front.push(badge(card.count.to_string(), fade(crate::color::to_iced(theme::active().accent)), scale));
    }
    let top = container(front).padding([spacing::XS + 2.0, spacing::SM + 4.0]).style(face(true));
    if card.count <= 1 {
        return container(top).padding(Padding { right: 8.0, bottom: 8.0, ..Padding::ZERO }).into();
    }
    // More than one: two blank cards behind, each offset a little further,
    // as a pile of papers is.
    let behind = |offset: f32| {
        container(container(Space::new().width(Length::Fill).height(Length::Fill)).style(face(false)))
            .padding(Padding { left: offset, top: offset, right: 8.0 - offset, bottom: 8.0 - offset })
            .width(Length::Fill)
            .height(Length::Fill)
    };
    stack![
        behind(8.0),
        behind(4.0),
        container(top).padding(Padding { right: 8.0, bottom: 8.0, ..Padding::ZERO }),
    ]
    .into()
}

/// A small round mark with a number or a sign in it.
fn badge<'a, Message: 'a>(text: String, fill: Color, scale: FontScale) -> Element<'a, Message> {
    container(scaled_text(text, 11.0, scale))
        .padding([1.0, 6.0])
        .style(move |_: &iced::Theme| container::Style {
            background: Some(Background::Color(fill)),
            text_color: Some(Color { a: fill.a, ..surface::root() }),
            border: Border { radius: 999.0.into(), ..Border::default() },
            ..container::Style::default()
        })
        .into()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_target_nothing_is_over_draws_nothing() {
        let style = drop_target_style(DropLook::Idle);
        assert!(style.background.is_none());
        assert_eq!(style.border.width, 0.0);
    }

    #[test]
    fn a_target_under_a_drag_has_an_outline_a_hover_does_not() {
        let style = drop_target_style(DropLook::Over { opening: 0.0 });
        assert!(style.border.width > 0.0);
    }

    /// Purple means selected — see the catalogue. A drop target is not a
    /// selection, so the accent appears nowhere in its look, whatever
    /// part of the wait has passed.
    #[test]
    fn a_drop_target_never_uses_the_accent() {
        let accent = crate::color::to_iced(theme::active().accent);
        for opening in [0.0, 0.5, 1.0] {
            let style = drop_target_style(DropLook::Over { opening });
            assert_ne!(style.border.color, accent);
            if let Some(Background::Color(c)) = style.background {
                assert_ne!(c, accent);
            }
        }
    }

    #[test]
    fn an_opening_target_fills_as_far_as_the_wait_has_gone() {
        let Some(Background::Gradient(iced::Gradient::Linear(linear))) =
            drop_target_style(DropLook::Over { opening: 0.4 }).background
        else {
            panic!("an opening target is filled part of the way")
        };
        let edges: Vec<f32> = linear.stops.iter().flatten().map(|s| s.offset).collect();
        assert!(edges.contains(&0.4), "{edges:?}");
    }
}
