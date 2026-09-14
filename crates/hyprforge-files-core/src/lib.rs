//! The model layer of the Files app: what a directory listing is, how it
//! is read, sorted and filtered, and what gets remembered between
//! launches.
//!
//! Phase 1 only — no iced, no widgets. `hyprforge-ui`/`hyprforge-look`
//! and the browser view itself arrive in phase 2, once this layer and
//! its seam onto the real filesystem exist to build on. See this crate's
//! `Cargo.toml` for why `iced` is not a dependency yet.

pub mod backend;
pub mod filter;
pub mod prefs;
pub mod sort;
pub mod types;

pub use backend::{FsBackend, StdBackend};
pub use types::{Entry, EntryKind, FilesError};
