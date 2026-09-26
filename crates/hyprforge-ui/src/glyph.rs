//! The marks every Hyprforge app draws — navigation, view modes, the
//! sidebar toggle and the settings pages — drawn rather than typed.
//!
//! These began in Files and moved here when Settings adopted the same
//! design, because the reasoning below is about fonts, not about files.
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

/// The sidebar toggle's mark: a pane with its left rail picked out.
///
/// Drawn rather than typed for the same reason as every mark here — and
/// the shape says what the control does, which `☰` does not: a hamburger
/// is a menu everywhere else in the world, and this opens and closes a
/// panel. The filled rail *is* the sidebar; the outline is the window
/// around it.
pub fn sidebar<'a, Message: 'a>(side: f32, color: iced::Color) -> Element<'a, Message> {
    canvas(SidebarGlyph { color }).width(Length::Fixed(side)).height(Length::Fixed(side)).into()
}

struct SidebarGlyph {
    color: iced::Color,
}

impl<Message> canvas::Program<Message, Theme, Renderer> for SidebarGlyph {
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
        let extent = side * VIEW_EXTENT_FRACTION;
        let left = bounds.width / 2.0 - extent / 2.0;
        let top = bounds.height / 2.0 - extent / 2.0;
        let stroke = (extent * 0.12).max(1.0);
        let fill = canvas::Fill::from(self.color);

        // The rail, filled: this is the panel the button shows and hides.
        let rail = extent * 0.34;
        frame.fill_rectangle(Point::new(left, top), iced::Size::new(rail, extent), fill);

        // The rest of the pane, outlined — four thin rectangles rather
        // than a stroked path, so the corners meet exactly at any size
        // instead of depending on the join style.
        let right = left + extent;
        let bottom = top + extent;
        let body_left = left + rail;
        frame.fill_rectangle(
            Point::new(body_left, top),
            iced::Size::new(right - body_left, stroke),
            fill,
        );
        frame.fill_rectangle(
            Point::new(body_left, bottom - stroke),
            iced::Size::new(right - body_left, stroke),
            fill,
        );
        frame.fill_rectangle(
            Point::new(right - stroke, top),
            iced::Size::new(stroke, extent),
            fill,
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

/// The marks beside a settings page in a sidebar.
///
/// Named for what the page is *about*, not for any one app's screen
/// list, so a page that moves between groups — or a second app with a
/// sidebar of its own — keeps its mark.
///
/// All outline, one weight, one colour. The mockup these came from gave
/// every page its own hue, and that is the one part of it this suite
/// does not follow: colour here is reserved for selection and state, so
/// a mark takes the text colour and turns accent only when its page is
/// the current one (see `selectable_row_style`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Page {
    Display,
    Power,
    Keyboard,
    Network,
    Bluetooth,
    Windows,
    WindowRules,
    Keybinds,
    Animation,
    Idle,
    Session,
    Advanced,
    Apps,
    Appearance,
    Wallpaper,
    NightLight,
    ScreenSharing,
    Tray,
}

/// A drawn page mark, sized to `side` and stroked `color`.
pub fn page<'a, Message: 'a>(kind: Page, side: f32, color: iced::Color) -> Element<'a, Message> {
    canvas(PageGlyph { kind, color }).width(Length::Fixed(side)).height(Length::Fixed(side)).into()
}

struct PageGlyph {
    kind: Page,
    color: iced::Color,
}

/// How much of its box a page mark occupies — a little more than the
/// view marks, because an outline carries less ink than a filled bar
/// and reads smaller at the same extent.
const PAGE_EXTENT_FRACTION: f32 = 0.78;

impl<Message> canvas::Program<Message, Theme, Renderer> for PageGlyph {
    type State = ();

