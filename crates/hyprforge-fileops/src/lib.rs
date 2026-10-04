//! Trash, copy, move and rename, per the freedesktop.org specification
//! where one applies — syscalls and arithmetic, no async runtime, no
//! D-Bus. See `fs` for why it needs to know which filesystem a path lives
//! on before it can trash or move it correctly, `trash` for the spec
//! itself, `ops` for copy/move/rename's step-driven design, and `batch`
//! for renaming a set of things at once — swaps included — all or none.

pub mod batch;
pub mod fs;
pub mod localtime;
pub mod ops;
pub mod percent;
pub mod trash;

pub use fs::{FileStatus, Filesystem, RealFilesystem};
pub use ops::{
    drive, Collision, CollisionDecision, CollisionPolicy, MoveStrategy, OpKind, Operation, OpsError,
    Progress, Report, StepOutcome,
};
pub use trash::{
    delete_permanently, erase, erase_stored, find_stored, home_trash_dir, list, list_per_filesystem, restore, trash, trash_into,
    TrashError, TrashedItem,
};

#[cfg(any(test, feature = "mock"))]
pub use fs::mock::MockFilesystem;
