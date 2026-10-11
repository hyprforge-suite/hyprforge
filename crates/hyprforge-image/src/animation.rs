//! An animated GIF's frames, inside the same budget as a still.
//!
//! [`decode_to_fit`](crate::decode_to_fit) hands back frame 0 of a GIF,
//! because that is what `image`'s `ImageReader::decode` does — which is
//! why every animation in the suite stood still. This asks the GIF
//! decoder for every frame instead, each already composited onto the
//! canvas by `image` (disposal and transparency handled there), and
//! scales each one down to the size the budget allows as it arrives.
//!
//! # What the budget bounds here
//!
//! A still keeps one picture; an animation keeps all of them, so the cap
//! is on the *sum*: every retained frame together stays under
//! [`Budget::max_retained_bytes`], the same number a still of this
//! viewport may keep. An animation that would not fit at the budget's
//! size is decoded again, smaller, at a size where it does — one extra
//! pass, paid only by animations that big, and never a frame kept that
//! the budget did not allow. Past [`MAX_FRAMES`] it is not decoded as an
//! animation at all and the caller shows its first frame, as before.
//!
//! What is transient is one frame at the source's size, which the
//! decoder composites into before it is scaled and dropped.
//!
//! # What is not here
//!
//! Animated WebP and APNG. `image` decodes both frame by frame too, and
//! they would slot in beside GIF, but no one has asked and the GIF case
//! is the one every screenshot tool and chat app produces. And loop
//! counts: GIF's "play N times" extension is not exposed by `image`, so
//! everything loops forever — which is what a loop count of zero, by far
//! the most common one, asks for anyway.

use crate::budget::{Budget, DecodePixels};
use crate::error::ImageError;
use crate::measure::{measure, Measured};
use image::AnimationDecoder;
use std::path::Path;
use std::time::Duration;

/// The most frames this will keep. A longer animation is shown still.
///
/// Not a memory guard — the budget is that — but a bound on decoding
/// time: each frame is composited at the source's size, and a two-hour
/// GIF someone made of a film would otherwise be decoded in full before
/// the first frame could be shown.
pub const MAX_FRAMES: usize = 1000;

/// One frame and how long it stays up.
#[derive(Clone, PartialEq, Eq)]
pub struct Frame {
    /// RGBA8, row-major, [`Animation::size`] by [`Animation::size`].
    pub pixels: Vec<u8>,
    pub delay: Duration,
}

/// Hand-written for the reason `Decoded`'s is: pixels in a log line are
/// noise.
impl std::fmt::Debug for Frame {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Frame").field("bytes", &self.pixels.len()).field("delay", &self.delay).finish()
    }
}

/// Every frame of an animation, all the same size.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Animation {
    pub frames: Vec<Frame>,
    /// The size of every frame's pixels.
    pub size: DecodePixels,
    /// What the header said, as [`crate::Decoded::measured`] keeps it.
    pub measured: Measured,
}

impl Animation {
    /// What every frame together keeps, in bytes.
    pub fn retained_bytes(&self) -> u64 {
        self.frames.iter().map(|f| f.pixels.len() as u64).sum()
    }
}

/// How long a frame with this stored delay is shown.
///
/// A GIF delay of 0 or 1 hundredths of a second is not played as written
/// by any browser: such files were made for browsers that ignored it,
/// and played literally they spin as fast as the screen redraws.
/// Firefox and Chromium both show those at 100ms, and so does this, so a
/// GIF looks here as it does everywhere else it is seen.
pub fn shown_for(stored: Duration) -> Duration {
    if stored <= Duration::from_millis(10) {
        Duration::from_millis(100)
    } else {
        stored
    }
}

