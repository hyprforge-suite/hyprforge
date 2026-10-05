//! Rubber-band selection: drag across empty space to draw a box, and
//! everything the box touches is selected — what every desktop file
//! manager does, and the one way to select a block of icons in a grid
//! without the keyboard.
//!
//! A widget of its own around the whole listing, not around the rows,
//! because the band almost always *starts* where there are no rows: in
//! the gap beside a short list, or below the last icon. It hands every
//! press to its content first and starts a band only when nothing there
//! took it ([`Shell::is_event_captured`]) — so a press on a row is still
//! that row's click, its double click, its drag, and the band is exactly
//! "a press on nothing".
//!
//! It knows nothing about rows. The caller tags each one with a container
//! [`Id`] and hands over `targets`, id to index; the band asks the layout
//! which of those containers it meets, through an [`Operation`] over its
//! own content — scrolled content reports bounds that have not moved with
//! the scroll, so the operation carries each scrollable's translation and
//! visible region, as `hyprforge-files-core`'s drop hit-test does. The
//! caller can leave `targets` empty until it hears `on_start`, so a frame
//! with no band in progress tags nothing.
//!
//! The band is anchored in the *content*, not the window: scroll while
//! dragging — the wheel, or the automatic scroll near an edge — and the
//! corner you started at moves with what was under it, the way it does in
//! every file manager, rather than staying put on the screen while the
//! rows slide past it.

use iced::advanced::layout;
use iced::advanced::mouse;
use iced::advanced::overlay;
use iced::advanced::renderer;
use iced::advanced::widget::operation::scrollable::AbsoluteOffset;
use iced::advanced::widget::operation::{self, Operation};
use iced::advanced::widget::{tree, Id, Tree};
use iced::advanced::{Clipboard, Layout, Shell, Widget};
use iced::{keyboard, window, Border, Color, Element, Event, Length, Point, Rectangle, Size, Vector};
use std::collections::HashMap;

/// How far a press on empty space travels before it is a band rather
/// than a click — the same as a row's drag threshold, so the hand finds
/// one distance for both.
pub const THRESHOLD: f32 = 8.0;

/// How close to the top or bottom edge the pointer has to be, in logical
/// pixels, before the listing scrolls on its own to follow the band.
pub const EDGE: f32 = 36.0;

/// The fastest automatic scroll, in logical pixels per frame — reached
/// with the pointer at or past the edge, scaled down further in.
pub const SPEED: f32 = 18.0;

/// What `on_select` is handed: the targets met, and whether the band adds.
type OnSelect<'a, Message> = Box<dyn Fn(Vec<usize>, bool) -> Message + 'a>;

/// `content`, with a rubber band drawn across empty space. See the
/// module doc.
pub struct Marquee<'a, Message, Theme = iced::Theme, Renderer = iced::Renderer> {
    content: Element<'a, Message, Theme, Renderer>,
    targets: HashMap<Id, usize>,
    scroll: Option<Id>,
    on_start: Option<Message>,
    on_select: Option<OnSelect<'a, Message>>,
    on_end: Option<Message>,
    on_clear: Option<Message>,
}

pub fn marquee<'a, Message, Theme, Renderer>(
    content: impl Into<Element<'a, Message, Theme, Renderer>>,
) -> Marquee<'a, Message, Theme, Renderer> {
    Marquee {
        content: content.into(),
        targets: HashMap::new(),
        scroll: None,
        on_start: None,
        on_select: None,
        on_end: None,
        on_clear: None,
    }
}

impl<'a, Message, Theme, Renderer> Marquee<'a, Message, Theme, Renderer> {
    /// The containers the band can select, by id, and what each is to the
    /// caller — a row's index.
    pub fn targets(mut self, targets: HashMap<Id, usize>) -> Self {
        self.targets = targets;
        self
    }

    /// The scrollable inside that holds the targets: what the band
    /// anchors to, and what scrolls when the pointer nears an edge.
    pub fn scroll(mut self, id: Id) -> Self {
        self.scroll = Some(id);
        self
    }

