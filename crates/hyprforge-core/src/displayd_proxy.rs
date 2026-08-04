//! Client-side D-Bus proxy for `dev.hyprforge.Displayd1`, shared between
//! `hyprforge-displayctl` and the Settings app's Displays module so the
//! wire contract is defined exactly once.

use zbus::proxy;

#[proxy(
    interface = "dev.hyprforge.Displayd1",
    default_service = "dev.hyprforge.Displayd",
    default_path = "/dev/hyprforge/Displayd"
)]
pub trait Displayd {
    /// Returns `(id, name, head_count, last_used_rfc3339)` per profile.
    fn list_profiles(&self) -> zbus::Result<Vec<(String, String, u32, String)>>;

    fn get_current_fingerprint(&self) -> zbus::Result<String>;

    /// JSON-encoded snapshot of the currently-applied layout.
    fn get_current_layout(&self) -> zbus::Result<String>;

    fn apply_profile(&self, profile_id: &str) -> zbus::Result<()>;

    fn rename_profile(&self, profile_id: &str, new_name: &str) -> zbus::Result<()>;

    fn swap_heads(
        &self,
        profile_id: &str,
        connector_a: &str,
        connector_b: &str,
    ) -> zbus::Result<()>;

    fn set_extra_output_policy(&self, profile_id: &str, policy: &str) -> zbus::Result<()>;

    #[zbus(property)]
    fn competing_monitor_rules(&self) -> zbus::Result<Vec<String>>;

    #[zbus(signal)]
    fn profile_applied(&self, id: String, name: String, tier: String) -> zbus::Result<()>;

    #[zbus(signal)]
    fn new_topology_seen(&self, fingerprint: String, summary: String) -> zbus::Result<()>;
}