    fn draw(
        &self,
        _state: &Self::State,
        renderer: &Renderer,
        _theme: &Theme,
        bounds: Rectangle,
        _cursor: mouse::Cursor,
    ) -> Vec<canvas::Geometry> {
        use iced::{Radians, Size};
        use std::f32::consts::PI;

        let mut frame = canvas::Frame::new(renderer, bounds.size());
        let side = bounds.width.min(bounds.height);
        let extent = side * PAGE_EXTENT_FRACTION;
        let left = bounds.width / 2.0 - extent / 2.0;
        let top = bounds.height / 2.0 - extent / 2.0;
        // Every coordinate below is in a unit square, so a mark is one
        // drawing at every size rather than a pixel layout per size.
        let at = |x: f32, y: f32| Point::new(left + x * extent, top + y * extent);
        let len = |v: f32| v * extent;
        let rect = |b: &mut canvas::path::Builder, x: f32, y: f32, w: f32, h: f32, r: f32| {
            b.rounded_rectangle(at(x, y), Size::new(len(w), len(h)), len(r).into());
        };
        let arc = |b: &mut canvas::path::Builder, x: f32, y: f32, r: f32, from: f32, to: f32| {
            let start = at(x, y) + iced::Vector::new(len(r) * from.cos(), len(r) * from.sin());
            b.move_to(start);
            b.arc(canvas::path::Arc {
                center: at(x, y),
                radius: len(r),
                start_angle: Radians(from),
                end_angle: Radians(to),
            });
        };
        let line = |b: &mut canvas::path::Builder, x0: f32, y0: f32, x1: f32, y1: f32| {
            b.move_to(at(x0, y0));
            b.line_to(at(x1, y1));
        };

        let path = canvas::Path::new(|b| match self.kind {
            // A screen on a short stand.
            Page::Display => {
                rect(b, 0.05, 0.14, 0.9, 0.58, 0.08);
                line(b, 0.5, 0.72, 0.5, 0.86);
                line(b, 0.3, 0.86, 0.7, 0.86);
            }
            // A battery on its side, with its terminal.
            Page::Power => {
                rect(b, 0.04, 0.26, 0.8, 0.48, 0.08);
                line(b, 0.92, 0.42, 0.92, 0.58);
            }
            // A keyboard: the body and a row of keys along it.
            Page::Keyboard => {
                rect(b, 0.02, 0.22, 0.96, 0.56, 0.08);
                for i in 0..4 {
                    let x = 0.2 + i as f32 * 0.2;
                    line(b, x, 0.42, x + 0.02, 0.42);
                }
                line(b, 0.28, 0.6, 0.72, 0.6);
            }
            // Signal: three arcs over a point.
            Page::Network => {
                for r in [0.44, 0.3, 0.16] {
                    arc(b, 0.5, 0.82, r, PI * 1.25, PI * 1.75);
                }
                line(b, 0.5, 0.8, 0.5, 0.82);
            }
            // The Bluetooth rune: a spine with two bows.
            Page::Bluetooth => {
                b.move_to(at(0.26, 0.3));
                b.line_to(at(0.72, 0.7));
                b.line_to(at(0.5, 0.9));
                b.line_to(at(0.5, 0.1));
                b.line_to(at(0.72, 0.3));
                b.line_to(at(0.26, 0.7));
            }
            // Two tiled windows.
            Page::Windows => {
                rect(b, 0.04, 0.12, 0.5, 0.76, 0.07);
                rect(b, 0.62, 0.12, 0.34, 0.34, 0.07);
                rect(b, 0.62, 0.54, 0.34, 0.34, 0.07);
            }
            // A window and the rule lines that describe it.
            Page::WindowRules => {
                rect(b, 0.04, 0.12, 0.92, 0.76, 0.07);
                line(b, 0.04, 0.32, 0.96, 0.32);
                line(b, 0.2, 0.52, 0.8, 0.52);
                line(b, 0.2, 0.68, 0.6, 0.68);
            }
            // A single keycap, seen slightly from above.
            Page::Keybinds => {
                rect(b, 0.12, 0.12, 0.76, 0.76, 0.14);
                rect(b, 0.24, 0.2, 0.52, 0.46, 0.08);
            }
            // A ball with the lines it left behind.
            Page::Animation => {
                b.circle(at(0.64, 0.5), len(0.26));
                line(b, 0.04, 0.34, 0.26, 0.34);
                line(b, 0.0, 0.5, 0.28, 0.5);
                line(b, 0.04, 0.66, 0.26, 0.66);
            }
            // A crescent moon: two arcs sharing their tips.
            Page::Idle => {
                arc(b, 0.5, 0.5, 0.4, PI * 0.35, PI * 1.65);
                let tip_top = at(0.5, 0.5)
                    + iced::Vector::new(len(0.4) * (PI * 1.65).cos(), len(0.4) * (PI * 1.65).sin());
                let tip_bottom = at(0.5, 0.5)
                    + iced::Vector::new(len(0.4) * (PI * 0.35).cos(), len(0.4) * (PI * 0.35).sin());
                b.move_to(tip_top);
                b.quadratic_curve_to(at(0.34, 0.5), tip_bottom);
            }
            // Play: what starts when the session does.
            Page::Session => {
                b.move_to(at(0.28, 0.14));
                b.line_to(at(0.82, 0.5));
                b.line_to(at(0.28, 0.86));
                b.close();
            }
            // Two slider tracks with their knobs — the knobs of last resort.
            Page::Advanced => {
                line(b, 0.04, 0.32, 0.96, 0.32);
                line(b, 0.04, 0.68, 0.96, 0.68);
                b.circle(at(0.34, 0.32), len(0.11));
                b.circle(at(0.66, 0.68), len(0.11));
            }
            // A grid of applications.
            Page::Apps => {
                for (x, y) in [(0.08, 0.08), (0.56, 0.08), (0.08, 0.56), (0.56, 0.56)] {
                    rect(b, x, y, 0.36, 0.36, 0.08);
                }
            }
            // Contrast: a circle split down the middle.
            Page::Appearance => {
                b.circle(at(0.5, 0.5), len(0.42));
                line(b, 0.5, 0.08, 0.5, 0.92);
            }
            // A picture: its frame and a mountain.
            Page::Wallpaper => {
                rect(b, 0.04, 0.14, 0.92, 0.72, 0.07);
                b.move_to(at(0.14, 0.76));
                b.line_to(at(0.4, 0.44));
                b.line_to(at(0.58, 0.64));
                b.line_to(at(0.7, 0.52));
                b.line_to(at(0.86, 0.76));
            }
            // A low sun: warm light in the evening.
            Page::NightLight => {
                arc(b, 0.5, 0.7, 0.26, PI, PI * 2.0);
                line(b, 0.04, 0.7, 0.96, 0.7);
                line(b, 0.5, 0.14, 0.5, 0.28);
                line(b, 0.16, 0.34, 0.25, 0.43);
                line(b, 0.84, 0.34, 0.75, 0.43);
            }
            // A screen with an arrow leaving it.
            Page::ScreenSharing => {
                rect(b, 0.04, 0.14, 0.92, 0.64, 0.07);
                line(b, 0.5, 0.64, 0.5, 0.32);
                line(b, 0.36, 0.44, 0.5, 0.3);
                line(b, 0.64, 0.44, 0.5, 0.3);
            }
            // A bar along the top with its icons.
            Page::Tray => {
                rect(b, 0.04, 0.16, 0.92, 0.68, 0.07);
                line(b, 0.04, 0.38, 0.96, 0.38);
                line(b, 0.62, 0.27, 0.64, 0.27);
                line(b, 0.76, 0.27, 0.78, 0.27);
            }
        });

        frame.stroke(
            &path,
            canvas::Stroke {
                style: canvas::Style::Solid(self.color),
                // Lighter than the nav chevrons: those are one mark in a
                // button, these are a column of marks beside labels, and
                // at the chevron weight the column reads louder than the
                // text it annotates.
                width: (side * STROKE_FRACTION * 0.85).max(1.0),
                line_cap: canvas::LineCap::Round,
                line_join: canvas::LineJoin::Round,
                ..canvas::Stroke::default()
            },
        );

        vec![frame.into_geometry()]
    }
}

