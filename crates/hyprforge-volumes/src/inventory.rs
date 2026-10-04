//! Which of UDisks2's block devices are volumes a person would want in a
//! sidebar, and what to call them.
//!
//! UDisks2 lists every block device on the machine: whole disks and
//! their partitions, the EFI partition, swap, the root filesystem, the
//! loop devices snap and flatpak leave behind. A sidebar that showed all
//! of them would offer to unmount `/`. The rules here follow the ones
//! GNOME's volume monitor applies to the same data, because that is what
//! anyone switching from it will expect to see:
//!
//! - **Something mountable, or locked.** A block with a filesystem, or
//!   an encrypted container nobody has unlocked. A partition table, a
//!   swap area or an empty partition is not something to open.
//! - **Not ignored.** `HintIgnore` is the system's — or an udev rule's —
//!   explicit "do not show this".
//! - **Not a system disk, unless it is mounted where removable things
//!   go.** `HintSystem` is UDisks2's answer to "is this part of the
//!   machine", and it is false for a stick and true for the disk `/` is
//!   on. A system disk mounted under `/run/media` was mounted by a
//!   person for a person, so it shows.
//! - **A loop device only when this user attached it.** Loop devices
//!   are `HintSystem` by nature; the one exception worth showing is a
//!   disk image somebody opened on purpose (`SetupByUID`).
//!
//! Plain data in, plain data out — [`RawBlock`] and [`RawDrive`] carry
//! exactly the properties these rules read, so every rule is a test that
//! needs no bus. `crate::udisks` turns D-Bus property maps into them.

use crate::types::{Detach, Volume, VolumeId, VolumeKind};
use std::path::{Path, PathBuf};

/// The properties of one `org.freedesktop.UDisks2.Block` object that
/// decide whether and how it is shown.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct RawBlock {
    /// The object path.
    pub id: String,
    /// `Block.Device`.
    pub device: PathBuf,
    /// `Block.Drive`, `None` for `/`.
    pub drive: Option<String>,
    pub size: u64,
    pub id_label: String,
    /// `Block.IdUsage`: `filesystem`, `crypto`, `other`, or empty.
    pub id_usage: String,
    /// `Block.IdType`: `vfat`, `crypto_LUKS`, `BitLocker`.
    pub id_type: String,
    pub hint_ignore: bool,
    pub hint_system: bool,
    pub hint_name: String,
    /// `Block.CryptoBackingDevice`: for an unlocked container's
    /// cleartext device, the container it came from.
    pub crypto_backing: Option<String>,
    /// Whether the object has the `Filesystem` interface at all.
    pub has_filesystem: bool,
    /// `Filesystem.MountPoints`, decoded.
    pub mount_points: Vec<PathBuf>,
    /// `Loop.SetupByUID`, for a loop device.
    pub loop_setup_by: Option<u32>,
    /// Whether the object has the `Encrypted` interface.
    pub encrypted: bool,
}

/// The properties of one `org.freedesktop.UDisks2.Drive` object that
/// these rules read.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct RawDrive {
    pub id: String,
    pub removable: bool,
    pub media_removable: bool,
    pub ejectable: bool,
    pub can_power_off: bool,
    pub optical: bool,
    /// `usb`, `sdio`, `ieee1394` or empty.
    pub connection_bus: String,
}

/// Where a person's own mounts land — UDisks2 puts them under
/// `/run/media/$USER`, older systems under `/media`.
fn mounted_for_a_person(mount_points: &[PathBuf]) -> bool {
    mount_points
        .iter()
        .any(|p| p.starts_with("/run/media") || p.starts_with("/media"))
}

/// Whether `block` belongs in a sidebar for the user `uid`. See the
/// module doc for the rules.
pub fn shows(block: &RawBlock, all: &[RawBlock], uid: u32) -> bool {
    if block.hint_ignore {
        return false;
    }
    let mountable = block.has_filesystem && block.id_usage == "filesystem";
    let locked_container = block.encrypted
        // Unlocked already: its cleartext device is what is shown, and
        // the container beside it would be the same drive twice.
        && !all.iter().any(|other| other.crypto_backing.as_deref() == Some(block.id.as_str()));
    if !mountable && !locked_container {
        return false;
    }
    if let Some(owner) = block.loop_setup_by {
        return owner == uid;
    }
    !block.hint_system || mounted_for_a_person(&block.mount_points)
}

