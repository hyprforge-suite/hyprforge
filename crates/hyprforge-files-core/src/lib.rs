//! The model layer of the Files app: what a directory listing is, how it
//! is read, sorted and filtered, and what gets remembered between
//! launches.
//!
//! Phase 2 adds the browser view itself: [`browser::Browser`], the
//! `update`/`view` state both the app window and the portal's open/save
//! dialog render — see that module's doc for the `Mode` seam that makes
//! sharing it possible. Everything below it (`sort`, `filter`, `prefs`,
//! `backend`) stays exactly what phase 1 built: pure model code, testable
//! without a window.

pub mod backend;
pub mod browser;
pub mod density;
pub mod filter;
pub mod format;
pub mod glyph;
pub mod icon;
pub mod keymap;
pub mod prefs;
pub mod sidebar;
pub mod sort;
pub mod types;
pub mod xdg_user_dirs;

pub use backend::{FsBackend, StdBackend};
pub use browser::{Browser, DialogKind, DirError, DirErrorKind, LoadState, Message, Mode, Outcome, Selection};
pub use format::human_readable_size;
pub use prefs::Prefs;
pub use sidebar::{build as build_sidebar, build_pinned, trash_path, PinnedItem, SidebarItem};
pub use types::{Entry, EntryKind, FilesError};
