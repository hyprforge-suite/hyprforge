//! Turning a file into pixels a renderer can take, within a budget.
//!
//! The order is the design, and each step exists because the one before
//! it made it safe:
//!
//! 1. [`crate::measure`](crate::measure()) reads the header. Nothing is allocated.
//! 2. [`crate::budget`] decides whether this may be decoded at all, and
//!    at what size.
//! 3. The decoder runs under `image`'s own `Limits`, so a file whose
//!    header lied is refused while allocating rather than after.
//! 4. The EXIF orientation is applied — **ours to apply**, because the
//!    renderer only does it for handle kinds a viewer cannot use. See
//!    [`crate::orientation`].
//! 5. The result is resized down to the budget and handed over as RGBA8.

use crate::budget::{Budget, DecodePixels};
use crate::error::ImageError;
use crate::measure::{measure, Measured};
use std::path::Path;

/// A decoded picture, ready to become a renderer's handle.
#[derive(Clone, PartialEq, Eq)]
pub struct Decoded {
    /// RGBA8, row-major, `size.width * size.height * 4` bytes.
    pub pixels: Vec<u8>,
    /// The size of `pixels` — what was *kept*, which is at or under both
    /// the budget and the source.
    pub size: DecodePixels,
    /// What the header said, before any of this. Kept so a viewer can
    /// show the picture's real dimensions rather than the decoded ones —
    /// an info panel that reports what it happened to decode would be
    /// lying about the file.
    pub measured: Measured,
}

/// Deliberately hand-written: the derived one would render several
/// megabytes of pixel data into a log line.
///
/// Not the keystroke rule, but the same habit — a `Debug` that dumps a
/// buffer is a `Debug` nobody can use, and someone debugging a viewer is
/// debugging sizes, not pixel values.
impl std::fmt::Debug for Decoded {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Decoded")
            .field("size", &self.size)
            .field("bytes", &self.pixels.len())
            .field("measured", &self.measured)
            .finish()
    }
}

