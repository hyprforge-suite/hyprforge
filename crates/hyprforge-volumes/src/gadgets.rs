//! Phones and cameras: what gvfs's MTP and gPhoto2 volume monitors can
//! see, what is plugged in that they cannot because their backend is not
//! installed, and what to say when opening one fails.
//!
//! # Why not UDisks2
//!
//! A phone is not a block device. Android stopped offering USB mass
//! storage years ago and speaks MTP instead; a camera speaks PTP. Neither
//! reaches UDisks2, so the drives this crate lists never include them.
//! gvfs has a backend for each (`gvfs-mtp`, `gvfs-gphoto2`) with a volume
//! monitor that notices the device and a daemon that mounts it into the
//! same FUSE directory a network share lands in — so once mounted, a
//! phone is browsed with exactly the code a home directory is.
//!
//! # Why `gio mount -li`
//!
//! The volume monitors talk to applications over a private D-Bus
//! protocol (`org.gtk.Private.RemoteVolumeMonitor`, private in its name).
//! `gio mount -li` is the published face of the same list, the reason
//! [`crate::gvfs`] mounts through `gio` rather than the bus. [`volumes`]
//! reads it; the fixtures in the tests below are real output, a Samsung
//! phone's and a Nikon camera's, copied from public bug reports rather
//! than written from memory of the format.
//!
//! # Not installed is a state, never an absence
//!
//! Without `gvfs-mtp` a plugged-in phone is invisible to gvfs, and a
//! sidebar that shows nothing is a sidebar that claims nothing is
//! plugged in. So [`usb`] reads the kernel's own view — sysfs, which
//! needs no daemon at all — for a USB interface that is a phone or a
//! camera, and [`assemble`] turns one that no monitor claimed into an
//! [`Unreadable`]: a row that says which package would open it.
//!
//! Everything here is pure — text and file contents in, a decision out.
//! Running `gio` and reading sysfs is `crate::network`'s.

use std::path::{Path, PathBuf};

/// What sort of thing it is — its icon, and which gvfs backend reads it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum GadgetKind {
    /// MTP: an Android phone, a tablet, a media player.
    Phone,
    /// PTP through gPhoto2: a camera — and an iPhone, which offers its
    /// photos the same way.
    Camera,
}

impl GadgetKind {
    /// The freedesktop icon name.
    pub fn icon_name(self) -> &'static str {
        match self {
            GadgetKind::Phone => "phone",
            GadgetKind::Camera => "camera-photo",
        }
    }

    /// The gvfs backend that reads it, as Arch packages it.
    pub fn package(self) -> &'static str {
        match self {
            GadgetKind::Phone => "gvfs-mtp",
            GadgetKind::Camera => "gvfs-gphoto2",
        }
    }

    /// The URI scheme gvfs gives it.
    pub fn scheme(self) -> &'static str {
        match self {
            GadgetKind::Phone => "mtp",
            GadgetKind::Camera => "gphoto2",
        }
    }

    fn from_scheme(scheme: &str) -> Option<GadgetKind> {
        match scheme {
            "mtp" => Some(GadgetKind::Phone),
            "gphoto2" => Some(GadgetKind::Camera),
            _ => None,
        }
    }

    /// What a person calls it in a sentence.
    fn noun(self) -> &'static str {
        match self {
            GadgetKind::Phone => "phone",
            GadgetKind::Camera => "camera",
        }
    }
}

/// A phone or camera gvfs can open.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Gadget {
    /// What gvfs calls it: "SAMSUNG Android", "NIKON DSC COOLPIX S203-PTP".
    pub label: String,
    pub kind: GadgetKind,
    /// Its activation root — `mtp://SAMSUNG_SAMSUNG_Android_R58M…/` —
    /// which is what `gio mount` and `gio mount -u` take, and the
    /// identity across one listing and the next.
    pub uri: String,
    /// Its FUSE directory, when it is mounted.
    pub mounted: Option<PathBuf>,
}

impl Gadget {
    pub fn icon_name(&self) -> &'static str {
        self.kind.icon_name()
    }
}

/// A phone or camera on USB that nothing here can open, because the
/// gvfs backend for it is not installed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Unreadable {
    /// What its USB descriptor calls it.
    pub label: String,
    pub kind: GadgetKind,
}

