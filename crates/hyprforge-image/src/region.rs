//! Part of a picture at a higher resolution than the whole of it may be
//! kept — what a viewer shows once it is zoomed in past its decode.
//!
//! [`crate::decode_to_fit`] keeps a picture at most twice the window's
//! size, which is the budget doing its job: a 36-megapixel photograph is
//! held at 5120 pixels wide on a 2560x1600 display rather than 8001. But
//! at 1:1 the window wants every one of those 8001, and stretching the
//! 5120 to cover them is visibly soft — the blur this module exists to
//! remove.
//!
//! Keeping the whole picture at full size would remove it too, at 144MB
//! retained for as long as it is open. The window cannot show more than
//! its own size at once, though, so this decodes the file and keeps only
//! the [`Region`] the window can see (plus whatever margin the caller
//! asks for), at the density the window draws it at. What is retained
//! stays under the same budget as the fitted picture.
//!
//! # What it costs
//!
//! The transient peak is a whole decode: `image` has no way to decode
//! part of a JPEG or a PNG, so the full picture exists for as long as it
//! takes to cut the region out — the 214MB `budget`'s table measured for
//! that photograph, briefly. The fitted decode already paid exactly that
//! for any window wanting more than half the picture, which a zoomed-in
//! window always does, so this adds a second peak of the same size and
//! no new kind of cost. A caller should ask once a zoom has settled, not
//! per step.
//!
//! The region is cut out in the file's *stored* orientation and only the
//! cut-out is turned, so a quarter-turned phone photograph never has a
//! second full-size copy made just to turn it.

use crate::budget::{Budget, DecodePixels};
use crate::error::ImageError;
use crate::measure::{measure, Measured, SourcePixels};
use crate::orientation::Orientation;
use std::path::Path;

/// A rectangle of a picture in the pixels it *presents* — after its EXIF
/// orientation, the same space as [`Measured::display_size`], which is
/// what a viewer's zoom and pan are measured in.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Region {
    pub x: u32,
    pub y: u32,
    pub width: u32,
    pub height: u32,
}

impl Region {
    /// This region moved and cut to lie inside a picture of `width` by
    /// `height`, at least one pixel each way — a zero-sized crop is an
    /// error rather than a picture, and a caller's rounding at an edge
    /// should not be able to produce one.
    pub fn within(self, width: u32, height: u32) -> Region {
        let (width, height) = (width.max(1), height.max(1));
        let x = self.x.min(width - 1);
        let y = self.y.min(height - 1);
        Region {
            x,
            y,
            width: self.width.clamp(1, width - x),
            height: self.height.clamp(1, height - y),
        }
    }
}

/// Part of a picture, decoded.
#[derive(Clone, PartialEq, Eq)]
pub struct Detail {
    /// RGBA8, row-major, `size.width * size.height * 4` bytes.
    pub pixels: Vec<u8>,
    /// The size of `pixels`, at or under the region's own size.
    pub size: DecodePixels,
    /// Which part of the picture `pixels` shows, in presented pixels —
    /// the request, cut to the picture. A viewer places the detail by
    /// this, so it has to be what was decoded and not what was asked.
    pub region: Region,
    /// What the header said.
    pub measured: Measured,
}

/// Hand-written for the reason [`crate::Decoded`]'s is: sizes, not
/// megabytes of pixels.
impl std::fmt::Debug for Detail {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Detail")
            .field("size", &self.size)
            .field("bytes", &self.pixels.len())
            .field("region", &self.region)
            .finish()
    }
}

