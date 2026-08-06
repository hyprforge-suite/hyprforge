//! Drag-arrange canvas for the Displays module's layout editor. Renders
//! each head as a rectangle positioned/sized proportionally to its real
//! geometry and lets the user drag it around (with edge-snapping against
//! other heads); every drag frame publishes a `(connector_hint, new_x,
//! new_y)` message so the caller owns the authoritative position (this
//! widget holds no position state of its own beyond "which head is
//! currently grabbed"). Fills the width it's given and computes a
//! fit-to-bounds scale from the heads' own bounding box every frame,
//! rather than a fixed pixels-per-unit constant, so a spread-out or
//! close-together arrangement is always fully visible.

use iced::mouse;
use iced::widget::canvas::{self, Canvas, Path, Text as CanvasText};
use iced::{Element, Length, Point, Rectangle, Renderer, Size, Theme, Vector};

pub const CANVAS_HEIGHT: f32 = 380.0;
/// Breathing room, in canvas pixels, around the fitted content.
const PADDING: f32 = 24.0;
/// Never zoom in past this many canvas px per logical px, even for a
/// single small/tightly-packed arrangement — keeps a lone 1920x1080 head
/// from filling the entire card at a jarring scale.
const MAX_SCALE: f32 = 0.35;
/// How close (in canvas/screen pixels) a dragged edge must get to another
/// head's edge before it snaps to it.
const SNAP_THRESHOLD_PX: f32 = 10.0;

#[derive(Debug, Clone, PartialEq)]
pub struct CanvasHead {
    pub connector_hint: String,
    /// What to draw on the rectangle — the monitor's friendly name, not
    /// its connector. `connector_hint` stays the identity used in messages.
    pub label: String,
    pub x: i32,
    pub y: i32,
    pub width: i32,
    pub height: i32,
    pub enabled: bool,
}

impl CanvasHead {
    fn rect(&self, scale: f32, offset: Vector) -> Rectangle {
        Rectangle::new(
            Point::new(self.x as f32 * scale, self.y as f32 * scale) + offset,
            Size::new(self.width as f32 * scale, self.height as f32 * scale),
        )
    }
}

/// Computes a scale (logical px -> canvas px) and offset that fits every
/// head's bounding box inside `bounds`, centered with `PADDING` to spare.
fn fit_transform(heads: &[CanvasHead], bounds: Size) -> (f32, Vector) {
    if heads.is_empty() {
        return (0.1, Vector::new(PADDING, PADDING));
    }

    let min_x = heads.iter().map(|h| h.x).min().unwrap() as f32;
    let min_y = heads.iter().map(|h| h.y).min().unwrap() as f32;
    let max_x = heads.iter().map(|h| h.x + h.width).max().unwrap() as f32;
    let max_y = heads.iter().map(|h| h.y + h.height).max().unwrap() as f32;
    let bbox_w = (max_x - min_x).max(1.0);
    let bbox_h = (max_y - min_y).max(1.0);

    let avail_w = (bounds.width - PADDING * 2.0).max(1.0);
    let avail_h = (bounds.height - PADDING * 2.0).max(1.0);

    let scale = (avail_w / bbox_w).min(avail_h / bbox_h).min(MAX_SCALE);
    let content_w = bbox_w * scale;
    let content_h = bbox_h * scale;
    let offset = Vector::new(
        PADDING + (avail_w - content_w) / 2.0 - min_x * scale,
        PADDING + (avail_h - content_h) / 2.0 - min_y * scale,
    );
    (scale, offset)
}

/// Snaps `pos`/`pos + size` against every candidate edge derived from
/// `others`' own start/end along one axis, within `threshold` canvas
/// pixels (already converted to logical units by the caller).
fn snap_axis(pos: i32, size: i32, others: &[(i32, i32)], threshold: f32) -> i32 {
    let mut best: Option<(i32, f32)> = None;
    let mut consider = |candidate: i32, distance: i32| {
        let d = distance.unsigned_abs() as f32;
        if d <= threshold && best.is_none_or(|(_, best_d)| d < best_d) {
            best = Some((candidate, d));
        }
    };
    for &(other_pos, other_size) in others {
        let other_end = other_pos + other_size;
        let end = pos + size;
        consider(other_pos, pos - other_pos); // my start <-> their start
        consider(other_end, pos - other_end); // my start <-> their end
        consider(other_pos - size, end - other_pos); // my end <-> their start
        consider(other_end - size, end - other_end); // my end <-> their end
    }
    best.map(|(candidate, _)| candidate).unwrap_or(pos)
}

#[derive(Default)]
pub struct CanvasState {
    dragging: Option<Dragging>,
}

