//! How many pixels it is safe to keep, and how that number is reached.
//!
//! The one place a cap lives, and pure arithmetic — every test here takes
//! numbers and touches no file.
//!
//! # Why a viewer's rule is the opposite of a thumbnail's
//!
//! `hyprforge-clipmenu` refuses any image over 16 megapixels and shows a
//! text row instead, which is right for a clipboard popup: the picture is
//! a convenience and the popup has other rows to draw. A viewer may not
//! refuse. Showing the photograph *is* the app, and "that image is too
//! big" is not an answer anyone accepts from the program they opened it
//! with.
//!
//! So the rule inverts: decode to fit the **window**, not the file. A
//! 36-megapixel photograph on a 2560x1600 display has no business
//! becoming a 144MB buffer, because the screen cannot show more than
//! four million of those pixels at once. What is retained is bounded by
//! the viewport and stays bounded no matter what the camera produced.
//!
//! # What this does and does not buy
//!
//! Honest about its own limits, because a budget that is believed to do
//! more than it does is worse than none:
//!
//! - **Retained** memory is capped. That is the allocation that lives for
//!   as long as the picture is on screen, and the one a cache multiplies.
//! - **Transient** memory is not, and cannot be here: `image` 0.25
//!   exposes no DCT-scaled decode, so a JPEG is decoded whole and then
//!   scaled down. The peak is a property of the source, not of this
//!   budget.
//!
//! # The measured numbers
//!
//! A 36-megapixel JPEG (8001x4501, 2.8MB on disk — a file no larger than
//! an email attachment) shown in a 2560x1600 viewport, measured through
//! `/proc/self/status`'s `VmHWM` in a process that had allocated nothing
//! else:
//!
//! | | peak | retained |
//! |---|---|---|
//! | decoded and scaled with `resize_exact` | 509MB | 56MB |
//! | decoded with no scaling at all | 252MB | 137MB |
//! | decoded and scaled with `thumbnail_exact` | **214MB** | 56MB |
//!
//! Two things in that table were worth the measuring. The scaling step
//! cost more than the decode it followed — `resize_exact` works through
//! floating-point intermediates, and on a 36-megapixel source those are
//! larger than the picture. And the cheap integer path is not merely
//! cheaper than the expensive one, it is cheaper than *not scaling*,
//! because it never materialises the full-size RGBA buffer on the way.
//!
//! `decode` uses the third row. See its note on why that is not a
//! quality compromise at this ratio.
//!
//! What is left is a 214MB peak on a machine with any amount of memory,
//! and the lock screen's lesson is that a peak is not free just because
//! it is brief. Bringing it lower needs a decoder that can scale while
//! decoding, which is a change of dependency rather than of arithmetic.

use crate::measure::SourcePixels;

/// The size something will be decoded to — always at or under the
/// source, never larger. A viewer showing a 64x64 icon full-screen
/// scales it up when *drawing*; decoding it big would allocate a buffer
/// full of invented pixels.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DecodePixels {
    pub width: u32,
    pub height: u32,
}

impl DecodePixels {
    /// Bytes an RGBA8 buffer of this size occupies, as `u64` because the
    /// arithmetic overflows `u32` well before the sizes do.
    pub fn rgba_bytes(self) -> u64 {
        u64::from(self.width) * u64::from(self.height) * 4
    }
}

/// The size of the area a picture is being shown in, in **physical**
/// pixels — the buffer's unit, not the widget tree's.
///
/// Named rather than a bare `(u32, u32)` because this is the one place
/// logical and physical coordinates meet, and mixing them is the failure
/// CLAUDE.md records twice: a 1.6-scale output makes a logical window
/// 1.6x smaller than the buffer it draws into, so a budget computed from
/// logical pixels would decode a picture visibly soft on exactly the
/// displays where it matters most.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ViewportPixels {
    pub width: u32,
    pub height: u32,
}

impl ViewportPixels {
    /// From a logical size and the output's scale factor. The conversion
    /// is here, once, so no caller has to remember which way it goes.
    pub fn from_logical(width: f32, height: f32, scale: f32) -> ViewportPixels {
        ViewportPixels {
            width: (width * scale).ceil().max(1.0) as u32,
            height: (height * scale).ceil().max(1.0) as u32,
        }
    }
}

