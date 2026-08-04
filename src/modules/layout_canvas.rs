//! Drag-arrange canvas for the Displays module's layout editor. Renders
//! each head as a rectangle positioned/sized proportionally to its real
//! geometry and lets the user drag it around; every drag frame publishes
//! a `(connector_hint, new_x, new_y)` message so the caller owns the
//! authoritative position (this widget holds no position state of its
//! own beyond "which head is currently grabbed").

use iced::mouse;
use iced::widget::canvas::{self, Canvas, Path, Text as CanvasText};
use iced::{Element, Length, Point, Rectangle, Renderer, Size, Theme, Vector};

/// Logical pixels -> canvas pixels. A 2560px-wide panel renders at 256px,
/// which comfortably fits a handful of heads in the fixed canvas size
/// below without needing to compute a dynamic fit-to-bounds scale.
pub const SCALE: f32 = 1.0 / 10.0;
pub const CANVAS_WIDTH: f32 = 700.0;
pub const CANVAS_HEIGHT: f32 = 420.0;

#[derive(Debug, Clone, PartialEq)]
pub struct CanvasHead {
    pub connector_hint: String,
    pub x: i32,
    pub y: i32,
    pub width: i32,
    pub height: i32,
    pub enabled: bool,
}

impl CanvasHead {
    fn rect(&self) -> Rectangle {
        Rectangle::new(
            Point::new(self.x as f32 * SCALE, self.y as f32 * SCALE),
            Size::new(self.width as f32 * SCALE, self.height as f32 * SCALE),
        )
    }
}

#[derive(Default)]
pub struct CanvasState {
    dragging: Option<Dragging>,
}

struct Dragging {
    connector_hint: String,
    grab_offset: Vector,
}

/// Owns its head list and callback (rather than borrowing) so a fresh
/// instance can be built from a temporary `Vec` inside `view()` on every
/// redraw without fighting `Element<'a, _>`'s borrow — the same reason
/// `text(String)` variants exist alongside `text(&str)` ones.
pub struct LayoutCanvas<Message> {
    heads: Vec<CanvasHead>,
    on_drag: Box<dyn Fn(String, i32, i32) -> Message>,
}

impl<Message: 'static> LayoutCanvas<Message> {
    pub fn new(
        heads: Vec<CanvasHead>,
        on_drag: impl Fn(String, i32, i32) -> Message + 'static,
    ) -> Self {
        LayoutCanvas {
            heads,
            on_drag: Box::new(on_drag),
        }
    }

    pub fn into_element<'a>(self) -> Element<'a, Message>
    where
        Message: 'a,
    {
        Canvas::new(self)
            .width(Length::Fixed(CANVAS_WIDTH))
            .height(Length::Fixed(CANVAS_HEIGHT))
            .into()
    }
}

impl<Message> canvas::Program<Message, Theme, Renderer> for LayoutCanvas<Message> {
    type State = CanvasState;

    fn update(
        &self,
        state: &mut Self::State,
        event: &canvas::Event,
        bounds: Rectangle,
        cursor: mouse::Cursor,
    ) -> Option<canvas::Action<Message>> {
        match event {
            canvas::Event::Mouse(mouse::Event::ButtonPressed(mouse::Button::Left)) => {
                let cursor_pos = cursor.position_in(bounds)?;
                // Last head drawn is on top, so search in reverse to grab
                // whichever rectangle the user actually sees at the click.
                let head = self.heads.iter().rev().find(|h| h.rect().contains(cursor_pos))?;
                state.dragging = Some(Dragging {
                    connector_hint: head.connector_hint.clone(),
                    grab_offset: cursor_pos - head.rect().position(),
                });
                Some(canvas::Action::capture())
            }
            canvas::Event::Mouse(mouse::Event::CursorMoved { .. }) => {
                let dragging = state.dragging.as_ref()?;
                let cursor_pos = cursor.position_in(bounds)?;
                let new_origin = cursor_pos - dragging.grab_offset;
                let new_x = (new_origin.x / SCALE).round() as i32;
                let new_y = (new_origin.y / SCALE).round() as i32;
                let message = (self.on_drag)(dragging.connector_hint.clone(), new_x, new_y);
                Some(canvas::Action::publish(message).and_capture())
            }
            canvas::Event::Mouse(mouse::Event::ButtonReleased(mouse::Button::Left)) => {
                state.dragging.take().map(|_| canvas::Action::capture())
            }
            _ => None,
        }
    }

    fn draw(
        &self,
        state: &Self::State,
        renderer: &Renderer,
        theme: &Theme,
        bounds: Rectangle,
        _cursor: mouse::Cursor,
    ) -> Vec<canvas::Geometry> {
        let mut frame = canvas::Frame::new(renderer, bounds.size());
        let palette = theme.extended_palette();

        let background = Path::rectangle(Point::ORIGIN, bounds.size());
        frame.fill(&background, palette.background.weak.color);

        for head in &self.heads {
            let rect = head.rect();
            let path = Path::rectangle(rect.position(), rect.size());

            let is_dragging = state
                .dragging
                .as_ref()
                .is_some_and(|d| d.connector_hint == head.connector_hint);

            let fill_color = if !head.enabled {
                palette.background.strong.color
            } else if is_dragging {
                palette.primary.strong.color
            } else {
                palette.primary.base.color
            };
            frame.fill(&path, fill_color);
            frame.stroke(
                &path,
                canvas::Stroke::default()
                    .with_width(2.0)
                    .with_color(palette.background.base.color),
            );

            frame.fill_text(CanvasText {
                content: head.connector_hint.clone(),
                position: Point::new(rect.x + 6.0, rect.y + 6.0),
                color: palette.primary.base.text,
                size: 12.0.into(),
                ..CanvasText::default()
            });
        }

        vec![frame.into_geometry()]
    }

    fn mouse_interaction(
        &self,
        state: &Self::State,
        bounds: Rectangle,
        cursor: mouse::Cursor,
    ) -> mouse::Interaction {
        if state.dragging.is_some() {
            return mouse::Interaction::Grabbing;
        }
        if let Some(cursor_pos) = cursor.position_in(bounds) {
            if self.heads.iter().any(|h| h.rect().contains(cursor_pos)) {
                return mouse::Interaction::Grab;
            }
        }
        mouse::Interaction::default()
    }
}

