//! A model's thumbnail, drawn on the CPU.
//!
//! The photo viewer draws a model on screen with wgpu; a thumbnail cannot
//! wait for a window, and asking for a GPU device to draw 128 pixels would
//! be the heaviest way to make the lightest thing. So this is a small
//! z-buffered rasteriser with the same camera ([`Camera`]'s default iso
//! view, fitted) and the same shading as the viewer's Shaded and Material
//! Color modes — fstl's formula, from `scene.wgsl` — so a tile looks like
//! the model you get when you open it.
//!
//! Drawn at twice the size and averaged down, which is all the
//! antialiasing a thumbnail needs, onto a transparent background so it
//! sits on whatever the tile is. A two-million-triangle mesh is two
//! million small triangles at 256 pixels: a projection per vertex and a
//! few pixels per triangle, well under a second — and the result goes in
//! the shared cache, so it is paid once per file.

use crate::camera::Camera;
use crate::mesh::Mesh;
use glam::Vec3;

/// A drawn thumbnail: RGBA rows, transparent where the model is not.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Image {
    pub width: u32,
    pub height: u32,
    pub pixels: Vec<u8>,
}

/// How much bigger the drawing is than the thumbnail, per side.
const SUPERSAMPLE: u32 = 2;

/// Draws `mesh` fitted inside an `edge`-pixel square, from the view the
/// viewer opens on.
pub fn render(mesh: &Mesh, edge: u32) -> Image {
    let edge = edge.max(1);
    let side = edge * SUPERSAMPLE;
    let (w, h) = (side as usize, side as usize);
    let mut camera = Camera::default();
    camera.fit(mesh.bounds.min, mesh.bounds.max, false, true);
    let mvp = camera.mvp(side as f32, side as f32);

    // Every vertex through the viewer's matrix once. `clip` is what the
    // shader calls `ec_pos` — clip-space xyz before the divide, where the
    // three axes are on comparable scales — and is what the face normal is
    // taken from. `screen` is where it lands: x right, y down, z the depth
    // the pipeline would compare (0 near, 1 far).
    let clip: Vec<Vec3> = mesh.verts.iter().map(|v| (mvp * Vec3::from(v.pos).extend(1.0)).truncate()).collect();
    let screen: Vec<Vec3> = mesh
        .verts
        .iter()
        .map(|v| {
            let p = mvp.project_point3(Vec3::from(v.pos));
            Vec3::new((p.x + 1.0) * 0.5 * w as f32, (1.0 - p.y) * 0.5 * h as f32, p.z)
        })
        .collect();

    let mut depth = vec![f32::INFINITY; w * h];
    let mut color = vec![[0u8; 4]; w * h];

    for tri in mesh.indices.chunks_exact(3) {
        let [a, b, c] = [tri[0] as usize, tri[1] as usize, tri[2] as usize];
        let (pa, pb, pc) = (screen[a], screen[b], screen[c]);
        let shade = shade(clip[a], clip[b], clip[c], mesh.has_colors.then(|| mesh.verts[a].color));
        let (minx, maxx) = (pa.x.min(pb.x).min(pc.x).floor().max(0.0), pa.x.max(pb.x).max(pc.x).ceil().min(w as f32 - 1.0));
        let (miny, maxy) = (pa.y.min(pb.y).min(pc.y).floor().max(0.0), pa.y.max(pb.y).max(pc.y).ceil().min(h as f32 - 1.0));
        if minx > maxx || miny > maxy {
            continue;
        }
        let area = edge_fn(pa, pb, pc);
        if area.abs() < 1e-9 {
            continue;
        }
        for y in miny as usize..=maxy as usize {
            for x in minx as usize..=maxx as usize {
                let p = Vec3::new(x as f32 + 0.5, y as f32 + 0.5, 0.0);
                let (w0, w1, w2) = (edge_fn(pb, pc, p) / area, edge_fn(pc, pa, p) / area, edge_fn(pa, pb, p) / area);
                if w0 < 0.0 || w1 < 0.0 || w2 < 0.0 {
                    continue;
                }
                let z = w0 * pa.z + w1 * pb.z + w2 * pc.z;
                let i = y * w + x;
                if z < depth[i] {
                    depth[i] = z;
                    color[i] = shade;
                }
            }
        }
    }

    downsample(&color, side, edge)
}

/// Twice the signed area of the triangle `a b p`, in screen space.
fn edge_fn(a: Vec3, b: Vec3, p: Vec3) -> f32 {
    (b.x - a.x) * (p.y - a.y) - (b.y - a.y) * (p.x - a.x)
}

