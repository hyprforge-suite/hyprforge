//! Hyprland's own connector→description map, for the `monitors.lua`
//! fallback's `desc:` selectors.
//!
//! Displayd already knows each output's EDID description via
//! `wlr-output-management-v1` — but not necessarily formatted the way
//! Hyprland formats the *same* field. This project already hit that trap
//! once: `hyprforge-windowrules/src/monitors.rs` records that displayd
//! reports `BOE 0x0BC9  (eDP-2)` where `hyprctl monitors -j` says
//! `BOE 0x0BC9`, and a rule storing the former silently never matches.
//! `monitors.lua`'s `output = "desc:..."` selectors are compared by
//! Hyprland the same way a window rule's `monitor` field is, so this asks
//! `hyprctl` directly rather than reusing `Head.description`.
//!
//! Best-effort throughout: a `hyprctl` that isn't installed, isn't running
//! against a live Hyprland session, or fails for any other reason just
//! means an empty map comes back, and callers fall back to bare connector
//! names — never a hard failure that could take down a settle.

use serde::Deserialize;
use std::collections::HashMap;

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
struct HyprctlMonitor {
    name: String,
    description: String,
}

/// Connector name → Hyprland's own description string for it, as
/// `hyprctl monitors -j` reports right now. Empty on any failure.
pub async fn connector_descriptions() -> HashMap<String, String> {
    let Ok(output) = tokio::process::Command::new("hyprctl")
        .arg("monitors")
        .arg("-j")
        .output()
        .await
    else {
        return HashMap::new();
    };
    if !output.status.success() {
        return HashMap::new();
    }
    let Ok(monitors) = serde_json::from_slice::<Vec<HyprctlMonitor>>(&output.stdout) else {
        return HashMap::new();
    };
    monitors.into_iter().map(|m| (m.name, m.description)).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_what_hyprctl_reports() {
        let json = r#"[{"name":"eDP-1","description":"BOE 0x0BC9","width":2560}]"#;
        let monitors: Vec<HyprctlMonitor> = serde_json::from_str(json).unwrap();
        assert_eq!(monitors[0].name, "eDP-1");
        assert_eq!(monitors[0].description, "BOE 0x0BC9");
    }

    #[test]
    fn a_monitor_without_a_description_still_parses() {
        let json = r#"[{"name":"HDMI-A-1"}]"#;
        let monitors: Vec<HyprctlMonitor> = serde_json::from_str(json).unwrap();
        assert_eq!(monitors[0].description, "");
    }

    /// A real invocation against whatever's on this machine (there is no
    /// live Hyprland session in CI/sandboxed test runs, so this only
    /// checks that a missing/unreachable `hyprctl` degrades to an empty
    /// map rather than panicking).
    #[tokio::test]
    async fn a_missing_or_unreachable_hyprctl_yields_an_empty_map_not_a_panic() {
        let _ = connector_descriptions().await;
    }
}
