pub mod mock;
pub mod wlr;

use crate::types::{LayoutPlan, TopologyEvent};
use tokio::sync::mpsc::UnboundedReceiver;

/// The boundary between profile-matching logic and the actual display
/// protocol. Implemented by a real `wlr-output-management-v1` backend and
/// by a mock, so the entire matching/fingerprinting/auto-learn flow is
/// testable without physical monitors.
pub trait OutputBackend: Send + Sync {
    /// Current heads, as of the last settled snapshot.
    fn list_outputs(&self) -> anyhow::Result<Vec<crate::types::Head>>;

    /// Applies `plan`. Implementations must test before applying and treat
    /// a `cancelled` response as transient contention to retry, not a
    /// terminal error — see `backend::wlr` for the real implementation.
    /// Callers must have already called [`LayoutPlan::validate`].
    fn apply_configuration(&self, plan: &LayoutPlan) -> anyhow::Result<()>;

    /// A stream of settled topology snapshots. Each item is a full
    /// snapshot, never a diff.
    fn subscribe(&self) -> UnboundedReceiver<TopologyEvent>;
}
