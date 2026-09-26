//! Camera / view transforms, ported from fstl's `Canvas`.
//!
//! The matrix conventions follow fstl exactly (including the odd-looking
//! aspect matrix with its negative X scale), with one addition: OpenGL clip
//! space has z in [-1, 1] while wgpu uses [0, 1], so [`Camera::mvp`] folds in a
//! remap of z' = 0.5 * (z + w).

use glam::{Mat4, Vec3};

pub const P_PERSPECTIVE: f32 = 0.25;
pub const P_ORTHOGRAPHIC: f32 = 0.0;

#[derive(Copy, Clone, PartialEq, Eq, Debug)]
pub enum ViewPoint {
    Iso,
    Top,
    Bottom,
    Front,
    Back,
    Left,
    Right,
    Center,
}

/// GL (z in [-1,1]) to wgpu (z in [0,1]) clip-space conversion.
fn gl_to_wgpu() -> Mat4 {
    Mat4::from_cols(
        glam::vec4(1.0, 0.0, 0.0, 0.0),
        glam::vec4(0.0, 1.0, 0.0, 0.0),
        glam::vec4(0.0, 0.0, 0.5, 0.0),
        glam::vec4(0.0, 0.0, 0.5, 1.0),
    )
}

fn rotate_deg(m: Mat4, degrees: f32, axis: Vec3) -> Mat4 {
    // Qt's QMatrix4x4::rotate post-multiplies and normalizes the axis for us.
    m * Mat4::from_axis_angle(axis.normalize(), degrees.to_radians())
}

/// Plain data, and `Copy`: the window keeps one and hands a copy to each
/// frame it draws, the way it hands the photo's transform to its view.
#[derive(Copy, Clone, Debug, PartialEq)]
pub struct Camera {
    /// Orientation only (fstl's `currentTransform`).
    pub orient: Mat4,
    pub center: Vec3,
    pub scale: f32,
    pub zoom: f32,
    pub perspective: f32,
    default_center: Vec3,
    default_scale: f32,
}

impl Default for Camera {
    fn default() -> Self {
        let mut c = Self {
            orient: Mat4::IDENTITY,
            center: Vec3::ZERO,
            scale: 1.0,
            zoom: 1.0,
            perspective: P_PERSPECTIVE,
            default_center: Vec3::ZERO,
            default_scale: 1.0,
        };
        c.reset_orientation();
        c
    }
}

impl Camera {
    pub fn reset_orientation(&mut self) {
        let mut m = Mat4::IDENTITY;
        m = rotate_deg(m, -90.0, Vec3::X);
        m = rotate_deg(m, 180.0 + 15.0, Vec3::Z);
        m = rotate_deg(
            m,
            15.0,
            Vec3::new(1.0, -(std::f32::consts::PI / 12.0).sin(), 0.0),
        );
        self.orient = m;
        self.zoom = 1.0;
    }

    /// Frame a freshly loaded mesh. `keep_view` is used on reload/autoreload so
    /// the user does not lose their vantage point.
    pub fn fit(&mut self, min: Vec3, max: Vec3, keep_view: bool, reset_orientation: bool) {
        if keep_view {
            return;
        }
        self.default_center = (min + max) * 0.5;
        self.center = self.default_center;
        let diag = (max - min).length();
        self.default_scale = if diag > 0.0 { 2.0 / diag } else { 1.0 };
        self.scale = self.default_scale;
        self.zoom = 1.0;
        if reset_orientation {
            self.reset_orientation();
        }
    }

    pub fn set_viewpoint(&mut self, v: ViewPoint) {
        if v == ViewPoint::Center {
            self.scale = self.default_scale;
            self.center = self.default_center;
            self.zoom = 1.0;
            return;
        }

        let mut m = rotate_deg(Mat4::IDENTITY, 180.0, Vec3::Z);
        match v {
            ViewPoint::Iso => {
                m = rotate_deg(m, 90.0, Vec3::X);
                m = rotate_deg(m, -45.0, Vec3::Z);
                m = rotate_deg(m, 35.264, Vec3::new(1.0, 1.0, 0.0));
            }
            ViewPoint::Top => m = rotate_deg(m, 180.0, Vec3::X),
            ViewPoint::Left => {
                m = rotate_deg(m, 180.0, Vec3::X);
                m = rotate_deg(m, 90.0, Vec3::Z);
                m = rotate_deg(m, 90.0, Vec3::Y);
            }
            ViewPoint::Right => {
                m = rotate_deg(m, 180.0, Vec3::X);
                m = rotate_deg(m, -90.0, Vec3::Y);
                m = rotate_deg(m, -90.0, Vec3::X);
            }
            ViewPoint::Front => m = rotate_deg(m, 90.0, Vec3::X),
            ViewPoint::Back => {
                m = rotate_deg(m, 90.0, Vec3::X);
                m = rotate_deg(m, 180.0, Vec3::Z);
            }
            ViewPoint::Bottom | ViewPoint::Center => {}
        }
        self.orient = m;
    }

