//! Drag-arrange canvas for the Displays module's layout editor. Renders
//! each head as a rectangle positioned/sized proportionally to its real
//! geometry and lets the user drag it around (with edge-snapping against
//! other heads); every drag frame publishes a `(connector_hint, new_x,
//! new_y)` message so the caller owns the authoritative position (this
//! widget holds no position state of its own beyond "which head is
//! currently grabbed"). Fills the width it's given and computes a
//! fit-to-bounds scale from the heads' own bounding box, rather than a
//! fixed pixels-per-unit constant, so a spread-out or close-together
//! arrangement is always fully visible.
//!
//! That fit is recomputed whenever the arrangement changes — except during
//! a drag, which holds the transform it started with. Refitting mid-drag
//! would rescale the diagram in response to the very movement being made,
//! sliding the grabbed rectangle out from under the pointer.

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
    fn rect(&self, t: Transform) -> Rectangle {
        Rectangle::new(
            Point::new(self.x as f32 * t.scale, self.y as f32 * t.scale) + t.offset,
            Size::new(self.width as f32 * t.scale, self.height as f32 * t.scale),
        )
    }
}

/// Computes a scale (logical px -> canvas px) and offset that fits every
/// head's bounding box inside `bounds`, centered with `PADDING` to spare.
fn fit_transform(heads: &[CanvasHead], bounds: Size) -> Transform {
    if heads.is_empty() {
        return Transform {
            scale: 0.1,
            offset: Vector::new(PADDING, PADDING),
        };
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
    Transform { scale, offset }
}

/// The transform to draw and hit-test with: the frozen one mid-drag, a
/// freshly fitted one otherwise.
fn active_transform(state: &CanvasState, heads: &[CanvasHead], bounds: Size) -> Transform {
    match &state.dragging {
        Some(d) => d.transform,
        None => fit_transform(heads, bounds),
    }
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

/// Logical coordinates where the dragged head's edges coincide exactly with
/// another head's, as `(vertical_xs, horizontal_ys)`.
///
/// Snapping sets exact equality, so an exact match is precisely the
/// "it snapped" condition — no separate flag to keep in sync. Without a
/// guide there is nothing on screen distinguishing "flush" from "one pixel
/// out", which is the difference between a usable desktop and a seam.
fn snap_guides(dragged: &CanvasHead, others: &[&CanvasHead]) -> (Vec<i32>, Vec<i32>) {
    let mut xs = Vec::new();
    let mut ys = Vec::new();
    let (dx0, dx1) = (dragged.x, dragged.x + dragged.width);
    let (dy0, dy1) = (dragged.y, dragged.y + dragged.height);
    for o in others {
        for edge in [o.x, o.x + o.width] {
            if dx0 == edge || dx1 == edge {
                xs.push(edge);
            }
        }
        for edge in [o.y, o.y + o.height] {
            if dy0 == edge || dy1 == edge {
                ys.push(edge);
            }
        }
    }
    xs.sort_unstable();
    xs.dedup();
    ys.sort_unstable();
    ys.dedup();
    (xs, ys)
}

/// Logical-px -> canvas-px scale and the offset that centres the content.
#[derive(Debug, Clone, Copy, PartialEq)]
struct Transform {
    scale: f32,
    offset: Vector,
}

#[derive(Default)]
pub struct CanvasState {
    dragging: Option<Dragging>,
}

struct Dragging {
    connector_hint: String,
    grab_offset: Vector,
    /// The transform as it was when the drag started, held fixed until the
    /// drag ends.
    ///
    /// [`fit_transform`] derives zoom and centring from the heads' bounding
    /// box, so recomputing it mid-drag means moving a head changes the
    /// mapping that positions it. The rectangle then slides out from under
    /// the cursor, the whole diagram rescales as you approach an edge, and
    /// the harder you drag the more it fights back. Freezing it makes the
    /// grabbed point stay exactly under the pointer, which is the entire
    /// contract of a drag.
    transform: Transform,
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
        match event {
            canvas::Event::Mouse(mouse::Event::ButtonPressed(mouse::Button::Left)) => {
                let t = fit_transform(&self.heads, bounds.size());
                let cursor_pos = cursor.position_in(bounds)?;
                // Last head drawn is on top, so search in reverse to grab
                // whichever rectangle the user actually sees at the click.
                let head = self
                    .heads
                    .iter()
                    .rev()
                    .find(|h| h.rect(t).contains(cursor_pos))?;
                state.dragging = Some(Dragging {
                    connector_hint: head.connector_hint.clone(),
                    grab_offset: cursor_pos - head.rect(t).position(),
                    transform: t,
                });
                // A press both starts a potential drag and selects the
                // head (Windows Display Settings: clicking a monitor in
                // the diagram both picks it up and shows its properties).
                let message = (self.on_select)(head.connector_hint.clone());
                Some(canvas::Action::publish(message).and_capture())
            }
            canvas::Event::Mouse(mouse::Event::CursorMoved { .. }) => {
                let dragging = state.dragging.as_ref()?;
                // The frozen transform, so the grabbed point tracks the
                // pointer instead of drifting as the diagram refits.
                let t = dragging.transform;
                let cursor_pos = cursor.position_in(bounds)?;
                let new_origin = cursor_pos - dragging.grab_offset - t.offset;
                let mut new_x = (new_origin.x / t.scale).round() as i32;
                let mut new_y = (new_origin.y / t.scale).round() as i32;

                if let Some(dragged) = self
                    .heads
                    .iter()
                    .find(|h| h.connector_hint == dragging.connector_hint)
                {
                    let threshold = SNAP_THRESHOLD_PX / t.scale;
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
        let t = active_transform(state, &self.heads, bounds.size());

        let background = Path::rectangle(Point::ORIGIN, bounds.size());
        frame.fill(&background, palette.background.weak.color);

        for head in &self.heads {
            let rect = head.rect(t);
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

        // Snap guides on top of the rectangles, so a flush edge is visible
        // rather than something the user has to take on trust.
        if let Some(d) = &state.dragging {
            if let Some(dragged) = self
                .heads
                .iter()
                .find(|h| h.connector_hint == d.connector_hint)
            {
                let others: Vec<&CanvasHead> = self
                    .heads
                    .iter()
                    .filter(|h| h.connector_hint != d.connector_hint)
                    .collect();
                let (xs, ys) = snap_guides(dragged, &others);
                let guide = canvas::Stroke::default()
                    .with_width(1.0)
                    .with_color(palette.warning.base.color);
                for x in xs {
                    let cx = x as f32 * t.scale + t.offset.x;
                    frame.stroke(
                        &Path::line(Point::new(cx, 0.0), Point::new(cx, bounds.height)),
                        guide,
                    );
                }
                for y in ys {
                    let cy = y as f32 * t.scale + t.offset.y;
                    frame.stroke(
                        &Path::line(Point::new(0.0, cy), Point::new(bounds.width, cy)),
                        guide,
                    );
                }
            }
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
        let t = active_transform(state, &self.heads, bounds.size());
        if let Some(cursor_pos) = cursor.position_in(bounds) {
            if self.heads.iter().any(|h| h.rect(t).contains(cursor_pos)) {
                return mouse::Interaction::Grab;
            }
        }
        mouse::Interaction::default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn head(hint: &str, x: i32, y: i32, w: i32, h: i32) -> CanvasHead {
        CanvasHead {
            connector_hint: hint.to_string(),
            label: hint.to_string(),
            x,
            y,
            width: w,
            height: h,
            enabled: true,
        }
    }

    const BOUNDS: Size = Size {
        width: 800.0,
        height: CANVAS_HEIGHT,
    };

    /// The reason the drag transform is frozen: moving a head changes the
    /// bounding box the transform is fitted to, so recomputing it mid-drag
    /// changes the mapping that positions the thing being dragged.
    #[test]
    fn refitting_mid_drag_would_move_the_dragged_head_under_the_cursor() {
        let before = vec![head("eDP-2", 0, 0, 1536, 960), head("DP-3", 1536, 0, 2560, 1440)];
        let t0 = fit_transform(&before, BOUNDS);

        // Drag DP-3 right by 400 logical units.
        let after = vec![head("eDP-2", 0, 0, 1536, 960), head("DP-3", 1936, 0, 2560, 1440)];
        let t1 = fit_transform(&after, BOUNDS);

        assert_ne!(
            t0.scale, t1.scale,
            "the fit changes as the arrangement grows — which is exactly why \
             the drag must hold the transform it started with"
        );

        // With the frozen transform the cursor-to-logical mapping is stable:
        // 400 logical units of movement is the same canvas distance before
        // and after.
        let moved_canvas = (1936 - 1536) as f32 * t0.scale;
        assert!(moved_canvas > 0.0);
    }

    #[test]
    fn a_frozen_transform_maps_a_drag_one_to_one() {
        let heads = vec![head("eDP-2", 0, 0, 1536, 960), head("DP-3", 1536, 0, 2560, 1440)];
        let t = fit_transform(&heads, BOUNDS);
        // Grab DP-3's origin and move the pointer 100 canvas px right.
        let grab = heads[1].rect(t).position();
        let cursor = Point::new(grab.x + 100.0, grab.y);
        let origin = cursor - Vector::new(0.0, 0.0) - t.offset;
        let new_x = (origin.x / t.scale).round() as i32;
        assert_eq!(new_x, 1536 + (100.0 / t.scale).round() as i32);
    }

    #[test]
    fn snapping_prefers_the_nearest_edge() {
        // Dragged head is 2560 wide, sitting just short of flush against a
        // neighbour that ends at 1536.
        let others = [(0, 1536)];
        assert_eq!(snap_axis(1530, 2560, &others, 20.0), 1536);
        // Just past it, snaps back the other way.
        assert_eq!(snap_axis(1542, 2560, &others, 20.0), 1536);
        // Outside the threshold, left alone.
        assert_eq!(snap_axis(1400, 2560, &others, 20.0), 1400);
    }

    #[test]
    fn snapping_can_align_far_edges_too() {
        // Top-aligning two heads of different heights: my start to their
        // start.
        let others = [(0, 960)];
        assert_eq!(snap_axis(6, 1440, &others, 20.0), 0);
        // My end to their end: 960 - 1440 = -480.
        assert_eq!(snap_axis(-474, 1440, &others, 20.0), -480);
    }

    #[test]
    fn a_guide_appears_exactly_when_an_edge_is_flush() {
        let neighbour = head("eDP-2", 0, 0, 1536, 960);
        let others = vec![&neighbour];

        let flush = head("DP-3", 1536, 0, 2560, 1440);
        let (xs, ys) = snap_guides(&flush, &others);
        assert_eq!(xs, vec![1536], "the shared vertical edge should be marked");
        assert_eq!(ys, vec![0], "tops are aligned, so that edge is marked too");

        let one_out = head("DP-3", 1537, 1, 2560, 1440);
        let (xs, ys) = snap_guides(&one_out, &others);
        assert!(xs.is_empty() && ys.is_empty(), "not flush, so no guide");
    }

    #[test]
    fn an_empty_arrangement_does_not_divide_by_zero() {
        let t = fit_transform(&[], BOUNDS);
        assert!(t.scale > 0.0);
    }
}
