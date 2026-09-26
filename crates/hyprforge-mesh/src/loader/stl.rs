//! STL reading: a memory-mapped, parallel binary path and an ASCII fallback.

use anyhow::{bail, Context as _, Result};
use rayon::prelude::*;
use std::path::Path;

use crate::mesh::{pack_rgb, weld, Mesh, WHITE};

const BINARY_HEADER: usize = 80;
const BINARY_RECORD: usize = 50;

pub fn load(path: &Path) -> Result<Mesh> {
    let file = std::fs::File::open(path).with_context(|| format!("opening {}", path.display()))?;
    // SAFETY: as with any mmap, concurrent truncation by another process is UB.
    // Autoreload only ever re-opens the file, so we never keep a stale mapping.
    let data = unsafe { memmap2::Mmap::map(&file) }
        .with_context(|| format!("mapping {}", path.display()))?;

    if is_binary(&data) {
        parse_binary(&data)
    } else {
        parse_ascii(&data)
    }
}

/// Trusts the size arithmetic rather than the "solid" prefix: plenty of binary
/// STLs start with the word "solid" in their header.
fn is_binary(data: &[u8]) -> bool {
    if data.len() < BINARY_HEADER + 4 {
        return false;
    }
    let count = u32::from_le_bytes(data[80..84].try_into().unwrap()) as usize;
    data.len() == BINARY_HEADER + 4 + count * BINARY_RECORD
}

fn parse_binary(data: &[u8]) -> Result<Mesh> {
    let body = &data[BINARY_HEADER + 4..];
    let f32_at = |b: &[u8], off: usize| f32::from_le_bytes(b[off..off + 4].try_into().unwrap());

    // Triangles are fixed-width records, so parsing is embarrassingly parallel.
    let positions: Vec<[f32; 3]> = body
        .par_chunks_exact(BINARY_RECORD)
        .flat_map_iter(|rec| {
            (0..3).map(move |v| {
                [
                    f32_at(rec, 12 + v * 12),
                    f32_at(rec, 12 + v * 12 + 4),
                    f32_at(rec, 12 + v * 12 + 8),
                ]
            })
        })
        .collect();

    // Magics-style per-facet color: bit 15 marks the attribute word as a color,
    // with five bits per channel (blue in the low bits).
    let attrs: Vec<u16> = body
        .chunks_exact(BINARY_RECORD)
        .map(|rec| u16::from_le_bytes(rec[48..50].try_into().unwrap()))
        .collect();
    let has_colors = attrs.iter().any(|a| a & 0x8000 != 0);

    let mut colors = Vec::with_capacity(positions.len());
    for &attr in &attrs {
        let color = if has_colors && attr & 0x8000 != 0 {
            let c5 = |shift: u16| ((attr >> shift) & 0x1f) as f32 / 31.0;
            pack_rgb(c5(10), c5(5), c5(0))
        } else {
            WHITE
        };
        colors.extend_from_slice(&[color; 3]);
    }

    Ok(weld(positions, colors, has_colors))
}

fn parse_ascii(data: &[u8]) -> Result<Mesh> {
    let text = std::str::from_utf8(data).context("STL is neither valid binary nor UTF-8 text")?;

    let mut positions: Vec<[f32; 3]> = Vec::new();
    for line in text.lines() {
        let mut it = line.split_ascii_whitespace();
        if it.next() != Some("vertex") {
            continue;
        }
        let mut p = [0.0f32; 3];
        for slot in &mut p {
            *slot = it
                .next()
                .and_then(|t| t.parse::<f32>().ok())
                .context("malformed vertex in ASCII STL")?;
        }
        positions.push(p);
    }

    if positions.is_empty() || !positions.len().is_multiple_of(3) {
        bail!("ASCII STL contains no complete triangles");
    }
    let colors = vec![WHITE; positions.len()];
    Ok(weld(positions, colors, false))
}