    /// A band began — the press on nothing has travelled past
    /// `THRESHOLD`.
    pub fn on_start(mut self, message: Message) -> Self {
        self.on_start = Some(message);
        self
    }

    /// The targets the band meets now, in ascending order, and whether
    /// Ctrl or Shift is held — adding to what was selected before the
    /// band began rather than replacing it. Published only when the set
    /// changes.
    pub fn on_select(mut self, f: impl Fn(Vec<usize>, bool) -> Message + 'a) -> Self {
        self.on_select = Some(Box::new(f));
        self
    }

    /// The band was let go.
    pub fn on_end(mut self, message: Message) -> Self {
        self.on_end = Some(message);
        self
    }

    /// A click on empty space that never became a band: in every file
    /// manager, what clears the selection.
    pub fn on_clear(mut self, message: Message) -> Self {
        self.on_clear = Some(message);
        self
    }
}

/// The band's own state between events.
#[derive(Debug, Clone, Default)]
struct State {
    /// Where an uncaptured left press went down, in window coordinates,
    /// while the button is held.
    press: Option<Point>,
    /// The same point in the scrolled content's coordinates — the corner
    /// that moves with the rows.
    anchor: Point,
    /// Past the threshold: a band, not a click.
    active: bool,
    /// The pointer, in window coordinates.
    cursor: Point,
    /// The scroll translation last seen for `scroll`.
    translation: Vector,
    /// What `on_select` last published, so an unchanged set is not sent
    /// again on every pixel of movement.
    last: Vec<usize>,
    modifiers: keyboard::Modifiers,
}

/// The rectangle between the anchor — moved by however far the content
/// has scrolled since — and the pointer, in window coordinates.
pub fn band_rect(anchor: Point, translation: Vector, cursor: Point) -> Rectangle {
    let start = anchor - translation;
    let (x0, x1) = (start.x.min(cursor.x), start.x.max(cursor.x));
    let (y0, y1) = (start.y.min(cursor.y), start.y.max(cursor.y));
    Rectangle { x: x0, y: y0, width: x1 - x0, height: y1 - y0 }
}

/// How far to scroll this frame for a pointer at `y` over a viewport
/// spanning `top..bottom`: nothing in the middle, up to `SPEED` at and
/// past either edge, proportionally within `EDGE` of it.
pub fn edge_scroll(y: f32, top: f32, bottom: f32) -> f32 {
    let ramp = |into: f32| SPEED * (1.0 - (into / EDGE)).clamp(0.0, 1.0);
    if y < top + EDGE {
        -ramp(y - top)
    } else if y > bottom - EDGE {
        ramp(bottom - y)
    } else {
        0.0
    }
}

/// Which targets a rectangle meets, and the translation of the
/// scrollable named `scroll`.
struct Gather<'t> {
    rect: Rectangle,
    targets: &'t HashMap<Id, usize>,
    scroll: Option<&'t Id>,
    found: Vec<usize>,
    translation: Option<Vector>,
    frames: Vec<Frame>,
    entering: Option<Frame>,
}

#[derive(Clone, Copy)]
struct Frame {
    translation: Vector,
    visible: Option<Rectangle>,
}

impl<'t> Gather<'t> {
    fn new(rect: Rectangle, targets: &'t HashMap<Id, usize>, scroll: Option<&'t Id>) -> Self {
        Gather {
            rect,
            targets,
            scroll,
            found: Vec::new(),
            translation: None,
            frames: vec![Frame { translation: Vector::ZERO, visible: None }],
            entering: None,
        }
    }

    fn frame(&self) -> Frame {
        self.frames.last().copied().unwrap_or(Frame { translation: Vector::ZERO, visible: None })
    }
}

