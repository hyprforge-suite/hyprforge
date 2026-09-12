//! D-Bus service exposing [`Daemon`] as `dev.hyprforge.Displayd1` at
//! `/dev/hyprforge/Displayd` on the session bus.

use crate::daemon::{Daemon, DaemonSignal};
use crate::profile::ExtraOutputPolicy;
use crate::types::Transform;
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

    /// The id of the profile currently in effect, or `""` if none matches.
    /// Empty rather than an error because "nothing matches yet" is a normal
    /// state, not a failure.
    async fn get_current_profile(&self) -> zbus::fdo::Result<String> {
        self.daemon
            .current_profile_id()
            .await
            .map(|id| id.unwrap_or_default())
            .map_err(to_zbus_error)
    }

    async fn get_current_layout(&self) -> zbus::fdo::Result<String> {
        self.daemon.current_layout_json().map_err(to_zbus_error)
    }

    /// `(width, height, refresh_mhz, preferred)` per mode a currently
    /// -connected head supports; empty if `connector_hint` isn't live.
    async fn get_available_modes(
        &self,
        connector_hint: &str,
    ) -> zbus::fdo::Result<Vec<(i32, i32, i32, bool)>> {
        self.daemon
            .available_modes(connector_hint)
            .map_err(to_zbus_error)
    }

    async fn apply_profile(&self, profile_id: &str) -> zbus::fdo::Result<()> {
        self.daemon
            .apply_profile(profile_id)
            .await
            .map_err(to_zbus_error)
    }

    /// Applies `profile_id` provisionally and returns the number of seconds
    /// before it rolls itself back. Call `ConfirmLayout` to keep it, or
    /// `RevertLayout` to undo immediately. Intended for GUIs — a change that
    /// blanks the screen recovers on its own.
    async fn apply_profile_reversible(&self, profile_id: &str) -> zbus::fdo::Result<u32> {
        self.daemon
            .apply_profile_reversible(profile_id)
            .await
            .map_err(to_zbus_error)
    }

    /// Keeps a provisional change. No-op when nothing is pending.
    async fn confirm_layout(&self) -> zbus::fdo::Result<()> {
        self.daemon.confirm_layout().await.map_err(to_zbus_error)
    }

    /// Rolls a provisional change back now rather than waiting out the
    /// countdown. Errors when nothing is pending.
    async fn revert_layout(&self) -> zbus::fdo::Result<()> {
        self.daemon.revert_layout().await.map_err(to_zbus_error)
    }

    async fn rename_profile(&self, profile_id: &str, new_name: &str) -> zbus::fdo::Result<()> {
        self.daemon
            .rename_profile(profile_id, new_name)
            .await
            .map_err(to_zbus_error)
    }

    /// Forgets a profile. For the currently-connected topology the daemon
    /// will auto-learn a fresh profile on the next settle — see
    /// [`Daemon::delete_profile`].
    async fn delete_profile(&self, profile_id: &str) -> zbus::fdo::Result<()> {
        self.daemon
            .delete_profile(profile_id)
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

    #[allow(clippy::too_many_arguments)]
    async fn set_head_geometry(
        &self,
        profile_id: &str,
        connector_hint: &str,
        x: i32,
        y: i32,
        width: i32,
        height: i32,
        refresh_mhz: i32,
        scale: f64,
        transform: &str,
    ) -> zbus::fdo::Result<()> {
        let transform = Transform::from_str(transform)
            .map_err(|_| zbus::fdo::Error::InvalidArgs(format!("unknown transform: {transform}")))?;
        self.daemon
            .set_head_geometry(
                profile_id,
                connector_hint,
                x,
                y,
                width,
                height,
                refresh_mhz,
                scale,
                transform,
            )
            .await
            .map_err(to_zbus_error)
    }

    /// JSON-encoded snapshot of one stored profile, including full head
    /// geometry — used by the Displays module's layout editor.
    async fn get_profile(&self, profile_id: &str) -> zbus::fdo::Result<String> {
        self.daemon
            .get_profile_json(profile_id)
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

    #[zbus(signal)]
    pub async fn revert_pending(emitter: &SignalEmitter<'_>, seconds: u32) -> zbus::Result<()>;

    #[zbus(signal)]
    pub async fn revert_resolved(emitter: &SignalEmitter<'_>, reverted: bool) -> zbus::Result<()>;
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

/// Parses the same variant-name strings `Transform`'s default `Serialize`
/// derive produces (used by `GetProfile`'s JSON), so the GUI can round
/// -trip a value straight from that JSON back into a `SetHeadGeometry`
/// call without a separate string mapping to keep in sync.
impl FromStr for Transform {
    type Err = ();

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "Normal" => Ok(Transform::Normal),
            "Rotate90" => Ok(Transform::Rotate90),
            "Rotate180" => Ok(Transform::Rotate180),
            "Rotate270" => Ok(Transform::Rotate270),
            "Flipped" => Ok(Transform::Flipped),
            "Flipped90" => Ok(Transform::Flipped90),
            "Flipped180" => Ok(Transform::Flipped180),
            "Flipped270" => Ok(Transform::Flipped270),
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
            DaemonSignal::RevertPending { seconds } => iface_ref.revert_pending(seconds).await,
            DaemonSignal::RevertResolved { reverted } => {
                iface_ref.revert_resolved(reverted).await
            }
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

#[cfg(test)]
mod tests {
    use super::*;

    /// Every `Transform`, listed — with a `match` that stops compiling if
    /// the enum grows.
    ///
    /// The list is the part that rots. A variant added to `Transform`
    /// would otherwise leave the round-trip below passing while saying
    /// nothing about the new one, so the match is what keeps the list
    /// honest; it is not decoration.
    fn every_transform() -> Vec<Transform> {
        fn _exhaustive(t: Transform) {
            match t {
                Transform::Normal
                | Transform::Rotate90
                | Transform::Rotate180
                | Transform::Rotate270
                | Transform::Flipped
                | Transform::Flipped90
                | Transform::Flipped180
                | Transform::Flipped270 => {}
            }
        }
        vec![
            Transform::Normal,
            Transform::Rotate90,
            Transform::Rotate180,
            Transform::Rotate270,
            Transform::Flipped,
            Transform::Flipped90,
            Transform::Flipped180,
            Transform::Flipped270,
        ]
    }

    fn every_policy() -> Vec<ExtraOutputPolicy> {
        fn _exhaustive(p: ExtraOutputPolicy) {
            match p {
                ExtraOutputPolicy::ExtendRight
                | ExtraOutputPolicy::Mirror
                | ExtraOutputPolicy::Disable => {}
            }
        }
        vec![
            ExtraOutputPolicy::ExtendRight,
            ExtraOutputPolicy::Mirror,
            ExtraOutputPolicy::Disable,
        ]
    }

    /// The GUI reads a transform out of `GetProfile`'s JSON and hands the
    /// same string straight back to `SetHeadGeometry`. That only works
    /// while the hand-written `FromStr` above agrees with the `Serialize`
    /// derive on `Transform` — two definitions of one name, in different
    /// files, with nothing but this test between them.
    ///
    /// A `#[serde(rename_all = ...)]` added to `Transform` would break
    /// the round-trip without touching a line the compiler objects to,
    /// and the symptom would be a rotation the Displays screen refuses to
    /// save with "unknown transform".
    #[test]
    fn every_transform_parses_back_from_the_json_the_gui_reads_it_from() {
        for transform in every_transform() {
            let json = serde_json::to_string(&transform).expect("a Transform serialises");
            let on_the_wire = json.trim_matches('"');
            assert_eq!(
                Transform::from_str(on_the_wire),
                Ok(transform),
                "GetProfile's JSON calls {transform:?} {on_the_wire:?}, and SetHeadGeometry \
                 cannot read that back — the FromStr in dbus.rs and the Serialize derive on \
                 Transform have stopped agreeing"
            );
        }
    }

    /// Same coupling, other enum: `SetExtraOutputPolicy` takes the name
    /// serde gives the variant, so its `rename_all` and the `FromStr`
    /// have to say the same thing.
    #[test]
    fn every_extra_output_policy_parses_back_from_the_name_serde_gives_it() {
        for policy in every_policy() {
            let json = serde_json::to_string(&policy).expect("a policy serialises");
            let on_the_wire = json.trim_matches('"');
            assert_eq!(
                ExtraOutputPolicy::from_str(on_the_wire),
                Ok(policy),
                "the profile stores {policy:?} as {on_the_wire:?} and the bus cannot parse it"
            );
        }
    }

    /// `Transform` derives `Default = Normal`, so anything that reached
    /// for `unwrap_or_default()` on this parse would quietly un-rotate a
    /// monitor the user had deliberately rotated — a wrong value, not a
    /// refused one. Saying no is the behaviour `SetHeadGeometry` turns
    /// into `InvalidArgs`.
    #[test]
    fn an_unrecognised_transform_is_refused_rather_than_becoming_normal() {
        for bad in ["", "rotate90", "ROTATE90", "Rotate45", "normal", "90"] {
            assert_eq!(
                Transform::from_str(bad),
                Err(()),
                "{bad:?} was accepted as a transform"
            );
        }
    }

    #[test]
    fn an_unrecognised_policy_is_refused_rather_than_becoming_extend_right() {
        for bad in ["", "ExtendRight", "extend-right", "clone", "off"] {
            assert_eq!(
                ExtraOutputPolicy::from_str(bad),
                Err(()),
                "{bad:?} was accepted as an extra-output policy"
            );
        }
    }

    /// The two enums do **not** use the same casing on the bus, and that
    /// is now a contract rather than an oversight: `Transform` reaches
    /// callers through serde's default variant names (`Rotate90`), while
    /// `ExtraOutputPolicy` carries `rename_all = "snake_case"`
    /// (`extend_right`). Making them agree would be tidier and would
    /// break every existing caller of one or the other, so the
    /// inconsistency is pinned where someone tempted to fix it will see
    /// what it costs.
    /// The field names `GetProfile` puts on the bus.
    ///
    /// The Displays screen parses this JSON into its own `ProfileDetail`
    /// and `HeadDetail`, declared in
    /// `hyprforge-settings/src/modules/displays.rs` — deliberately *not*
    /// a dependency on this crate, which would drag Wayland into the
    /// GUI's build. The comment there says the field names "mirror
    /// `hyprforge_displayd::profile::Profile` exactly", and until now
    /// nothing checked that claim.
    ///
    /// Renaming a field here, or dropping the `#[serde(rename = "head")]`
    /// on `Profile::heads`, compiles cleanly on both sides and breaks the
    /// layout editor at runtime. There is a matching test on the other
    /// side of the contract:
    /// `a_profile_detail_parses_the_json_the_daemon_actually_sends`.
    /// Change one and the other fails, which is the whole point of there
    /// being two.
    #[test]
    fn get_profile_names_every_field_the_displays_screen_reads() {
        let profile = crate::profile::Profile {
            id: "abc123".to_string(),
            name: "Desk".to_string(),
            last_used: "2026-09-12T10:00:00Z".to_string(),
            extra_output_policy: ExtraOutputPolicy::ExtendRight,
            head_swaps: vec![("DP-1".to_string(), "DP-2".to_string())],
            heads: vec![crate::profile::HeadRecord {
                make: "Dell".to_string(),
                model: "U2720Q".to_string(),
                serial: "ABC".to_string(),
                connector_hint: "DP-1".to_string(),
                x: 0,
                y: 0,
                width: 3840,
                height: 2160,
                refresh_mhz: 59997,
                scale: 1.5,
                transform: Transform::Normal,
                enabled: true,
            }],
        };
        let json: serde_json::Value =
            serde_json::to_value(&profile).expect("a Profile serialises");

        for field in ["id", "name", "extra_output_policy", "head_swaps", "head"] {
            assert!(
                json.get(field).is_some(),
                "GetProfile no longer sends {field:?}; the Displays screen's ProfileDetail                  still expects it"
            );
        }
        let head = json["head"]
            .get(0)
            .expect("`head` is the array of per-head records");
        for field in [
            "make",
            "model",
            "serial",
            "connector_hint",
            "x",
            "y",
            "width",
            "height",
            "refresh_mhz",
            "scale",
            "transform",
            "enabled",
        ] {
            assert!(
                head.get(field).is_some(),
                "GetProfile's head records no longer carry {field:?}; the Displays screen's                  HeadDetail still expects it"
            );
        }
    }

    /// `GetCurrentLayout` serialises `Head`, and the Displays screen reads
    /// two fields out of it to resolve a `desc:` monitor selector into a
    /// connector. Same duplicated contract as above, smaller surface —
    /// see `LiveHead` in the settings crate.
    #[test]
    fn get_current_layout_names_the_two_fields_that_resolve_a_monitor_selector() {
        let head = crate::types::Head {
            connector: "DP-1".to_string(),
            identity: crate::types::Identity {
                make: "Dell".to_string(),
                model: "U2720Q".to_string(),
                serial: "ABC".to_string(),
            },
            description: "Dell U2720Q (DP-1)".to_string(),
            modes: vec![],
            current_mode: None,
            position: (0, 0),
            transform: Transform::Normal,
            scale: 1.0,
            enabled: true,
        };
        let json: serde_json::Value = serde_json::to_value(&head).expect("a Head serialises");
        for field in ["connector", "description"] {
            assert!(
                json.get(field).is_some(),
                "GetCurrentLayout no longer sends {field:?}; the Displays screen's LiveHead                  needs it to resolve a desc: selector"
            );
        }
    }

    #[test]
    fn the_two_bus_enums_keep_the_casing_their_callers_already_send() {
        assert_eq!(
            serde_json::to_string(&Transform::Rotate90).unwrap(),
            "\"Rotate90\""
        );
        assert_eq!(
            serde_json::to_string(&ExtraOutputPolicy::ExtendRight).unwrap(),
            "\"extend_right\""
        );
    }
}
