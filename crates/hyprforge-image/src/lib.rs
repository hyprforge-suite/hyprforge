//! What a picture is, before anything draws it.
//!
//! Measure it without decoding it, work out what may safely be decoded,
//! decode that much, and turn it the right way up. No iced, no Wayland,
//! no async runtime, nothing Hyprland-shaped — so a viewer, a preview
//! pane and a test without a window can all ask the same questions.
//!
//! # The finding this crate exists for
//!
//! iced decodes an image handle at **full resolution with no cap**, and
//! applies EXIF orientation — but only for two of its three handle kinds.
//! In `iced_graphics-0.14.0/src/image.rs`, `Handle::Path` and
//! `Handle::Bytes` go through `image::open` / `load_from_memory` and are
//! then rotated per `exif::Tag::Orientation`. `Handle::Rgba` is a pure
//! passthrough of the pixels it is given.
//!
//! For a wallpaper or a thumbnail that is fine. For a viewer it is not: a
//! 36-megapixel photograph decodes to 144MB and peaks near 300MB once the
//! renderer has its own premultiplied copy — the allocation profile that
//! already cost this suite a lock screen, where a failure inside
//! `iced_tiny_skia` caches as "no entry" and panics on the *next* frame.
//!
//! So a viewer must hand the renderer `Handle::Rgba` built from pixels it
//! decoded within a budget of its own — and the moment it does, it takes
//! on the orientation work too. Two halves, and getting either one alone
//! is a bug: skip orientation and every portrait phone photograph is
//! sideways; apply it here *and* use a path handle elsewhere and the same
//! picture is turned twice.
//!
//! # Where to start
//!
//! [`measure`](measure()) first, always — it allocates nothing and everything else
//! depends on its answer. Then [`budget`], which is pure arithmetic and
//! the only place a cap lives. Then [`decode::decode_to_fit`].
//!
//! # Honest about what the budget bounds
//!
//! What is *retained* is capped by the viewport: 56MB for a
//! 36-megapixel photograph in a 2560x1600 window, against 137MB
//! uncapped. What is *transient* is the decode's own peak, and for most
//! formats it belongs to the source: `image` decodes a picture whole and
//! this crate then scales it down.
//!
//! That peak was measured rather than assumed, and measuring it changed
//! the code twice. It began at 509MB and became 214MB, because the
//! obvious downscaler turned out to cost more than the decode it
//! followed — the table and the reasoning are in [`budget`], the line
//! itself in [`decode`]. Then a JPEG wanted at under half its size
//! stopped being decoded whole at all: `image` 0.25's JPEG decoder has no
//! DCT scaling and `jpeg-decoder` does, so a grid thumbnail of a phone
//! photograph peaks at 9MB rather than 116MB. That table is in
//! `jpeg.rs`. A window big enough to want more than half the picture
//! still pays the whole decode.
//!
//! It also corrected something this doc used to claim: `image`'s own
//! `Limits` does *not* bound that peak — the measurement is identical
//! with and without it.

pub mod budget;
pub mod camera;
pub mod decode;
pub mod error;
pub mod format;
mod jpeg;
pub mod measure;
pub mod orientation;

pub use budget::{Budget, DecodePixels, ViewportPixels};
pub use camera::Camera;
pub use decode::{decode_to_fit, Decoded};
pub use error::ImageError;
pub use measure::{measure, Measured, SourcePixels};
pub use orientation::Orientation;
