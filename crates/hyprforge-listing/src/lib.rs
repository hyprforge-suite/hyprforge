//! What a directory listing is, how it is read, and what order it is in.
//!
//! The layer underneath any app that shows the contents of a folder. The
//! file manager was the first; the image viewer is the second, and it is
//! the reason this is a crate rather than five modules inside that app.
//!
//! # Why a viewer needs this at all
//!
//! Open a folder in the file manager, double-click the third photograph,
//! press Right. The picture you get must be the fourth one *the file
//! manager was showing* — which is a question about sort order, hidden
//! files and directories-first, not about images. A viewer that answered
//! it independently would agree with the file manager right up until
//! somebody sorted by date, and for a folder of photographs that is the
//! common case rather than the edge case.
//!
//! [`Order`] is that answer as one value, and the file manager writes it
//! into `files.toml` as part of its own preferences. A second app reads
//! it and never writes it.
//!
//! # A leaf
//!
//! No iced, no toml, no async runtime, nothing that knows about
//! Hyprland. The model above [`backend::FsBackend`] is pure, so a test
//! can assert on a listing of ten thousand entries without a disk, and
//! `mock` exposes the fake backend that makes that possible from another
//! crate.

pub mod backend;
pub mod filter;
pub mod order;
pub mod sort;
pub mod types;
pub mod users;

pub use backend::{FsBackend, StdBackend};
pub use order::Order;
pub use sort::{SortColumn, SortDirection};
pub use types::{Entry, EntryKind, EntrySize, FilesError, ItemCount};
