//! The navigation glyphs, drawn rather than typed.
//!
//! These were `‹`, `›` and `↑` — characters set in the UI font — and
//! they looked like what they are: punctuation. `‹` and `›` are
//! guillemets, quotation marks borrowed from French typesetting; they
//! are built to sit inside a line of prose at x-height, so in a 26px
//! button they read as small, thin and slightly high rather than as
//! chevrons. `↑` is a real arrow but its weight, head angle and length
//! are whatever the desktop's font happens to think, which is a
//! different answer on every machine.
//!
//! This is the same lesson the file badges already cost: a glyph's shape
//! and colour belong to a font, and a font is not ours. There the
//! symptom was colour — a colour emoji ignored the theme entirely.
//! Here it is proportion. Both end the same way: draw the mark.
//!
//! Stroked paths rather than the filled rectangles the badges use,
//! because a chevron is a line with a corner in it and cannot be built
//! out of axis-aligned boxes. That is the whole reason this crate asks
//! for iced's `canvas` feature.
//!
//! Every dimension here is a fraction of the button it is drawn in, so
//! these scale with `FontScale` like everything else rather than
//! becoming three fixed pixel sizes that drift apart at 125%.

use iced::mouse;
use iced::widget::canvas;
use iced::{Element, Length, Point, Rectangle, Renderer, Theme};

/// Which way a chevron points, or that it is the "up a level" arrow.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Nav {
    Back,
    Forward,
    Up,
}

/// Stroke weight as a fraction of the button's side.
///
/// 1.5px in a 26px button at 100% scale, matching the design's other
/// drawn outlines — the file badge's edge and the search field's ring —
/// so every hairline in the window is the same weight.
const STROKE_FRACTION: f32 = 1.75 / 26.0;

/// Half the mark's height, as a fraction of the button's side.
///
/// A chevron is `EXTENT` tall above the midline and `EXTENT` below, so
/// the mark stands `2 × EXTENT` overall — at 0.22 that is a 11.4px
/// chevron in a 26px button, which is the proportion the design draws.
///
/// This depends on the button having **no padding**: iced buttons carry
/// a default padding, and with it the canvas was handed about 16px of
/// the 26 and drew a mark scaled to *that*, which is why the first
/// attempt came out visibly tiny. See `nav_button`.
const EXTENT_FRACTION: f32 = 0.22;

/// A drawn navigation mark, sized to `side` and coloured `color`.
pub fn nav<'a, Message: 'a>(kind: Nav, side: f32, color: iced::Color) -> Element<'a, Message> {
    canvas(NavGlyph { kind, color })
        .width(Length::Fixed(side))
        .height(Length::Fixed(side))
        .into()
}

struct NavGlyph {
    kind: Nav,
    color: iced::Color,
}

impl<Message> canvas::Program<Message, Theme, Renderer> for NavGlyph {
    type State = ();

    fn draw(
        &self,
        _state: &Self::State,
        renderer: &Renderer,
        _theme: &Theme,
        bounds: Rectangle,
        _cursor: mouse::Cursor,
    ) -> Vec<canvas::Geometry> {
        let mut frame = canvas::Frame::new(renderer, bounds.size());
        let side = bounds.width.min(bounds.height);
        let mid = Point::new(bounds.width / 2.0, bounds.height / 2.0);
        let extent = side * EXTENT_FRACTION;

        let path = canvas::Path::new(|b| match self.kind {
            // A chevron: two strokes meeting at a point. Drawn as one
            // open path rather than two lines so the corner is a single
            // join — two separate strokes leave a notch at the vertex
            // that is invisible at 26px and obvious at 200%.
            Nav::Back => {
                b.move_to(Point::new(mid.x + extent * 0.5, mid.y - extent));
                b.line_to(Point::new(mid.x - extent * 0.5, mid.y));
                b.line_to(Point::new(mid.x + extent * 0.5, mid.y + extent));
            }
            Nav::Forward => {
                b.move_to(Point::new(mid.x - extent * 0.5, mid.y - extent));
                b.line_to(Point::new(mid.x + extent * 0.5, mid.y));
                b.line_to(Point::new(mid.x - extent * 0.5, mid.y + extent));
            }
            // An arrow: a shaft with a head, drawn as two subpaths. The
            // head is deliberately wider than a chevron's — at this size
            // a narrow head reads as a vertical line with a kink.
            Nav::Up => {
                let head = extent * 0.72;
                b.move_to(Point::new(mid.x, mid.y + extent));
                b.line_to(Point::new(mid.x, mid.y - extent));
                b.move_to(Point::new(mid.x - head, mid.y - extent + head));
                b.line_to(Point::new(mid.x, mid.y - extent));
                b.line_to(Point::new(mid.x + head, mid.y - extent + head));
            }
        });

        frame.stroke(
            &path,
            canvas::Stroke {
                style: canvas::Style::Solid(self.color),
                width: side * STROKE_FRACTION,
                // Round caps and joins: a chevron with butt caps has
                // visibly chopped ends at this weight, and the corner
                // reads as a chip out of the stroke rather than a bend.
                line_cap: canvas::LineCap::Round,
                line_join: canvas::LineJoin::Round,
                ..canvas::Stroke::default()
            },
        );

        vec![frame.into_geometry()]
    }
}

