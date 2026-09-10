//! The connected monitors' connector names, for settings that name one.
//!
//! Deliberately separate from `hyprforge_windowrules::monitors`, which
//! answers a different question. A window rule refers to a monitor by
//! `desc:BOE 0x0BC9` — an EDID description that survives replugging. The
//! settings here (`cursor:default_monitor`, `input:touchdevice:output`)
//! take the **connector** name, `eDP-2`, because that is what Hyprland's
//! own docs point at (`see hyprctl monitors for names`). Offering the
//! wrong one of the two would be a value Hyprland silently ignores.

use std::process::Command;

/// Connector names of the connected monitors, e.g. `eDP-2`, `DP-3`.
///
/// An empty list means the compositor couldn't be read, which a caller
/// must treat as "can't offer a list" rather than "no monitors".
pub fn connector_names() -> Vec<String> {
    let queried = crate::command::output(
        Command::new("hyprctl").args(["monitors", "-j"]),
        crate::command::TIMEOUT,
    );
    let Ok(out) = queried else {
        return Vec::new();
    };
    let Ok(value) = serde_json::from_slice::<serde_json::Value>(&out.stdout) else {
        return Vec::new();
    };
    value
        .as_array()
        .map(|monitors| {
            monitors
                .iter()
                .filter_map(|m| m.get("name")?.as_str())
                .filter(|n| !n.is_empty())
                .map(str::to_string)
                .collect()
        })
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    #[test]
    fn reading_monitors_never_panics() {
        let _ = super::connector_names();
    }
}