impl Operation for Gather<'_> {
    fn traverse(&mut self, operate: &mut dyn FnMut(&mut dyn Operation)) {
        let next = self.entering.take().unwrap_or_else(|| self.frame());
        self.frames.push(next);
        operate(self);
        self.frames.pop();
    }

    fn container(&mut self, id: Option<&Id>, bounds: Rectangle) {
        let Some(&index) = id.and_then(|id| self.targets.get(id)) else { return };
        let frame = self.frame();
        // Where the container is drawn: its layout, moved by the scroll.
        let drawn = Rectangle { x: bounds.x - frame.translation.x, y: bounds.y - frame.translation.y, ..bounds };
        // Only the part actually on screen: a row scrolled out of sight
        // still has a layout, and must not be selected by a band drawn
        // over whatever is shown where it would have been.
        let shown = match frame.visible {
            Some(visible) => visible.intersection(&drawn),
            None => Some(drawn),
        };
        if shown.is_some_and(|shown| shown.intersects(&self.rect)) {
            self.found.push(index);
        }
    }

    fn scrollable(
        &mut self,
        id: Option<&Id>,
        bounds: Rectangle,
        _content_bounds: Rectangle,
        translation: Vector,
        _state: &mut dyn operation::Scrollable,
    ) {
        if self.scroll.is_some() && id == self.scroll {
            self.translation = Some(translation);
        }
        let outer = self.frame();
        let viewport = Rectangle { x: bounds.x - outer.translation.x, y: bounds.y - outer.translation.y, ..bounds };
        let visible = match outer.visible {
            Some(v) => v.intersection(&viewport),
            None => Some(viewport),
        };
        self.entering = Some(Frame {
            translation: outer.translation + translation,
            visible: Some(visible.unwrap_or(Rectangle::new(Point::ORIGIN, Size::ZERO))),
        });
    }
}

impl<'a, Message, Theme, Renderer> Marquee<'a, Message, Theme, Renderer>
where
    Renderer: renderer::Renderer,
    Message: Clone,
{
    /// Asks the layout what `rect` meets, and remembers the scroll.
    fn gather(&mut self, tree: &mut Tree, layout: Layout<'_>, renderer: &Renderer, rect: Rectangle) -> (Vec<usize>, Vector) {
        let mut gather = Gather::new(rect, &self.targets, self.scroll.as_ref());
        self.content.as_widget_mut().operate(&mut tree.children[0], layout, renderer, &mut gather);
        let mut found = gather.found;
        found.sort_unstable();
        found.dedup();
        (found, gather.translation.unwrap_or(Vector::ZERO))
    }

    /// Re-reads what the band meets and publishes it if it changed.
    fn reselect(&mut self, tree: &mut Tree, layout: Layout<'_>, renderer: &Renderer, shell: &mut Shell<'_, Message>) {
        let state: &State = tree.state.downcast_ref();
        let rect = band_rect(state.anchor, state.translation, state.cursor);
        let (found, translation) = self.gather(tree, layout, renderer, rect);
        let state: &mut State = tree.state.downcast_mut();
        state.translation = translation;
        if found != state.last {
            state.last = found.clone();
            if let Some(on_select) = &self.on_select {
                let additive = state.modifiers.control() || state.modifiers.shift();
                shell.publish(on_select(found, additive));
            }
        }
    }
}

