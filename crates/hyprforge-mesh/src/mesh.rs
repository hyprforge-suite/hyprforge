//! Format-independent mesh representation shared by every loader.

use glam::Vec3;
use rayon::prelude::*;
use rustc_hash::FxHashMap;

/// A single mesh vertex: position plus a packed RGBA color.
///
/// Normals are deliberately absent: the fragment shaders derive the face normal
/// from screen-space derivatives (as fstl does), which keeps this at 16 bytes.
#[repr(C)]
#[derive(Copy, Clone, Debug, Default, bytemuck::Pod, bytemuck::Zeroable)]
pub struct Vertex {
    pub pos: [f32; 3],
    /// RGBA, one byte per channel, little-endian (r in the low byte).
    pub color: u32,
}

pub const WHITE: u32 = 0xffff_ffff;

#[derive(Copy, Clone, Debug)]
pub struct Aabb {
    pub min: Vec3,
    pub max: Vec3,
}

impl Default for Aabb {
    fn default() -> Self {
        Self {
            min: Vec3::splat(f32::INFINITY),
            max: Vec3::splat(f32::NEG_INFINITY),
        }
    }
}

impl Aabb {
    pub fn extend(&mut self, p: Vec3) {
        self.min = self.min.min(p);
        self.max = self.max.max(p);
    }

    pub fn is_empty(&self) -> bool {
        self.min.x > self.max.x
    }

    pub fn size(&self) -> Vec3 {
        if self.is_empty() {
            Vec3::ZERO
        } else {
            self.max - self.min
        }
    }
}

#[derive(Default, Debug)]
pub struct Mesh {
    pub verts: Vec<Vertex>,
    pub indices: Vec<u32>,
    pub bounds: Aabb,
    /// True when the source file carried per-triangle/per-vertex colors.
    pub has_colors: bool,
}

impl Mesh {
    pub fn tri_count(&self) -> usize {
        self.indices.len() / 3
    }

    /// Unique line-list indices for wireframe drawing (each edge once).
    pub fn edge_indices(&self) -> Vec<u32> {
        let mut seen = FxHashMap::with_capacity_and_hasher(self.indices.len(), Default::default());
        let mut out = Vec::with_capacity(self.indices.len() * 2);
        for tri in self.indices.chunks_exact(3) {
            for (a, b) in [(tri[0], tri[1]), (tri[1], tri[2]), (tri[2], tri[0])] {
                let key = if a < b { (a, b) } else { (b, a) };
                if seen.insert(key, ()).is_none() {
                    out.push(a);
                    out.push(b);
                }
            }
        }
        out
    }
}

/// Welds identical (position, color) pairs into an indexed mesh. The sort is
/// parallel, which is what keeps multi-million-triangle files fast.
pub fn weld(positions: Vec<[f32; 3]>, colors: Vec<u32>, has_colors: bool) -> Mesh {
    let n = positions.len();
    let key = |i: usize| {
        let p = positions[i];
        ([p[0].to_bits(), p[1].to_bits(), p[2].to_bits()], colors[i])
    };

    let mut order: Vec<u32> = (0..n as u32).collect();
    order.par_sort_unstable_by_key(|&i| key(i as usize));

    let mut remap = vec![0u32; n];
    let mut verts: Vec<Vertex> = Vec::with_capacity(n / 2 + 1);
    let mut prev: Option<([u32; 3], u32)> = None;
    for &old in &order {
        let k = key(old as usize);
        if prev != Some(k) {
            verts.push(Vertex {
                pos: positions[old as usize],
                color: colors[old as usize],
            });
            prev = Some(k);
        }
        remap[old as usize] = verts.len() as u32 - 1;
    }

    // Drop degenerate triangles, which contribute no surface and no normal.
    let mut indices = Vec::with_capacity(n);
    for tri in remap.chunks_exact(3) {
        if tri[0] != tri[1] && tri[1] != tri[2] && tri[0] != tri[2] {
            indices.extend_from_slice(tri);
        }
    }

    let mut bounds = Aabb::default();
    for v in &verts {
        bounds.extend(glam::Vec3::from(v.pos));
    }

    Mesh {
        verts,
        indices,
        bounds,
        has_colors,
    }
}

/// Packs a linear-ish RGB float triple into the vertex color format.
pub fn pack_rgb(r: f32, g: f32, b: f32) -> u32 {
    let q = |v: f32| (v.clamp(0.0, 1.0) * 255.0 + 0.5) as u32;
    q(r) | q(g) << 8 | q(b) << 16 | 0xff00_0000
}
