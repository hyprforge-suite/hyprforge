//! An animated picture — a GIF — played through [`film`](crate::film).
//!
//! Through the film and not an `image::Handle` per frame, for the reason
//! the film exists: `iced_wgpu` uploads anything of 2MB or more on a
//! worker and draws nothing until it lands, so a large GIF swapped
//! handle by handle would show an empty pane, exactly as video did. The
//! film keeps one texture and rewrites it when the serial moves.
//!
//! This holds the frames and which one is up; the window owns the clock.
//! [`Animation::delay`] is how long the current frame stays, and the
//! window turns that into a subscription and calls [`Animation::advance`]
//! when it fires — so an animation off screen, or in a window that
//! stopped asking, costs nothing at all.
//!
//! The film has no blending: it writes what the texture holds. So every
//! frame is flattened onto the theme's backdrop here, once, when the
//! animation is built — a transparent GIF then looks as it would drawn
//! over that surface, rather than showing whatever colour its
//! transparent pixels happen to store (usually black).

use crate::film::FilmProgram;
use hyprforge_video::Frame;
use std::sync::Arc;
use std::time::Duration;

/// Every frame of an animation, and which one is up.
#[derive(Debug, Clone)]
pub struct Animation {
    frames: Vec<(Arc<Frame>, Duration)>,
    current: usize,
    /// Bumped per [`Self::advance`], so the film writes its texture
    /// exactly when the picture changed.
    serial: u64,
    backdrop: [u8; 4],
}

impl Animation {
    /// From frames of one size and how long each stays up, flattened onto
    /// the theme's root surface. `None` for fewer than two frames, which
    /// is a still picture.
    pub fn new(frames: Vec<(Frame, Duration)>) -> Option<Animation> {
        let [r, g, b] = crate::video::backdrop();
        Animation::over(frames, [r, g, b])
    }

    /// [`Self::new`] onto a given colour — the seam a test uses, since the
    /// theme's colour is whoever's machine runs it.
    pub fn over(frames: Vec<(Frame, Duration)>, backdrop: [u8; 3]) -> Option<Animation> {
        if frames.len() < 2 {
            return None;
        }
        let frames = frames
            .into_iter()
            .map(|(mut frame, delay)| {
                flatten(&mut frame.pixels, backdrop);
                (Arc::new(frame), delay)
            })
            .collect();
        let [r, g, b] = backdrop;
        Some(Animation { frames, current: 0, serial: 0, backdrop: [r, g, b, 255] })
    }

    /// How long the frame that is up stays up.
    pub fn delay(&self) -> Duration {
        self.frames[self.current].1
    }

    /// The next frame, round to the first after the last: a GIF loops.
    pub fn advance(&mut self) {
        self.current = (self.current + 1) % self.frames.len();
        self.serial += 1;
    }

    /// Which frame is up, from 0.
    pub fn current(&self) -> usize {
        self.current
    }

    pub fn len(&self) -> usize {
        self.frames.len()
    }

    /// Never true: [`Self::new`] refuses fewer than two frames.
    pub fn is_empty(&self) -> bool {
        self.frames.is_empty()
    }

    /// The film to put in a `shader` widget. Like every film it draws
    /// nothing on iced's software renderer, so a window places it over
    /// the still picture it would otherwise have shown.
    pub fn program(&self) -> FilmProgram {
        FilmProgram { frame: self.frames[self.current].0.clone(), serial: self.serial, backdrop: self.backdrop }
    }
}

/// The same frames — the same allocations, not equal bytes, which would
/// compare megabytes to answer a question a window's message asks — at
/// the same place. Windows derive `PartialEq` on their messages, and an
/// animation rides in one.
impl PartialEq for Animation {
    fn eq(&self, other: &Animation) -> bool {
        self.current == other.current
            && self.serial == other.serial
            && self.backdrop == other.backdrop
            && self.frames.len() == other.frames.len()
            && self.frames.iter().zip(&other.frames).all(|((a, ad), (b, bd))| Arc::ptr_eq(a, b) && ad == bd)
    }
}

/// RGBA over an opaque colour, in place: `alpha * pixel + (1 - alpha) *
/// backdrop`, on the stored bytes. Not in linear light — the same sRGB
/// blend `iced_tiny_skia` does and the still picture next to it gets
/// (CLAUDE.md, "A shared Theme does not give you a shared look").
fn flatten(pixels: &mut [u8], backdrop: [u8; 3]) {
    for px in pixels.chunks_exact_mut(4) {
        let a = u32::from(px[3]);
        if a == 255 {
            continue;
        }
        for (c, under) in px.iter_mut().take(3).zip(backdrop) {
            *c = ((u32::from(*c) * a + u32::from(under) * (255 - a) + 127) / 255) as u8;
        }
        px[3] = 255;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn frame(rgba: [u8; 4]) -> Frame {
        Frame { width: 1, height: 1, pixels: rgba.to_vec() }
    }

    fn ms(n: u64) -> Duration {
        Duration::from_millis(n)
    }

    /// It loops, each frame for its own delay, and every step is a new
    /// serial so the film rewrites its texture.
    #[test]
    fn it_plays_each_frame_for_its_delay_and_loops() {
        let mut animation =
            Animation::over(vec![(frame([1, 0, 0, 255]), ms(50)), (frame([2, 0, 0, 255]), ms(70))], [0; 3]).unwrap();
        assert_eq!((animation.current(), animation.delay()), (0, ms(50)));
        let first = animation.program().serial;
        animation.advance();
        assert_eq!((animation.current(), animation.delay()), (1, ms(70)));
        assert_ne!(animation.program().serial, first);
        animation.advance();
        assert_eq!(animation.current(), 0, "round to the start");
    }

    /// One frame is a still picture; the window shows it as one.
    #[test]
    fn one_frame_is_not_an_animation() {
        assert!(Animation::over(vec![(frame([0, 0, 0, 255]), ms(50))], [0; 3]).is_none());
    }

    /// The film writes what it is given, so transparency has to be gone
    /// before it gets there: clear over the backdrop is the backdrop,
    /// half over it is halfway, opaque is untouched.
    #[test]
    fn transparency_is_flattened_onto_the_backdrop() {
        let animation = Animation::over(
            vec![(frame([0, 0, 0, 0]), ms(50)), (frame([200, 0, 100, 128]), ms(50))],
            [100, 100, 100],
        )
        .unwrap();
        assert_eq!(animation.program().frame.pixels, [100, 100, 100, 255]);
        let mut half = vec![200, 0, 100, 128];
        flatten(&mut half, [100, 100, 100]);
        assert_eq!(half, [150, 50, 100, 255]);
        let mut opaque = vec![9, 8, 7, 255];
        flatten(&mut opaque, [100, 100, 100]);
        assert_eq!(opaque, [9, 8, 7, 255]);
    }
}
