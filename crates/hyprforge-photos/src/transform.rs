//! Where the picture sits in the window, and how big.
//!
//! Pure, and the reason the view module can stay thin: "does zooming
//! about the pointer keep that point under it" is arithmetic, and
//! arithmetic can be asserted without a compositor.
//!
//! # Units, named in the types
//!
//! Everything here is **logical** pixels — the widget tree's unit, and
//! the unit Wayland delivers pointer coordinates in by contract. Physical
//! pixels appear in exactly one place in this app, `hyprforge_image`'s
//! budget, which decides how much to decode.
//!
//! That split is deliberate and load-bearing: the buffer being twice the
//! size on a scaled output must not move what a click lands on. CLAUDE.md
//! records the same division from the other side in `hyprforge-popup`,
//! and records what mixing them costs — a crop taken at an `hyprctl` box
//! on a 1.6-scale display lands two-thirds of the way up and left of the
//! window you meant.
//!
//! So: [`ImageSize`] is the size of the *picture*, in image pixels.
//! [`Viewport`] is the window, logical. [`PanLogical`] is an offset,
//! logical. Nothing here takes a scale factor at all, which is how it
//! stays impossible to mix them up.

/// The size of the picture being shown, in its own pixels.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ImageSize {
    pub width: f32,
    pub height: f32,
}

/// The area the picture is shown in, in logical pixels.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Viewport {
    pub width: f32,
    pub height: f32,
}

/// An offset of the picture within the viewport, logical pixels.
///
/// Positive x moves the picture right. The value is the position of the
/// picture's centre relative to the viewport's centre, so zero is
/// centred whatever the sizes are — which makes "centre it" the default
/// rather than a calculation.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct PanLogical {
    pub x: f32,
    pub y: f32,
}

/// A point in the viewport, logical pixels, measured from its top-left.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct LogicalPoint {
    pub x: f32,
    pub y: f32,
}

/// How the picture is scaled.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Zoom {
    /// As large as fits, never larger than 1:1 — a thumbnail opened
    /// full-screen shows at its own size rather than as a blur.
    Fit,
    /// An explicit factor: 1.0 is one image pixel per logical pixel.
    Factor(f32),
}

/// The smallest and largest explicit zoom.
///
/// The lower bound keeps a picture from being zoomed to invisibility and
/// then lost; the upper one is where a photograph becomes squares, which
/// is far enough for checking focus.
const MIN_ZOOM: f32 = 0.02;
const MAX_ZOOM: f32 = 32.0;

/// How much one zoom step changes the factor. A twelfth root of two
/// would be neat; a fifth is what feels like one step.
const ZOOM_STEP: f32 = 1.25;

/// Everything about how the picture is currently presented.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Transform {
    pub zoom: Zoom,
    pub pan: PanLogical,
}

impl Default for Transform {
    fn default() -> Self {
        Transform { zoom: Zoom::Fit, pan: PanLogical::default() }
    }
}

impl Transform {
    /// The scale factor actually applied, resolving [`Zoom::Fit`]
    /// against the sizes.
    pub fn scale(&self, image: ImageSize, viewport: Viewport) -> f32 {
        match self.zoom {
            Zoom::Fit => fit_scale(image, viewport),
            Zoom::Factor(f) => f.clamp(MIN_ZOOM, MAX_ZOOM),
        }
    }

    /// The size the picture is drawn at, logical pixels.
    pub fn drawn_size(&self, image: ImageSize, viewport: Viewport) -> (f32, f32) {
        let scale = self.scale(image, viewport);
        (image.width * scale, image.height * scale)
    }

    /// Fit the whole picture, centred.
    pub fn fit(&mut self) {
        self.zoom = Zoom::Fit;
        self.pan = PanLogical::default();
    }

    /// One image pixel per logical pixel, centred.
    pub fn actual_size(&mut self) {
        self.zoom = Zoom::Factor(1.0);
        self.pan = PanLogical::default();
    }

    /// One step in or out, about the centre of the window.
    pub fn zoom_by(&mut self, steps: i32, image: ImageSize, viewport: Viewport) {
        let from = self.scale(image, viewport);
        let to = (from * ZOOM_STEP.powi(steps)).clamp(MIN_ZOOM, MAX_ZOOM);
        self.zoom = Zoom::Factor(to);
        // The pan is in logical pixels, so growing the picture has to
        // grow the offset with it or the point under the centre drifts.
        let ratio = to / from;
        self.pan = PanLogical { x: self.pan.x * ratio, y: self.pan.y * ratio };
        self.clamp_pan(image, viewport);
    }

