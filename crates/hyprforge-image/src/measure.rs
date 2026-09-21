//! How big a picture is, and which way up — without decoding it.
//!
//! Always the first thing done to a file, because every other decision
//! depends on it: whether it may be decoded at all, what it should be
//! decoded *to*, and what shape it will present once its orientation is
//! applied. Decoding to find out how big something is would be the whole
//! problem this crate exists to avoid.
//!
//! # What "without decoding" costs
//!
//! Less than a decode, and not nothing. `image`'s `into_dimensions()`
//! reads a header, but a JPEG decoder in 0.25 buffers the file before it
//! will answer — so this is O(file bytes) of reading rather than O(1), and
//! it is O(1) in *allocation*, which is the part that matters here.
//!
//! The same header-first check `hyprforge-authui`'s wallpaper path and
//! `hyprforge-clipmenu`'s thumbnails already do, for the same reason.

use crate::error::ImageError;
use crate::orientation::Orientation;
use std::path::{Path, PathBuf};

/// The dimensions as stored in the file, before orientation.
///
/// "Source" rather than "size" because after a quarter turn these are not
/// the dimensions the picture presents — see [`Measured::display_size`],
/// and the orientation module's note on why that half is easy to miss.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SourcePixels {
    pub width: u32,
    pub height: u32,
}

impl SourcePixels {
    /// Total pixels, as `u64` — 36 megapixels times four bytes overflows
    /// nothing, but a hostile header multiplied in `u32` would.
    pub fn count(self) -> u64 {
        u64::from(self.width) * u64::from(self.height)
    }
}

/// What a header says about a picture.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Measured {
    pub source: SourcePixels,
    pub orientation: Orientation,
    pub format: image::ImageFormat,
}

impl Measured {
    /// The size the picture presents once its orientation is applied —
    /// what a fit-to-window must be computed against.
    pub fn display_size(&self) -> (u32, u32) {
        self.orientation.applied_size(self.source.width, self.source.height)
    }
}

/// Reads `path`'s header: how big, what format, which way up.
pub fn measure(path: &Path) -> Result<Measured, ImageError> {
    let unreadable = |source| ImageError::Unreadable { path: path.to_path_buf(), source };

    let file = std::fs::File::open(path).map_err(unreadable)?;
    let reader = image::ImageReader::new(std::io::BufReader::new(file))
        .with_guessed_format()
        .map_err(unreadable)?;

    let format = reader.format().ok_or_else(|| ImageError::Undecodable {
        path: path.to_path_buf(),
        source: image::ImageError::Unsupported(
            image::error::ImageFormatHint::Unknown.into(),
        ),
    })?;

    let (width, height) = reader.into_dimensions().map_err(|source| ImageError::Undecodable {
        path: path.to_path_buf(),
        source,
    })?;

    Ok(Measured {
        source: SourcePixels { width, height },
        orientation: read_orientation(path),
        format,
    })
}

/// The EXIF orientation, or [`Orientation::Upright`] when there is none
/// to read.
///
/// Never an error: a picture with no EXIF, a truncated EXIF block or a
/// format that has no concept of it are all simply upright, and failing
/// to *show* a photograph because its metadata was odd would be the wrong
/// trade every time.
fn read_orientation(path: &Path) -> Orientation {
    let Ok(file) = std::fs::File::open(path) else {
        return Orientation::Upright;
    };
    let mut reader = std::io::BufReader::new(file);
    match image::ImageReader::new(&mut reader).with_guessed_format() {
        Ok(r) => match r.into_decoder() {
            Ok(mut decoder) => match image::ImageDecoder::orientation(&mut decoder) {
                Ok(o) => from_image_orientation(o),
                Err(_) => Orientation::Upright,
            },
            Err(_) => Orientation::Upright,
        },
        Err(_) => Orientation::Upright,
    }
}

fn from_image_orientation(o: image::metadata::Orientation) -> Orientation {
    match o {
        image::metadata::Orientation::NoTransforms => Orientation::Upright,
        image::metadata::Orientation::Rotate90 => Orientation::Rotate90,
        image::metadata::Orientation::Rotate180 => Orientation::Rotate180,
        image::metadata::Orientation::Rotate270 => Orientation::Rotate270,
        image::metadata::Orientation::FlipHorizontal => Orientation::FlipHorizontal,
        image::metadata::Orientation::FlipVertical => Orientation::FlipVertical,
        image::metadata::Orientation::Rotate90FlipH => Orientation::Transpose,
        image::metadata::Orientation::Rotate270FlipH => Orientation::Transverse,
    }
}

/// The path, for callers that keep a `Measured` around without one.
pub fn measured_path(path: &Path) -> PathBuf {
    path.to_path_buf()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn write_png(dir: &Path, name: &str, width: u32, height: u32) -> PathBuf {
        let path = dir.join(name);
        image::RgbaImage::from_pixel(width, height, image::Rgba([1, 2, 3, 255]))
            .save(&path)
            .unwrap();
        path
    }

    #[test]
    fn a_header_gives_the_size_without_decoding_the_picture() {
        let dir = tempfile::tempdir().unwrap();
        let path = write_png(dir.path(), "a.png", 640, 480);
        let measured = measure(&path).unwrap();
        assert_eq!(measured.source, SourcePixels { width: 640, height: 480 });
        assert_eq!(measured.format, image::ImageFormat::Png);
        assert_eq!(measured.display_size(), (640, 480));
    }

    /// A file that is not a picture is `Undecodable`, never `Unreadable`:
    /// it read perfectly well, it just is not an image.
    #[test]
    fn a_file_that_is_not_a_picture_is_undecodable_not_unreadable() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("notes.txt");
        std::fs::write(&path, b"this is not a picture").unwrap();
        assert!(matches!(measure(&path), Err(ImageError::Undecodable { .. })));
    }

    #[test]
    fn a_file_that_is_not_there_is_unreadable() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("gone.png");
        assert!(matches!(measure(&path), Err(ImageError::Unreadable { .. })));
    }

    /// A picture with no EXIF is upright rather than an error — the
    /// common case for a screenshot or anything ever edited.
    #[test]
    fn a_picture_with_no_exif_measures_as_upright() {
        let dir = tempfile::tempdir().unwrap();
        let path = write_png(dir.path(), "b.png", 100, 50);
        assert_eq!(measure(&path).unwrap().orientation, Orientation::Upright);
    }

    /// Truncated bytes must not panic or hang: a half-copied file in a
    /// folder being written to is a case a viewer meets in practice.
    #[test]
    fn a_truncated_file_is_reported_rather_than_panicking() {
        let dir = tempfile::tempdir().unwrap();
        let full = write_png(dir.path(), "c.png", 200, 200);
        let bytes = std::fs::read(&full).unwrap();
        let cut = dir.path().join("cut.png");
        std::fs::write(&cut, &bytes[..bytes.len() / 3]).unwrap();
        // Either answer is acceptable — a PNG header may survive the cut
        // and give dimensions. What must not happen is a panic.
        let _ = measure(&cut);
    }
}