/// fstl's shading for one face: `fs_shaded`, or `fs_material` when the
/// file carries colours. The normal is the face's in clip space — the
/// one the shader recovers from derivatives of `ec_pos` — turned to face
/// the viewer, since the shader's is always the visible side's.
///
/// Not from screen space: there x and y are in pixels and depth runs
/// 0 to 1, so every face's normal came out pointing straight at the
/// viewer and the whole model drew in one flat shade.
/// `the_faces_are_shaded_by_how_they_face` pins this.
fn shade(a: Vec3, b: Vec3, c: Vec3, material: Option<u32>) -> [u8; 4] {
    let mut n = (b - a).cross(c - a).normalize_or_zero();
    if n.z < 0.0 {
        n = -n;
    }
    let light_a = n.dot(Vec3::Z);
    let light_b = n.dot(Vec3::new(-0.57, -0.57, 0.57));
    let rgb = match material {
        Some(packed) => {
            let base = Vec3::new(
                (packed & 0xff) as f32 / 255.0,
                ((packed >> 8) & 0xff) as f32 / 255.0,
                ((packed >> 16) & 0xff) as f32 / 255.0,
            );
            base * (0.35 + 0.45 * light_a.clamp(0.0, 1.0) + 0.30 * light_b.clamp(0.0, 1.0))
        }
        None => {
            let base3 = Vec3::new(0.99, 0.96, 0.89);
            let base2 = Vec3::new(0.92, 0.91, 0.83);
            let base00 = Vec3::new(0.40, 0.48, 0.51);
            (light_a * base2 + (1.0 - light_a) * base00) * 0.5 + (light_b * base3 + (1.0 - light_b) * base00) * 0.5
        }
    };
    // The viewer's shaders write these values into an sRGB target, which
    // encodes them on the way out; written raw here, every tile came out
    // darker and more saturated than the model it opens to. So the same
    // encoding, by hand.
    let q = |v: f32| (srgb_encode(v.clamp(0.0, 1.0)) * 255.0 + 0.5) as u8;
    [q(rgb.x), q(rgb.y), q(rgb.z), 255]
}

/// Linear light to sRGB, the transfer a `*Srgb` render target applies.
fn srgb_encode(linear: f32) -> f32 {
    if linear <= 0.003_130_8 {
        linear * 12.92
    } else {
        1.055 * linear.powf(1.0 / 2.4) - 0.055
    }
}

/// Averages each `SUPERSAMPLE`-square block into one pixel, alpha
/// included, so an edge half covered comes out half transparent.
fn downsample(color: &[[u8; 4]], side: u32, edge: u32) -> Image {
    let (side, edge, s) = (side as usize, edge as usize, SUPERSAMPLE as usize);
    let mut pixels = Vec::with_capacity(edge * edge * 4);
    for y in 0..edge {
        for x in 0..edge {
            let mut sum = [0u32; 4];
            let mut alpha_weighted = [0u32; 3];
            for dy in 0..s {
                for dx in 0..s {
                    let px = color[(y * s + dy) * side + x * s + dx];
                    for (k, total) in sum.iter_mut().enumerate() {
                        *total += px[k] as u32;
                    }
                    for k in 0..3 {
                        alpha_weighted[k] += px[k] as u32 * px[3] as u32;
                    }
                }
            }
            let alpha = sum[3];
            // Colour weighted by coverage, so a half-covered edge pixel is
            // the model's colour at half opacity, not darkened toward the
            // transparent black around it.
            for weighted in alpha_weighted {
                pixels.push(weighted.checked_div(alpha).unwrap_or(0) as u8);
            }
            pixels.push((alpha / (s * s) as u32) as u8);
        }
    }
    Image { width: edge as u32, height: edge as u32, pixels }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mesh::{weld, WHITE};

    /// A unit-ish cube as loose triangles, welded the way the loaders do.
    fn cube(size: f32, color: u32) -> Mesh {
        let c = |i: usize| -> [f32; 3] {
            [if i & 1 != 0 { size } else { 0.0 }, if i & 2 != 0 { size } else { 0.0 }, if i & 4 != 0 { size } else { 0.0 }]
        };
        let faces = [[0, 2, 3, 1], [4, 5, 7, 6], [0, 1, 5, 4], [2, 6, 7, 3], [0, 4, 6, 2], [1, 3, 7, 5]];
        let mut positions = Vec::new();
        for f in faces {
            for t in [[f[0], f[1], f[2]], [f[0], f[2], f[3]]] {
                for i in t {
                    positions.push(c(i));
                }
            }
        }
        let colors = vec![color; positions.len()];
        weld(positions, colors, color != WHITE)
    }

    fn alpha_at(img: &Image, x: u32, y: u32) -> u8 {
        img.pixels[((y * img.width + x) * 4 + 3) as usize]
    }

    #[test]
    fn a_model_fills_the_middle_and_leaves_the_corners_transparent() {
        let img = render(&cube(10.0, WHITE), 128);
        assert_eq!((img.width, img.height), (128, 128));
        assert_eq!(img.pixels.len(), 128 * 128 * 4);
        assert_eq!(alpha_at(&img, 64, 64), 255, "nothing drawn in the middle");
        assert_eq!(alpha_at(&img, 0, 0), 0, "the background is not transparent");
    }

    /// Faces turned different ways are shaded differently — a cube drawn
    /// in one flat colour would be a hexagon-shaped blob.
    #[test]
    fn the_faces_are_shaded_by_how_they_face() {
        let img = render(&cube(10.0, WHITE), 128);
        let mut colours = std::collections::HashSet::new();
        for p in img.pixels.chunks_exact(4).filter(|p| p[3] == 255) {
            colours.insert([p[0] / 8, p[1] / 8, p[2] / 8]);
        }
        assert!(colours.len() >= 3, "only {} distinct shades", colours.len());
    }

    /// A model that brought its own colours is drawn in them.
    #[test]
    fn a_coloured_model_keeps_its_colour() {
        let red = 0xff00_00ffu32;
        let img = render(&cube(10.0, red), 64);
        let i = ((32 * 64 + 32) * 4) as usize;
        let (r, g, b) = (img.pixels[i], img.pixels[i + 1], img.pixels[i + 2]);
        assert!(r > 80 && g < 30 && b < 30, "{r},{g},{b}");
    }

    /// An empty mesh is a transparent square, not a panic.
    #[test]
    fn an_empty_mesh_draws_nothing() {
        let img = render(&Mesh::default(), 32);
        assert!(img.pixels.chunks_exact(4).all(|p| p[3] == 0));
    }
}