impl Unreadable {
    /// The sentence its row shows.
    pub fn sentence(&self) -> String {
        format!("Install {} to open \u{201C}{}\u{201D}", self.kind.package(), self.label)
    }
}

/// Every phone and camera, as the Devices section shows them.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Gadgets {
    pub list: Vec<Gadget>,
    pub unreadable: Vec<Unreadable>,
}

/// One volume from `gio mount -li` that an MTP or gPhoto2 monitor
/// reported.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GioVolume {
    pub label: String,
    pub kind: GadgetKind,
    pub uri: String,
    /// `/dev/bus/usb/001/008` — how it is matched against sysfs.
    pub unix_device: Option<String>,
}

/// The phones and cameras in `gio mount -li`'s output.
///
/// A volume starts at a `Volume(N): label` line at any depth — at the
/// top level for a phone, which has no drive, or under a `Drive(N)` —
/// and its fields are the lines indented further than it until one that
/// is not. Only volumes whose monitor is MTP's or gPhoto2's, or whose
/// activation root has their scheme, are kept: the rest are UDisks2's
/// drives, which this crate already lists directly.
pub fn volumes(text: &str) -> Vec<GioVolume> {
    let lines: Vec<&str> = text.lines().collect();
    let mut found = Vec::new();
    let mut i = 0;
    while i < lines.len() {
        let line = lines[i];
        let depth = indent(line);
        let Some(label) = header(line.trim_start(), "Volume") else {
            i += 1;
            continue;
        };
        let mut monitor = None;
        let mut uri = None;
        let mut unix_device = None;
        let mut j = i + 1;
        while j < lines.len() && (lines[j].trim().is_empty() || indent(lines[j]) > depth) {
            let field = lines[j].trim();
            // A mount nested under the volume ends its own fields; its
            // `Type:` is the mount's, not the volume's.
            if header(field, "Mount").is_some() {
                j += 1;
                while j < lines.len() && indent(lines[j]) > depth + 2 {
                    j += 1;
                }
                continue;
            }
            if let Some(t) = field.strip_prefix("Type: ") {
                monitor = Some(t.to_string());
            } else if let Some(root) = field.strip_prefix("activation_root=") {
                uri = Some(root.to_string());
            } else if let Some(dev) = field.strip_prefix("unix-device: ") {
                unix_device = Some(dev.trim_matches('\'').to_string());
            }
            j += 1;
        }
        i = j;
        let Some(uri) = uri else { continue };
        let by_scheme = uri.split_once("://").and_then(|(s, _)| GadgetKind::from_scheme(s));
        let by_monitor = monitor.as_deref().and_then(|m| {
            if m.contains("VolumeMonitorMTP") {
                Some(GadgetKind::Phone)
            } else if m.contains("VolumeMonitorGPhoto2") {
                Some(GadgetKind::Camera)
            } else {
                None
            }
        });
        if let Some(kind) = by_scheme.or(by_monitor) {
            found.push(GioVolume { label: label.to_string(), kind, uri, unix_device });
        }
    }
    found
}

fn indent(line: &str) -> usize {
    line.len() - line.trim_start().len()
}

/// `Volume(0): SAMSUNG Android` → `SAMSUNG Android`.
fn header<'a>(line: &'a str, word: &str) -> Option<&'a str> {
    let rest = line.strip_prefix(word)?.strip_prefix('(')?;
    let (n, rest) = rest.split_once(')')?;
    if !n.chars().all(|c| c.is_ascii_digit()) {
        return None;
    }
    rest.strip_prefix(": ").map(str::trim)
}

/// A USB device, as `/sys/bus/usb/devices/<name>` describes it.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct UsbDevice {
    /// `busnum` and `devnum`, which name its node under `/dev/bus/usb`.
    pub bus: u32,
    pub dev: u32,
    pub manufacturer: Option<String>,
    pub product: Option<String>,
    /// Each interface's `bInterfaceClass` (two hex digits, as sysfs
    /// writes it) and its `interface` string, when it has one.
    pub interfaces: Vec<(String, Option<String>)>,
}

