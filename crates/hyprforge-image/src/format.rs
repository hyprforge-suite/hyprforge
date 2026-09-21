//! Which formats *this build* can decode.
//!
//! Asked of `image` rather than kept as a list here, because the answer
//! is decided by cargo features in the workspace root and a second list
//! would be a second opinion — right until someone switched a feature on
//! and forgot this file.
//!
//! What it is for: a viewer needs to know whether a file is worth opening
//! before it opens it (for the filmstrip, which meets a folder of mixed
//! things), and the `.desktop` file must claim exactly the MIME types the
//! binary can actually open. A `.desktop` that claims `image/webp` while
//! the decoder is absent is a file association that opens a window and
//! shows an error.
//!
//! # The trap in `reading_enabled`
//!
//! `image`'s own `ImageFormat::reading_enabled()` answers `cfg!(feature =
//! "avif")` for AVIF — but that feature is the *encoder* (ravif).
//! Decoding AVIF needs `avif-native` and the system libdav1d. So if
//! anyone ever enables `avif` to write one, this module would start
//! claiming AVIF is readable and the viewer would advertise a MIME type
//! it cannot open. [`decodable_formats`] filters that case out by hand,
//! and [`tests::avif_is_never_reported_as_decodable`] fails if the
//! situation changes.

use std::path::Path;

/// Every format this build can actually decode.
pub fn decodable_formats() -> Vec<image::ImageFormat> {
    image::ImageFormat::all()
        .filter(|f| f.reading_enabled())
        // See the module doc: `reading_enabled` keys AVIF off the
        // encoder feature. Until `avif-native` and libdav1d are a
        // deliberate decision, AVIF is not something this can open.
        .filter(|f| *f != image::ImageFormat::Avif)
        .collect()
}

/// Every MIME type this build can decode, sorted, for the `.desktop`
/// file's `MimeType=` line and for matching against a shared-MIME lookup.
pub fn decodable_mime_types() -> Vec<&'static str> {
    let mut types: Vec<&'static str> =
        decodable_formats().into_iter().map(|f| f.to_mime_type()).collect();
    types.sort_unstable();
    types.dedup();
    types
}

/// Whether this path *looks* like a picture this build can open.
///
/// By extension, and deliberately: this is asked once per entry while
/// building a filmstrip for a folder, and opening every file in a
/// directory to read a magic number turns listing it into as many opens
/// as there are files — the same reasoning
/// `hyprforge_listing::types::EntryKind::classify` gives for classifying
/// by name.
///
/// So it can be wrong, in one direction that matters: a file this says
/// yes to may still fail to decode, and the caller has to be ready for
/// that. It is never the *only* check.
pub fn looks_decodable(path: &Path) -> bool {
    image::ImageFormat::from_path(path).map(|f| decodable_formats().contains(&f)).unwrap_or(false)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The two the suite has always had. If these ever come back false,
    /// something removed a feature from the workspace root and every app
    /// that draws a wallpaper broke with it.
    #[test]
    fn png_and_jpeg_are_always_decodable() {
        assert!(looks_decodable(Path::new("/a/photo.png")));
        assert!(looks_decodable(Path::new("/a/photo.jpg")));
        assert!(looks_decodable(Path::new("/a/photo.JPEG")));
    }

    /// The guard from the module doc. This test is the thing that turns
    /// a silent lie into a red build if someone enables `avif` for
    /// encoding without `avif-native`.
    #[test]
    fn avif_is_never_reported_as_decodable() {
        assert!(!decodable_formats().contains(&image::ImageFormat::Avif));
        assert!(!looks_decodable(Path::new("/a/photo.avif")));
        assert!(!decodable_mime_types().contains(&"image/avif"));
    }

    /// HEIC is not a format `image` supports under any feature, so a
    /// viewer must never claim it — the file manager's own
    /// `EntryKind::classify` calls `.heic` an image, which is what makes
    /// this worth pinning rather than assuming.
    #[test]
    fn heic_is_not_claimed_either() {
        assert!(!looks_decodable(Path::new("/a/photo.heic")));
    }

    #[test]
    fn a_file_that_is_not_a_picture_is_not_claimed() {
        assert!(!looks_decodable(Path::new("/a/notes.txt")));
        assert!(!looks_decodable(Path::new("/a/clip.mp4")));
        assert!(!looks_decodable(Path::new("/a/no-extension")));
    }

    /// What the `.desktop` file's `MimeType=` line is built from — it has
    /// to be non-empty and every entry has to look like a MIME type.
    #[test]
    fn the_mime_list_is_usable_in_a_desktop_entry() {
        let types = decodable_mime_types();
        assert!(types.contains(&"image/png"));
        assert!(types.contains(&"image/jpeg"));
        for t in &types {
            assert!(t.contains('/'), "{t} is not a mime type");
        }
    }
}