/// What to call a volume: the name a person or the system gave it, or
/// failing both, how big it is — which is what tells two unnamed sticks
/// apart, and what every other file manager falls back to.
pub fn label(block: &RawBlock) -> String {
    if !block.hint_name.trim().is_empty() {
        return block.hint_name.trim().to_string();
    }
    if !block.id_label.trim().is_empty() {
        return block.id_label.trim().to_string();
    }
    format!("{} Volume", size_words(block.size))
}

/// `32.0 MB`, `1.0 TB` — the units a drive is sold in, which are
/// decimal; a "1 TB" card that reads "931.5 GiB" here would look like it
/// was missing a sixth of itself.
pub fn size_words(bytes: u64) -> String {
    const UNITS: [&str; 5] = ["bytes", "kB", "MB", "GB", "TB"];
    let mut value = bytes as f64;
    let mut unit = 0;
    while value >= 1000.0 && unit + 1 < UNITS.len() {
        value /= 1000.0;
        unit += 1;
    }
    if unit == 0 {
        format!("{bytes} bytes")
    } else {
        format!("{value:.1} {}", UNITS[unit])
    }
}

fn kind_and_detach(block: &RawBlock, drive: Option<&RawDrive>) -> (VolumeKind, Option<Detach>) {
    if block.loop_setup_by.is_some() {
        return (VolumeKind::Loop, Some(Detach::Loop));
    }
    let Some(drive) = drive else { return (VolumeKind::Internal, None) };
    if drive.optical {
        return (VolumeKind::Optical, drive.ejectable.then_some(Detach::Eject));
    }
    let removable = drive.removable
        || drive.media_removable
        || drive.can_power_off
        || matches!(drive.connection_bus.as_str(), "usb" | "sdio" | "ieee1394");
    if !removable {
        return (VolumeKind::Internal, None);
    }
    // Powering off is what makes a stick safe to pull; an ejectable
    // drive that cannot power off (a card reader) at least ejects the
    // medium. A removable drive that can do neither is still Removable
    // — it is just unmounted and then left alone.
    let detach = if drive.can_power_off {
        Some(Detach::PowerOff)
    } else if drive.ejectable {
        Some(Detach::Eject)
    } else {
        None
    };
    (VolumeKind::Removable, detach)
}

/// Every volume to show, from everything UDisks2 listed — in a stable
/// order: by drive, then by device, so a stick's partitions stay
/// together and a new stick does not reshuffle the ones already there;
/// disk images last.
pub fn assemble(blocks: &[RawBlock], drives: &[RawDrive], uid: u32) -> Vec<Volume> {
    let mut volumes: Vec<(Option<String>, Volume)> = blocks
        .iter()
        .filter(|block| shows(block, blocks, uid))
        .map(|block| {
            let drive = block
                .drive
                .as_deref()
                .and_then(|id| drives.iter().find(|d| d.id == id));
            let (kind, detach) = kind_and_detach(block, drive);
            let locked = !block.has_filesystem && block.encrypted;
            let volume = Volume {
                id: VolumeId(block.id.clone()),
                device: block.device.clone(),
                label: label(block),
                size: block.size,
                filesystem: (!block.id_type.is_empty() && !locked).then(|| block.id_type.clone()),
                mount_point: block.mount_points.first().cloned(),
                kind,
                detach,
                locked,
            };
            (block.drive.clone(), volume)
        })
        .collect();
    // Drives first and disk images after: a stick is the thing a person
    // just plugged in, and an image is a file they opened.
    volumes.sort_by(|(a, va), (b, vb)| {
        (a.is_none(), a).cmp(&(b.is_none(), b)).then_with(|| va.device.cmp(&vb.device))
    });
    volumes.into_iter().map(|(_, v)| v).collect()
}

/// The volume whose mount point holds `path`, deepest first — what
/// "Eject" from the palette acts on while you are browsing a stick.
pub fn mounted_at<'a>(volumes: &'a [Volume], path: &Path) -> Option<&'a Volume> {
    volumes
        .iter()
        .filter(|v| v.mount_point.as_deref().is_some_and(|m| path.starts_with(m)))
        .max_by_key(|v| v.mount_point.as_ref().map_or(0, |m| m.components().count()))
}

#[cfg(test)]
mod tests {
    use super::*;

    const UID: u32 = 1000;

