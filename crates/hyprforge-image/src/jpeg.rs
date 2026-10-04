//! Decoding a JPEG at a fraction of its size, rather than whole and then
//! shrinking it.
//!
//! A JPEG is stored as 8x8 blocks of frequencies, and the inverse
//! transform that turns a block back into pixels can produce 4x4, 2x2 or
//! 1x1 of them instead — the picture at a half, a quarter or an eighth of
//! its size, without the full-size one ever existing. libjpeg calls this
//! DCT scaling; `image` 0.25 decodes JPEGs with `zune-jpeg`, which has no
//! such thing, so a 256-pixel thumbnail of a phone photograph used to
//! decode all 37 million pixels first. `jpeg-decoder` does have it, and
//! this module is the one place it is used.
//!
//! Measured on a real 8160x4590 phone photograph, through
//! `/proc/self/status`'s `VmHWM` in a process that had allocated nothing
//! else — the same instrument as the table in [`crate::budget`]:
//!
//! | asked for | before | after |
//! |---|---|---|
//! | 256 (a grid thumbnail) | 116MB, 124ms | **9MB, 73ms** |
//! | 1024 (the preview pane) | 116MB, 162ms | **18MB, 96ms** |
//! | 1336 (Files' Quick Look card) | 117MB, 177ms | **19MB, 95ms** |
//! | 2560 | 140MB, 253ms | **63MB, 180ms** |
//! | 5120 (Media in a 2560 window, with its 2x zoom headroom) | 214MB, 470ms | 214MB, 458ms — unchanged, see below |
//!
//! A progressive JPEG is the case it helps least, because its
//! coefficients are held for the whole picture until the last scan
//! arrives: a 36-megapixel one went from 323MB to 221MB for a thumbnail,
//! and a 4:2:0 one from 217MB to 114MB. Better, still far from free.
//!
//! The pictures are the same pictures. Against the whole decode shrunk
//! the old way, the phone photograph's thumbnails came out at 36–46dB
//! PSNR with each channel's mean within about one level in 255 — a box
//! average of the full picture and an inverse transform at reduced size
//! are two honest ways of reaching the same small image, not identical
//! arithmetic.
//!
//! # When it is not used
//!
//! Returning `None` hands the file back to the ordinary path in
//! [`crate::decode`], which is never wrong, only bigger:
//!
//! - **When no reduction is possible.** Asking for more than half the
//!   picture's size can only be met at full size, and then there is
//!   nothing to gain from a second decoder — and something to lose,
//!   because the same file would come out of two different decoders
//!   depending on the window size.
//! - **For CMYK and 16-bit greyscale**, which `jpeg-decoder` hands back in
//!   layouts that would need converting by hand, for formats rare enough
//!   that the full decode is the right trade.
//! - **When `jpeg-decoder` refuses the file.** `zune-jpeg` is more
//!   forgiving of damaged files, so a refusal here is a reason to ask it,
//!   not an answer. If the file really is broken, the ordinary path says
//!   so in its own words.

use crate::budget::DecodePixels;
use crate::measure::SourcePixels;
use std::path::Path;

/// `path`, decoded at the smallest of 1/8, 1/4, 1/2 that is still at
/// least `target` — so whatever comes back is only ever scaled *down*
/// afterwards, never up.
///
/// `source` and `target` are in the file's stored orientation, before
/// any EXIF turn: this decodes the blocks as they are stored.
pub(crate) fn decode_scaled(
    path: &Path,
    source: SourcePixels,
    target: DecodePixels,
) -> Option<image::DynamicImage> {
    if !reduces(source, target) {
        return None;
    }
    let file = std::fs::File::open(path).ok()?;
    let mut decoder = jpeg_decoder::Decoder::new(std::io::BufReader::new(file));
    // The same 512MiB ceiling `image::Limits::default()` keeps on the
    // ordinary path (see `Budget::limits`); `jpeg-decoder`'s own default
    // is no ceiling at all. A file whose header claims more is refused
    // here, and then refused again in the ordinary path's own words.
    decoder.set_max_decoding_buffer_size(512 * 1024 * 1024);
    // Saturating, because JPEG's own dimensions are u16 and a target is
    // never larger than the source.
    let ask = |n: u32| u16::try_from(n).unwrap_or(u16::MAX);
    let (width, height) = decoder.scale(ask(target.width), ask(target.height)).ok()?;
    let format = decoder.info()?.pixel_format;
    let pixels = match decoder.decode() {
        Ok(pixels) => pixels,
        Err(e) => {
            tracing::debug!(error = %e, "jpeg-decoder refused a JPEG; decoding it whole instead");
            return None;
        }
    };
    let (width, height) = (u32::from(width), u32::from(height));
    match format {
        jpeg_decoder::PixelFormat::RGB24 => {
            image::RgbImage::from_raw(width, height, pixels).map(image::DynamicImage::ImageRgb8)
        }
        jpeg_decoder::PixelFormat::L8 => {
            image::GrayImage::from_raw(width, height, pixels).map(image::DynamicImage::ImageLuma8)
        }
        jpeg_decoder::PixelFormat::CMYK32 | jpeg_decoder::PixelFormat::L16 => None,
    }
}

/// Whether DCT scaling can make `source` any smaller on the way to
/// `target`: the half-size decode must still cover the target on both
/// edges. `jpeg-decoder` rounds a scaled edge up, and so does this.
fn reduces(source: SourcePixels, target: DecodePixels) -> bool {
    source.width.div_ceil(2) >= target.width && source.height.div_ceil(2) >= target.height
}

#[cfg(test)]
mod tests {
    use super::*;

    fn source(width: u32, height: u32) -> SourcePixels {
        SourcePixels { width, height }
    }

    fn target(width: u32, height: u32) -> DecodePixels {
        DecodePixels { width, height }
    }

    #[test]
    fn a_target_under_half_the_picture_is_worth_scaling_for() {
        assert!(reduces(source(8160, 4590), target(256, 144)));
        assert!(reduces(source(8160, 4590), target(4080, 2295)));
    }

    /// The case where a second decoder would buy nothing and cost a
    /// picture that looks different depending on the window size.
    #[test]
    fn a_target_over_half_the_picture_is_left_to_the_ordinary_decode() {
        assert!(!reduces(source(8160, 4590), target(5120, 2880)));
        assert!(!reduces(source(320, 240), target(320, 240)));
    }

    /// Odd edges round up when halved, the way the decoder does it, so a
    /// 7x5 picture's half is 4x3 — and a 4x3 target fits in it.
    #[test]
    fn an_odd_edge_is_halved_the_way_the_decoder_halves_it() {
        assert!(reduces(source(7, 5), target(4, 3)));
        assert!(!reduces(source(7, 5), target(5, 3)));
    }
}
