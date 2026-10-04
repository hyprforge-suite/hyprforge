//! The model layer of the Files app: what a directory listing is, how it
//! is read, sorted and filtered, and what gets remembered between
//! launches.
//!
//! And the browser view itself: [`browser::Browser`], the
//! `update`/`view` state both the app window and the portal's open/save
//! dialog render — see that module's doc for the `Mode` seam that makes
//! sharing it possible. Everything below it stays pure model code,
//! testable without a window: `prefs` here, and `sort`, `filter` and
//! `backend`, which now live in `hyprforge-listing` (see below).

// `backend`, `filter`, `sort`, `types` and `users` moved down into
// `hyprforge-listing` when the image viewer needed them: paging through a
// folder in the order the file manager was showing is a question about
// listings, not about pictures, and the viewer has no use for a browser
// widget.
//
// Re-exported under the names they already had, so nothing in this crate,
// in the app, or in the portal's dialog changed by a line. A path like
// `crate::types::Entry` still resolves, and still to the same type.
pub use hyprforge_listing::{backend, filter, sort, types, users};

pub mod action;
pub mod archive;
pub mod browser;
pub mod bulk_rename;
pub mod click;
pub mod clipboard;
pub mod columns;
pub mod config;
pub mod config_edit;
pub mod content;
pub mod density;
pub mod drag;
pub mod drop;
pub mod format;
// The drawn marks moved to the shared UI crate when Settings adopted the
// same design; re-exported so `crate::glyph` still names them here.
pub use hyprforge_ui::glyph;
pub mod icon;
pub mod jump;
pub mod keymap;
pub mod menu;
pub mod naming;
pub mod palette;
pub mod preferences;
pub mod prefs;
pub mod preview;
pub mod properties;
pub mod query;
pub mod reveal;
pub mod search;
pub mod sidebar;
pub mod trash;
pub mod undo;
/// Moved to `hyprforge-paths` when the photo viewer's sidebar needed it
/// too; re-exported under its old name so nothing here had to change.
pub use hyprforge_paths::user_dirs as xdg_user_dirs;

pub use action::{Action, ActionContext, Scope};
pub use backend::{FsBackend, StdBackend};
pub use click::{Click, ClickTracker};
pub use archive::ArchiveFsBackend;
pub use trash::{RoutingBackend, TrashBackend};
pub use browser::{Browser, DialogKind, DirError, DirErrorKind, EntryFilter, LoadState, Message, Mode, Outcome, Selection};
pub use format::{format_size, human_readable_size};
pub use prefs::Prefs;
pub use sidebar::{build as build_sidebar, build_pinned, trash_path, PinnedItem, SidebarItem};
pub use types::{Entry, EntryKind, EntrySize, FilesError, ItemCount};