    /// The 1TB expansion card on the machine this was written on, as
    /// `udisksctl dump` printed it: an NTFS partition on a USB drive
    /// that can power off, not a system disk, not mounted.
    fn card() -> (RawBlock, RawDrive) {
        let drive = RawDrive {
            id: "/org/freedesktop/UDisks2/drives/1TB_Card_500014a000000001".into(),
            removable: true,
            media_removable: false,
            ejectable: false,
            can_power_off: true,
            optical: false,
            connection_bus: "usb".into(),
        };
        let block = RawBlock {
            id: "/org/freedesktop/UDisks2/block_devices/sda1".into(),
            device: "/dev/sda1".into(),
            drive: Some(drive.id.clone()),
            size: 1_000_202_043_392,
            id_label: "Development".into(),
            id_usage: "filesystem".into(),
            id_type: "ntfs".into(),
            has_filesystem: true,
            ..RawBlock::default()
        };
        (block, drive)
    }

    /// The same machine's root filesystem: a system disk, mounted at `/`.
    fn root() -> RawBlock {
        RawBlock {
            id: "/org/freedesktop/UDisks2/block_devices/nvme1n1p2".into(),
            device: "/dev/nvme1n1p2".into(),
            drive: Some("/org/freedesktop/UDisks2/drives/WD_BLACK".into()),
            size: 999_128_301_568,
            id_usage: "filesystem".into(),
            id_type: "btrfs".into(),
            hint_system: true,
            has_filesystem: true,
            mount_points: vec!["/".into()],
            ..RawBlock::default()
        }
    }

    #[test]
    fn a_usb_card_shows_by_its_label_and_powers_off_to_eject() {
        let (block, drive) = card();
        let volumes = assemble(&[block], &[drive], UID);
        assert_eq!(volumes.len(), 1);
        assert_eq!(volumes[0].label, "Development");
        assert_eq!(volumes[0].kind, VolumeKind::Removable);
        assert_eq!(volumes[0].detach, Some(Detach::PowerOff));
        assert_eq!(volumes[0].filesystem.as_deref(), Some("ntfs"));
        assert!(!volumes[0].is_mounted());
    }

    /// Nobody wants "unmount /" one click away in a sidebar.
    #[test]
    fn the_system_disk_is_not_offered() {
        assert!(assemble(&[root()], &[], UID).is_empty());
    }

    /// The same machine's BitLocker partition: a system disk, so hidden,
    /// even though it is a locked container.
    #[test]
    fn a_locked_container_on_a_system_disk_stays_hidden() {
        let bitlocker = RawBlock {
            id: "/b/nvme0n1p3".into(),
            id_usage: "crypto".into(),
            id_type: "BitLocker".into(),
            hint_system: true,
            encrypted: true,
            ..RawBlock::default()
        };
        assert!(assemble(&[bitlocker], &[], UID).is_empty());
    }

    #[test]
    fn a_locked_container_on_a_stick_shows_as_locked_with_no_filesystem() {
        let (_, drive) = card();
        let luks = RawBlock {
            id: "/b/sdb1".into(),
            device: "/dev/sdb1".into(),
            drive: Some(drive.id.clone()),
            size: 8_000_000_000,
            id_usage: "crypto".into(),
            id_type: "crypto_LUKS".into(),
            encrypted: true,
            ..RawBlock::default()
        };
        let volumes = assemble(&[luks], &[drive], UID);
        assert_eq!(volumes.len(), 1);
        assert!(volumes[0].locked);
        assert_eq!(volumes[0].filesystem, None);
        assert_eq!(volumes[0].label, "8.0 GB Volume");
    }

    /// Unlocked, the cleartext filesystem is the one row; the container
    /// beside it would be the same stick twice.
    #[test]
    fn an_unlocked_container_shows_once_as_its_filesystem() {
        let container = RawBlock {
            id: "/b/sdb1".into(),
            id_usage: "crypto".into(),
            encrypted: true,
            ..RawBlock::default()
        };
        let cleartext = RawBlock {
            id: "/b/dm_2d0".into(),
            id_usage: "filesystem".into(),
            id_type: "ext4".into(),
            id_label: "Secret".into(),
            has_filesystem: true,
            crypto_backing: Some("/b/sdb1".into()),
            ..RawBlock::default()
        };
        let volumes = assemble(&[container, cleartext], &[], UID);
        assert_eq!(volumes.iter().map(|v| v.label.as_str()).collect::<Vec<_>>(), ["Secret"]);
    }

