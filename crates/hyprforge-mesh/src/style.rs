//! How a mesh is drawn: the five draw modes, the two projections, and the
//! 26 light directions — view3d's settings, less the ones that belonged to
//! its window (recent files, the menu bar) rather than to the drawing.
//!
//! Here rather than beside the renderer so the choices are plain data a
//! test and a `media.toml` can hold; the renderer maps each one to a
//! shader entry point.

use crate::camera::{P_ORTHOGRAPHIC, P_PERSPECTIVE};

#[derive(Copy, Clone, PartialEq, Eq, Debug, Default)]
pub enum Projection {
    #[default]
    Perspective,
    Orthographic,
}

impl Projection {
    /// The value fstl's view matrix puts in its `w = p * z` slot.
    pub fn value(self) -> f32 {
        match self {
            Self::Perspective => P_PERSPECTIVE,
            Self::Orthographic => P_ORTHOGRAPHIC,
        }
    }

    pub fn toggled(self) -> Self {
        match self {
            Self::Perspective => Self::Orthographic,
            Self::Orthographic => Self::Perspective,
        }
    }

    /// The word `media.toml` stores. Part of the file format.
    pub fn id(self) -> &'static str {
        match self {
            Self::Perspective => "perspective",
            Self::Orthographic => "orthographic",
        }
    }

    pub fn from_id(id: &str) -> Option<Self> {
        [Self::Perspective, Self::Orthographic].into_iter().find(|p| p.id() == id)
    }
}

#[derive(Copy, Clone, PartialEq, Eq, Debug, Default)]
pub enum DrawMode {
    /// The fstl look: flat shading from a normal recovered per fragment.
    #[default]
    Shaded,
    Wireframe,
    /// Face orientation as colour — overhangs stand out.
    SurfaceAngle,
    /// An ambient and a directional light, from one of 26 directions.
    MeshLight,
    /// The colours the file carries (3MF materials, OBJ/MTL diffuse).
    Material,
}

impl DrawMode {
    /// Modes drawn as filled triangles, in pipeline order.
    pub const FILLED: [Self; 4] = [Self::Shaded, Self::SurfaceAngle, Self::MeshLight, Self::Material];

    pub const ALL: [Self; 5] = [Self::Shaded, Self::Wireframe, Self::SurfaceAngle, Self::MeshLight, Self::Material];

    /// Which filled pipeline draws this mode; `None` for the wireframe.
    pub fn fill_index(self) -> Option<usize> {
        Self::FILLED.iter().position(|m| *m == self)
    }

    /// The fragment shader entry point.
    pub fn entry_point(self) -> &'static str {
        match self {
            Self::Shaded => "fs_shaded",
            Self::Wireframe => "fs_wireframe",
            Self::SurfaceAngle => "fs_surface_angle",
            Self::MeshLight => "fs_mesh_light",
            Self::Material => "fs_material",
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::Shaded => "Shaded",
            Self::Wireframe => "Wireframe",
            Self::SurfaceAngle => "Surface Angle",
            Self::MeshLight => "Mesh Light",
            Self::Material => "Material Color",
        }
    }

    /// A short label for a segmented control.
    pub fn short_label(self) -> &'static str {
        match self {
            Self::Shaded => "Shaded",
            Self::Wireframe => "Wire",
            Self::SurfaceAngle => "Angle",
            Self::MeshLight => "Light",
            Self::Material => "Color",
        }
    }

    /// The next mode, wrapping — what one key press cycles through.
    pub fn next(self) -> Self {
        let i = Self::ALL.iter().position(|m| *m == self).unwrap_or(0);
        Self::ALL[(i + 1) % Self::ALL.len()]
    }

    /// The word `media.toml` stores. Part of the file format.
    pub fn id(self) -> &'static str {
        match self {
            Self::Shaded => "shaded",
            Self::Wireframe => "wireframe",
            Self::SurfaceAngle => "surface-angle",
            Self::MeshLight => "mesh-light",
            Self::Material => "material",
        }
    }

    pub fn from_id(id: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|m| m.id() == id)
    }
}

/// The 26 light directions fstl offers: every combination of -1, 0 and 1
/// on each axis except all three zero, with a name for each.
pub fn light_directions() -> Vec<([f32; 3], String)> {
    let xname = ["right ", " ", "left "];
    let yname = ["top ", " ", "bottom "];
    let zname = ["rear ", " ", "front "];
    let mut out = Vec::with_capacity(26);
    for i in -1..2i32 {
        for j in -1..2i32 {
            for k in -1..2i32 {
                if i == 0 && j == 0 && k == 0 {
                    continue;
                }
                let name = format!("{}{}{}", xname[(i + 1) as usize], yname[(j + 1) as usize], zname[(k + 1) as usize]);
                let name = name.split_whitespace().collect::<Vec<_>>().join(" ");
                out.push(([i as f32, j as f32, k as f32], name));
            }
        }
    }
    out
}

/// view3d's lighting defaults, which are fstl's.
pub const AMBIENT: [f32; 4] = [0.22, 0.8, 1.0, 0.67];
pub const DIRECTIVE: [f32; 4] = [1.0, 1.0, 1.0, 0.5];
pub const LIGHT_DIRECTION: usize = 1;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn there_are_twenty_six_light_directions_and_none_is_zero() {
        let dirs = light_directions();
        assert_eq!(dirs.len(), 26);
        assert!(dirs.iter().all(|(d, _)| d.iter().any(|v| *v != 0.0)));
        assert!(dirs.iter().all(|(_, name)| !name.is_empty() && !name.contains("  ")));
    }

    #[test]
    fn every_filled_mode_has_a_pipeline_and_the_wireframe_does_not() {
        for mode in DrawMode::ALL {
            assert_eq!(mode.fill_index().is_none(), mode == DrawMode::Wireframe, "{mode:?}");
        }
    }

    #[test]
    fn cycling_the_draw_mode_visits_every_mode_and_comes_back() {
        let mut mode = DrawMode::Shaded;
        let mut seen = vec![mode];
        for _ in 0..DrawMode::ALL.len() - 1 {
            mode = mode.next();
            seen.push(mode);
        }
        assert_eq!(mode.next(), DrawMode::Shaded);
        seen.sort_by_key(|m| m.id());
        seen.dedup();
        assert_eq!(seen.len(), DrawMode::ALL.len());
    }

    /// The ids are what `media.toml` stores, so each must read back.
    #[test]
    fn every_style_reads_back_from_its_id() {
        for mode in DrawMode::ALL {
            assert_eq!(DrawMode::from_id(mode.id()), Some(mode));
        }
        for p in [Projection::Perspective, Projection::Orthographic] {
            assert_eq!(Projection::from_id(p.id()), Some(p));
        }
        assert_eq!(DrawMode::from_id("sparkly"), None);
    }
}