struct Dragging {
    connector_hint: String,
    grab_offset: Vector,
}

/// Owns its head list and callbacks (rather than borrowing) so a fresh
/// instance can be built from a temporary `Vec` inside `view()` on every
/// redraw without fighting `Element<'a, _>`'s borrow — the same reason
/// `text(String)` variants exist alongside `text(&str)` ones.
pub struct LayoutCanvas<Message> {
    heads: Vec<CanvasHead>,
    selected: Option<String>,
    on_select: Box<dyn Fn(String) -> Message>,
    on_drag: Box<dyn Fn(String, i32, i32) -> Message>,
}

impl<Message: 'static> LayoutCanvas<Message> {
    pub fn new(
        heads: Vec<CanvasHead>,
        selected: Option<String>,
        on_select: impl Fn(String) -> Message + 'static,
        on_drag: impl Fn(String, i32, i32) -> Message + 'static,
    ) -> Self {
        LayoutCanvas {
            heads,
            selected,
            on_select: Box::new(on_select),
            on_drag: Box::new(on_drag),
        }
    }

    pub fn into_element<'a>(self) -> Element<'a, Message>
    where
        Message: 'a,
    {
        Canvas::new(self)
            .width(Length::Fill)
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
        let (scale, offset) = fit_transform(&self.heads, bounds.size());

        match event {
            canvas::Event::Mouse(mouse::Event::ButtonPressed(mouse::Button::Left)) => {
                let cursor_pos = cursor.position_in(bounds)?;
                // Last head drawn is on top, so search in reverse to grab
                // whichever rectangle the user actually sees at the click.
                let head = self
                    .heads
                    .iter()
                    .rev()
                    .find(|h| h.rect(scale, offset).contains(cursor_pos))?;
                state.dragging = Some(Dragging {
                    connector_hint: head.connector_hint.clone(),
                    grab_offset: cursor_pos - head.rect(scale, offset).position(),
                });
                // A press both starts a potential drag and selects the
                // head (Windows Display Settings: clicking a monitor in
                // the diagram both picks it up and shows its properties).
                let message = (self.on_select)(head.connector_hint.clone());
                Some(canvas::Action::publish(message).and_capture())
            }
            canvas::Event::Mouse(mouse::Event::CursorMoved { .. }) => {
                let dragging = state.dragging.as_ref()?;
                let cursor_pos = cursor.position_in(bounds)?;
                let new_origin = cursor_pos - dragging.grab_offset - offset;
                let mut new_x = (new_origin.x / scale).round() as i32;
                let mut new_y = (new_origin.y / scale).round() as i32;

                if let Some(dragged) = self
                    .heads
                    .iter()
                    .find(|h| h.connector_hint == dragging.connector_hint)
                {
                    let threshold = SNAP_THRESHOLD_PX / scale;
                    let others_x: Vec<(i32, i32)> = self
                        .heads
                        .iter()
                        .filter(|h| h.connector_hint != dragging.connector_hint)
                        .map(|h| (h.x, h.width))
                        .collect();
                    let others_y: Vec<(i32, i32)> = self
                        .heads
                        .iter()
                        .filter(|h| h.connector_hint != dragging.connector_hint)
                        .map(|h| (h.y, h.height))
                        .collect();
                    new_x = snap_axis(new_x, dragged.width, &others_x, threshold);
                    new_y = snap_axis(new_y, dragged.height, &others_y, threshold);
                }

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
        let (scale, offset) = fit_transform(&self.heads, bounds.size());

        let background = Path::rectangle(Point::ORIGIN, bounds.size());
        frame.fill(&background, palette.background.weak.color);

        for head in &self.heads {
            let rect = head.rect(scale, offset);
            let path = Path::rectangle(rect.position(), rect.size());

            let is_dragging = state
                .dragging
                .as_ref()
                .is_some_and(|d| d.connector_hint == head.connector_hint);
            let is_selected = self.selected.as_deref() == Some(head.connector_hint.as_str());

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
                    .with_width(if is_selected { 3.0 } else { 2.0 })
                    .with_color(if is_selected {
                        palette.warning.base.color
                    } else {
                        palette.background.base.color
                    }),
            );

            frame.fill_text(CanvasText {
                content: head.label.clone(),
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
        let (scale, offset) = fit_transform(&self.heads, bounds.size());
        if let Some(cursor_pos) = cursor.position_in(bounds) {
            if self.heads.iter().any(|h| h.rect(scale, offset).contains(cursor_pos)) {
                return mouse::Interaction::Grab;
            }
        }
        mouse::Interaction::default()
    }
}
