//! Media you can watch and turn, as pieces of a window: a playing video
//! and a 3D model.
//!
//! Media's viewer shows them, and Files' Quick Look shows them too; this
//! crate is so that both are the same player and the same model view
//! rather than two that drift. Each is a pane the window owns — its
//! state, its messages, its widgets — with the window's own chrome drawn
//! around it:
//!
//! - [`video::VideoPane`] plays through libmpv
//!   ([`hyprforge_video::Player`]), shows each frame with [`film`] — one
//!   GPU texture rewritten per frame, because an image handle per frame
//!   is never drawn — and has the bar beneath: play, the clock, the seek
//!   bar, sound.
//! - [`model_pane::ModelPane`] holds a mesh ([`hyprforge_mesh`]) and its
//!   camera, drawn by [`model`]'s renderer on iced's wgpu, with drag to
//!   turn, right-drag to pan and scroll to zoom.
//!
//! Both draw through iced `shader` primitives, so both need iced's GPU
//! renderer; on its software renderer each pane says so in a sentence
//! behind the picture rather than staying blank. Every WGSL shader here
//! is parsed and validated in the tests by naga, the compiler wgpu uses,
//! because a shader wgpu refuses at run time takes the whole window down.

pub mod film;
pub mod model;
pub mod model_pane;
pub mod video;
