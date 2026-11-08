//! OBJ reading via `tobj`, with MTL diffuse colors and an optional Y-up to
//! Z-up correction (OBJ files are conventionally Y-up; STL and 3MF are Z-up).

use anyhow::{bail, Context as _, Result};
use std::fs::File;
use std::io::{self, BufRead, BufReader, Read as _};
use std::path::{Component, Path};

use crate::mesh::{pack_rgb, weld, Mesh, WHITE};

/// The largest OBJ this will read — several times a ten-million-triangle
/// mesh's worth of text.
const MAX_OBJ_BYTES: u64 = 2 << 30;
/// A material library is a few lines per material; this is thousands.
const MAX_MTL_BYTES: u64 = 4 << 20;
/// A line is read whole before it is parsed, so this is the most one
/// allocation can be. A real line is a vertex or a face, under a hundred
/// bytes; a long comment is a few kilobytes.
const MAX_LINE_BYTES: usize = 1 << 20;

pub fn load(path: &Path, y_up_to_z_up: bool) -> Result<(Mesh, Option<String>)> {
    let opts = tobj::LoadOptions {
        triangulate: true,
        single_index: true,
        ignore_points: true,
        ignore_lines: true,
    };
    let file = open_bounded(path, MAX_OBJ_BYTES)
        .with_context(|| format!("reading {}", path.display()))?;
    let folder = path.parent().unwrap_or(Path::new("."));
    // Not `tobj::load_obj`, whose material loader opens whatever path
    // `mtllib` names: `/dev/zero` is a line that never ends, read until
    // memory runs out, and a FIFO is a read that never returns — from a
    // thumbnailer, merely by browsing the folder. A library is read here
    // only from inside the model's own folder, only as a regular file,
    // and only up to a library's size.
    let (models, materials) = tobj::load_obj_buf(&mut LineBounded::new(file), &opts, |lib| {
        let Some(lib) = inside(folder, lib) else {
            return Err(tobj::LoadError::OpenFileFailed);
        };
        let file = open_bounded(&lib, MAX_MTL_BYTES).map_err(|_| tobj::LoadError::OpenFileFailed)?;
        tobj::load_mtl_buf(&mut LineBounded::new(file))
    })
    .with_context(|| format!("reading {}", path.display()))?;

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
    super::ensure_finite(&positions)?;
    Ok((weld(positions, colors, has_colors), warning))
}

/// `lib` resolved against `folder`, or `None` when it names somewhere
/// else: an absolute path, or one that climbs out with `..`.
fn inside(folder: &Path, lib: &Path) -> Option<std::path::PathBuf> {
    lib.components()
        .all(|c| matches!(c, Component::Normal(_) | Component::CurDir))
        .then(|| folder.join(lib))
}

/// Opens `path` if it is a regular file of at most `max` bytes, and reads
/// no more than that from it even if it grows. The size is checked rather
/// than merely truncated at: a file cut short would parse as one that
/// ended there, drawn wrong with nothing said. The type is checked before
/// the open, because opening a FIFO for reading is itself what blocks.
fn open_bounded(path: &Path, max: u64) -> io::Result<io::Take<File>> {
    let meta = std::fs::metadata(path)?;
    if !meta.is_file() {
        return Err(io::Error::new(io::ErrorKind::InvalidInput, "not a regular file"));
    }
    if meta.len() > max {
        return Err(io::Error::new(io::ErrorKind::InvalidData, "too large to open"));
    }
    Ok(File::open(path)?.take(max))
}

/// A reader that fails a line longer than [`MAX_LINE_BYTES`] instead of
/// buffering it: `BufRead::lines` grows its `String` until it finds a
/// newline, so without this one line is as large as the file.
struct LineBounded<R> {
    inner: BufReader<R>,
    /// Bytes handed out since the last newline.
    line: usize,
}

impl<R: io::Read> LineBounded<R> {
    fn new(inner: R) -> Self {
        Self { inner: BufReader::new(inner), line: 0 }
    }
}

impl<R: io::Read> io::Read for LineBounded<R> {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        let available = self.fill_buf()?;
        let n = available.len().min(buf.len());
        buf[..n].copy_from_slice(&available[..n]);
        self.consume(n);
        Ok(n)
    }
}

impl<R: io::Read> BufRead for LineBounded<R> {
    fn fill_buf(&mut self) -> io::Result<&[u8]> {
        if self.line > MAX_LINE_BYTES {
            return Err(io::Error::new(io::ErrorKind::InvalidData, "line too long"));
        }
        self.inner.fill_buf()
    }

    fn consume(&mut self, amt: usize) {
        let consumed = &self.inner.buffer()[..amt];
        self.line = match consumed.iter().rposition(|&b| b == b'\n') {
            Some(i) => amt - i - 1,
            None => self.line + amt,
        };
        self.inner.consume(amt);
    }
}