impl<Message, Theme, Renderer> Widget<Message, Theme, Renderer> for Marquee<'_, Message, Theme, Renderer>
where
    Renderer: renderer::Renderer,
    Message: Clone,
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
        if let Event::Keyboard(keyboard::Event::ModifiersChanged(modifiers)) = event {
            tree.state.downcast_mut::<State>().modifiers = *modifiers;
        }
        let active = tree.state.downcast_ref::<State>().active;

        match event {
            // The content first: a press it takes is a row's, and the
            // band only ever starts on a press nothing took.
            Event::Mouse(mouse::Event::ButtonPressed(mouse::Button::Left)) => {
                self.content
                    .as_widget_mut()
                    .update(&mut tree.children[0], event, layout, cursor, renderer, clipboard, shell, viewport);
                if shell.is_event_captured() {
                    return;
                }
                let Some(at) = cursor.position_over(layout.bounds()) else { return };
                let (_, translation) = self.gather(tree, layout, renderer, Rectangle::new(at, Size::ZERO));
                let state: &mut State = tree.state.downcast_mut();
                *state = State {
                    press: Some(at),
                    anchor: at + translation,
                    cursor: at,
                    translation,
                    modifiers: state.modifiers,
                    ..State::default()
                };
                return;
            }
            Event::Mouse(mouse::Event::CursorMoved { position }) => {
                let state: &mut State = tree.state.downcast_mut();
                if let Some(press) = state.press {
                    state.cursor = *position;
                    if !state.active && press.distance(*position) >= THRESHOLD {
                        state.active = true;
                        if let Some(start) = &self.on_start {
                            shell.publish(start.clone());
                        }
                    }
                    if state.active {
                        self.reselect(tree, layout, renderer, shell);
                        shell.request_redraw();
                        // The band has the pointer: rows under it must
                        // not light up as hovered while it passes.
                        shell.capture_event();
                        return;
                    }
                }
            }
            Event::Mouse(mouse::Event::ButtonReleased(mouse::Button::Left)) => {
                let state: &mut State = tree.state.downcast_mut();
                let (pressed, was_active) = (state.press.take(), state.active);
                state.active = false;
                state.last.clear();
                if was_active {
                    if let Some(end) = &self.on_end {
                        shell.publish(end.clone());
                    }
                    shell.request_redraw();
                    shell.capture_event();
                    return;
                }
                if pressed.is_some() {
                    if let Some(clear) = &self.on_clear {
                        shell.publish(clear.clone());
                    }
                }
            }
            // Following the band past an edge: one step per frame while
            // the pointer is near it, and another frame asked for until
            // it moves away or lets go.
            Event::Window(window::Event::RedrawRequested(_)) if active => {
                let bounds = layout.bounds();
                let y = tree.state.downcast_ref::<State>().cursor.y;
                let step = edge_scroll(y, bounds.y, bounds.y + bounds.height);
                if step != 0.0 {
                    if let Some(id) = self.scroll.clone() {
                        let mut scroll = operation::scrollable::scroll_by::<()>(id, AbsoluteOffset { x: 0.0, y: step });
                        self.content.as_widget_mut().operate(&mut tree.children[0], layout, renderer, &mut scroll);
                        self.reselect(tree, layout, renderer, shell);
                    }
                    shell.request_redraw();
                }
            }
            _ => {}
        }

        self.content
            .as_widget_mut()
            .update(&mut tree.children[0], event, layout, cursor, renderer, clipboard, shell, viewport);

        // The wheel scrolled the rows under a band: what it meets changed.
        if active && matches!(event, Event::Mouse(mouse::Event::WheelScrolled { .. })) {
            self.reselect(tree, layout, renderer, shell);
            shell.request_redraw();
        }
    }

    fn mouse_interaction(
        &self,
        tree: &Tree,
        layout: Layout<'_>,
        cursor: mouse::Cursor,
        viewport: &Rectangle,
        renderer: &Renderer,
    ) -> mouse::Interaction {
        if tree.state.downcast_ref::<State>().active {
            return mouse::Interaction::Crosshair;
        }
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
        let state: &State = tree.state.downcast_ref();
        if !state.active {
            return;
        }
        let Some(clip) = layout.bounds().intersection(viewport) else { return };
        let Some(band) = band_rect(state.anchor, state.translation, state.cursor).intersection(&clip) else {
            return;
        };
        // The accent, because the band is a selection being made and the
        // accent is what selected means; faint inside so the rows stay
        // readable through it, solid at the edge so its extent is clear.
        let accent = crate::color::to_iced(crate::theme::active().accent);
        renderer.with_layer(clip, |renderer| {
            renderer.fill_quad(
                renderer::Quad {
                    bounds: band,
                    border: Border { color: accent, width: 1.0, radius: 2.0.into() },
                    ..renderer::Quad::default()
                },
                Color { a: 0.18, ..accent },
            );
        });
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

impl<'a, Message, Theme, Renderer> From<Marquee<'a, Message, Theme, Renderer>> for Element<'a, Message, Theme, Renderer>
where
    Message: 'a + Clone,
    Theme: 'a,
    Renderer: 'a + renderer::Renderer,
{
    fn from(marquee: Marquee<'a, Message, Theme, Renderer>) -> Self {
        Element::new(marquee)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_band_spans_its_corners_whichever_way_it_was_drawn() {
        let up_left = band_rect(Point::new(100.0, 100.0), Vector::ZERO, Point::new(40.0, 20.0));
        assert_eq!(up_left, Rectangle { x: 40.0, y: 20.0, width: 60.0, height: 80.0 });
    }

    /// Scrolled down 50 since the press, the corner it started at is 50
    /// higher on screen — it moved with the row it was beside.
    #[test]
    fn the_anchor_moves_with_the_scrolled_content() {
        let rect = band_rect(Point::new(10.0, 200.0), Vector::new(0.0, 50.0), Point::new(60.0, 300.0));
        assert_eq!(rect.y, 150.0);
        assert_eq!(rect.height, 150.0);
    }

    #[test]
    fn the_listing_scrolls_only_near_an_edge_and_fastest_at_it() {
        assert_eq!(edge_scroll(300.0, 0.0, 600.0), 0.0);
        assert_eq!(edge_scroll(0.0, 0.0, 600.0), -SPEED);
        assert_eq!(edge_scroll(-40.0, 0.0, 600.0), -SPEED, "past the edge is the fastest, not faster");
        assert_eq!(edge_scroll(600.0, 0.0, 600.0), SPEED);
        let partway = edge_scroll(600.0 - EDGE / 2.0, 0.0, 600.0);
        assert!(partway > 0.0 && partway < SPEED);
    }

    /// Only targets actually on screen are met: a row scrolled out of the
    /// viewport keeps its layout, and must not be selected by a band
    /// drawn over whatever is there now.
    #[test]
    fn a_target_scrolled_out_of_sight_is_not_met() {
        let a = Id::unique();
        let b = Id::unique();
        let targets = HashMap::from([(a.clone(), 0), (b.clone(), 1)]);
        let scroll = Id::unique();
        let mut gather = Gather::new(Rectangle::new(Point::new(0.0, 0.0), Size::new(100.0, 100.0)), &targets, Some(&scroll));
        struct Inert;
        impl operation::Scrollable for Inert {
            fn snap_to(&mut self, _: operation::scrollable::RelativeOffset<Option<f32>>) {}
            fn scroll_to(&mut self, _: AbsoluteOffset<Option<f32>>) {}
            fn scroll_by(&mut self, _: AbsoluteOffset, _: Rectangle, _: Rectangle) {}
        }
        // A viewport 0..100 scrolled down by 200.
        gather.scrollable(
            Some(&scroll),
            Rectangle::new(Point::ORIGIN, Size::new(100.0, 100.0)),
            Rectangle::new(Point::ORIGIN, Size::new(100.0, 1000.0)),
            Vector::new(0.0, 200.0),
            &mut Inert,
        );
        gather.traverse(&mut |op| {
            // Row a is laid out at 0..20: scrolled away above the top.
            op.container(Some(&a), Rectangle::new(Point::new(0.0, 0.0), Size::new(100.0, 20.0)));
            // Row b at 220..240: on screen at 20..40.
            op.container(Some(&b), Rectangle::new(Point::new(0.0, 220.0), Size::new(100.0, 20.0)));
        });
        assert_eq!(gather.found, [1]);
        assert_eq!(gather.translation, Some(Vector::new(0.0, 200.0)));
    }
}
