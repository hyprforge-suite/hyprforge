//! Telling a drag from a click.
//!
//! A row is an iced `button`, and a button *captures* the left press —
//! which is why a `mouse_area` wrapped around one never hears it (see
//! `with_row_menu` in `browser`). So a drag cannot be noticed from
//! outside the button. [`DragSource`] sits outside it anyway and looks
//! at every event *before* handing it on, capturing nothing: the button
//! still gets its press and its release, so a click is still a click and
//! a double click is still a double click. All this adds is one message
//! when a press has travelled far enough, with the button still held, to
//! stop being a click.
//!
//! What happens next — handing the files to the compositor as a
//! drag-and-drop — is the host's, because it needs the window's own
//! Wayland surface and the open/save dialog has no business starting a
//! drag at all. Once the compositor takes the drag, the pointer leaves
//! this window, so the button never hears a release and publishes no
//! click. That is the right answer: a drag was not a click.

use iced::advanced::layout;
use iced::advanced::mouse;
use iced::advanced::overlay;
use iced::advanced::renderer;
use iced::advanced::widget::{tree, Operation, Tree};
use iced::advanced::{Clipboard, Layout, Shell, Widget};
use iced::{Element, Event, Length, Point, Rectangle, Size, Vector};

/// How far, in logical pixels, a press has to travel before it is a
/// drag. GTK's own default (`gtk-dnd-drag-threshold`), so a hand used to
/// Nautilus finds the same point here. Much smaller and a slightly
/// shaky click starts a drag nobody meant; much larger and a short drag
/// reads as the app not responding.
pub const THRESHOLD: f32 = 8.0;

/// The decision itself, apart from any widget, so a test can reach it.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct Gesture {
    /// Where the left button went down over the source, while it is
    /// still down and has not yet become a drag.
    pressed_at: Option<Point>,
}

impl Gesture {
    /// The left button went down. `over` is whether it was over the
    /// source: a press that began somewhere else and slid onto a row is
    /// not a drag *of* that row.
    pub fn press(&mut self, at: Point, over: bool) {
        self.pressed_at = over.then_some(at);
    }

    /// The left button came up, or the pointer left the window: whatever
    /// was pending is over.
    pub fn release(&mut self) {
        self.pressed_at = None;
    }

    /// The pointer moved. `true` exactly once per press — the move that
    /// first crosses [`THRESHOLD`] — so a drag is started once and not
    /// again on every move after it.
    pub fn moved(&mut self, to: Point) -> bool {
        let Some(from) = self.pressed_at else {
            return false;
        };
        if from.distance(to) < THRESHOLD {
            return false;
        }
        self.pressed_at = None;
        true
    }
}

/// Publishes `on_drag` when a left press on `content` becomes a drag.
/// Wraps a button without taking anything away from it — see the module
/// doc.
pub struct DragSource<'a, Message, Theme = iced::Theme, Renderer = iced::Renderer> {
    content: Element<'a, Message, Theme, Renderer>,
    on_drag: Option<Message>,
}

/// `None` makes the wrapper inert, which is how a row being renamed
/// stays one: dragging across a text field selects its text.
pub fn drag_source<'a, Message, Theme, Renderer>(
    content: impl Into<Element<'a, Message, Theme, Renderer>>,
    on_drag: Option<Message>,
) -> DragSource<'a, Message, Theme, Renderer> {
    DragSource { content: content.into(), on_drag }
}

impl<Message, Theme, Renderer> Widget<Message, Theme, Renderer> for DragSource<'_, Message, Theme, Renderer>
where
    Renderer: renderer::Renderer,
    Message: Clone,
{
    fn tag(&self) -> tree::Tag {
        tree::Tag::of::<Gesture>()
    }

    fn state(&self) -> tree::State {
        tree::State::new(Gesture::default())
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
        // Looked at *before* the content, which is the whole trick: the
        // button below captures the press, and after that nothing
        // outside it would be told.
        if let Some(on_drag) = &self.on_drag {
            let gesture: &mut Gesture = tree.state.downcast_mut();
            match event {
                Event::Mouse(mouse::Event::ButtonPressed(mouse::Button::Left)) => {
                    if let Some(at) = cursor.position() {
                        gesture.press(at, cursor.is_over(layout.bounds()));
                    }
                }
                Event::Mouse(mouse::Event::ButtonReleased(mouse::Button::Left) | mouse::Event::CursorLeft) => {
                    gesture.release();
                }
                Event::Mouse(mouse::Event::CursorMoved { .. }) => {
                    if let Some(at) = cursor.position() {
                        if gesture.moved(at) {
                            shell.publish(on_drag.clone());
                        }
                    }
                }
                _ => {}
            }
        }
        self.content
            .as_widget_mut()
            .update(&mut tree.children[0], event, layout, cursor, renderer, clipboard, shell, viewport);
    }

    fn mouse_interaction(
        &self,
        tree: &Tree,
        layout: Layout<'_>,
        cursor: mouse::Cursor,
        viewport: &Rectangle,
        renderer: &Renderer,
    ) -> mouse::Interaction {
        self.content
            .as_widget()
            .mouse_interaction(&tree.children[0], layout, cursor, viewport, renderer)
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
        self.content
            .as_widget()
            .draw(&tree.children[0], renderer, theme, style, layout, cursor, viewport);
    }

    fn overlay<'b>(
        &'b mut self,
        tree: &'b mut Tree,
        layout: Layout<'b>,
        renderer: &Renderer,
        viewport: &Rectangle,
        translation: Vector,
    ) -> Option<overlay::Element<'b, Message, Theme, Renderer>> {
        self.content
            .as_widget_mut()
            .overlay(&mut tree.children[0], layout, renderer, viewport, translation)
    }
}

impl<'a, Message, Theme, Renderer> From<DragSource<'a, Message, Theme, Renderer>> for Element<'a, Message, Theme, Renderer>
where
    Message: 'a + Clone,
    Theme: 'a,
    Renderer: 'a + renderer::Renderer,
{
    fn from(source: DragSource<'a, Message, Theme, Renderer>) -> Self {
        Element::new(source)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_click_that_wobbles_is_still_a_click() {
        let mut g = Gesture::default();
        g.press(Point::new(10.0, 10.0), true);
        assert!(!g.moved(Point::new(13.0, 14.0)));
        g.release();
        assert!(!g.moved(Point::new(100.0, 100.0)));
    }

    #[test]
    fn a_press_that_travels_past_the_threshold_starts_one_drag_not_many() {
        let mut g = Gesture::default();
        g.press(Point::new(10.0, 10.0), true);
        assert!(g.moved(Point::new(10.0, 10.0 + THRESHOLD)));
        assert!(!g.moved(Point::new(50.0, 50.0)), "every move after the first would start another drag");
    }

    #[test]
    fn a_press_that_began_elsewhere_never_drags_this_source() {
        let mut g = Gesture::default();
        g.press(Point::new(10.0, 10.0), false);
        assert!(!g.moved(Point::new(80.0, 80.0)));
    }

    #[test]
    fn moving_without_a_press_is_hovering() {
        let mut g = Gesture::default();
        assert!(!g.moved(Point::new(80.0, 80.0)));
    }
}