    /// Zoom about a point — the wheel-over-the-pointer behaviour.
    ///
    /// The property this has to keep is the one in its test: whatever
    /// part of the picture was under `at` is still under `at` afterwards.
    /// Anything else makes zooming feel like the picture is running away.
    pub fn zoom_about(
        &mut self,
        steps: i32,
        at: LogicalPoint,
        image: ImageSize,
        viewport: Viewport,
    ) {
        let from = self.scale(image, viewport);
        let to = (from * ZOOM_STEP.powi(steps)).clamp(MIN_ZOOM, MAX_ZOOM);
        if (to - from).abs() < f32::EPSILON {
            return;
        }

        // Where the cursor is relative to the viewport's centre.
        let cx = at.x - viewport.width / 2.0;
        let cy = at.y - viewport.height / 2.0;
        // The same point in the picture's own space, before the change.
        let ix = (cx - self.pan.x) / from;
        let iy = (cy - self.pan.y) / from;

        self.zoom = Zoom::Factor(to);
        // Solve for the pan that puts that image point back under the
        // cursor at the new scale.
        self.pan = PanLogical { x: cx - ix * to, y: cy - iy * to };
        self.clamp_pan(image, viewport);
    }

    /// Move the picture by a delta in logical pixels.
    pub fn pan_by(&mut self, dx: f32, dy: f32, image: ImageSize, viewport: Viewport) {
        self.pan = PanLogical { x: self.pan.x + dx, y: self.pan.y + dy };
        self.clamp_pan(image, viewport);
    }

    /// Keeps the picture from being dragged off the screen.
    ///
    /// On an axis where the picture is smaller than the window it is
    /// pinned to the centre — a small picture sliding around inside a
    /// large window is nobody's idea of panning. On an axis where it is
    /// larger, the offset is bounded so an edge can reach the middle of
    /// the window but no further, which is what keeps some of the
    /// picture always visible.
    pub fn clamp_pan(&mut self, image: ImageSize, viewport: Viewport) {
        let (drawn_w, drawn_h) = self.drawn_size(image, viewport);

        let limit = |drawn: f32, available: f32| -> f32 {
            if drawn <= available {
                0.0
            } else {
                (drawn - available) / 2.0
            }
        };

        let max_x = limit(drawn_w, viewport.width);
        let max_y = limit(drawn_h, viewport.height);
        self.pan = PanLogical {
            x: self.pan.x.clamp(-max_x, max_x),
            y: self.pan.y.clamp(-max_y, max_y),
        };
    }
}

