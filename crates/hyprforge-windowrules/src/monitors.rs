//! The connected monitors, for pinning a workspace to one.
//!
//! Read from `hyprctl` rather than from displayd, even though displayd knows
//! the same hardware. A workspace rule's `monitor` field is compared by
//! Hyprland against its *own* description string, and the two don't format it
//! identically — displayd reports `BOE 0x0BC9  (eDP-2)` where Hyprland says
//! `BOE 0x0BC9`. Offering a description Hyprland won't recognise would be the
//! silent-no-op failure this project keeps running into, so the string has to
//! come from the thing that does the comparing.

use serde::Deserialize;
use std::process::Command;

#[derive(Debug, thiserror::Error)]
pub enum MonitorsError {
    #[error("could not run hyprctl: {0}")]
    Spawn(#[source] std::io::Error),
    #[error("hyprctl monitors failed: {stderr}")]
    Failed { stderr: String },
    #[error("could not parse hyprctl monitors output: {0}")]
    Parse(#[source] serde_json::Error),
}

/// One connected monitor.
#[derive(Debug, Clone, Default, Deserialize, PartialEq)]
#[serde(default)]
pub struct Monitor {
    /// The connector, e.g. `eDP-2`. Assigned in probe order, so it can move
    /// between boots — usable, but not what a stored rule should prefer.
    pub name: String,
    /// EDID-derived, e.g. `BOE 0x0BC9`. Stable across replugs, which is what
    /// makes it the right thing to persist.
    pub description: String,
}

impl Monitor {
    /// How a workspace rule should refer to this monitor.
    ///
    /// `desc:` where there's a description, falling back to the connector
    /// when a monitor reports none — a nameless `desc:` would match nothing.
    pub fn rule_selector(&self) -> String {
        if self.description.trim().is_empty() {
            self.name.clone()
        } else {
            format!("desc:{}", self.description.trim())
        }
    }

    /// What the dropdown shows: the description leads, since that's the
    /// physical panel, with the connector after it for orientation.
    pub fn label(&self) -> String {
        if self.description.trim().is_empty() {
            self.name.clone()
        } else {
            format!("{} ({})", self.description.trim(), self.name)
        }
    }
}

pub fn list_monitors() -> Result<Vec<Monitor>, MonitorsError> {
    let out = Command::new("hyprctl")
        .arg("monitors")
        .arg("-j")
        .output()
        .map_err(MonitorsError::Spawn)?;
    if !out.status.success() {
        return Err(MonitorsError::Failed {
            stderr: String::from_utf8_lossy(&out.stderr).into_owned(),
        });
    }
    parse_monitors(&String::from_utf8_lossy(&out.stdout))
}

pub fn parse_monitors(json: &str) -> Result<Vec<Monitor>, MonitorsError> {
    serde_json::from_str(json).map_err(MonitorsError::Parse)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Shape and spelling taken from this machine's `hyprctl monitors -j`.
    #[test]
    fn parses_hyprctl_output() {
        let json = r#"[{"id":0,"name":"eDP-2","description":"BOE 0x0BC9","width":2560}]"#;
        let monitors = parse_monitors(json).unwrap();
        assert_eq!(monitors[0].name, "eDP-2");
        assert_eq!(monitors[0].description, "BOE 0x0BC9");
    }

    /// The selector is the whole point: it must be the EDID description, not
    /// the connector, or the rule stops working when the connector moves.
    #[test]
    fn the_selector_prefers_the_description() {
        let m = Monitor {
            name: "DP-3".into(),
            description: "GWD ARZOPA".into(),
        };
        assert_eq!(m.rule_selector(), "desc:GWD ARZOPA");
        assert_eq!(m.label(), "GWD ARZOPA (DP-3)");
    }

    /// `desc:` with nothing after it would match no monitor at all.
    #[test]
    fn a_monitor_without_a_description_falls_back_to_its_connector() {
        let m = Monitor { name: "HDMI-A-1".into(), description: "  ".into() };
        assert_eq!(m.rule_selector(), "HDMI-A-1");
        assert_eq!(m.label(), "HDMI-A-1");
    }

    #[test]
    fn unknown_keys_and_missing_keys_both_survive() {
        let monitors = parse_monitors(r#"[{"name":"eDP-2","somethingNew":1}]"#).unwrap();
        assert_eq!(monitors[0].description, "");
    }
}