/// The view-mode marks: list, grid, columns.
///
/// Drawn for the same reason as [`Nav`] — `☰`, `⊞` and `‖` are font
/// characters, and in a 20px segment they sat off-centre because a
/// glyph's own bearings decide where it lands, not the box around it.
/// Three rectangles cannot be off-centre.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum View {
    List,
    Grid,
    Columns,
}

/// A drawn view-mode mark, sized to fit `side` and filled `color`.
pub fn view<'a, Message: 'a>(kind: View, side: f32, color: iced::Color) -> Element<'a, Message> {
    canvas(ViewGlyph { kind, color }).width(Length::Fixed(side)).height(Length::Fixed(side)).into()
}

struct ViewGlyph {
    kind: View,
    color: iced::Color,
}

impl<Message> canvas::Program<Message, Theme, Renderer> for ViewGlyph {
    type State = ();

    fn draw(
        &self,
        _state: &Self::State,
        renderer: &Renderer,
        _theme: &Theme,
        bounds: Rectangle,
        _cursor: mouse::Cursor,
    ) -> Vec<canvas::Geometry> {
        let mut frame = canvas::Frame::new(renderer, bounds.size());
        let side = bounds.width.min(bounds.height);
        // The mark occupies a square this wide, centred in `bounds` —
        // so a non-square canvas still centres rather than stretching.
        let extent = side * VIEW_EXTENT_FRACTION;
        let left = bounds.width / 2.0 - extent / 2.0;
        let top = bounds.height / 2.0 - extent / 2.0;
        let bar = (extent * 0.16).max(1.0);

        let fill = canvas::Fill::from(self.color);
        match self.kind {
            // Three full-width bars: rows.
            View::List => {
                for i in 0..3 {
                    let gap = (extent - bar * 3.0) / 2.0;
                    let y = top + i as f32 * (bar + gap);
                    frame.fill_rectangle(Point::new(left, y), iced::Size::new(extent, bar), fill);
                }
            }
            // Four squares: cells.
            View::Grid => {
                let cell = extent * 0.42;
                let gap = extent - cell * 2.0;
                for (cx, cy) in [(0.0, 0.0), (1.0, 0.0), (0.0, 1.0), (1.0, 1.0)] {
                    frame.fill_rectangle(
                        Point::new(left + cx * (cell + gap), top + cy * (cell + gap)),
                        iced::Size::new(cell, cell),
                        fill,
                    );
                }
            }
            // Two full-height bars: panes side by side.
            View::Columns => {
                let w = extent * 0.30;
                let gap = extent - w * 2.0;
                for i in 0..2 {
                    frame.fill_rectangle(
                        Point::new(left + i as f32 * (w + gap), top),
                        iced::Size::new(w, extent),
                        fill,
                    );
                }
            }
        }

        vec![frame.into_geometry()]
    }
}

/// How much of the segment the view mark occupies.
const VIEW_EXTENT_FRACTION: f32 = 0.55;

// No tests here, deliberately.
//
// What this module decides is a *shape*, and the only instrument that
// can check a shape is a screenshot. The two things a test could reach —
// that the three `Nav` variants differ, and that the fractions above are
// between 0 and 1 — are both true by construction: clippy rejects the
// second as a constant assertion, which is the correct verdict on a test
// that cannot fail.
//
// What is verified instead, and how: rendered at 26px and looked at
// beside the design, which is how the typed guillemets this replaced
// were found wanting in the first place. The property that keeps it
// honest at other sizes is that every dimension is a fraction of the
// button rather than a pixel count, and that is visible in the source
// above rather than assertable below.