/// Decodes every frame of the animation at `path`, inside `budget`.
///
/// `Ok(None)` for anything that is not an animation: a format other than
/// GIF, a GIF of a single frame, or one longer than [`MAX_FRAMES`]. The
/// caller shows it with [`decode_to_fit`](crate::decode_to_fit) as it
/// always did.
pub fn decode_animation(path: &Path, budget: &Budget) -> Result<Option<Animation>, ImageError> {
    let measured = measure(path)?;
    if measured.format != image::ImageFormat::Gif {
        return Ok(None);
    }
    if !budget.allows_source(measured.source) {
        return Err(ImageError::TooLarge {
            path: path.to_path_buf(),
            width: measured.source.width,
            height: measured.source.height,
        });
    }

    let cap = budget.max_retained_bytes();
    let first = budget.fit(measured.source);
    match decode_frames(path, budget, first, cap)? {
        Pass::Kept(frames) => Ok(animation(frames, first, measured)),
        Pass::TooMany => Ok(None),
        Pass::OverBudget(count) => {
            // Small enough that `count` frames fit under the cap.
            let smaller = shrink_to_fit(first, count, cap);
            match decode_frames(path, budget, smaller, cap)? {
                Pass::Kept(frames) => Ok(animation(frames, smaller, measured)),
                // The file changed between the passes, or the arithmetic
                // is wrong; either way, show it still rather than keep
                // more than the budget allows.
                Pass::TooMany | Pass::OverBudget(_) => Ok(None),
            }
        }
    }
}

fn animation(frames: Vec<Frame>, size: DecodePixels, measured: Measured) -> Option<Animation> {
    (frames.len() > 1).then_some(Animation { frames, size, measured })
}

enum Pass {
    Kept(Vec<Frame>),
    /// The frames would not fit at this size; how many there are.
    OverBudget(usize),
    TooMany,
}

fn decode_frames(path: &Path, budget: &Budget, size: DecodePixels, cap: u64) -> Result<Pass, ImageError> {
    let undecodable = |source| ImageError::Undecodable { path: path.to_path_buf(), source };
    let file = std::fs::File::open(path)
        .map_err(|source| ImageError::Unreadable { path: path.to_path_buf(), source })?;
    let mut decoder = image::codecs::gif::GifDecoder::new(std::io::BufReader::new(file)).map_err(undecodable)?;
    image::ImageDecoder::set_limits(&mut decoder, budget.limits()).map_err(undecodable)?;

    let per_frame = u64::from(size.width) * u64::from(size.height) * 4;
    let mut kept = Vec::new();
    let mut count = 0usize;
    for frame in decoder.into_frames() {
        let frame = frame.map_err(undecodable)?;
        count += 1;
        if count > MAX_FRAMES {
            return Ok(Pass::TooMany);
        }
        // Once over, keep counting and stop keeping: the second pass
        // needs the whole count to choose its size.
        if (count as u64) * per_frame > cap {
            kept.clear();
            continue;
        }
        let (numer, denom) = frame.delay().numer_denom_ms();
        let delay = shown_for(Duration::from_micros(u64::from(numer) * 1000 / u64::from(denom.max(1))));
        let buffer = frame.into_buffer();
        let pixels = if buffer.width() == size.width && buffer.height() == size.height {
            buffer.into_raw()
        } else {
            // A box average straight to the small buffer, for the memory
            // reason `decode` gives for `thumbnail_exact`.
            image::imageops::thumbnail(&buffer, size.width, size.height).into_raw()
        };
        kept.push(Frame { pixels, delay });
    }
    if (count as u64) * per_frame > cap {
        Ok(Pass::OverBudget(count))
    } else {
        Ok(Pass::Kept(kept))
    }
}

