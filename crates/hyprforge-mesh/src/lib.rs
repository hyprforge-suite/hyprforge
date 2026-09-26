//! 3D models — STL, 3MF and OBJ — as something to draw: load one into an
//! indexed, welded [`mesh::Mesh`], and look at it through fstl's camera.
//!
//! The model half of view3d (<https://github.com/adamrpostjr/view3d>),
//! brought into the suite so the photo viewer can open a model the way it
//! opens a picture: from Files, in its folder's order, with ←/→ to the
//! next. view3d is itself a re-implementation of fstl; `LICENSE` beside
//! this crate carries both projects' MIT terms and says which parts are
//! translations of fstl's source.
//!
//! # A leaf, like `hyprforge-image`
//!
//! No iced, no wgpu, no Wayland. The renderer that turns a [`mesh::Mesh`]
//! into pixels lives with the window that draws it (`hyprforge-photos`),
//! because its wgpu must be the exact one iced was built with. What is
//! here — parsing, welding, the camera's matrices, which draw modes exist
//! — is pure enough to test without a GPU, and is what a thumbnailer or a
//! preview pane would ask too.
//!
//! # Fast on purpose
//!
//! view3d's reason to exist was that opening a mesh should be instant:
//! binary STL is memory-mapped and parsed in parallel, and the weld is a
//! parallel sort. That is kept as it was. What a load *allocates* is worth
//! knowing, per the suite's rule about testing the resource: a binary STL
//! of `n` triangles holds `3n` positions (36 bytes a triangle) and `3n`
//! colours while welding, then settles at roughly 16 bytes a vertex and 12
//! bytes a triangle — about 60MB for two million triangles once welded.
//!
//! | Question | Module |
//! |---|---|
//! | What format is this file, and what is in it? | [`loader`] |
//! | What does a loaded model hold? | [`mesh`] |
//! | Where is the eye, and how does a drag turn it? | [`camera`] |
//! | Which ways of drawing it exist? | [`style`] |

pub mod camera;
pub mod loader;
pub mod mesh;
pub mod style;
pub mod thumbnail;

pub use loader::{detect, load, Format};
pub use mesh::Mesh;
