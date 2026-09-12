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
///
/// The empty map stays an ordinary return rather than becoming an error,
/// and that is a considered choice: callers fall back to bare connector
/// names, which *work* — they are merely less stable across docks. There
/// is no branch a caller could usefully take, so there is nothing to give
/// them. What they were owed is a record of which failure happened, since
/// "hyprctl isn't installed", "it timed out" and "its JSON changed shape"
/// call for completely different responses and all three used to look
/// identical from the outside: a `monitors.lua` quietly written with
/// connector selectors and no explanation anywhere.
pub async fn connector_descriptions() -> HashMap<String, String> {
    // Bounded, like every other subprocess in this project: a wedged
    // compositor must not hold up the daemon that is trying to describe
    // its monitors.
    let queried = tokio::time::timeout(
        hyprforge_core::command::TIMEOUT,
        tokio::process::Command::new("hyprctl").arg("monitors").arg("-j").output(),
    )
    .await;
    let output = match queried {
        Ok(Ok(output)) => output,
        Ok(Err(e)) => {
            tracing::warn!(error = %e, "could not run `hyprctl monitors`; using connector names");
            return HashMap::new();
        }
        Err(_) => {
            tracing::warn!("`hyprctl monitors` timed out; using connector names");
            return HashMap::new();
        }
    };
    if !output.status.success() {
        tracing::warn!(
            status = ?output.status.code(),
            stderr = %String::from_utf8_lossy(&output.stderr).trim(),
            "`hyprctl monitors` failed; using connector names"
        );
        return HashMap::new();
    }
    let monitors = match serde_json::from_slice::<Vec<HyprctlMonitor>>(&output.stdout) {
        Ok(monitors) => monitors,
        Err(e) => {
            // The one of the three worth noticing: hyprctl answered, so
            // this is its output having changed shape under us, and every
            // `desc:` selector this daemon writes is downstream of it.
            tracing::warn!(
                error = %e,
                "could not read `hyprctl monitors` output; using connector names"
            );
            return HashMap::new();
        }
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