/// `region` of the picture at `path`, at `density` output pixels per
/// picture pixel — 1.0 or more is the region at full resolution, which is
/// as far as this goes: past that a renderer enlarges it, and decoding it
/// bigger would only invent pixels.
///
/// The result is kept inside `budget` as well, so a caller asking for a
/// generous margin gets a slightly softer detail rather than an
/// allocation the budget would have refused.
pub fn decode_region(path: &Path, region: Region, density: f32, budget: &Budget) -> Result<Detail, ImageError> {
    let measured = measure(path)?;
    if !budget.allows_source(measured.source) {
        return Err(ImageError::TooLarge {
            path: path.to_path_buf(),
            width: measured.source.width,
            height: measured.source.height,
        });
    }
    let whole = crate::decode::decode_whole(path, &measured, budget)?;
    let cut = cut_out(&whole, measured.orientation, region);
    // The whole picture goes before anything else is allocated: from
    // here on only the region exists.
    drop(whole);
    let (cut, region) = cut;

    let density = if density.is_finite() { density.clamp(0.0, 1.0) } else { 1.0 };
    let wanted = SourcePixels {
        width: ((region.width as f32 * density).round() as u32).clamp(1, region.width),
        height: ((region.height as f32 * density).round() as u32).clamp(1, region.height),
    };
    let target = budget.fit(wanted);
    let cut = if (target.width, target.height) == (cut.width(), cut.height()) {
        cut
    } else {
        // `thumbnail_exact` for the reason `decode` gives: a box average
        // that reads every pixel, and only ever a reduction here.
        cut.thumbnail_exact(target.width, target.height)
    };
    let rgba = cut.into_rgba8();
    Ok(Detail {
        size: DecodePixels { width: rgba.width(), height: rgba.height() },
        pixels: rgba.into_raw(),
        region,
        measured,
    })
}

/// `region` (presented pixels) of a picture stored as `stored` and shown
/// turned by `orientation`, turned the right way up; and the region,
/// cut to the picture. Only the region is copied and only the region is
/// turned.
fn cut_out(
    stored: &image::DynamicImage,
    orientation: Orientation,
    region: Region,
) -> (image::DynamicImage, Region) {
    let (shown_w, shown_h) = orientation.applied_size(stored.width(), stored.height());
    let region = region.within(shown_w, shown_h);
    let at = stored_rect(orientation, stored.width(), stored.height(), region);
    let mut cut = stored.crop_imm(at.x, at.y, at.width, at.height);
    cut.apply_orientation(orientation.as_image());
    (cut, region)
}