/// How much of a picture may be kept in memory at once.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Budget {
    /// The longest edge a retained decode may have.
    max_edge: u32,
    /// An absolute ceiling on the *source* dimensions, used to refuse a
    /// file before decoding rather than after.
    max_source_edge: u32,
}

/// How much bigger than the viewport a decode is allowed to be.
///
/// Not 1.0. Zooming in is the second thing anyone does in a viewer, and
/// re-decoding on every zoom step would make it stutter. At 2x a picture
/// can be examined at twice the window's resolution before anything has
/// to be read again, for four times the bytes of a fit-to-window decode —
/// on a 2560x1600 display that is 4096x2560 kept, about 42MB, which is a
/// tenth of what one uncapped 36-megapixel photograph was costing.
const ZOOM_HEADROOM: f32 = 2.0;

/// The largest source this will decode at all, on any edge.
///
/// Not a judgement about photographs — 65536 is past every camera — but a
/// guard against a file whose header claims a size no allocator can
/// serve. `image`'s own `Limits` enforces it during decode, before the
/// allocation, which is the only place it can be enforced usefully.
const MAX_SOURCE_EDGE: u32 = 65_536;

impl Budget {
    /// The budget for showing a picture in a given viewport.
    pub fn for_viewport(viewport: ViewportPixels) -> Budget {
        let longest = viewport.width.max(viewport.height).max(1);
        Budget {
            max_edge: ((longest as f32) * ZOOM_HEADROOM).ceil() as u32,
            max_source_edge: MAX_SOURCE_EDGE,
        }
    }

    /// A budget with an explicit edge, for a thumbnail or a filmstrip
    /// frame, where the size wanted is known exactly.
    pub fn for_edge(max_edge: u32) -> Budget {
        Budget { max_edge: max_edge.max(1), max_source_edge: MAX_SOURCE_EDGE }
    }

    /// What `source` should be decoded to under this budget.
    ///
    /// Aspect ratio is preserved, and a picture already inside the budget
    /// is decoded at its own size — never scaled up, which would allocate
    /// a buffer of invented pixels.
    pub fn fit(&self, source: SourcePixels) -> DecodePixels {
        let (w, h) = (source.width.max(1), source.height.max(1));
        let longest = w.max(h);
        if longest <= self.max_edge {
            return DecodePixels { width: w, height: h };
        }
        let ratio = f64::from(self.max_edge) / f64::from(longest);
        DecodePixels {
            // `max(1)`: a panorama 20000x1 pixels scaled by ratio would
            // round its short edge to zero, and a zero-sized decode is an
            // error rather than a picture.
            width: ((f64::from(w) * ratio).round() as u32).max(1),
            height: ((f64::from(h) * ratio).round() as u32).max(1),
        }
    }

    /// Whether a source this size may be decoded at all.
    pub fn allows_source(&self, source: SourcePixels) -> bool {
        source.width <= self.max_source_edge && source.height <= self.max_source_edge
    }

    /// The limits handed to `image`'s decoder, which refuse an absurd
    /// file *before* it allocates rather than after.
    ///
    /// Built on `Limits::default()` rather than `no_limits()`, which
    /// keeps that default's 512MiB ceiling on decoder allocation.
    ///
    /// Worth being precise about what that ceiling does, because it is
    /// easy to credit it with more: measured on a 36-megapixel JPEG, the
    /// peak is **the same with it and without it** (509MB either way
    /// before the fix in `decode`). It is not what bounds the transient
    /// cost, and the near-match between 509MB and 512MiB is coincidence —
    /// the sort of suspiciously round number CLAUDE.md warns is usually
    /// somebody else's limit, which here it was not.
    ///
    /// What it does buy is a refusal for a file whose header claims a
    /// size no allocator can serve, alongside the two edge caps. That is
    /// worth keeping; it is simply not a defence against a large ordinary
    /// photograph.
    pub fn limits(&self) -> image::Limits {
        let mut limits = image::Limits::default();
        limits.max_image_width = Some(self.max_source_edge);
        limits.max_image_height = Some(self.max_source_edge);
        limits
    }

