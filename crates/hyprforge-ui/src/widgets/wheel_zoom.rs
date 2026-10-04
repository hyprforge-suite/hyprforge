//! Ctrl+wheel over something to zoom it, the way every file manager and
//! browser does — without stealing the plain wheel from what is inside.
//!
//! It has to be a widget of its own, around the thing being zoomed. A
//! `mouse_area` with `on_scroll` hears the wheel only *after* its child
//! has had it, and the child here is a `scrollable`, which takes every
//! wheel event for scrolling and captures it — so Ctrl+wheel would scroll
//! the listing and never zoom it. This looks at the event first: with
//! Ctrl held and the pointer over it, the wheel is a zoom and the child
//! never sees it; otherwise everything goes to the child untouched.
//!
//! It keeps track of Ctrl itself from the window's `ModifiersChanged`,
//! because a wheel event carries no modifiers.
//!
//! A mouse wheel moves in lines, one per notch, and each notch is one
//! step. A touchpad scrolls in pixels, dozens of small events per swipe;
//! those are added up and a step is made per [`PIXELS_PER_STEP`], so a
//! two-finger swipe zooms a few levels rather than all of them.

use iced::advanced::layout;
use iced::advanced::mouse;
use iced::advanced::renderer;
use iced::advanced::widget::{tree, Operation, Tree};
use iced::advanced::{Clipboard, Layout, Shell, Widget};
use iced::{keyboard, Element, Event, Length, Rectangle, Size};

/// How far a touchpad has to scroll, in logical pixels, to make one step.
pub const PIXELS_PER_STEP: f32 = 60.0;

/// `content`, zoomed by Ctrl+wheel: `on_zoom(1)` for each step in (away
/// from you, the way a page zooms in), `on_zoom(-1)` for each step out.
pub struct WheelZoom<'a, Message, Theme = iced::Theme, Renderer = iced::Renderer> {
    content: Element<'a, Message, Theme, Renderer>,
    on_zoom: Box<dyn Fn(i32) -> Message + 'a>,
}

pub fn wheel_zoom<'a, Message, Theme, Renderer>(
    content: impl Into<Element<'a, Message, Theme, Renderer>>,
    on_zoom: impl Fn(i32) -> Message + 'a,
) -> WheelZoom<'a, Message, Theme, Renderer> {
    WheelZoom { content: content.into(), on_zoom: Box::new(on_zoom) }
}

#[derive(Debug, Default)]
struct State {
    modifiers: keyboard::Modifiers,
    /// Touchpad scrolling not yet turned into a step.
    pixels: f32,
}

/// Whole steps in `pixels`, and what is left over. Pure, so the
/// touchpad arithmetic is tested without a window.
pub fn steps_from_pixels(pixels: f32) -> (i32, f32) {
    let steps = (pixels / PIXELS_PER_STEP).trunc();
    (steps as i32, pixels - steps * PIXELS_PER_STEP)
}

