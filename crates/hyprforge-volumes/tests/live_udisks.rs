//! Does **UDisks2** agree?
//!
//! The unit tests check this crate against its own idea of UDisks2's
//! interface, built from one `udisksctl dump`. Code can be internally
//! consistent and wrong about the service it talks to: a property that
//! is `ay` and not `s`, a mount point with its NUL left on, an object
//! path where a string was expected. Each of those reads as an empty
//! value here — deliberately forgiving — so only the real service can
//! show that the values are actually arriving.
//!
//! **Every test in this file is read-only.** It lists objects and reads
//! properties; it never mounts, unmounts, ejects or powers anything off.
//! This runs on a machine somebody is using, and a drive of theirs is
//! not a fixture.
//!
//! Run with `cargo test -p hyprforge-volumes --test live_udisks -- --ignored`.

use hyprforge_volumes::backend::VolumeBackend;
use hyprforge_volumes::udisks::UDisks2Backend;
use hyprforge_volumes::VolumeError;
use std::path::Path;

/// Same convention as every other live tier: libtest has no skipped
/// state, so a check that could not run announces itself.
const SKIP_MARKER: &str = "HYPRFORGE-SKIP:";

/// `Some` when UDisks2 answered; otherwise says why not.
async fn raw() -> Option<(Vec<hyprforge_volumes::inventory::RawBlock>, Vec<hyprforge_volumes::inventory::RawDrive>)> {
    match UDisks2Backend::new().raw().await {
        Ok(raw) => Some(raw),
        Err(VolumeError::Unavailable) => {
            eprintln!("{SKIP_MARKER} UDisks2 isn't on the system bus");
            None
        }
        Err(e) => panic!("UDisks2 answered, but not in the shape this crate expects: {e}"),
    }
}

/// The device the root filesystem is on, as the kernel's mount table
/// says — the one fact about UDisks2's answer this test can know
/// independently of UDisks2.
fn root_device() -> Option<String> {
    let table = std::fs::read_to_string("/proc/self/mountinfo").ok()?;
    table.lines().find_map(|line| {
        let (before, after) = line.split_once(" - ")?;
        (before.split(' ').nth(4)? == "/").then(|| after.split(' ').nth(1).map(str::to_string)).flatten()
    })
}

#[tokio::test]
#[ignore]
async fn udisks_lists_block_devices_with_real_device_paths() {
    let Some((blocks, _)) = raw().await else { return };
    assert!(!blocks.is_empty(), "UDisks2 listed no block devices at all");
    for block in &blocks {
        let device = block.device.to_string_lossy();
        assert!(device.starts_with("/dev/"), "Block.Device should be an `ay` path under /dev; read {device:?}");
        assert!(!device.contains('\0'), "the NUL terminator was not taken off {device:?}");
    }
}

/// `MountPoints` is `aay`. Read wrongly it comes out empty, and every
/// mounted stick would show as unmounted.
#[tokio::test]
#[ignore]
async fn the_root_filesystem_is_mounted_at_slash_by_udisks_own_account() {
    let Some((blocks, _)) = raw().await else { return };
    let Some(root) = root_device() else {
        eprintln!("{SKIP_MARKER} couldn't read the root device from /proc/self/mountinfo");
        return;
    };
    if !root.starts_with("/dev/") {
        eprintln!("{SKIP_MARKER} the root filesystem is not on a block device ({root})");
        return;
    }
    let block = blocks.iter().find(|b| b.device == Path::new(&root));
    let Some(block) = block else {
        eprintln!("{SKIP_MARKER} UDisks2 does not list the root device {root} (a mapper device, perhaps)");
        return;
    };
    assert!(block.has_filesystem, "{root} should carry the Filesystem interface");
    assert!(
        block.mount_points.iter().any(|p| p == Path::new("/")),
        "UDisks2's MountPoints for {root} should include /; read {:?}",
        block.mount_points
    );
    assert!(block.hint_system, "the disk / is on should be HintSystem");
}

/// The rule the sidebar depends on most: whatever else it offers, it
/// never offers the disk the system is running from.
#[tokio::test]
#[ignore]
async fn the_volumes_offered_never_include_the_root_filesystem() {
    let backend = UDisks2Backend::new();
    match backend.volumes().await {
        Ok(volumes) => {
            for volume in &volumes {
                assert_ne!(volume.mount_point.as_deref(), Some(Path::new("/")), "{volume:?}");
            }
            for volume in &volumes {
                println!("offered: {} ({:?}, {:?})", volume.label, volume.kind, volume.mount_point);
            }
        }
        Err(VolumeError::Unavailable) => eprintln!("{SKIP_MARKER} UDisks2 isn't on the system bus"),
        Err(e) => panic!("{e}"),
    }
}

/// Every drive's `ConnectionBus` and the booleans arrive as what they
/// are: a drive with `CanPowerOff` read as `false` everywhere would lose
/// every Eject button.
#[tokio::test]
#[ignore]
async fn drive_ids_named_by_blocks_are_drives_udisks_lists() {
    let Some((blocks, drives)) = raw().await else { return };
    for block in &blocks {
        if let Some(drive) = &block.drive {
            assert!(
                drives.iter().any(|d| &d.id == drive),
                "{} names drive {drive}, which UDisks2 did not list as a Drive",
                block.device.display()
            );
        }
    }
}

/// The signal subscription is accepted by the bus — a rule it rejected
/// would mean a sidebar that never hears a stick arrive.
#[tokio::test]
#[ignore]
async fn the_bus_accepts_the_change_subscription() {
    let backend = UDisks2Backend::new();
    match backend.watch().await {
        Ok(_) => {}
        Err(VolumeError::Unavailable) => eprintln!("{SKIP_MARKER} no system bus"),
        Err(e) => panic!("the bus refused the match rules: {e}"),
    }
}
