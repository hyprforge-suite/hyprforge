//! Which way up a picture is, according to its EXIF.
//!
//! # Why this crate has to own it
//!
//! iced already applies EXIF orientation — but only on two of its three
//! handle kinds. In `iced_graphics-0.14.0/src/image.rs`, `Handle::Path`
//! and `Handle::Bytes` are decoded with `image::open` / `load_from_memory`
//! and then rotated according to `exif::Tag::Orientation`; `Handle::Rgba`
//! is a straight passthrough of whatever pixels it is handed.
//!
//! Those same two paths decode at **full resolution with no cap**, which
//! is exactly the allocation a viewer cannot afford (see [`crate::budget`]).
//! So the viewer must hand over `Handle::Rgba` — and the moment it does,
//! it inherits the orientation job iced had been doing for it.
//!
//! Half of that is a bug people see immediately: every portrait phone
//! photograph sideways. The other half is worse, because it looks like
//! the first: apply orientation here *and* hand iced a path handle
//! somewhere else, and that picture is rotated twice.

/// How the stored pixels must be transformed to be shown the right way up.
///
/// The eight EXIF orientation values, named for what they do rather than
/// numbered — `Rotate90` is what value 6 means, and nobody remembers that.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Orientation {
    /// Value 1: already upright. Also what a file with no EXIF gets.
    #[default]
    Upright,
    /// Value 2.
    FlipHorizontal,
    /// Value 3.
    Rotate180,
    /// Value 4.
    FlipVertical,
    /// Value 5.
    Transpose,
    /// Value 6 — the common one: a phone held upright.
    Rotate90,
    /// Value 7.
    Transverse,
    /// Value 8.
    Rotate270,
}

impl Orientation {
    /// From the raw EXIF value. Anything outside 1..=8 is
    /// [`Orientation::Upright`] — a corrupt tag must not turn a picture
    /// sideways, and "no opinion" is the safe reading of a value nobody
    /// defined.
    pub fn from_exif(value: u16) -> Orientation {
        match value {
            2 => Orientation::FlipHorizontal,
            3 => Orientation::Rotate180,
            4 => Orientation::FlipVertical,
            5 => Orientation::Transpose,
            6 => Orientation::Rotate90,
            7 => Orientation::Transverse,
            8 => Orientation::Rotate270,
            _ => Orientation::Upright,
        }
    }

    /// Whether applying this swaps width and height.
    ///
    /// The half of orientation that is easy to forget, because it is not
    /// about pixels: a fit-to-window computed on the stored dimensions of
    /// a phone photograph fits the wrong rectangle, and the picture is
    /// letterboxed on the wrong axis before anyone notices it is also
    /// sideways.
    pub fn swaps_axes(self) -> bool {
        matches!(
            self,
            Orientation::Transpose
                | Orientation::Rotate90
                | Orientation::Transverse
                | Orientation::Rotate270
        )
    }

    /// The size a picture presents after this orientation is applied.
    pub fn applied_size(self, width: u32, height: u32) -> (u32, u32) {
        if self.swaps_axes() {
            (height, width)
        } else {
            (width, height)
        }
    }

    /// The equivalent `image` operation, which is what actually moves the
    /// pixels. Delegated rather than hand-rolled: `image::metadata::Orientation` is
    /// the same eight cases, and `DynamicImage::apply_orientation` is
    /// tested by people who do only this.
    pub fn as_image(self) -> image::metadata::Orientation {
        match self {
            Orientation::Upright => image::metadata::Orientation::NoTransforms,
            Orientation::FlipHorizontal => image::metadata::Orientation::FlipHorizontal,
            Orientation::Rotate180 => image::metadata::Orientation::Rotate180,
            Orientation::FlipVertical => image::metadata::Orientation::FlipVertical,
            Orientation::Transpose => image::metadata::Orientation::Rotate90FlipH,
            Orientation::Rotate90 => image::metadata::Orientation::Rotate90,
            Orientation::Transverse => image::metadata::Orientation::Rotate270FlipH,
            Orientation::Rotate270 => image::metadata::Orientation::Rotate270,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The common case, and the one anybody notices: a phone held
    /// upright stores its pixels landscape and tags them 6.
    #[test]
    fn a_portrait_phone_photo_is_tagged_as_a_quarter_turn() {
        let o = Orientation::from_exif(6);
        assert_eq!(o, Orientation::Rotate90);
        assert!(o.swaps_axes());
        assert_eq!(o.applied_size(4032, 3024), (3024, 4032));
    }

    #[test]
    fn a_file_with_no_exif_at_all_is_upright() {
        assert_eq!(Orientation::default(), Orientation::Upright);
        assert_eq!(Orientation::Upright.applied_size(800, 600), (800, 600));
    }

    /// A corrupt tag must not turn a picture sideways. Every value
    /// outside the defined range reads as "no opinion".
    #[test]
    fn a_value_nobody_defined_is_upright_rather_than_a_guess() {
        for value in [0, 9, 42, u16::MAX] {
            assert_eq!(Orientation::from_exif(value), Orientation::Upright, "{value}");
        }
    }

    #[test]
    fn every_defined_value_round_trips_to_a_distinct_transform() {
        let all: Vec<Orientation> = (1..=8).map(Orientation::from_exif).collect();
        for (i, a) in all.iter().enumerate() {
            for b in &all[i + 1..] {
                assert_ne!(a, b, "two EXIF values map to the same transform");
            }
        }
    }

    /// Only the four quarter-turn cases change the shape of the frame.
    #[test]
    fn only_the_quarter_turns_swap_width_and_height() {
        assert!(!Orientation::from_exif(1).swaps_axes());
        assert!(!Orientation::from_exif(2).swaps_axes());
        assert!(!Orientation::from_exif(3).swaps_axes());
        assert!(!Orientation::from_exif(4).swaps_axes());
        assert!(Orientation::from_exif(5).swaps_axes());
        assert!(Orientation::from_exif(6).swaps_axes());
        assert!(Orientation::from_exif(7).swaps_axes());
        assert!(Orientation::from_exif(8).swaps_axes());
    }
}