    pub fn transform_matrix(&self) -> Mat4 {
        self.orient
            * Mat4::from_scale(Vec3::splat(self.scale))
            * Mat4::from_translation(-self.center)
    }

    pub fn aspect_matrix(&self, width: f32, height: f32) -> Mat4 {
        if width > height {
            Mat4::from_scale(Vec3::new(-height / width, 1.0, 0.5))
        } else {
            Mat4::from_scale(Vec3::new(-1.0, width / height, 0.5))
        }
    }

    pub fn view_matrix(&self, width: f32, height: f32) -> Mat4 {
        let mut m = self.aspect_matrix(width, height)
            * Mat4::from_scale(Vec3::new(self.zoom, self.zoom, 1.0));
        // Qt's m(3, 2) = perspective, i.e. w = perspective * z.
        m.z_axis.w = self.perspective;
        m
    }

    /// Full model-view-projection in wgpu clip space.
    pub fn mvp(&self, width: f32, height: f32) -> Mat4 {
        gl_to_wgpu() * self.view_matrix(width, height) * self.transform_matrix()
    }

    /// Maps a matrix built in fstl's GL conventions into wgpu clip space.
    pub fn to_wgpu_clip(m: Mat4) -> Mat4 {
        gl_to_wgpu() * m
    }

    /// Screen point (pixels, y down) to fstl's [-1, 1] canvas coordinates.
    fn canvas_coords(p: [f32; 2], width: f32, height: f32) -> [f32; 2] {
        [p[0] / (width / 2.0) - 1.0, p[1] / (height / 2.0) - 1.0]
    }

    /// Arcball rotation between two screen points (pixels, y down).
    pub fn rotate(&mut self, from: [f32; 2], to: [f32; 2], width: f32, height: f32) {
        let p1 = Self::canvas_coords(from, width, height);
        let p2 = Self::canvas_coords(to, width, height);

        let on_ball = |p: [f32; 2]| -> Vec3 {
            let sq = p[0] * p[0] + p[1] * p[1];
            if sq <= 1.0 {
                Vec3::new(p[0], p[1], (1.0 - sq).sqrt())
            } else {
                let l = sq.sqrt();
                Vec3::new(p[0] / l, p[1] / l, 0.0)
            }
        };

        let v1 = on_ball(p1);
        let v2 = on_ball(p2);
        let axis_eye = v1.cross(v2);
        if axis_eye.length_squared() < 1e-12 {
            return;
        }
        let axis_obj = self.orient.inverse().transform_vector3(axis_eye);
        if axis_obj.length_squared() < 1e-12 {
            return;
        }
        let angle = v1.dot(v2).min(1.0).acos().to_degrees();
        self.orient = rotate_deg(self.orient, angle, axis_obj);
    }

    /// Pan by a screen-space delta in pixels (y down).
    pub fn pan(&mut self, delta: [f32; 2], width: f32, height: f32) {
        let v = Vec3::new(-delta[0] / (0.5 * width), delta[1] / (0.5 * height), 0.0);
        let inv = self.transform_matrix().inverse() * self.view_matrix(width, height).inverse();
        self.center = inv.project_point3(v);
    }

