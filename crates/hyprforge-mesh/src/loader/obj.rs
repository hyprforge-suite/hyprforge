//! OBJ reading via `tobj`, with MTL diffuse colors and an optional Y-up to
//! Z-up correction (OBJ files are conventionally Y-up; STL and 3MF are Z-up).

use anyhow::{bail, Context as _, Result};
use std::path::Path;

use crate::mesh::{pack_rgb, weld, Mesh, WHITE};

pub fn load(path: &Path, y_up_to_z_up: bool) -> Result<(Mesh, Option<String>)> {
    let opts = tobj::LoadOptions {
        triangulate: true,
        single_index: true,
        ignore_points: true,
        ignore_lines: true,
    };
    let (models, materials) =
        tobj::load_obj(path, &opts).with_context(|| format!("reading {}", path.display()))?;

    // A missing or broken .mtl is common and harmless: fall back to no colors.
    let (materials, warning) = match materials {
        Ok(m) => (m, None),
        Err(e) => (
            Vec::new(),
            Some(format!("material library not loaded: {e}")),
        ),
    };

    let palette: Vec<u32> = materials
        .iter()
        .map(|m| match m.diffuse {
            Some([r, g, b]) => pack_rgb(r, g, b),
            None => WHITE,
        })
        .collect();

    let mut positions: Vec<[f32; 3]> = Vec::new();
    let mut colors: Vec<u32> = Vec::new();
    let mut has_colors = false;

    for model in &models {
        let m = &model.mesh;
        let color = m
            .material_id
            .and_then(|id| palette.get(id).copied())
            .unwrap_or(WHITE);
        if color != WHITE {
            has_colors = true;
        }
        for &i in &m.indices {
            let i = i as usize * 3;
            let Some(p) = m.positions.get(i..i + 3) else {
                bail!("OBJ face references vertex {i} past the end of the vertex list");
            };
            positions.push(if y_up_to_z_up {
                // +Y up, -Z forward  ->  +Z up, +Y forward
                [p[0], -p[2], p[1]]
            } else {
                [p[0], p[1], p[2]]
            });
            colors.push(color);
        }
    }

    if positions.is_empty() {
        bail!("OBJ contains no triangles");
    }
    Ok((weld(positions, colors, has_colors), warning))
}