impl<Message, Theme, Renderer> Widget<Message, Theme, Renderer> for WheelZoom<'_, Message, Theme, Renderer>
where
    Renderer: renderer::Renderer,
{
    fn tag(&self) -> tree::Tag {
        tree::Tag::of::<State>()
    }

    fn state(&self) -> tree::State {
        tree::State::new(State::default())
    }

    fn children(&self) -> Vec<Tree> {
        vec![Tree::new(&self.content)]
    }

    fn diff(&self, tree: &mut Tree) {
        tree.diff_children(std::slice::from_ref(&self.content));
    }

    fn size(&self) -> Size<Length> {
        self.content.as_widget().size()
    }

    fn size_hint(&self) -> Size<Length> {
        self.content.as_widget().size_hint()
    }

    fn layout(&mut self, tree: &mut Tree, renderer: &Renderer, limits: &layout::Limits) -> layout::Node {
        self.content.as_widget_mut().layout(&mut tree.children[0], renderer, limits)
    }

    fn operate(&mut self, tree: &mut Tree, layout: Layout<'_>, renderer: &Renderer, operation: &mut dyn Operation) {
        self.content.as_widget_mut().operate(&mut tree.children[0], layout, renderer, operation);
    }

    fn update(
        &mut self,
        tree: &mut Tree,
        event: &Event,
        layout: Layout<'_>,
        cursor: mouse::Cursor,
        renderer: &Renderer,
        clipboard: &mut dyn Clipboard,
        shell: &mut Shell<'_, Message>,
        viewport: &Rectangle,
    ) {
        let state = tree.state.downcast_mut::<State>();
        match event {
            Event::Keyboard(keyboard::Event::ModifiersChanged(modifiers)) => {
                state.modifiers = *modifiers;
                if !modifiers.control() {
                    state.pixels = 0.0;
                }
            }
            Event::Mouse(mouse::Event::WheelScrolled { delta })
                if state.modifiers.control() && cursor.is_over(layout.bounds()) =>
            {
                let steps = match delta {
                    mouse::ScrollDelta::Lines { y, .. } => y.signum() as i32 * (y.abs().round().max(1.0) as i32),
                    mouse::ScrollDelta::Pixels { y, .. } => {
                        let (steps, rest) = steps_from_pixels(state.pixels + y);
                        state.pixels = rest;
                        steps
                    }
                };
                if steps != 0 {
                    shell.publish((self.on_zoom)(steps));
                }
                // Taken either way: a Ctrl+wheel that did not make a whole
                // step yet is still a zoom in progress, not a scroll.
                shell.capture_event();
                return;
            }
            _ => {}
        }
        self.content.as_widget_mut().update(
            &mut tree.children[0],
            event,
            layout,
            cursor,
            renderer,
            clipboard,
            shell,
            viewport,
        );
    }

    fn mouse_interaction(
        &self,
        tree: &Tree,
        layout: Layout<'_>,
        cursor: mouse::Cursor,
        viewport: &Rectangle,
        renderer: &Renderer,
    ) -> mouse::Interaction {
        self.content.as_widget().mouse_interaction(&tree.children[0], layout, cursor, viewport, renderer)
    }

    fn draw(
        &self,
        tree: &Tree,
        renderer: &mut Renderer,
        theme: &Theme,
        style: &renderer::Style,
        layout: Layout<'_>,
        cursor: mouse::Cursor,
        viewport: &Rectangle,
    ) {
        self.content.as_widget().draw(&tree.children[0], renderer, theme, style, layout, cursor, viewport);
    }

    fn overlay<'b>(
        &'b mut self,
        tree: &'b mut Tree,
        layout: Layout<'b>,
        renderer: &Renderer,
        viewport: &Rectangle,
        translation: iced::Vector,
    ) -> Option<iced::advanced::overlay::Element<'b, Message, Theme, Renderer>> {
        self.content.as_widget_mut().overlay(&mut tree.children[0], layout, renderer, viewport, translation)
    }
}

impl<'a, Message, Theme, Renderer> From<WheelZoom<'a, Message, Theme, Renderer>> for Element<'a, Message, Theme, Renderer>
where
    Message: 'a,
    Theme: 'a,
    Renderer: renderer::Renderer + 'a,
{
    fn from(zoom: WheelZoom<'a, Message, Theme, Renderer>) -> Self {
        Element::new(zoom)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_short_touchpad_scroll_makes_no_step_and_keeps_what_it_scrolled() {
        assert_eq!(steps_from_pixels(25.0), (0, 25.0));
    }

    #[test]
    fn a_long_touchpad_scroll_makes_whole_steps_and_keeps_the_rest() {
        let (steps, rest) = steps_from_pixels(2.5 * PIXELS_PER_STEP);
        assert_eq!(steps, 2);
        assert!((rest - 0.5 * PIXELS_PER_STEP).abs() < 0.001);
    }

    #[test]
    fn scrolling_back_steps_the_other_way() {
        assert_eq!(steps_from_pixels(-PIXELS_PER_STEP).0, -1);
    }
}