/// Signal strength as four rising bars, lit up to `strength` percent —
/// the mark beside a network in a list.
///
/// Lit bars take `on`, the rest `off`, so a weak network still shows the
/// shape of the scale it is low on rather than one lonely bar.
pub fn signal<'a, Message: 'a>(
    strength: u8,
    side: f32,
    on: iced::Color,
    off: iced::Color,
) -> Element<'a, Message> {
    canvas(SignalGlyph { lit: signal_bars(strength), on, off })
        .width(Length::Fixed(side))
        .height(Length::Fixed(side))
        .into()
}

/// How many of the four bars a strength lights: at least one for any
/// signal at all, four only near the top.
pub fn signal_bars(strength: u8) -> u8 {
    match strength {
        0 => 0,
        1..=29 => 1,
        30..=54 => 2,
        55..=79 => 3,
        _ => 4,
    }
}

struct SignalGlyph {
    lit: u8,
    on: iced::Color,
    off: iced::Color,
}

impl<Message> canvas::Program<Message, Theme, Renderer> for SignalGlyph {
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
        let extent = side * PAGE_EXTENT_FRACTION;
        let left = bounds.width / 2.0 - extent / 2.0;
        let bottom = bounds.height / 2.0 + extent / 2.0;
        let bar = extent / 5.5;
        let gap = (extent - bar * 4.0) / 3.0;
        for i in 0..4u8 {
            let height = extent * (0.25 + 0.25 * i as f32);
            let colour = if i < self.lit { self.on } else { self.off };
            frame.fill_rectangle(
                Point::new(left + i as f32 * (bar + gap), bottom - height),
                iced::Size::new(bar, height),
                canvas::Fill::from(colour),
            );
        }
        vec![frame.into_geometry()]
    }
}