/// The scale that fits `image` inside `viewport`, never above 1:1.
fn fit_scale(image: ImageSize, viewport: Viewport) -> f32 {
    if image.width <= 0.0 || image.height <= 0.0 {
        return 1.0;
    }
    let sx = viewport.width / image.width;
    let sy = viewport.height / image.height;
    // `clamp` is safe here where it is not in general: the bounds are
    // constants and MIN_ZOOM is far below 1.0, so the panic it warns
    // about (max < min) cannot happen.
    sx.min(sy).clamp(MIN_ZOOM, 1.0)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn image(w: f32, h: f32) -> ImageSize {
        ImageSize { width: w, height: h }
    }

    fn viewport(w: f32, h: f32) -> Viewport {
        Viewport { width: w, height: h }
    }

    #[test]
    fn fitting_shows_the_whole_picture() {
        let t = Transform::default();
        let (w, h) = t.drawn_size(image(4000.0, 2000.0), viewport(1000.0, 1000.0));
        assert!(w <= 1000.0 && h <= 1000.0);
        assert_eq!((w, h), (1000.0, 500.0));
    }

    /// A small picture opened full-screen shows at its own size rather
    /// than blown up into a blur.
    #[test]
    fn fitting_never_enlarges_a_small_picture() {
        let t = Transform::default();
        assert_eq!(t.drawn_size(image(64.0, 64.0), viewport(1920.0, 1080.0)), (64.0, 64.0));
    }

    /// The property that makes wheel-zoom feel right: the part of the
    /// picture under the pointer stays under the pointer.
    ///
    /// Only where there is room to pan, which is why the fixture starts
    /// already zoomed past fit. `zoom_about` solves for the pan exactly,
    /// and then `clamp_pan` overrules it on any axis where the picture
    /// is smaller than the viewport — a picture that fits is pinned to
    /// the centre, and dragging it off-centre is not something the
    /// pointer should be able to ask for. An axis with no freedom
    /// cannot keep a point under the cursor, and a test that asked it
    /// to was asserting against the clamp rather than against the zoom:
    /// 2000x1000 at fit in an 800x600 viewport draws 800x400, so the
    /// vertical was pinned the whole time.
    #[test]
    fn zooming_about_the_pointer_keeps_that_point_under_it() {
        // 4000x3000 in 800x600 fits at 0.2; starting at 0.4 draws
        // 1600x1200, larger than the viewport both ways, and one step
        // in stays larger. So both axes have room throughout.
        let img = image(4000.0, 3000.0);
        let vp = viewport(800.0, 600.0);
        let at = LogicalPoint { x: 200.0, y: 150.0 };

        let mut t = Transform { zoom: Zoom::Factor(0.4), pan: PanLogical::default() };
        let image_point = |t: &Transform| {
            let scale = t.scale(img, vp);
            let cx = at.x - vp.width / 2.0;
            let cy = at.y - vp.height / 2.0;
            ((cx - t.pan.x) / scale, (cy - t.pan.y) / scale)
        };

        let before = image_point(&t);
        t.zoom_about(1, at, img, vp);
        let after = image_point(&t);

        assert!((before.0 - after.0).abs() < 1.0, "{before:?} vs {after:?}");
        assert!((before.1 - after.1).abs() < 1.0, "{before:?} vs {after:?}");
    }

    #[test]
    fn zooming_in_and_back_out_returns_to_about_where_it_started() {
        let img = image(1600.0, 1200.0);
        let vp = viewport(800.0, 600.0);
        let mut t = Transform::default();
        let start = t.scale(img, vp);
        t.zoom_by(3, img, vp);
        t.zoom_by(-3, img, vp);
        assert!((t.scale(img, vp) - start).abs() < 0.001);
    }

    /// Zoom is bounded at both ends: a picture cannot be shrunk to
    /// nothing and lost, nor enlarged past the point of usefulness.
    #[test]
    fn zoom_is_bounded_at_both_ends() {
        let img = image(1000.0, 1000.0);
        let vp = viewport(500.0, 500.0);
        let mut t = Transform::default();

        t.zoom_by(200, img, vp);
        assert!(t.scale(img, vp) <= MAX_ZOOM);
        t.zoom_by(-500, img, vp);
        assert!(t.scale(img, vp) >= MIN_ZOOM);
    }

    /// The pan rule: a picture that fits is pinned to the centre, so it
    /// cannot be nudged around inside a window it does not fill.
    #[test]
    fn a_picture_that_fits_cannot_be_panned_off_centre() {
        let img = image(200.0, 200.0);
        let vp = viewport(800.0, 600.0);
        let mut t = Transform::default();
        t.pan_by(300.0, 300.0, img, vp);
        assert_eq!(t.pan, PanLogical::default());
    }

    /// And a picture larger than the window can never be dragged
    /// entirely out of it.
    #[test]
    fn a_pan_can_never_leave_the_picture_off_screen_entirely() {
        let img = image(4000.0, 3000.0);
        let vp = viewport(800.0, 600.0);
        let mut t = Transform::default();
        t.actual_size();

        for (dx, dy) in [(10_000.0, 0.0), (-20_000.0, 0.0), (0.0, 9_000.0), (0.0, -18_000.0)] {
            t.pan_by(dx, dy, img, vp);
            let (drawn_w, drawn_h) = t.drawn_size(img, vp);
            assert!(t.pan.x.abs() <= (drawn_w - vp.width) / 2.0 + 0.001, "{:?}", t.pan);
            assert!(t.pan.y.abs() <= (drawn_h - vp.height) / 2.0 + 0.001, "{:?}", t.pan);
        }
    }

    #[test]
    fn fit_and_actual_size_both_recentre() {
        let img = image(4000.0, 3000.0);
        let vp = viewport(800.0, 600.0);
        let mut t = Transform::default();
        t.actual_size();
        t.pan_by(100.0, 100.0, img, vp);
        assert_ne!(t.pan, PanLogical::default());

        t.fit();
        assert_eq!(t.pan, PanLogical::default());
        assert_eq!(t.zoom, Zoom::Fit);

        t.pan_by(100.0, 100.0, img, vp);
        t.actual_size();
        assert_eq!(t.pan, PanLogical::default());
    }

    /// A viewport of zero — a window mid-resize, or before the first
    /// layout — must not produce a NaN that poisons every later frame.
    #[test]
    fn a_degenerate_viewport_never_produces_a_nonsense_scale() {
        let t = Transform::default();
        let scale = t.scale(image(100.0, 100.0), viewport(0.0, 0.0));
        assert!(scale.is_finite() && scale > 0.0, "{scale}");

        let scale = t.scale(image(0.0, 0.0), viewport(800.0, 600.0));
        assert!(scale.is_finite() && scale > 0.0, "{scale}");
    }
}
