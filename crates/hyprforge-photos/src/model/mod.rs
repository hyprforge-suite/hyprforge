//! The model view: a 3D model drawn in the photo viewer's pane, by an
//! iced `shader` widget carrying view3d's renderer.
//!
//! The window owns the [`Camera`] and the mesh; each frame it hands this a
//! copy of the camera and the style, and [`ModelProgram::draw`] turns them
//! into a [`ScenePrimitive`] that [`render::Scene`] draws. Input — drag to
//! turn, right-drag to pan, scroll to zoom — arrives through a
//! `mouse_area` around the widget and changes the window's camera, the
//! same way a photograph's transform is changed; nothing here holds state
//! of its own between frames.
//!
//! # When the GPU is not there
//!
//! iced falls back to its software renderer when wgpu cannot start, and a
//! `shader` primitive draws nothing there. The view puts a sentence
//! *behind* the widget saying so: with wgpu the scene's backdrop covers
//! it, and without wgpu it is what shows — a state with a message rather
//! than a blank pane, which is this suite's rule for anything absent.

pub mod render;

use glam::{Mat4, Vec3};
use hyprforge_mesh::camera::Camera;
use hyprforge_mesh::mesh::Mesh;
use hyprforge_mesh::style::{self, DrawMode};
use iced::widget::shader;
use iced::{mouse, Rectangle};
use render::{Scene, SceneParams, LINE_SLOTS};
use std::sync::Arc;

/// How the model is drawn this frame.
#[derive(Debug, Clone, Copy)]
pub struct Style {
    pub draw_mode: DrawMode,
    pub axes: bool,
    /// The theme's root surface — the backdrop.
    pub backdrop: iced::Color,
}

/// The `shader::Program` the view builds each frame.
pub struct ModelProgram {
    pub mesh: Arc<Mesh>,
    /// Bumped each time the window loads a mesh, so the GPU copy is
    /// replaced exactly when the model is.
    pub generation: u64,
    pub camera: Camera,
    pub style: Style,
}

impl<Message> shader::Program<Message> for ModelProgram {
    type State = ();
    type Primitive = ScenePrimitive;

    fn draw(&self, _state: &(), _cursor: mouse::Cursor, bounds: Rectangle) -> ScenePrimitive {
        ScenePrimitive {
            mesh: self.mesh.clone(),
            generation: self.generation,
            params: scene_params(&self.camera, bounds.width, bounds.height, &self.mesh, self.style),
        }
    }
}

/// One frame of the model view.
pub struct ScenePrimitive {
    mesh: Arc<Mesh>,
    generation: u64,
    params: SceneParams,
}

/// By hand: the derive would print every vertex of the mesh, and a
/// two-million-triangle model in a debug log is its own kind of outage.
impl std::fmt::Debug for ScenePrimitive {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ScenePrimitive")
            .field("triangles", &self.mesh.tri_count())
            .field("generation", &self.generation)
            .field("draw_mode", &self.params.draw_mode)
            .finish()
    }
}

impl shader::Pipeline for Scene {
    fn new(device: &iced::wgpu::Device, _queue: &iced::wgpu::Queue, format: iced::wgpu::TextureFormat) -> Self {
        Scene::new(device, format)
    }
}

impl shader::Primitive for ScenePrimitive {
    type Pipeline = Scene;

    fn prepare(
        &self,
        scene: &mut Scene,
        device: &iced::wgpu::Device,
        queue: &iced::wgpu::Queue,
        bounds: &Rectangle,
        viewport: &shader::Viewport,
    ) {
        if scene.generation != Some(self.generation) {
            scene.upload_mesh(device, queue, &self.mesh);
            scene.generation = Some(self.generation);
        }
        if self.params.draw_mode == DrawMode::Wireframe {
            scene.ensure_edges(device, &self.mesh);
        }
        let target = viewport.physical_size();
        scene.ensure_depth(device, target.width, target.height);
        // Bounds arrive logical; the pass's viewport is physical pixels.
        let s = viewport.scale_factor();
        scene.frame = [bounds.x * s, bounds.y * s, bounds.width * s, bounds.height * s];
        scene.write_uniforms(queue, &self.params);
    }

    fn render(
        &self,
        scene: &Scene,
        encoder: &mut iced::wgpu::CommandEncoder,
        target: &iced::wgpu::TextureView,
        clip_bounds: &Rectangle<u32>,
    ) {
        scene.render(encoder, target, clip_bounds, &self.params);
    }
}

/// The frame's matrices and lighting, from the camera and the style —
/// view3d's `scene_params`, with its lighting settings at their (fstl's)
/// defaults.
fn scene_params(camera: &Camera, w: f32, h: f32, mesh: &Mesh, style: Style) -> SceneParams {
    let (w, h) = (w.max(1.0), h.max(1.0));
    let dirs = style::light_directions();
    let light = dirs.get(style::LIGHT_DIRECTION).map(|(d, _)| *d).unwrap_or([0.0, 0.0, 1.0]);

    // The axis flower: a small copy of the orientation in a bottom
    // corner, drawn over the model. The translation reads as bottom right,
    // but fstl's aspect matrix mirrors X, so it lands bottom left — where
    // fstl and view3d draw it.
    let aspect = camera.aspect_matrix(w, h);
    let ar = w / h;
    let hud_size = 0.2;
    let hud = Mat4::from_translation(if ar > 1.0 {
        Vec3::new(ar - 2.0 * hud_size, -1.0 + 2.0 * hud_size, 0.0)
    } else {
        Vec3::new(1.0 - 2.0 * hud_size, -1.0 / ar + 2.0 * hud_size, 0.0)
    }) * Mat4::from_scale(Vec3::new(hud_size, hud_size, 1.0));
    let hud_view = aspect * hud;

    let mut line_mvps = [Mat4::IDENTITY; LINE_SLOTS];
    line_mvps[0] = camera.mvp(w, h);
    line_mvps[1] = Camera::to_wgpu_clip(hud_view * camera.orient);
    for axis in 0..3 {
        let mut v = Vec3::ZERO;
        v[axis] = 1.25;
        let label = Mat4::from_translation(camera.orient.transform_point3(v));
        line_mvps[2 + axis] = Camera::to_wgpu_clip(hud_view * label);
    }

    SceneParams {
        mvp: camera.mvp(w, h),
        ambient: style::AMBIENT,
        directive: style::DIRECTIVE,
        light_dir: light,
        zoom_inv: 1.0 / camera.zoom,
        has_colors: mesh.has_colors,
        draw_mode: style.draw_mode,
        draw_axes: style.axes,
        line_mvps,
        backdrop: style.backdrop,
    }
}

#[cfg(test)]
mod tests {
    /// Both of the model view's shaders compile — checked here, because
    /// wgpu refusing one at run time takes the whole window with it.
    #[test]
    fn the_model_shaders_compile() {
        crate::film::assert_valid_wgsl("scene.wgsl", include_str!("scene.wgsl"));
        crate::film::assert_valid_wgsl("lines.wgsl", include_str!("lines.wgsl"));
    }
}