/// Decodes `path` to fit within `budget`.
pub fn decode_to_fit(path: &Path, budget: &Budget) -> Result<Decoded, ImageError> {
    let measured = measure(path)?;

    if !budget.allows_source(measured.source) {
        return Err(ImageError::TooLarge {
            path: path.to_path_buf(),
            width: measured.source.width,
            height: measured.source.height,
        });
    }

    let undecodable = |source| ImageError::Undecodable { path: path.to_path_buf(), source };

    let file = std::fs::File::open(path)
        .map_err(|source| ImageError::Unreadable { path: path.to_path_buf(), source })?;
    let mut reader = image::ImageReader::new(std::io::BufReader::new(file));
    reader.set_format(measured.format);
    reader.limits(budget.limits());
    let mut decoded = reader.decode().map_err(undecodable)?;

    // Orientation before fitting, not after: a quarter turn swaps the
    // axes, so fitting first would fit the wrong rectangle and then turn
    // it — landing at a size that is inside the budget on paper and the
    // wrong shape on screen.
    //
    // `apply_orientation` mutates in place and returns `()`, unlike the
    // `rotate90`-style methods next to it that return a new image.
    decoded.apply_orientation(measured.orientation.as_image());

    let target = budget.fit(crate::measure::SourcePixels {
        width: decoded.width(),
        height: decoded.height(),
    });
    let decoded = if target.width == decoded.width() && target.height == decoded.height() {
        decoded
    } else {
        // `thumbnail_exact`, not `resize_exact`, and this is the single
        // most consequential line in the crate — measured, because it
        // looks like a quality choice and is really a memory one.
        //
        // On the 36-megapixel photograph in `budget`'s table,
        // `resize_exact` peaks at 509MB and this peaks at 214MB. It is
        // cheaper even than not scaling at all (252MB), because it goes
        // straight to the small buffer rather than building a full-size
        // one on the way. `resize_exact` works through floating-point
        // intermediates which, at this source size, are larger than the
        // picture.
        //
        // Not a quality compromise at this ratio: `thumbnail_exact` is a
        // box average that samples every source pixel, where a triangle
        // filter's support is narrow enough to skip most of them and
        // alias. It would be the wrong tool for an *upscale*, which is
        // why `fit` never asks for one.
        decoded.thumbnail_exact(target.width, target.height)
    };

    let rgba = decoded.into_rgba8();
    Ok(Decoded {
        size: DecodePixels { width: rgba.width(), height: rgba.height() },
        pixels: rgba.into_raw(),
        measured,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::budget::ViewportPixels;
    use crate::orientation::Orientation;
    use std::path::PathBuf;

    fn write_png(dir: &Path, name: &str, width: u32, height: u32) -> PathBuf {
        let path = dir.join(name);
        image::RgbaImage::from_pixel(width, height, image::Rgba([10, 20, 30, 255]))
            .save(&path)
            .unwrap();
        path
    }

    /// The claim the whole crate is built around, measured rather than
    /// asserted in prose: a photograph far larger than the screen is
    /// *retained* at a fraction of its full size.
    #[test]
    fn a_picture_larger_than_the_window_is_never_retained_at_full_size() {
        let dir = tempfile::tempdir().unwrap();
        // 4000x3000 rather than a real 36MP file: the arithmetic is the
        // same and the test does not spend a second encoding one.
        let path = write_png(dir.path(), "big.png", 4000, 3000);
        let budget = Budget::for_viewport(ViewportPixels { width: 800, height: 600 });
        let decoded = decode_to_fit(&path, &budget).unwrap();

        assert!(decoded.size.width < 4000, "{:?}", decoded.size);
        assert_eq!(decoded.pixels.len() as u64, decoded.size.rgba_bytes());
        assert!(decoded.pixels.len() as u64 <= budget.max_retained_bytes());
        // And the file's real size is still reported, not the decoded one.
        assert_eq!(decoded.measured.source.width, 4000);
    }

    #[test]
    fn a_picture_that_already_fits_is_decoded_as_it_is() {
        let dir = tempfile::tempdir().unwrap();
        let path = write_png(dir.path(), "small.png", 320, 240);
        let budget = Budget::for_viewport(ViewportPixels { width: 1920, height: 1080 });
        let decoded = decode_to_fit(&path, &budget).unwrap();
        assert_eq!(decoded.size, DecodePixels { width: 320, height: 240 });
    }

    #[test]
    fn the_decoded_buffer_is_exactly_four_bytes_a_pixel() {
        let dir = tempfile::tempdir().unwrap();
        let path = write_png(dir.path(), "rgba.png", 7, 5);
        let budget = Budget::for_edge(1000);
        let decoded = decode_to_fit(&path, &budget).unwrap();
        assert_eq!(decoded.pixels.len(), 7 * 5 * 4);
    }

    /// A `Debug` that printed the buffer would make every log line
    /// useless and every panic message enormous.
    #[test]
    fn debugging_a_decoded_picture_shows_sizes_and_not_pixels() {
        let dir = tempfile::tempdir().unwrap();
        let path = write_png(dir.path(), "d.png", 4, 4);
        let decoded = decode_to_fit(&path, &Budget::for_edge(100)).unwrap();
        let shown = format!("{decoded:?}");
        assert!(shown.contains("bytes"));
        assert!(shown.len() < 400, "{shown}");
    }

    #[test]
    fn a_file_that_is_not_a_picture_is_reported_and_not_decoded() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("x.txt");
        std::fs::write(&path, b"not a picture at all").unwrap();
        assert!(decode_to_fit(&path, &Budget::for_edge(100)).is_err());
    }

    /// A JPEG with a real EXIF orientation tag, built by hand because
    /// `image` can write pixels but not metadata.
    ///
    /// The picture is 32x16 with its top half red and its bottom half
    /// blue, so a quarter turn is unmistakable: rotating clockwise sends
    /// the top row to the right-hand column.
    fn jpeg_with_orientation(dir: &Path, name: &str, exif: Option<u16>) -> PathBuf {
        let mut rgb = image::RgbImage::new(32, 16);
        for (_, y, pixel) in rgb.enumerate_pixels_mut() {
            *pixel = if y < 8 { image::Rgb([255, 0, 0]) } else { image::Rgb([0, 0, 255]) };
        }
        let mut bytes = Vec::new();
        image::DynamicImage::ImageRgb8(rgb)
            .write_to(&mut std::io::Cursor::new(&mut bytes), image::ImageFormat::Jpeg)
            .unwrap();

        if let Some(value) = exif {
            // An APP1 segment spliced in directly after the SOI marker.
            //   FF E1 <len> "Exif\0\0"
            //   TIFF header, big-endian: "MM" 002A <offset to IFD0 = 8>
            //   IFD0: one entry — tag 0x0112 (Orientation), type 3
            //   (SHORT), count 1, value in the high half of the 4-byte
            //   value field, then a null next-IFD offset.
            let mut app1: Vec<u8> = Vec::new();
            app1.extend_from_slice(b"Exif\0\0");
            app1.extend_from_slice(&[0x4D, 0x4D, 0x00, 0x2A, 0x00, 0x00, 0x00, 0x08]);
            app1.extend_from_slice(&[0x00, 0x01]);
            app1.extend_from_slice(&[0x01, 0x12, 0x00, 0x03, 0x00, 0x00, 0x00, 0x01]);
            app1.extend_from_slice(&value.to_be_bytes());
            app1.extend_from_slice(&[0x00, 0x00]);
            app1.extend_from_slice(&[0x00, 0x00, 0x00, 0x00]);

            let len = (app1.len() + 2) as u16;
            let mut spliced = vec![0xFF, 0xD8, 0xFF, 0xE1];
            spliced.extend_from_slice(&len.to_be_bytes());
            spliced.extend_from_slice(&app1);
            spliced.extend_from_slice(&bytes[2..]);
            bytes = spliced;
        }

        let path = dir.join(name);
        std::fs::write(&path, &bytes).unwrap();
        path
    }

    fn pixel_at(decoded: &Decoded, x: u32, y: u32) -> (u8, u8, u8) {
        let i = ((y * decoded.size.width + x) * 4) as usize;
        (decoded.pixels[i], decoded.pixels[i + 1], decoded.pixels[i + 2])
    }

    fn is_reddish((r, _g, b): (u8, u8, u8)) -> bool {
        r > 150 && b < 100
    }

    fn is_bluish((r, _g, b): (u8, u8, u8)) -> bool {
        b > 150 && r < 100
    }

    /// The control, and the reason the test below is worth anything: the
    /// same pixels with no EXIF tag come back unturned. A rig that
    /// cannot fail on purpose is not evidence — CLAUDE.md's rule about
    /// checking the instrument before trusting what it says.
    #[test]
    fn without_an_exif_tag_the_picture_is_not_turned() {
        let dir = tempfile::tempdir().unwrap();
        let path = jpeg_with_orientation(dir.path(), "plain.jpg", None);
        let decoded = decode_to_fit(&path, &Budget::for_edge(1000)).unwrap();

        assert_eq!((decoded.size.width, decoded.size.height), (32, 16));
        assert!(is_reddish(pixel_at(&decoded, 16, 2)), "top should be red");
        assert!(is_bluish(pixel_at(&decoded, 16, 13)), "bottom should be blue");
    }

    /// The headline: a photograph a phone tagged as needing a quarter
    /// turn is shown upright, by us, because the renderer will not do it
    /// for the handle kind a viewer has to use.
    #[test]
    fn a_portrait_photo_is_not_shown_sideways() {
        let dir = tempfile::tempdir().unwrap();
        let path = jpeg_with_orientation(dir.path(), "turned.jpg", Some(6));
        let decoded = decode_to_fit(&path, &Budget::for_edge(1000)).unwrap();

        // Rotated a quarter turn clockwise: the frame stands up...
        assert_eq!((decoded.size.width, decoded.size.height), (16, 32));
        // ...and the row that was along the top is now the right column.
        assert!(is_reddish(pixel_at(&decoded, 13, 16)), "right column should be red");
        assert!(is_bluish(pixel_at(&decoded, 2, 16)), "left column should be blue");
    }

    /// And the size reported for the file is the size it *presents*, so
    /// an info panel and a fit-to-window agree with each other.
    #[test]
    fn a_turned_photo_reports_the_size_it_presents() {
        let dir = tempfile::tempdir().unwrap();
        let path = jpeg_with_orientation(dir.path(), "turned2.jpg", Some(6));
        let measured = crate::measure::measure(&path).unwrap();
        assert_eq!(measured.orientation, Orientation::Rotate90);
        assert_eq!(measured.source, crate::measure::SourcePixels { width: 32, height: 16 });
        assert_eq!(measured.display_size(), (16, 32));
    }

    /// Orientation is applied before fitting, so a quarter-turned
    /// picture comes back in the shape it will be shown in.
    #[test]
    fn a_decoded_picture_reports_the_shape_it_will_be_shown_in() {
        let dir = tempfile::tempdir().unwrap();
        let path = write_png(dir.path(), "wide.png", 400, 200);
        let decoded = decode_to_fit(&path, &Budget::for_edge(1000)).unwrap();
        assert_eq!(decoded.measured.orientation, Orientation::Upright);
        assert_eq!((decoded.size.width, decoded.size.height), decoded.measured.display_size());
    }
}