/// A phone or camera on USB.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UsbGadget {
    pub label: String,
    pub kind: GadgetKind,
    /// `/dev/bus/usb/001/008`, the form `gio` prints.
    pub node: String,
}

/// Which USB devices are phones or cameras.
///
/// From Android's own descriptors (`frameworks/av`'s
/// `MtpDescriptors.cpp`): MTP and PTP are both interface class `06`,
/// Still Image, and MTP's interface carries the string `"MTP"`. So
/// `"MTP"` is a phone, whatever the class (older phones put it on a
/// vendor class, `ff`), and class `06` without it is PTP — a camera, or
/// a phone set to "PTP" in its USB menu.
pub fn usb(devices: &[UsbDevice]) -> Vec<UsbGadget> {
    devices
        .iter()
        .filter_map(|d| {
            let mtp = d.interfaces.iter().any(|(_, name)| name.as_deref().map(str::trim) == Some("MTP"));
            let ptp = d.interfaces.iter().any(|(class, _)| class.trim() == "06");
            let kind = if mtp {
                GadgetKind::Phone
            } else if ptp {
                GadgetKind::Camera
            } else {
                return None;
            };
            Some(UsbGadget { label: usb_label(d, kind), kind, node: format!("/dev/bus/usb/{:03}/{:03}", d.bus, d.dev) })
        })
        .collect()
}

/// "Google Pixel 8": the maker, then the product — once, because plenty
/// of products already begin with their maker's name.
fn usb_label(d: &UsbDevice, kind: GadgetKind) -> String {
    let maker = d.manufacturer.as_deref().map(str::trim).filter(|s| !s.is_empty());
    let product = d.product.as_deref().map(str::trim).filter(|s| !s.is_empty());
    match (maker, product) {
        (Some(m), Some(p)) if p.to_lowercase().starts_with(&m.to_lowercase()) => p.to_string(),
        (Some(m), Some(p)) => format!("{m} {p}"),
        (None, Some(p)) => p.to_string(),
        (Some(m), None) => format!("{m} {}", kind.noun()),
        (None, None) => format!("A {}", kind.noun()),
    }
}

/// Whether a gvfs FUSE directory name is a phone's or a camera's —
/// those are listed here, not as network shares.
pub fn is_gadget_mount(name: &str) -> bool {
    name.split_once(':').is_some_and(|(kind, _)| GadgetKind::from_scheme(kind).is_some())
}

/// Puts it together: what gio listed, with each one's FUSE directory if
/// it is mounted, and what is on USB that no backend here can open.
///
/// `fuse_names` are the entries of gvfs's FUSE root; `have` is whether
/// the backend for each kind is installed. A USB device whose node a
/// gio volume names is that volume, and is never also unreadable.
pub fn assemble(
    gio: &[GioVolume],
    fuse_root: &Path,
    fuse_names: &[String],
    usb: &[UsbGadget],
    have: impl Fn(GadgetKind) -> bool,
) -> Gadgets {
    let list = gio
        .iter()
        .map(|v| Gadget {
            label: v.label.clone(),
            kind: v.kind,
            uri: v.uri.clone(),
            mounted: fuse_names.iter().find(|name| mounted_as(name, v)).map(|name| fuse_root.join(name)),
        })
        .collect();
    let unreadable = usb
        .iter()
        .filter(|u| !have(u.kind) && !gio.iter().any(|v| v.unix_device.as_deref() == Some(u.node.as_str())))
        .map(|u| Unreadable { label: u.label.clone(), kind: u.kind })
        .collect();
    Gadgets { list, unreadable }
}

/// Whether `name` — `mtp:host=SAMSUNG_SAMSUNG_Android_R58M83EFHJY` — is
/// the FUSE directory of `volume`, by kind and host. Both sides are
/// compared unescaped: gvfs escapes a camera's `[usb:001,005]` in the
/// directory name and not in the URI.
fn mounted_as(name: &str, volume: &GioVolume) -> bool {
    let Some((kind, spec)) = name.split_once(':') else { return false };
    if kind != volume.kind.scheme() {
        return false;
    }
    let host = spec
        .split(',')
        .find_map(|pair| pair.strip_prefix("host="))
        .map(crate::gvfs::unescape);
    let wanted = volume
        .uri
        .split_once("://")
        .map(|(_, rest)| crate::gvfs::unescape(rest.trim_end_matches('/')));
    host.is_some() && host == wanted
}

