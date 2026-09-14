//! Trash, per the freedesktop.org specification — the only file
//! operation in scope for this crate's first pass.
//!
//! Copy, move and rename are out of scope here and belong to
//! `hyprforge-files-core`; this crate's job is narrower and lower-level:
//! syscalls and arithmetic, no async runtime, no D-Bus. See `fs` for why
//! it needs to know which filesystem a path lives on before it can trash
//! it correctly, and `trash` for the spec itself.

pub mod fs;
mod localtime;
mod percent;
pub mod trash;

pub use fs::{FileStatus, Filesystem, RealFilesystem};
pub use trash::{home_trash_dir, list, list_per_filesystem, restore, trash, trash_into, TrashError, TrashedItem};

#[cfg(any(test, feature = "mock"))]
pub use fs::mock::MockFilesystem;
