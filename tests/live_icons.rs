//! Do this daemon's icon names resolve against a **real** icon theme?
//!
//! The unit tests check each name against an allow-list of names this
//! project believes are standard. That catches a typo and nothing else:
//! a name can be perfectly standard, perfectly spelled, and still absent
//! from the theme the user actually has.
//!
//! When that happens the bar draws a blank gap. No error is raised at any
//! layer — not by the daemon, not by the host, not by the icon loader —
//! so the only way to find out is to resolve the names against the themes
//! installed on a real machine. That is what this does.
//!
//! It found four: GNOME's `night-light-symbolic` and the `dialog-*-symbolic`
//! fallbacks exist only in Adwaita, and `bluetooth-disabled` is shipped by
//! neither Adwaita nor Breeze. On a Breeze-derived theme all four were
//! invisible, one of them in code that had already shipped.
//!
//! Read-only: it looks at files, and touches nothing.

use std::collections::HashSet;
use std::path::{Path, PathBuf};

const SKIP_MARKER: &str = "HYPRFORGE-SKIP:";

/// Every icon name `hyprforge-trayd` can put on the bus.
///
/// Kept here by hand rather than imported, on purpose: the binary's
/// allow-lists are `const`s inside its own `mod tests`, and a test that
/// reads the same list the code reads proves only that a list equals
/// itself. This is the second opinion.
const EVERY_ICON: &[&str] = &[
    // Network
    "network-wireless-signal-excellent",
    "network-wireless-signal-good",
    "network-wireless-signal-ok",
    "network-wireless-signal-weak",
    "network-wireless-signal-none",
    "network-wireless-disconnected",
    // Bluetooth
    "network-bluetooth-activated-symbolic",
    "network-bluetooth-inactive-symbolic",
    // Keep awake
    "changes-prevent-symbolic",
    "changes-allow-symbolic",
    // Night light
    "redshift-status-on-symbolic",
    "redshift-status-off-symbolic",
    // Shared fallbacks
    "dialog-warning",
    "dialog-information",
];

/// The user's theme, from gsettings — the same source
/// `hyprforge-appearance` resolves the rest of the look from.
fn configured_theme() -> Option<String> {
    let out = std::process::Command::new("gsettings")
        .args(["get", "org.gnome.desktop.interface", "icon-theme"])
        .output()
        .ok()?;
    let name = String::from_utf8_lossy(&out.stdout)
        .trim()
        .trim_matches('\'')
        .to_string();
    (!name.is_empty()).then_some(name)
}

fn theme_roots() -> Vec<PathBuf> {
    let mut roots = Vec::new();
    if let Some(home) = std::env::var_os("HOME") {
        roots.push(PathBuf::from(&home).join(".icons"));
        roots.push(PathBuf::from(&home).join(".local/share/icons"));
    }
    roots.push(PathBuf::from("/usr/share/icons"));
    roots
}

fn theme_dir(name: &str) -> Option<PathBuf> {
    theme_roots()
        .into_iter()
        .map(|r| r.join(name))
        .find(|p| p.is_dir())
}

/// A theme's `Inherits=` line, which is what makes a name resolvable
/// through a theme that does not carry it.
fn inherits(theme: &Path) -> Vec<String> {
    let Ok(index) = std::fs::read_to_string(theme.join("index.theme")) else {
        return Vec::new();
    };
    index
        .lines()
        .find_map(|l| l.strip_prefix("Inherits="))
        .map(|v| v.split(',').map(|s| s.trim().to_string()).collect())
        .unwrap_or_default()
}

/// Every icon basename reachable from `theme`, following inheritance.
///
/// Only themes that are actually installed count. A theme may inherit
/// from half a dozen it does not have — this machine's inherits six that
/// are absent — and a name that resolves only through a missing theme
/// does not resolve at all.
fn reachable_names(theme_name: &str) -> HashSet<String> {
    let mut names = HashSet::new();
    let mut queue = vec![theme_name.to_string()];
    let mut visited = HashSet::new();
    // hicolor is the end of every chain by specification.
    queue.push("hicolor".to_string());

    while let Some(name) = queue.pop() {
        if !visited.insert(name.clone()) {
            continue;
        }
        let Some(dir) = theme_dir(&name) else { continue };
        for entry in walkdir(&dir) {
            if let Some(stem) = entry.file_stem().and_then(|s| s.to_str()) {
                names.insert(stem.to_string());
            }
        }
        queue.extend(inherits(&dir));
    }
    names
}

fn walkdir(dir: &Path) -> Vec<PathBuf> {
    let mut out = Vec::new();
    let mut stack = vec![dir.to_path_buf()];
    while let Some(d) = stack.pop() {
        let Ok(entries) = std::fs::read_dir(&d) else { continue };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                stack.push(path);
            } else {
                out.push(path);
            }
        }
    }
    out
}

/// The claim: every name this daemon can emit draws something.
#[test]
#[ignore]
fn every_icon_this_daemon_can_emit_resolves_in_the_configured_theme() {
    let Some(theme) = configured_theme() else {
        eprintln!("{SKIP_MARKER} no icon theme configured in gsettings to resolve against");
        return;
    };
    if theme_dir(&theme).is_none() {
        eprintln!("{SKIP_MARKER} the configured icon theme {theme:?} is not installed");
        return;
    }

    let reachable = reachable_names(&theme);
    if reachable.is_empty() {
        eprintln!("{SKIP_MARKER} {theme:?} resolved to no icons at all; nothing to check against");
        return;
    }

    let missing: Vec<&str> = EVERY_ICON
        .iter()
        .copied()
        .filter(|name| !reachable.contains(*name))
        .collect();

    println!(
        "{} icon names checked against {theme:?} ({} names reachable)",
        EVERY_ICON.len(),
        reachable.len()
    );
    assert!(
        missing.is_empty(),
        "these icon names do not resolve in {theme:?} and would each draw a blank gap \
         in the bar, with nothing logged anywhere: {missing:?}"
    );
}
