//! Something drawn under a widget, at that widget's width, over whatever
//! is below it — the shape of every "suggestions under a field" in the
//! suite.
//!
//! Files' path bar was the first: its candidates hang from the path field. They cannot be a
//! row in the window's column — that would push the listing down a line
//! per keystroke — and they cannot be a layer of the window stacked at a
//! computed offset either: the field's left edge depends on the widths of
//! everything beside it on the bar, the view toggle's included, and iced
//! gives a layout function no way to ask. Only the field's own layout
//! knows where the field is, so the panel is placed from there, as an
//! overlay — the same mechanism iced's own tooltip and pick list use, and
//! one that draws above every layer of the window.

use iced::advanced::layout;
use iced::advanced::mouse;
use iced::advanced::overlay;
use iced::advanced::renderer;
use iced::advanced::widget::{Operation, Tree};
use iced::advanced::{Clipboard, Layout, Shell, Widget};
use iced::{Element, Event, Length, Point, Rectangle, Size, Vector};

/// `base`, with `below` hung from its bottom edge when there is one.
pub struct Anchored<'a, Message, Theme = iced::Theme, Renderer = iced::Renderer> {
    base: Element<'a, Message, Theme, Renderer>,
    below: Option<Element<'a, Message, Theme, Renderer>>,
    /// Space between the two, in logical pixels.
    gap: f32,
}

pub fn anchored<'a, Message, Theme, Renderer>(
    base: impl Into<Element<'a, Message, Theme, Renderer>>,
    below: Option<Element<'a, Message, Theme, Renderer>>,
    gap: f32,
) -> Anchored<'a, Message, Theme, Renderer> {
    Anchored { base: base.into(), below, gap }
}

impl<Message, Theme, Renderer> Widget<Message, Theme, Renderer> for Anchored<'_, Message, Theme, Renderer>
where
    Renderer: renderer::Renderer,
{
    fn children(&self) -> Vec<Tree> {
        std::iter::once(&self.base).chain(self.below.as_ref()).map(Tree::new).collect()
    }

    fn diff(&self, tree: &mut Tree) {
        // One child or two: `diff_children` adds or drops the second as
        // the panel comes and goes, and the field's own state — its
        // focus and cursor — stays put in the first.
        match &self.below {
            Some(below) => tree.diff_children(&[&self.base, below]),
            None => tree.diff_children(std::slice::from_ref(&self.base)),
        }
    }

    fn size(&self) -> Size<Length> {
        self.base.as_widget().size()
    }

    fn size_hint(&self) -> Size<Length> {
        self.base.as_widget().size_hint()
    }

    fn layout(&mut self, tree: &mut Tree, renderer: &Renderer, limits: &layout::Limits) -> layout::Node {
        self.base.as_widget_mut().layout(&mut tree.children[0], renderer, limits)
    }

    fn operate(&mut self, tree: &mut Tree, layout: Layout<'_>, renderer: &Renderer, operation: &mut dyn Operation) {
        self.base.as_widget_mut().operate(&mut tree.children[0], layout, renderer, operation);
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
        self.base.as_widget_mut().update(
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
        self.base.as_widget().mouse_interaction(&tree.children[0], layout, cursor, viewport, renderer)
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
        self.base.as_widget().draw(&tree.children[0], renderer, theme, style, layout, cursor, viewport);
    }

    fn overlay<'b>(
        &'b mut self,
        tree: &'b mut Tree,
        layout: Layout<'b>,
        renderer: &Renderer,
        viewport: &Rectangle,
        translation: Vector,
    ) -> Option<overlay::Element<'b, Message, Theme, Renderer>> {
        let mut children = tree.children.iter_mut();
        let base_tree = children.next()?;
        let own = self.base.as_widget_mut().overlay(base_tree, layout, renderer, viewport, translation);
        let hung = match (self.below.as_mut(), children.next()) {
            (Some(below), Some(below_tree)) => {
                let bounds = layout.bounds();
                Some(overlay::Element::new(Box::new(Hung {
                    content: below,
                    tree: below_tree,
                    at: Point::new(bounds.x, bounds.y + bounds.height + self.gap) + translation,
                    width: bounds.width,
                })))
            }
            _ => None,
        };
        match (own, hung) {
            (None, None) => None,
            (Some(one), None) | (None, Some(one)) => Some(one),
            (Some(own), Some(hung)) => Some(overlay::Group::with_children(vec![own, hung]).overlay()),
        }
    }
}

impl<'a, Message, Theme, Renderer> From<Anchored<'a, Message, Theme, Renderer>> for Element<'a, Message, Theme, Renderer>
where
    Message: 'a,
    Theme: 'a,
    Renderer: renderer::Renderer + 'a,
{
    fn from(anchored: Anchored<'a, Message, Theme, Renderer>) -> Self {
        Element::new(anchored)
    }
}

/// The overlay itself: `content`, laid out at `width` from `at`.
struct Hung<'a, 'b, Message, Theme, Renderer> {
    content: &'b mut Element<'a, Message, Theme, Renderer>,
    tree: &'b mut Tree,
    at: Point,
    width: f32,
}

impl<Message, Theme, Renderer> overlay::Overlay<Message, Theme, Renderer> for Hung<'_, '_, Message, Theme, Renderer>
where
    Renderer: renderer::Renderer,
{
    fn layout(&mut self, renderer: &Renderer, bounds: Size) -> layout::Node {
        // Exactly the anchor's width, and no taller than what is left of
        // the window below it — a long list is cut off by the window
        // edge rather than drawn past it.
        let room = (bounds.height - self.at.y).max(0.0);
        let limits = layout::Limits::new(Size::new(self.width, 0.0), Size::new(self.width, room));
        self.content
            .as_widget_mut()
            .layout(self.tree, renderer, &limits)
            .move_to(self.at)
    }

    fn draw(
        &self,
        renderer: &mut Renderer,
        theme: &Theme,
        style: &renderer::Style,
        layout: Layout<'_>,
        cursor: mouse::Cursor,
    ) {
        let bounds = layout.bounds();
        self.content.as_widget().draw(self.tree, renderer, theme, style, layout, cursor, &bounds);
    }

    fn operate(&mut self, layout: Layout<'_>, renderer: &Renderer, operation: &mut dyn Operation) {
        self.content.as_widget_mut().operate(self.tree, layout, renderer, operation);
    }

    fn update(
        &mut self,
        event: &Event,
        layout: Layout<'_>,
        cursor: mouse::Cursor,
        renderer: &Renderer,
        clipboard: &mut dyn Clipboard,
        shell: &mut Shell<'_, Message>,
    ) {
        let bounds = layout.bounds();
        self.content
            .as_widget_mut()
            .update(self.tree, event, layout, cursor, renderer, clipboard, shell, &bounds);
    }

    fn mouse_interaction(&self, layout: Layout<'_>, cursor: mouse::Cursor, renderer: &Renderer) -> mouse::Interaction {
        let bounds = layout.bounds();
        self.content.as_widget().mouse_interaction(self.tree, layout, cursor, &bounds, renderer)
    }
}
