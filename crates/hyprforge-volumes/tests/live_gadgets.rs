//! Does **gio** agree about phones and cameras — and does the kernel?
//!
//! The unit tests read two published samples of `gio mount -li`, a
//! Samsung phone's and a Nikon camera's. This asks the `gio` installed
//! here, and the kernel's own USB listing, so a format that has moved on
//! since those samples shows up as a failure rather than as a phone that
//! never appears.
//!
//! **Read-only.** It lists; it never mounts, unmounts or claims a
//! device. A phone plugged into this machine is somebody's phone.
//!
//! Run with `cargo test -p hyprforge-volumes --test live_gadgets -- --ignored`.

use hyprforge_volumes::gadgets;
use std::path::Path;
use std::process::Command;

/// Same convention as every other live tier: libtest has no skipped
/// state, so a check that could not run announces itself.
const SKIP_MARKER: &str = "HYPRFORGE-SKIP:";

/// What `gio mount -li` prints here, in English as the runner asks for
/// it; `None` when there is no `gio` to ask.
fn gio_listing() -> Option<String> {
    let mut command = Command::new("gio");
    command.args(["mount", "-li"]).env("LC_ALL", "C.UTF-8").env_remove("LANGUAGE");
    match hyprforge_process::output(&mut command, std::time::Duration::from_secs(10)) {
        Ok(out) => {
            assert!(out.status.success(), "gio mount -li failed: {}", String::from_utf8_lossy(&out.stderr));
            Some(String::from_utf8_lossy(&out.stdout).into_owned())
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            eprintln!("{SKIP_MARKER} gio isn't installed");
            None
        }
        Err(e) => panic!("gio mount -li didn't answer: {e}"),
    }
}

/// Every volume an MTP or gPhoto2 monitor reported is read as one —
/// counted the plainest way there is, by its `Type:` line — and nothing
/// UDisks2 reported is.
#[test]
#[ignore]
fn every_phone_and_camera_gio_lists_is_read_and_no_drive_is() {
    let Some(text) = gio_listing() else { return };
    let by_type = text
        .lines()
        .filter(|l| {
            let l = l.trim();
            l == "Type: GProxyVolume (GProxyVolumeMonitorMTP)" || l == "Type: GProxyVolume (GProxyVolumeMonitorGPhoto2)"
        })
        .count();
    let read = gadgets::volumes(&text);
    assert_eq!(read.len(), by_type, "{read:?}\n---\n{text}");
    for volume in &read {
        assert!(volume.uri.contains("://"), "{volume:?}");
    }
    if by_type == 0 {
        eprintln!("{SKIP_MARKER} no phone or camera is plugged in for gio to list");
    }
}

/// The kernel's listing reads as devices with interfaces. Every machine
/// that runs this has a USB controller with something on it, and a
/// device read without its interfaces could never be recognised as a
/// phone.
#[test]
#[ignore]
fn the_kernels_usb_listing_reads_with_interfaces() {
    let root = Path::new("/sys/bus/usb/devices");
    if !root.is_dir() {
        eprintln!("{SKIP_MARKER} no USB in sysfs on this machine");
        return;
    }
    let devices = hyprforge_volumes::network::read_usb(root);
    if devices.is_empty() {
        eprintln!("{SKIP_MARKER} nothing on USB but the root hubs");
        return;
    }
    assert!(devices.iter().any(|d| !d.interfaces.is_empty()), "{devices:?}");
    for device in &devices {
        for (class, _) in &device.interfaces {
            assert!(class.len() == 2 && class.chars().all(|c| c.is_ascii_hexdigit()), "{class:?} isn't sysfs's two hex digits");
        }
    }
}
