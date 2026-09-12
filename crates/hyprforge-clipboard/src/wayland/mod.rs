//! The real backend: watches the compositor's clipboard over
//! `ext-data-control-v1`, falling back to `zwlr-data-control-v1`.
//!
//! Two protocols do this. `ext-data-control-v1` (in `wayland-protocols`,
//! still under `staging` — it has not graduated to stable) is the
//! standardised successor; `zwlr-data-control-v1` (in
//! `wayland-protocols-wlr`) is the older wlroots-only one its own XML
//! now calls deprecated. Hyprland 0.56 on this machine answers
//! `wl-paste --list-types` over data-control, which the `ext` module's
//! doc comment records as the check that was actually run. A compositor
//! that only ever shipped the older protocol still needs to work, so
//! [`connect`] tries `ext` first and falls back to `wlr` rather than
//! choosing one at compile time.
//!
//! `ext.rs` and `wlr.rs` are close to line-for-line copies of each
//! other: the two protocols share requests, events and argument order,
//! and `wayland-client`'s generated types make each an entirely
//! different Rust type even though nothing about the *logic* differs.
//! What is shared lives in `pipe.rs` (the bounded read) and
//! `crate::resolve` (the pure selection and the sensitivity gate) —
//! everything outside those is unavoidably protocol-specific glue.

mod ext;
mod pipe;
mod wlr;

use crate::backend::ClipboardWatcher;
use crate::types::Entry;
use std::sync::{Arc, Mutex};
use tokio::sync::mpsc::{self, UnboundedReceiver, UnboundedSender};

/// The real clipboard watcher: a compositor connection running on its
/// own thread, publishing every recordable copy to whoever subscribed.
pub struct WaylandWatcher {
    subscribers: Arc<Mutex<Vec<UnboundedSender<Entry>>>>,
}

impl WaylandWatcher {
    /// Connects to the compositor on `$WAYLAND_DISPLAY`, preferring
    /// `ext-data-control-v1` and falling back to
    /// `zwlr-data-control-v1`, and spawns the dispatch thread.
    ///
    /// Fails only when *neither* protocol is available — callers should
    /// surface this as a clear, actionable startup error (a compositor
    /// with no clipboard protocol at all is not something to retry
    /// silently against), matching `hyprforge-displayd::backend::wlr`'s
    /// `WlrBackend::connect`.
    pub fn connect() -> anyhow::Result<Self> {
        let subscribers = Arc::new(Mutex::new(Vec::new()));

        match ext::connect(subscribers.clone()) {
            Ok(()) => return Ok(WaylandWatcher { subscribers }),
            Err(e) => {
                tracing::info!(
                    error = %e,
                    "ext-data-control-v1 unavailable; falling back to zwlr-data-control-v1"
                );
            }
        }

        wlr::connect(subscribers.clone())
            .map_err(|e| anyhow::anyhow!("no clipboard protocol available (tried ext-data-control-v1, then zwlr-data-control-v1): {e}"))?;
        Ok(WaylandWatcher { subscribers })
    }
}

impl ClipboardWatcher for WaylandWatcher {
    fn subscribe(&self) -> UnboundedReceiver<Entry> {
        let (tx, rx) = mpsc::unbounded_channel();
        self.subscribers
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .push(tx);
        rx
    }
}