    /// The most an image decoded under this budget can retain, in bytes.
    /// What a cache multiplies, and what a test asserts on.
    pub fn max_retained_bytes(&self) -> u64 {
        let edge = u64::from(self.max_edge);
        edge * edge * 4
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn source(width: u32, height: u32) -> SourcePixels {
        SourcePixels { width, height }
    }

    /// The headline claim of this module, in the units people think in:
    /// a 36-megapixel photograph — the one that peaked at 296MB in the
    /// lock screen — is never retained at full size.
    #[test]
    fn a_thirty_six_megapixel_photograph_is_never_retained_at_full_size() {
        let budget = Budget::for_viewport(ViewportPixels { width: 2560, height: 1600 });
        let fitted = budget.fit(source(8001, 4501));
        assert!(fitted.width < 8001, "{fitted:?}");
        // 144MB at full size; a small fraction of it kept.
        assert!(fitted.rgba_bytes() < 60 * 1024 * 1024, "{} bytes", fitted.rgba_bytes());
        assert!(fitted.rgba_bytes() <= budget.max_retained_bytes());
    }

    #[test]
    fn fitting_preserves_the_shape_of_the_picture() {
        let budget = Budget::for_edge(1000);
        let fitted = budget.fit(source(4000, 2000));
        assert_eq!(fitted, DecodePixels { width: 1000, height: 500 });
    }

    /// Decoding a small picture large would allocate a buffer of pixels
    /// that were never in the file.
    #[test]
    fn a_picture_smaller_than_the_budget_is_never_scaled_up() {
        let budget = Budget::for_edge(4000);
        assert_eq!(budget.fit(source(64, 64)), DecodePixels { width: 64, height: 64 });
    }

    /// A panorama's short edge rounds towards zero, and a zero-sized
    /// decode is an error rather than a picture.
    #[test]
    fn an_extreme_panorama_keeps_at_least_one_pixel_on_its_short_edge() {
        let budget = Budget::for_edge(100);
        let fitted = budget.fit(source(20_000, 1));
        assert_eq!(fitted.width, 100);
        assert_eq!(fitted.height, 1);
    }

    /// The budget is a function of the window, not of the file — which is
    /// the whole design. Two very different photographs shown in the same
    /// window retain the same amount.
    #[test]
    fn what_is_kept_depends_on_the_window_and_not_on_the_file() {
        let budget = Budget::for_viewport(ViewportPixels { width: 1920, height: 1080 });
        let huge = budget.fit(source(12_000, 8_000)).rgba_bytes();
        let large = budget.fit(source(9_000, 6_000)).rgba_bytes();
        assert_eq!(huge, large);
    }

    /// A scaled output decodes more, because its buffer really is bigger.
    /// Computing this from logical pixels is what would make a picture
    /// soft on exactly the displays where it shows most.
    #[test]
    fn a_scaled_display_gets_a_bigger_budget_than_its_logical_size() {
        let logical = ViewportPixels::from_logical(1600.0, 1000.0, 1.0);
        let scaled = ViewportPixels::from_logical(1600.0, 1000.0, 1.6);
        assert_eq!(scaled.width, 2560);
        assert!(
            Budget::for_viewport(scaled).max_retained_bytes()
                > Budget::for_viewport(logical).max_retained_bytes()
        );
    }

    #[test]
    fn a_file_claiming_an_impossible_size_is_refused_before_decoding() {
        let budget = Budget::for_edge(2000);
        assert!(budget.allows_source(source(8001, 4501)));
        assert!(!budget.allows_source(source(200_000, 4)));
    }

    #[test]
    fn a_zero_sized_viewport_still_yields_a_usable_budget() {
        let budget = Budget::for_viewport(ViewportPixels { width: 0, height: 0 });
        let fitted = budget.fit(source(4000, 3000));
        assert!(fitted.width >= 1 && fitted.height >= 1);
    }
}