/// What to say when opening `label` failed, from gio's own words.
///
/// The strings matched are gvfs's (`gvfsbackendmtp.c`) and libgphoto2's
/// (`gphoto2-port-result.c`), in English because the runner asks for
/// `C.UTF-8`. Each match is a different thing to do: a locked phone
/// wants unlocking, a camera another program holds wants that program
/// closed.
pub fn failure(kind: GadgetKind, label: &str, said: &str) -> String {
    let name = format!("\u{201C}{label}\u{201D}");
    if said.contains("Unable to open MTP device") || said.contains("Unable to connect to MTP device") {
        return format!(
            "Couldn't open {name}. Unlock it, and if it asks, choose File transfer in its USB notification — then try again."
        );
    }
    if said.contains("Could not claim the USB device") || said.contains("Could not lock the device") {
        return format!("Another program is using {name} — close it (a photo importer, say), then try again.");
    }
    if said.contains("No MTP devices found") || said.contains("Device not found") {
        return format!("{name} is no longer connected.");
    }
    let said = said.trim();
    if said.is_empty() {
        format!("Couldn't open the {} {name}.", kind.noun())
    } else {
        format!("Couldn't open {name}: {said}")
    }
}

/// What an empty phone means. An Android phone that is locked, or set
/// to charge only, mounts and then lists nothing at all — no storage —
/// which looks exactly like an empty phone and never is one.
pub const EMPTY_PHONE: &str =
    "Nothing on this phone can be seen yet. Unlock it, and choose File transfer in its USB notification.";

#[cfg(test)]
mod tests {
    use super::*;

    /// `gio mount -li` with a Samsung phone mounted, from
    /// github.com/linuxmint/nemo issue 3758 (2025), verbatim — the
    /// drives above it in that report were omitted by its author.
    const SAMSUNG: &str = "\
Volume(0): SAMSUNG Android
  Type: GProxyVolume (GProxyVolumeMonitorMTP)
  ids:
   unix-device: '/dev/bus/usb/001/008'
  activation_root=mtp://SAMSUNG_SAMSUNG_Android_R58M83EFHJY/
  themed icons:  [multimedia-player]
  symbolic themed icons:  [multimedia-player-symbolic]  [multimedia-symbolic]  [multimedia-player]  [multimedia]
  can_mount=1
  can_eject=0
  should_automount=1
  Mount(0): SAMSUNG Android -> mtp://SAMSUNG_SAMSUNG_Android_R58M83EFHJY/
    Type: GProxyShadowMount (GProxyVolumeMonitorMTP)
    default_location=mtp://SAMSUNG_SAMSUNG_Android_R58M83EFHJY/
    themed icons:  [multimedia-player]
    symbolic themed icons:  [multimedia-player-symbolic]  [multimedia-symbolic]  [multimedia-player]  [multimedia]
    can_unmount=1
    can_eject=0
    is_shadowed=0
Mount(1): mtp -> mtp://SAMSUNG_SAMSUNG_Android_R58M83EFHJY/
  Type: GDaemonMount
  default_location=mtp://SAMSUNG_SAMSUNG_Android_R58M83EFHJY/
  themed icons:  [multimedia-player]  [multimedia]  [multimedia-player-symbolic]  [multimedia-symbolic]
  symbolic themed icons:  [drive-removable-media-symbolic]
  can_unmount=1
  can_eject=0
  is_shadowed=1
";