    /// Zoom about the cursor. `units` is a scroll amount in egui points;
    /// positive means scrolling up/away, matching fstl's wheel direction.
    pub fn zoom_at(&mut self, cursor: [f32; 2], units: f32, invert: bool, width: f32, height: f32) {
        let v = Vec3::new(
            1.0 - cursor[0] / (0.5 * width),
            cursor[1] / (0.5 * height) - 1.0,
            0.0,
        );
        let unproject = |cam: &Self| -> Vec3 {
            let inv = cam.transform_matrix().inverse() * cam.view_matrix(width, height).inverse();
            inv.project_point3(v)
        };

        let a = unproject(self);
        // fstl multiplies by 1.001 per wheel unit (120 units per notch); egui
        // reports ~50 points per notch, so scale the exponent to match.
        let k = 0.0024 * units;
        self.zoom *= if invert { k.exp() } else { (-k).exp() };
        self.zoom = self.zoom.clamp(1e-4, 1e6);
        let b = unproject(self);
        // `a - b`, not view3d's `b - a`. `center` enters the transform as
        // `translate(-center)`, so moving it by `d` moves the point under
        // the cursor by `+d`; putting that point back at `a` needs
        // `d = a - b`. With `b - a` the correction doubled the drift
        // instead of cancelling it — the point under the cursor slid
        // further than an uncorrected zoom would have moved it.
        // `zooming_keeps_the_point_under_the_cursor_still` pins this.
        self.center += a - b;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn clip(camera: &Camera, p: Vec3, w: f32, h: f32) -> Vec3 {
        camera.mvp(w, h).project_point3(p)
    }

    /// A freshly fitted model sits in view: its centre at the middle of the
    /// screen and every corner of its box inside clip space.
    #[test]
    fn fitting_a_model_puts_all_of_it_on_screen() {
        let (min, max) = (Vec3::new(10.0, -40.0, 3.0), Vec3::new(90.0, 20.0, 60.0));
        let mut camera = Camera::default();
        camera.fit(min, max, false, true);
        let centre = clip(&camera, (min + max) * 0.5, 800.0, 600.0);
        assert!(centre.x.abs() < 1e-4 && centre.y.abs() < 1e-4, "{centre:?}");
        for i in 0..8 {
            let corner = Vec3::new(
                if i & 1 == 0 { min.x } else { max.x },
                if i & 2 == 0 { min.y } else { max.y },
                if i & 4 == 0 { min.z } else { max.z },
            );
            let c = clip(&camera, corner, 800.0, 600.0);
            assert!(c.x.abs() <= 1.0 && c.y.abs() <= 1.0, "corner {corner} at {c}");
            assert!((0.0..=1.0).contains(&c.z), "corner {corner} depth {}", c.z);
        }
    }

    /// Reloading a file that changed on disk keeps the view — autoreload
    /// is useless if every save throws away where you were looking.
    #[test]
    fn a_reload_keeps_the_view() {
        let mut camera = Camera::default();
        camera.fit(Vec3::ZERO, Vec3::splat(10.0), false, true);
        camera.set_viewpoint(ViewPoint::Top);
        let before = camera.transform_matrix();
        camera.fit(Vec3::ZERO, Vec3::splat(50.0), true, true);
        assert_eq!(camera.transform_matrix(), before);
    }

    /// Every viewpoint is a pure rotation: nothing stretched, nothing
    /// mirrored, so a model never looks inside out from one side.
    #[test]
    fn every_viewpoint_is_a_rotation() {
        for v in [
            ViewPoint::Iso,
            ViewPoint::Top,
            ViewPoint::Bottom,
            ViewPoint::Front,
            ViewPoint::Back,
            ViewPoint::Left,
            ViewPoint::Right,
        ] {
            let mut camera = Camera::default();
            camera.set_viewpoint(v);
            let m = glam::Mat3::from_mat4(camera.orient);
            assert!((m.determinant() - 1.0).abs() < 1e-4, "{v:?}: det {}", m.determinant());
            assert!((m * m.transpose()).abs_diff_eq(glam::Mat3::IDENTITY, 1e-4), "{v:?}");
        }
    }

    /// A drag moves the model with the pointer: the point that was at the
    /// middle of the screen ends up the drag's distance away, same way.
    #[test]
    fn panning_moves_the_model_with_the_pointer() {
        let (w, h) = (800.0, 600.0);
        let mut camera = Camera { perspective: P_ORTHOGRAPHIC, ..Camera::default() };
        camera.fit(Vec3::ZERO, Vec3::splat(10.0), false, true);
        let middle = Vec3::splat(5.0);
        let before = clip(&camera, middle, w, h);
        camera.pan([80.0, 60.0], w, h);
        let after = clip(&camera, middle, w, h);
        // 80px right of 400 is +0.2 in NDC; 60px down of 300 is -0.2
        // (NDC's y points up, the screen's down).
        assert!((after.x - before.x - 0.2).abs() < 1e-3, "x moved {}", after.x - before.x);
        assert!((after.y - before.y + 0.2).abs() < 1e-3, "y moved {}", after.y - before.y);
    }

    /// Zooming about the pointer keeps what is under the pointer under it —
    /// the thing that makes zoom-to-cursor feel right rather than drift.
    #[test]
    fn zooming_keeps_the_point_under_the_cursor_still() {
        let (w, h) = (800.0, 600.0);
        let mut camera = Camera::default();
        camera.fit(Vec3::ZERO, Vec3::splat(10.0), false, true);
        let cursor = [600.0, 150.0];
        // The model-space point the cursor is over, the way `zoom_at` finds
        // it, and where on screen that point is drawn before and after.
        let v = Vec3::new(1.0 - cursor[0] / (0.5 * w), cursor[1] / (0.5 * h) - 1.0, 0.0);
        let point = (camera.transform_matrix().inverse() * camera.view_matrix(w, h).inverse()).project_point3(v);
        let before = clip(&camera, point, w, h);
        camera.zoom_at(cursor, 240.0, false, w, h);
        assert!(camera.zoom != 1.0);
        let after = clip(&camera, point, w, h);
        assert!(
            (after.x - before.x).abs() < 1e-3 && (after.y - before.y).abs() < 1e-3,
            "the point under the cursor moved on screen from {before} to {after}"
        );
    }
}
