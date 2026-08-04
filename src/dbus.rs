//! D-Bus service exposing [`Daemon`] as `dev.hyprforge.Displayd1` at
//! `/dev/hyprforge/Displayd` on the session bus.

use crate::daemon::{Daemon, DaemonSignal};
use crate::profile::ExtraOutputPolicy;
use std::str::FromStr;
use std::sync::Arc;
use zbus::object_server::SignalEmitter;

pub const BUS_NAME: &str = "dev.hyprforge.Displayd";
pub const OBJECT_PATH: &str = "/dev/hyprforge/Displayd";

pub struct DisplaydService {
    daemon: Arc<Daemon>,
}

impl DisplaydService {
    pub fn new(daemon: Arc<Daemon>) -> Self {
        DisplaydService { daemon }
    }
}

fn to_zbus_error(e: anyhow::Error) -> zbus::fdo::Error {
    zbus::fdo::Error::Failed(e.to_string())
}

#[zbus::interface(name = "dev.hyprforge.Displayd1")]
impl DisplaydService {
    /// Returns `(id, name, head_count, last_used_rfc3339)` per profile.
    async fn list_profiles(&self) -> Vec<(String, String, u32, String)> {
        self.daemon
            .profiles()
            .await
            .into_iter()
            .map(|p| (p.id, p.name, p.heads.len() as u32, p.last_used))
            .collect()
    }

    async fn get_current_fingerprint(&self) -> zbus::fdo::Result<String> {
        self.daemon.current_fingerprint().map_err(to_zbus_error)
    }

    async fn get_current_layout(&self) -> zbus::fdo::Result<String> {
        self.daemon.current_layout_json().map_err(to_zbus_error)
    }

    async fn apply_profile(&self, profile_id: &str) -> zbus::fdo::Result<()> {
        self.daemon
            .apply_profile(profile_id)
            .await
            .map_err(to_zbus_error)
    }

    async fn rename_profile(&self, profile_id: &str, new_name: &str) -> zbus::fdo::Result<()> {
        self.daemon
            .rename_profile(profile_id, new_name)
            .await
            .map_err(to_zbus_error)
    }

    async fn swap_heads(
        &self,
        profile_id: &str,
        connector_a: &str,
        connector_b: &str,
    ) -> zbus::fdo::Result<()> {
        self.daemon
            .swap_heads(profile_id, connector_a, connector_b)
            .await
            .map_err(to_zbus_error)
    }

    async fn set_extra_output_policy(
        &self,
        profile_id: &str,
        policy: &str,
    ) -> zbus::fdo::Result<()> {
        let policy = ExtraOutputPolicy::from_str(policy)
            .map_err(|_| zbus::fdo::Error::InvalidArgs(format!("unknown policy: {policy}")))?;
        self.daemon
            .set_extra_output_policy(profile_id, policy)
            .await
            .map_err(to_zbus_error)
    }

    #[zbus(property)]
    async fn competing_monitor_rules(&self) -> Vec<String> {
        self.daemon.competing_monitor_rules().await
    }

    #[zbus(signal)]
    pub async fn profile_applied(
        emitter: &SignalEmitter<'_>,
        id: String,
        name: String,
        tier: String,
    ) -> zbus::Result<()>;

    #[zbus(signal)]
    pub async fn new_topology_seen(
        emitter: &SignalEmitter<'_>,
        fingerprint: String,
        summary: String,
    ) -> zbus::Result<()>;
}

impl FromStr for ExtraOutputPolicy {
    type Err = ();

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "extend_right" => Ok(ExtraOutputPolicy::ExtendRight),
            "mirror" => Ok(ExtraOutputPolicy::Mirror),
            "disable" => Ok(ExtraOutputPolicy::Disable),
            _ => Err(()),
        }
    }
}

/// Bridges [`DaemonSignal`]s from `Daemon::run` onto the D-Bus signals
/// above. Runs for the lifetime of the connection.
pub async fn forward_signals(
    connection: zbus::Connection,
    mut signal_rx: tokio::sync::mpsc::UnboundedReceiver<DaemonSignal>,
) {
    let iface_ref = match connection
        .object_server()
        .interface::<_, DisplaydService>(OBJECT_PATH)
        .await
    {
        Ok(r) => r,
        Err(e) => {
            tracing::error!(error = %e, "failed to obtain D-Bus interface reference for signal forwarding");
            return;
        }
    };

    while let Some(signal) = signal_rx.recv().await {
        let result = match signal {
            DaemonSignal::ProfileApplied { id, name, tier } => {
                iface_ref.profile_applied(id, name, tier).await
            }
            DaemonSignal::NewTopologySeen {
                fingerprint,
                summary,
            } => iface_ref.new_topology_seen(fingerprint, summary).await,
            DaemonSignal::ProfileAutoLearned { id, name } => {
                // Auto-learn overwrites a profile's stored layout silently
                // from the daemon's own perspective, but the GUI still
                // benefits from knowing — reuse ProfileApplied's signal
                // shape with a distinct tier label rather than adding a
                // fourth D-Bus signal for what's semantically a variant of
                // "a profile changed".
                iface_ref
                    .profile_applied(id, name, "auto-learned".to_string())
                    .await
            }
        };
        if let Err(e) = result {
            tracing::error!(error = %e, "failed to emit D-Bus signal");
        }
    }
}