    /// `gvfs-mount -li` with a Nikon camera, from Launchpad bug 576178,
    /// verbatim.
    const NIKON: &str = "\
Volume(0): NIKON DSC COOLPIX S203-PTP
  Type: GProxyVolume (GProxyVolumeMonitorGPhoto2)
  ids:
   unix-device: '/dev/bus/usb/001/005'
  activation_root=gphoto2://[usb:001,005]/
  themed icons:  [camera-photo]
  can_mount=1
  can_eject=0
  should_automount=1
  Mount(0): NIKON DSC COOLPIX S203-PTP -> gphoto2://[usb:001,005]/
    Type: GProxyShadowMount (GProxyVolumeMonitorGPhoto2)
    default_location=gphoto2://[usb:001,005]/
    themed icons:  [camera-photo]
    x_content_types: x-content/image-dcf
    can_unmount=1
    can_eject=0
    is_shadowed=0
Mount(1): NIKON DSC COOLPIX S203-PTP -> gphoto2://[usb:001,005]/
  Type: GDaemonMount
  default_location=gphoto2://[usb:001,005]/
  themed icons:  [camera-photo]  [camera]
  x_content_types: x-content/image-dcf
  can_unmount=1
  can_eject=0
  is_shadowed=1
";

    /// The start of `gio mount -li` on the machine this was written on:
    /// UDisks2's drives, one with a volume nested under it.
    const DRIVES: &str = "\
Drive(1): 2550 Micron 512GB
  Type: GProxyDrive (GProxyVolumeMonitorUDisks2)
  ids:
   unix-device: '/dev/nvme1n1'
  is_removable=0
  sort_key=00coldplug/00fixed/nvme1
  Volume(0): DEV-LT-001 C: 12/29/2025
    Type: GProxyVolume (GProxyVolumeMonitorUDisks2)
    ids:
     class: 'device'
     unix-device: '/dev/nvme1n1p3'
     uuid: '311e9a8b-b855-47a3-8112-5f703dc6e18a'
     label: 'DEV-LT-001 C: 12/29/2025'
    uuid=311e9a8b-b855-47a3-8112-5f703dc6e18a
    can_mount=1
    can_eject=0
    should_automount=0
";

    #[test]
    fn a_phone_and_a_camera_are_read_from_real_gio_output() {
        let text = format!("{DRIVES}{SAMSUNG}{NIKON}");
        assert_eq!(
            volumes(&text),
            [
                GioVolume {
                    label: "SAMSUNG Android".into(),
                    kind: GadgetKind::Phone,
                    uri: "mtp://SAMSUNG_SAMSUNG_Android_R58M83EFHJY/".into(),
                    unix_device: Some("/dev/bus/usb/001/008".into()),
                },
                GioVolume {
                    label: "NIKON DSC COOLPIX S203-PTP".into(),
                    kind: GadgetKind::Camera,
                    uri: "gphoto2://[usb:001,005]/".into(),
                    unix_device: Some("/dev/bus/usb/001/005".into()),
                },
            ]
        );
    }

    /// UDisks2's volumes are this crate's to list directly; reading them
    /// again here would show every stick twice.
    #[test]
    fn a_drives_volume_is_not_a_gadget() {
        assert!(volumes(DRIVES).is_empty());
    }

    /// The same phone before it is mounted: the sample with its mounts
    /// taken out, which is the shape gio prints for an unmounted volume.
    #[test]
    fn an_unmounted_phone_is_still_listed() {
        let unmounted: String = SAMSUNG.lines().take(10).map(|l| format!("{l}\n")).collect();
        assert_eq!(volumes(&unmounted).len(), 1);
    }

    #[test]
    fn a_mounted_phone_and_camera_find_their_fuse_directories() {
        let gio = volumes(&format!("{SAMSUNG}{NIKON}"));
        let root = Path::new("/run/user/1000/gvfs");
        // The phone's name is from the same issue; the camera's is gvfs's
        // escaping of its host.
        let names = vec![
            "mtp:host=SAMSUNG_SAMSUNG_Android_R58M83EFHJY".to_string(),
            "gphoto2:host=%5Busb%3A001%2C005%5D".to_string(),
            "smb-share:server=nas,share=music".to_string(),
        ];
        let found = assemble(&gio, root, &names, &[], |_| true);
        assert_eq!(found.list[0].mounted, Some(root.join(&names[0])));
        assert_eq!(found.list[1].mounted, Some(root.join(&names[1])));
        let none = assemble(&gio, root, &names[2..], &[], |_| true);
        assert!(none.list.iter().all(|g| g.mounted.is_none()));
    }

