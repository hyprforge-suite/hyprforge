//! The clipboard history: what was copied, and what may be kept.
//!
//! Replaces copyq. Text and images, a history that survives a restart,
//! and a popup that appears where the pointer is.
//!
//! # The rule this crate is built around
//!
//! A clipboard manager writes a history to disk. A password manager puts
//! your password on the clipboard. Without care, using both means your
//! vault ends up in a file in your home directory and neither program
//! ever mentions it.
//!
//! So [`types::Sensitivity`] is checked *before* an offer's bytes are
//! ever requested: a secret is not read, not hashed, not stored and not
//! logged. `Debug` on an entry renders a description rather than the
//! content, for the same reason `hyprforge-authui` hand-writes its own.

pub mod types;

pub use types::{Content, Entry, EntryId, Mime, Sensitivity};
