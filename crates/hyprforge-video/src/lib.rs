//! Videos: playing one inside a window, and finding its thumbnail.
//!
//! Two halves, each resting on a program the desktop already has rather
//! than a decoder of this suite's own:
//!
//! - [`player`] plays through **libmpv**, loaded at run time ([`mpv`]), with
//!   mpv's software renderer drawing frames into memory for the window to
//!   show — sound, every format mpv plays, seeking, hardware decode where
//!   it is safe. A machine without mpv still runs the viewer; videos then
//!   say what is missing.
//! - [`frame`] finds a video's first real frame — not the black one most
//!   videos start on — with **ffmpeg**, for the shared thumbnail cache both
//!   the file manager and the photo viewer read.
//!
//! No iced and no async runtime: frames and playback state arrive through
//! a callback, and the window wraps it in whatever channel it likes.
//!
//! Every `unsafe` in the crate is in [`mpv`], behind safe wrappers.

pub mod frame;
pub mod mpv;
pub mod player;

pub use mpv::MpvError;
pub use player::{Command, Frame, Options, Playback, Player, Update};