    /// The loop device this was tested against: `udisksctl loop-setup`
    /// gave it `HintSystem: true`, no drive, and `SetupByUID: 1000`.
    #[test]
    fn a_disk_image_shows_only_for_whoever_attached_it() {
        let image = RawBlock {
            id: "/b/loop0".into(),
            device: "/dev/loop0".into(),
            id_label: "HFTEST".into(),
            id_usage: "filesystem".into(),
            id_type: "vfat".into(),
            hint_system: true,
            has_filesystem: true,
            loop_setup_by: Some(UID),
            ..RawBlock::default()
        };
        let volumes = assemble(std::slice::from_ref(&image), &[], UID);
        assert_eq!(volumes.len(), 1);
        assert_eq!(volumes[0].kind, VolumeKind::Loop);
        assert_eq!(volumes[0].detach, Some(Detach::Loop));
        let theirs = RawBlock { loop_setup_by: Some(0), ..image };
        assert!(assemble(&[theirs], &[], UID).is_empty(), "snap's and flatpak's loops are not ours");
    }

    #[test]
    fn a_system_disk_a_person_mounted_under_run_media_shows() {
        let mounted = RawBlock { mount_points: vec!["/run/media/apost/Data".into()], ..root() };
        assert_eq!(assemble(&[mounted], &[], UID).len(), 1);
    }

    #[test]
    fn swap_and_partition_tables_are_not_volumes() {
        let swap = RawBlock { id_usage: "other".into(), id_type: "swap".into(), ..RawBlock::default() };
        let table = RawBlock::default();
        assert!(assemble(&[swap, table], &[], UID).is_empty());
    }

    #[test]
    fn hint_ignore_wins_over_everything() {
        let (block, drive) = card();
        let ignored = RawBlock { hint_ignore: true, ..block };
        assert!(assemble(&[ignored], &[drive], UID).is_empty());
    }

    /// A drive's own name for a partition beats its label: an udev rule
    /// that names it is somebody's deliberate choice.
    #[test]
    fn the_hint_name_beats_the_label_and_size_is_the_last_resort() {
        let (block, _) = card();
        assert_eq!(label(&RawBlock { hint_name: "Backups".into(), ..block.clone() }), "Backups");
        assert_eq!(label(&RawBlock { id_label: String::new(), ..block }), "1.0 TB Volume");
    }

    #[test]
    fn sizes_read_in_the_units_drives_are_sold_in() {
        assert_eq!(size_words(512), "512 bytes");
        assert_eq!(size_words(32 * 1024 * 1024), "33.6 MB");
        assert_eq!(size_words(1_000_204_886_016), "1.0 TB");
    }

    #[test]
    fn an_optical_drive_ejects() {
        let drive = RawDrive { id: "/d/sr0".into(), optical: true, ejectable: true, ..RawDrive::default() };
        let disc = RawBlock {
            id: "/b/sr0".into(),
            drive: Some("/d/sr0".into()),
            id_usage: "filesystem".into(),
            id_type: "iso9660".into(),
            id_label: "MOVIE".into(),
            has_filesystem: true,
            ..RawBlock::default()
        };
        let volumes = assemble(&[disc], &[drive], UID);
        assert_eq!(volumes[0].kind, VolumeKind::Optical);
        assert_eq!(volumes[0].detach, Some(Detach::Eject));
        assert_eq!(volumes[0].icon_name(), "media-optical");
    }

    #[test]
    fn partitions_of_one_drive_stay_together_in_device_order() {
        let (b1, drive) = card();
        let b2 = RawBlock { id: "/b/sda2".into(), device: "/dev/sda2".into(), id_label: "Two".into(), ..b1.clone() };
        let volumes = assemble(&[b2, b1], &[drive], UID);
        assert_eq!(volumes.iter().map(|v| v.label.as_str()).collect::<Vec<_>>(), ["Development", "Two"]);
    }

    #[test]
    fn the_deepest_mount_holding_a_path_is_the_one_it_is_on() {
        let (block, drive) = card();
        let outer = RawBlock { mount_points: vec!["/run/media/u/A".into()], ..block.clone() };
        let inner = RawBlock {
            id: "/b/x".into(),
            device: "/dev/x".into(),
            mount_points: vec!["/run/media/u/A/inner".into()],
            ..block
        };
        let volumes = assemble(&[outer, inner], &[drive], UID);
        let on = mounted_at(&volumes, Path::new("/run/media/u/A/inner/docs")).unwrap();
        assert_eq!(on.device, PathBuf::from("/dev/x"));
        assert!(mounted_at(&volumes, Path::new("/home/u")).is_none());
    }
}