/// `size`, shrunk evenly so that `count` frames of it fit in `cap`.
fn shrink_to_fit(size: DecodePixels, count: usize, cap: u64) -> DecodePixels {
    let per_frame = cap / (count.max(1) as u64);
    let area = f64::from(size.width) * f64::from(size.height);
    let ratio = ((per_frame as f64 / 4.0) / area).sqrt().min(1.0);
    // Floor, then `max(1)`: rounding up could land one byte over the cap,
    // and a zero edge is an error rather than a picture.
    // `ratio` is at most 1: the budget never scales up, and neither does
    // this.
    DecodePixels {
        width: ((f64::from(size.width) * ratio).floor() as u32).max(1),
        height: ((f64::from(size.height) * ratio).floor() as u32).max(1),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use image::codecs::gif::GifEncoder;
    use image::{Delay, Rgba, RgbaImage};

    /// A GIF of `colours.len()` frames, `w`x`h`, each one solid.
    fn gif(dir: &Path, w: u32, h: u32, colours: &[[u8; 4]], delay_ms: u32) -> std::path::PathBuf {
        let path = dir.join("anim.gif");
        let file = std::fs::File::create(&path).unwrap();
        let mut encoder = GifEncoder::new(file);
        let frames = colours.iter().map(|c| {
            image::Frame::from_parts(RgbaImage::from_pixel(w, h, Rgba(*c)), 0, 0, Delay::from_numer_denom_ms(delay_ms, 1))
        });
        encoder.encode_frames(frames).unwrap();
        drop(encoder);
        path
    }

    const RED: [u8; 4] = [255, 0, 0, 255];
    const BLUE: [u8; 4] = [0, 0, 255, 255];

    /// The whole point: every frame, in order, with its delay — not frame
    /// 0 alone, which is what a still decode returns.
    #[test]
    fn every_frame_comes_back_in_order_with_its_delay() {
        let dir = tempfile::tempdir().unwrap();
        let path = gif(dir.path(), 8, 4, &[RED, BLUE, RED], 50);
        let animation = decode_animation(&path, &Budget::for_edge(64)).unwrap().unwrap();
        assert_eq!(animation.frames.len(), 3);
        assert_eq!(animation.size, DecodePixels { width: 8, height: 4 });
        assert_eq!(&animation.frames[0].pixels[..4], &RED);
        assert_eq!(&animation.frames[1].pixels[..4], &BLUE);
        assert!(animation.frames.iter().all(|f| f.delay == Duration::from_millis(50)));
    }

    /// One frame is a picture, not an animation: the caller's still path
    /// shows it, as it always has.
    #[test]
    fn a_single_frame_gif_is_not_an_animation() {
        let dir = tempfile::tempdir().unwrap();
        let path = gif(dir.path(), 4, 4, &[RED], 50);
        assert_eq!(decode_animation(&path, &Budget::for_edge(64)).unwrap(), None);
    }

    #[test]
    fn a_format_other_than_gif_is_not_an_animation() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("still.png");
        RgbaImage::from_pixel(4, 4, Rgba(RED)).save(&path).unwrap();
        assert_eq!(decode_animation(&path, &Budget::for_edge(64)).unwrap(), None);
    }

    /// Every frame is scaled to the budget's size, as a still would be.
    #[test]
    fn frames_are_scaled_to_the_budget() {
        let dir = tempfile::tempdir().unwrap();
        let path = gif(dir.path(), 64, 32, &[RED, BLUE], 50);
        let animation = decode_animation(&path, &Budget::for_edge(16)).unwrap().unwrap();
        assert_eq!(animation.size, DecodePixels { width: 16, height: 8 });
        assert!(animation.frames.iter().all(|f| f.pixels.len() == 16 * 8 * 4));
    }

    /// The headline claim: all the frames together keep no more than one
    /// still under the same budget may. Too many to fit at full size
    /// means smaller frames, never more bytes.
    #[test]
    fn all_the_frames_together_stay_inside_what_one_still_may_keep() {
        let dir = tempfile::tempdir().unwrap();
        let colours: Vec<[u8; 4]> = (0..10).map(|i| if i % 2 == 0 { RED } else { BLUE }).collect();
        let path = gif(dir.path(), 32, 32, &colours, 50);
        let budget = Budget::for_edge(32);
        let animation = decode_animation(&path, &budget).unwrap().unwrap();
        assert_eq!(animation.frames.len(), 10, "nothing dropped to make it fit");
        assert!(animation.retained_bytes() <= budget.max_retained_bytes(), "{}", animation.retained_bytes());
        assert!(animation.size.width < 32, "shrunk rather than over budget: {:?}", animation.size);
        assert_eq!(animation.size.width, animation.size.height, "and still square");
    }

    /// The browsers' rule, so a GIF made for them plays at their speed.
    #[test]
    fn a_zero_or_tiny_delay_is_shown_for_a_tenth_of_a_second() {
        assert_eq!(shown_for(Duration::ZERO), Duration::from_millis(100));
        assert_eq!(shown_for(Duration::from_millis(10)), Duration::from_millis(100));
        assert_eq!(shown_for(Duration::from_millis(20)), Duration::from_millis(20));
    }
}