    fn pixel(interfaces: Vec<(&str, Option<&str>)>) -> UsbDevice {
        UsbDevice {
            bus: 1,
            dev: 9,
            manufacturer: Some("Google".into()),
            product: Some("Pixel 8".into()),
            interfaces: interfaces.into_iter().map(|(c, n)| (c.to_string(), n.map(str::to_string))).collect(),
        }
    }

    #[test]
    fn mtp_is_a_phone_and_bare_still_image_is_a_camera() {
        let mtp = usb(&[pixel(vec![("06", Some("MTP"))])]);
        assert_eq!(mtp[0].kind, GadgetKind::Phone);
        assert_eq!(mtp[0].label, "Google Pixel 8");
        assert_eq!(mtp[0].node, "/dev/bus/usb/001/009");
        let vendor_class = usb(&[pixel(vec![("ff", Some("MTP"))])]);
        assert_eq!(vendor_class[0].kind, GadgetKind::Phone, "older phones put MTP on a vendor class");
        let ptp = usb(&[pixel(vec![("06", None)])]);
        assert_eq!(ptp[0].kind, GadgetKind::Camera);
        let mouse = usb(&[pixel(vec![("03", Some("2.4G Dual Mode Mouse"))])]);
        assert!(mouse.is_empty());
    }

    #[test]
    fn a_product_that_begins_with_its_maker_is_not_named_twice() {
        let samsung = UsbDevice {
            manufacturer: Some("SAMSUNG".into()),
            product: Some("SAMSUNG_Android".into()),
            interfaces: vec![("06".into(), Some("MTP".into()))],
            ..UsbDevice::default()
        };
        assert_eq!(usb(&[samsung])[0].label, "SAMSUNG_Android");
    }

    /// The point of reading sysfs at all: without gvfs-mtp the phone is
    /// still there, and its row says what would open it.
    #[test]
    fn a_phone_with_no_backend_says_which_package_opens_it() {
        let on_usb = usb(&[pixel(vec![("06", Some("MTP"))])]);
        let found = assemble(&[], Path::new("/g"), &[], &on_usb, |_| false);
        assert!(found.list.is_empty());
        assert_eq!(found.unreadable[0].sentence(), "Install gvfs-mtp to open \u{201C}Google Pixel 8\u{201D}");
        let installed = assemble(&[], Path::new("/g"), &[], &on_usb, |_| true);
        assert!(installed.unreadable.is_empty(), "installed, just not listed yet: no advice to install it");
    }

    /// A camera gvfs-mtp claimed while gvfs-gphoto2 is missing is
    /// opened by gvfs-mtp — the node matches — and so is not unreadable.
    #[test]
    fn a_device_a_monitor_claimed_is_never_also_unreadable() {
        let gio = volumes(NIKON);
        let on_usb = vec![UsbGadget { label: "NIKON".into(), kind: GadgetKind::Camera, node: "/dev/bus/usb/001/005".into() }];
        assert!(assemble(&gio, Path::new("/g"), &[], &on_usb, |_| false).unreadable.is_empty());
    }

    #[test]
    fn a_gadgets_fuse_directory_is_not_a_share() {
        assert!(is_gadget_mount("mtp:host=SAMSUNG_SAMSUNG_Android_R58M83EFHJY"));
        assert!(is_gadget_mount("gphoto2:host=%5Busb%3A001%2C005%5D"));
        assert!(!is_gadget_mount("sftp:host=box,user=u"));
    }

    /// gvfs's and libgphoto2's own strings, each to what a person does
    /// about it.
    #[test]
    fn a_locked_phone_and_a_busy_camera_never_read_alike() {
        let locked = failure(GadgetKind::Phone, "Pixel 8", "gio: mtp://x/: Unable to open MTP device \u{201C}001,009\u{201D}");
        let busy = failure(GadgetKind::Camera, "NIKON", "Error initializing camera: -60: Could not lock the device");
        let gone = failure(GadgetKind::Phone, "Pixel 8", "Device not found");
        assert!(locked.contains("Unlock it"), "{locked}");
        assert!(busy.contains("Another program"), "{busy}");
        assert!(gone.contains("no longer connected"), "{gone}");
        assert!(failure(GadgetKind::Phone, "P", "Something else").ends_with("Something else"));
    }
}