/// A battery on its side, filled to `fraction` in `fill` — the mark that
/// leads a power page. The outline takes `outline`.
///
/// `side` is the height; the battery is drawn twice as wide, the shape
/// people read as a battery before they read the number beside it.
pub fn battery<'a, Message: 'a>(
    fraction: f32,
    side: f32,
    outline: iced::Color,
    fill: iced::Color,
) -> Element<'a, Message> {
    canvas(BatteryGlyph { fraction: fraction.clamp(0.0, 1.0), outline, fill })
        .width(Length::Fixed(side * 2.0))
        .height(Length::Fixed(side))
        .into()
}

struct BatteryGlyph {
    fraction: f32,
    outline: iced::Color,
    fill: iced::Color,
}

impl<Message> canvas::Program<Message, Theme, Renderer> for BatteryGlyph {
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
        let h = bounds.height;
        let stroke = (h * 0.08).max(1.5);
        let nub = h * 0.12;
        let body_w = bounds.width - nub - stroke;
        let body = canvas::Path::rounded_rectangle(
            Point::new(stroke / 2.0, stroke / 2.0),
            iced::Size::new(body_w, h - stroke),
            (h * 0.18).into(),
        );
        frame.stroke(
            &body,
            canvas::Stroke::default().with_width(stroke).with_color(self.outline),
        );
        // The terminal, centred on the right end.
        frame.fill_rectangle(
            Point::new(body_w + stroke / 2.0, h * 0.32),
            iced::Size::new(nub, h * 0.36),
            canvas::Fill::from(self.outline),
        );
        // The charge, inset from the outline so it reads as contents.
        let inset = stroke * 2.0;
        let full = body_w - inset * 2.0 + stroke;
        if self.fraction > 0.0 {
            frame.fill(
                &canvas::Path::rounded_rectangle(
                    Point::new(inset, inset),
                    iced::Size::new(full * self.fraction, h - inset * 2.0),
                    (h * 0.08).into(),
                ),
                self.fill,
            );
        }
        vec![frame.into_geometry()]
    }
}

// Only `signal_bars` is tested below, because it is a decision rather
// than a shape. For the rest there are no tests, deliberately.
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

#[cfg(test)]
mod tests {
    use super::*;

    /// Any signal at all lights a bar — a network that is there but weak
    /// must not look like one that is gone — and four is kept for strong.
    #[test]
    fn any_signal_lights_a_bar_and_only_a_strong_one_lights_four() {
        assert_eq!(signal_bars(0), 0);
        assert_eq!(signal_bars(1), 1);
        assert_eq!(signal_bars(54), 2);
        assert_eq!(signal_bars(79), 3);
        assert_eq!(signal_bars(97), 4);
        assert_eq!(signal_bars(100), 4);
    }
}