/// Where a presented rectangle lies in the stored pixels, for a picture
/// stored `w` by `h`.
///
/// Each case is the inverse of the turn: for `Rotate90` a stored pixel
/// `(sx, sy)` is shown at `(h - 1 - sy, sx)`, so the shown columns
/// `x..x + width` come from the stored rows counted up from the bottom.
/// The test checks every case against turning the whole picture, which
/// is the definition this has to agree with.
fn stored_rect(orientation: Orientation, w: u32, h: u32, r: Region) -> Region {
    let swap = |x, y| Region { x, y, width: r.height, height: r.width };
    match orientation {
        Orientation::Upright => r,
        Orientation::FlipHorizontal => Region { x: w - r.x - r.width, ..r },
        Orientation::Rotate180 => Region { x: w - r.x - r.width, y: h - r.y - r.height, ..r },
        Orientation::FlipVertical => Region { y: h - r.y - r.height, ..r },
        Orientation::Transpose => swap(r.y, r.x),
        Orientation::Rotate90 => swap(r.y, h - r.x - r.width),
        Orientation::Transverse => swap(w - r.y - r.height, h - r.x - r.width),
        Orientation::Rotate270 => swap(w - r.y - r.height, r.x),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::budget::ViewportPixels;

    const ALL: [Orientation; 8] = [
        Orientation::Upright,
        Orientation::FlipHorizontal,
        Orientation::Rotate180,
        Orientation::FlipVertical,
        Orientation::Transpose,
        Orientation::Rotate90,
        Orientation::Transverse,
        Orientation::Rotate270,
    ];

    /// Every pixel different, so a crop from the wrong place cannot
    /// match by accident.
    fn numbered(w: u32, h: u32) -> image::DynamicImage {
        image::DynamicImage::ImageRgba8(image::RgbaImage::from_fn(w, h, |x, y| {
            image::Rgba([x as u8, y as u8, (x * 7 + y * 13) as u8, 255])
        }))
    }

    /// The definition the mapping has to agree with: turn the whole
    /// picture, then cut. Every orientation, regions at the corners and
    /// in the middle, on a picture that is not square — a square one
    /// would let a swapped width and height pass.
    #[test]
    fn cutting_before_turning_matches_turning_before_cutting() {
        let stored = numbered(13, 7);
        for orientation in ALL {
            let mut turned = stored.clone();
            turned.apply_orientation(orientation.as_image());
            let (tw, th) = (turned.width(), turned.height());
            let regions = [
                Region { x: 0, y: 0, width: 3, height: 2 },
                Region { x: tw - 4, y: th - 3, width: 4, height: 3 },
                Region { x: 1, y: 2, width: tw - 2, height: 1 },
                Region { x: 0, y: 0, width: tw, height: th },
            ];
            for region in regions {
                let expected = turned.crop_imm(region.x, region.y, region.width, region.height);
                let (cut, kept) = cut_out(&stored, orientation, region);
                assert_eq!(kept, region);
                assert_eq!(cut.to_rgba8(), expected.to_rgba8(), "{orientation:?} {region:?}");
            }
        }
    }

    #[test]
    fn a_region_past_the_edge_is_cut_to_the_picture_and_never_empty() {
        let r = Region { x: 90, y: 5, width: 50, height: 0 };
        assert_eq!(r.within(100, 10), Region { x: 90, y: 5, width: 10, height: 1 });
        let gone = Region { x: 500, y: 500, width: 10, height: 10 };
        assert_eq!(gone.within(100, 10), Region { x: 99, y: 9, width: 1, height: 1 });
    }

    fn write_png(dir: &Path, w: u32, h: u32) -> std::path::PathBuf {
        let path = dir.join("p.png");
        numbered(w, h).save(&path).unwrap();
        path
    }

    /// The point of it: at full density the region comes back at full
    /// resolution — sharper than the fitted decode of the same picture
    /// under the same budget — and the pixels are the file's own.
    #[test]
    fn a_region_at_full_density_is_the_files_own_pixels() {
        let dir = tempfile::tempdir().unwrap();
        let path = write_png(dir.path(), 240, 120);
        let budget = Budget::for_edge(100);
        let fitted = crate::decode_to_fit(&path, &budget).unwrap();
        assert_eq!(fitted.size.width, 100, "the fitted decode is softer than the file");

        let region = Region { x: 40, y: 30, width: 60, height: 50 };
        let detail = decode_region(&path, region, 1.0, &budget).unwrap();
        assert_eq!(detail.size, DecodePixels { width: 60, height: 50 });
        assert_eq!(detail.region, region);
        let file = numbered(240, 120).crop_imm(40, 30, 60, 50).to_rgba8().into_raw();
        assert_eq!(detail.pixels, file);
    }

    /// Below full density it is scaled to what is drawn, and past 1.0
    /// it is not scaled up.
    #[test]
    fn a_region_is_decoded_at_the_density_it_is_drawn_at_and_never_above_its_own_size() {
        let dir = tempfile::tempdir().unwrap();
        let path = write_png(dir.path(), 240, 120);
        let budget = Budget::for_edge(1000);
        let region = Region { x: 0, y: 0, width: 200, height: 100 };
        assert_eq!(decode_region(&path, region, 0.5, &budget).unwrap().size, DecodePixels { width: 100, height: 50 });
        assert_eq!(decode_region(&path, region, 4.0, &budget).unwrap().size, DecodePixels { width: 200, height: 100 });
    }

    /// The resource: a region asked for with more margin than the budget
    /// allows is kept inside the budget, as the fitted decode is.
    #[test]
    fn what_a_region_keeps_stays_inside_the_budget() {
        let dir = tempfile::tempdir().unwrap();
        let path = write_png(dir.path(), 240, 120);
        let budget = Budget::for_viewport(ViewportPixels { width: 40, height: 20 });
        let region = Region { x: 0, y: 0, width: 240, height: 120 };
        let detail = decode_region(&path, region, 1.0, &budget).unwrap();
        assert!(detail.pixels.len() as u64 <= budget.max_retained_bytes(), "{detail:?}");
        assert_eq!(detail.region, region, "the region is still all of it, only softer");
    }
}
