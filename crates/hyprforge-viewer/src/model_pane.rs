//! A 3D model as a piece of a window: the mesh, its camera, and the
//! hands on it — drag to turn, right-drag to pan, scroll to zoom about
//! the pointer.
//!
//! Media's viewer and Files' Quick Look both show one; the drawing is
//! [`crate::model`]'s, and this is the state and input around it, so a
//! model turns the same way in both.
//!
//! Loading is the window's to start ([`load`], off the UI thread) and to
//! decide about — Media keeps the last good model up through a failed
//! reload, a glance has nothing to keep — so the pane only exists once a
//! mesh does.

use crate::model::{ModelProgram, Style};
use hyprforge_mesh::camera::Camera;
use hyprforge_mesh::mesh::Mesh;
use hyprforge_mesh::style::Projection;
use hyprforge_ui::theme::{self, surface, FontScale};
use hyprforge_ui::widgets::meta_text;
use iced::widget::{container, mouse_area, stack};
use iced::{mouse, Element, Length, Point, Size, Task};
use std::path::{Path, PathBuf};
use std::sync::Arc;

/// Scroll units per wheel notch for the camera, which counts in view3d's
/// (egui's) points; iced's `Lines` are notches.
pub const UNITS_PER_LINE: f32 = 50.0;

/// Which way a drag moves the camera.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Drag {
    Turn,
    Pan,
}

/// What the pane is told.
#[derive(Debug, Clone, PartialEq)]
pub enum ModelMessage {
    Press(Drag),
    Release,
    /// The pointer, over the pane, in its own coordinates.
    Pointer(Point),
    Scrolled(mouse::ScrollDelta),
}

/// A mesh read off the disk, on its way to a pane.
#[derive(Clone)]
pub struct Loaded {
    pub mesh: Arc<Mesh>,
    /// A file that loaded but not entirely — an OBJ whose materials could
    /// not be read, drawn uncoloured.
    pub warning: Option<String>,
    pub modified: Option<std::time::SystemTime>,
    pub bytes: Option<u64>,
}

/// The same load: the same mesh — by identity, not vertex by vertex —
/// and the same facts about the file.
impl PartialEq for Loaded {
    fn eq(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.mesh, &other.mesh)
            && self.warning == other.warning
            && self.modified == other.modified
            && self.bytes == other.bytes
    }
}

impl std::fmt::Debug for Loaded {
    /// A line, not every vertex of the model.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Loaded").field("warning", &self.warning).finish_non_exhaustive()
    }
}

/// Reads `path` off the UI thread. `Err` is a sentence.
pub fn load(path: PathBuf) -> Task<Result<Loaded, String>> {
    Task::future(async move {
        let (tx, rx) = iced::futures::channel::oneshot::channel();
        std::thread::Builder::new()
            .name("model-load".into())
            .spawn(move || {
                let meta = std::fs::metadata(&path).ok();
                let modified = meta.as_ref().and_then(|m| m.modified().ok());
                let bytes = meta.map(|m| m.len());
                let result = hyprforge_mesh::load(&path, true)
                    .map(|(mesh, warning)| Loaded { mesh: Arc::new(mesh), warning, modified, bytes })
                    .map_err(|e| format!("Couldn't read this model: {e:#}"));
                let _ = tx.send(result);
            })
            .map_err(|e| format!("The model loader couldn't start: {e}"))?;
        rx.await.unwrap_or_else(|_| Err("The model loader stopped unexpectedly.".to_string()))
    })
}

/// A model in a window.
pub struct ModelPane {
    path: PathBuf,
    mesh: Arc<Mesh>,
    generation: u64,
    camera: Camera,
    drag: Option<(Drag, Point)>,
    cursor: Option<Point>,
}

impl std::fmt::Debug for ModelPane {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ModelPane").field("path", &self.path).field("generation", &self.generation).finish_non_exhaustive()
    }
}

impl ModelPane {
    /// `loaded`, seen from the front and fitted to the pane. `generation`
    /// tells the GPU this is a new mesh; the window bumps it per load.
    pub fn new(path: PathBuf, loaded: &Loaded, generation: u64, projection: Projection) -> ModelPane {
        let mut camera = Camera::default();
        camera.perspective = projection.value();
        let b = loaded.mesh.bounds;
        camera.fit(b.min, b.max, false, true);
        ModelPane { path, mesh: loaded.mesh.clone(), generation, camera, drag: None, cursor: None }
    }

    /// The same file read again: the new mesh, the view kept.
    pub fn reloaded(&mut self, loaded: &Loaded, generation: u64) {
        self.mesh = loaded.mesh.clone();
        self.generation = generation;
        let b = loaded.mesh.bounds;
        self.camera.fit(b.min, b.max, true, true);
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    pub fn mesh(&self) -> &Arc<Mesh> {
        &self.mesh
    }

    pub fn camera(&self) -> &Camera {
        &self.camera
    }

    /// For the window's own camera keys — viewpoints, projection, zoom.
    pub fn camera_mut(&mut self) -> &mut Camera {
        &mut self.camera
    }

    /// Whether a drag is under way — what the cursor shows.
    pub fn dragging(&self) -> bool {
        self.drag.is_some()
    }

    /// One message, for a pane of `size` logical pixels.
    pub fn update(&mut self, message: ModelMessage, size: Size) {
        let (w, h) = (size.width, size.height);
        match message {
            ModelMessage::Press(drag) => self.drag = self.cursor.map(|at| (drag, at)),
            ModelMessage::Release => self.drag = None,
            ModelMessage::Pointer(point) => {
                if let Some((drag, last)) = self.drag {
                    match drag {
                        Drag::Turn => self.camera.rotate([last.x, last.y], [point.x, point.y], w, h),
                        Drag::Pan => self.camera.pan([point.x - last.x, point.y - last.y], w, h),
                    }
                    self.drag = Some((drag, point));
                }
                self.cursor = Some(point);
            }
            ModelMessage::Scrolled(delta) => {
                let units = match delta {
                    mouse::ScrollDelta::Lines { y, .. } => y * UNITS_PER_LINE,
                    mouse::ScrollDelta::Pixels { y, .. } => y,
                };
                let at = self.cursor.map_or([w / 2.0, h / 2.0], |p| [p.x, p.y]);
                self.camera.zoom_at(at, units, false, w, h);
            }
        }
    }

    /// The model, `size` logical pixels, in `style`, with the hands on it.
    pub fn view(&self, size: Size, style: Style, scale: FontScale) -> Element<'_, ModelMessage> {
        let program = ModelProgram { mesh: self.mesh.clone(), generation: self.generation, camera: self.camera, style };
        let scene = mouse_area(iced::widget::shader(program).width(Length::Fixed(size.width)).height(Length::Fixed(size.height)))
            .on_press(ModelMessage::Press(Drag::Turn))
            .on_release(ModelMessage::Release)
            .on_right_press(ModelMessage::Press(Drag::Pan))
            .on_right_release(ModelMessage::Release)
            .on_move(ModelMessage::Pointer)
            .on_scroll(ModelMessage::Scrolled)
            .interaction(if self.drag.is_some() { mouse::Interaction::Grabbing } else { mouse::Interaction::Grab });
        // Behind the scene, and covered by its backdrop whenever wgpu is
        // drawing — see `model`'s module doc.
        let fallback = container(meta_text(
            "3D models need GPU rendering, and this window is drawing without it.",
            theme::BASE_TEXT_SIZE,
            scale,
        ))
        .center(Length::Fill)
        .style(|_t: &iced::Theme| container::Style { background: Some(surface::root().into()), ..container::Style::default() });
        stack![fallback, scene].into()
    }
}
