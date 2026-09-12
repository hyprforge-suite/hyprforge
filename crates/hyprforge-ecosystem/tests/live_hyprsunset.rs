//! Does the real hyprsunset agree with what `sunset_control` assumes?
//!
//! Not run by default — `cargo test -p hyprforge-ecosystem --test
//! live_hyprsunset -- --ignored`, on a machine running hyprsunset.
//!
//! **Every test in this file is strictly read-only: none of them changes
//! the screen's colour temperature.** This runs on a machine somebody is
//! using, and `live_ecosystem.rs` already carries the one test in this
//! project that changes hyprsunset's temperature and restores it
//! (`hyprsunset_takes_and_reports_a_temperature`), gated behind the same
//! `--ignored` flag and wired into `check.sh`'s "Live tests against
//! Hyprland" step. Adding a second temperature-changing test here would
//! double that risk for no new coverage, so this file only reads.
//!
//! Also read-only for a second reason named in `CLAUDE.md`: there are
//! several stale Hyprland instance directories under
//! `/run/user/*/hypr/` on this machine, left behind by past sessions.
//! Picking "the newest one" by mtime can hand a test the wrong session,
//! because the live one is the one still writing its log, not
//! necessarily the one with the newest directory timestamp. `hyprctl
//! instances -j` is asked instead, and its `wl_socket` field is matched
//! against `$WAYLAND_DISPLAY` when it's set, so a test run from inside
//! a nested compositor (see `crates/hyprforge-lock/testing/nested.sh`)
//! checks that nested instance and not whichever one happens to be
//! logged in on this machine.

use hyprforge_ecosystem::sunset_control::{self, Hyprsunset, SunsetBackend, SunsetControlError};
use std::process::Command;

/// Same convention as the ecosystem parse tests and the BlueZ/
/// NetworkManager live tests: libtest has no skipped state, so a check
/// that could not run announces itself rather than returning early and
/// printing `ok` like one that passed.
const SKIP_MARKER: &str = "HYPRFORGE-SKIP:";

/// One entry from `hyprctl instances -j`.
#[derive(serde::Deserialize)]
struct Instance {
    instance: String,
    wl_socket: String,
}

/// The live Hyprland instance, or `None` with a skip line already
/// printed.
///
/// Keyed on `wl_socket` against `$WAYLAND_DISPLAY`, per `CLAUDE.md`:
/// picking the newest directory under `/run/user/*/hypr/` can hand back
/// a stale session's log directory instead of the one actually running.
/// When `$WAYLAND_DISPLAY` isn't set (this test wasn't launched from
/// inside a Hyprland session) the first reported instance is used, which
/// is only ever a guess — `hyprctl instances -j` at least limits the
/// guess to instances Hyprland itself reports as running, never a
/// directory left over from one that has since exited.
fn live_instance() -> Option<Instance> {
    let output = match hyprforge_core::command::output(
        Command::new("hyprctl").args(["instances", "-j"]),
        hyprforge_core::command::TIMEOUT,
    ) {
        Ok(o) if o.status.success() => o,
        Ok(_) => {
            eprintln!("{SKIP_MARKER} hyprctl instances -j did not succeed");
            return None;
        }
        Err(e) => {
            eprintln!("{SKIP_MARKER} hyprctl isn't runnable ({e})");
            return None;
        }
    };

    let instances: Vec<Instance> = match serde_json::from_slice(&output.stdout) {
        Ok(v) => v,
        Err(e) => {
            eprintln!("{SKIP_MARKER} hyprctl instances -j didn't parse as expected ({e})");
            return None;
        }
    };
    if instances.is_empty() {
        eprintln!("{SKIP_MARKER} hyprctl instances -j reported no running instance");
        return None;
    }

    let wanted = std::env::var("WAYLAND_DISPLAY").ok();
    let chosen = wanted
        .as_deref()
        .and_then(|w| instances.iter().find(|i| i.wl_socket == w))
        .or_else(|| instances.first());
    chosen.map(|i| Instance { instance: i.instance.clone(), wl_socket: i.wl_socket.clone() })
}

/// The claim `sunset_control`'s module doc rests on: `hyprsunset` is a
/// real program on `$PATH` and answers `--version`, so driving it through
/// `hyprctl` rather than reimplementing its socket protocol is a choice
/// this crate can actually make.
#[test]
#[ignore]
fn hyprsunset_the_binary_reports_a_version() {
    let output = match hyprforge_core::command::output(
        Command::new("hyprsunset").arg("--version"),
        hyprforge_core::command::TIMEOUT,
    ) {
        Ok(o) => o,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            eprintln!("{SKIP_MARKER} hyprsunset isn't on PATH");
            return;
        }
        Err(e) => panic!("hyprsunset --version didn't run: {e}"),
    };
    assert!(output.status.success(), "hyprsunset --version exited non-zero");
    let text = String::from_utf8_lossy(&output.stdout);
    assert!(!text.trim().is_empty(), "hyprsunset --version printed nothing");
}

/// hyprsunset's socket lives where `sunset_control`'s module doc says it
/// does — under the *live* instance's runtime directory, not any stale
/// one. This does not connect to the socket; only that Hyprland reports
/// an instance and the socket file exists under it.
#[test]
#[ignore]
fn the_live_instances_hyprsunset_socket_exists_where_expected() {
    let Some(instance) = live_instance() else { return };
    let Ok(runtime_dir) = std::env::var("XDG_RUNTIME_DIR") else {
        eprintln!("{SKIP_MARKER} XDG_RUNTIME_DIR isn't set");
        return;
    };
    let socket = std::path::Path::new(&runtime_dir)
        .join("hypr")
        .join(&instance.instance)
        .join(".hyprsunset.sock");
    if !socket.exists() {
        eprintln!(
            "{SKIP_MARKER} hyprsunset isn't running under instance {} (wl_socket {})",
            instance.instance, instance.wl_socket
        );
        return;
    }
    // A socket, specifically — not some other file hyprsunset happens to
    // leave at that path.
    let metadata = std::fs::symlink_metadata(&socket)
        .unwrap_or_else(|e| panic!("could not stat {}: {e}", socket.display()));
    assert!(
        std::os::unix::fs::FileTypeExt::is_socket(&metadata.file_type()),
        "{} exists but is not a socket",
        socket.display()
    );
}

/// The one live claim `Hyprsunset::current_temperature` makes: that
/// `hyprctl hyprsunset temperature` with no argument reports a plain
/// integer within the catalogued range, rather than the crate's parsing
/// being tuned to a wiki example that turns out to be wrong. Read-only —
/// nothing is set.
#[test]
#[ignore]
fn hyprsunset_reports_its_current_temperature_within_the_declared_range() {
    let backend = Hyprsunset;
    match backend.current_temperature() {
        Ok(kelvin) => {
            assert!(
                sunset_control::validate_temperature(kelvin).is_ok(),
                "hyprsunset reported {kelvin}K, outside the range this crate declares valid"
            );
        }
        Err(SunsetControlError::NotRunning) => {
            eprintln!("{SKIP_MARKER} hyprsunset is not running");
        }
        Err(SunsetControlError::CouldNotCheck(reason)) => {
            eprintln!("{SKIP_MARKER} could not tell whether hyprsunset is running: {reason}");
        }
        Err(e) => panic!("hyprsunset answered, but not in the shape this crate expects: {e}"),
    }
}
