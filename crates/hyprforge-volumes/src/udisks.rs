//! The real [`VolumeBackend`], talking to UDisks2 on the system bus.
//!
//! Everything here is D-Bus mechanics. Which block devices are volumes,
//! what they are called and how each lets go are [`crate::inventory`]'s
//! decisions over plain data; this file turns UDisks2's property maps
//! into that data ([`raw`]) and turns a person's request into one of its
//! methods.
//!
//! # No root, and polkit decides
//!
//! UDisks2 mounts on behalf of whoever asks, under polkit: on a normal
//! desktop the person at the seat may mount and power off removable
//! drives without a password (`filesystem-mount` is `allow_active: yes`),
//! and anything more needs an administrator's password typed into the
//! session's polkit agent. So every call leaves interaction on — a prompt
//! is how polkit asks — and the bound on [`MOUNT_TIMEOUT`] is long enough
//! for a person to type one. A denial comes back as
//! [`VolumeError::NotAuthorized`] or, when the prompt was cancelled,
//! [`VolumeError::Dismissed`].

use crate::backend::VolumeBackend;
use crate::inventory::{self, RawBlock, RawDrive};
use crate::types::{Volume, VolumeError, VolumeId};
use futures_util::StreamExt;
use std::collections::HashMap;
use std::path::PathBuf;
use std::time::Duration;
use tokio::sync::{mpsc, OnceCell};
use zbus::fdo::ManagedObjects;
use zbus::zvariant::{OwnedValue, Value};
use zbus::Connection;

/// Listing, and connecting to the bus: nothing that waits on hardware.
pub const TIMEOUT: Duration = Duration::from_secs(5);

/// Mounting. Long, because polkit may be asking someone for a password
/// in the middle of it, and a person typing one is not a service that
/// stopped answering.
pub const MOUNT_TIMEOUT: Duration = Duration::from_secs(120);

/// Unmounting and ejecting. Longer still: an unmount waits for every
/// write still buffered for the drive, and a stick that was just sent
/// four gigabytes can take minutes to drain. Stopping early only stops
/// the *waiting* — UDisks2 carries on — and the sentence for it says not
/// to unplug yet.
pub const UNMOUNT_TIMEOUT: Duration = Duration::from_secs(300);

const SERVICE: &str = "org.freedesktop.UDisks2";
const ROOT: &str = "/org/freedesktop/UDisks2";
const BLOCK: &str = "org.freedesktop.UDisks2.Block";
const FILESYSTEM: &str = "org.freedesktop.UDisks2.Filesystem";
const DRIVE: &str = "org.freedesktop.UDisks2.Drive";
const LOOP: &str = "org.freedesktop.UDisks2.Loop";
const ENCRYPTED: &str = "org.freedesktop.UDisks2.Encrypted";

#[zbus::proxy(interface = "org.freedesktop.UDisks2.Filesystem", default_service = "org.freedesktop.UDisks2")]
trait Filesystem {
    fn mount(&self, options: HashMap<&str, Value<'_>>) -> zbus::Result<String>;
    fn unmount(&self, options: HashMap<&str, Value<'_>>) -> zbus::Result<()>;
}

#[zbus::proxy(interface = "org.freedesktop.UDisks2.Drive", default_service = "org.freedesktop.UDisks2")]
trait Drive {
    fn eject(&self, options: HashMap<&str, Value<'_>>) -> zbus::Result<()>;
    fn power_off(&self, options: HashMap<&str, Value<'_>>) -> zbus::Result<()>;
}

#[zbus::proxy(interface = "org.freedesktop.UDisks2.Loop", default_service = "org.freedesktop.UDisks2")]
trait Loop {
    fn delete(&self, options: HashMap<&str, Value<'_>>) -> zbus::Result<()>;
}

/// Bounds a call and says what its failure means.
async fn bounded<T>(limit: Duration, call: impl std::future::Future<Output = zbus::Result<T>>) -> Result<T, VolumeError> {
    match tokio::time::timeout(limit, call).await {
        Err(_) => Err(VolumeError::TimedOut(limit)),
        Ok(Ok(value)) => Ok(value),
        Ok(Err(e)) => Err(classify(e)),
    }
}

/// Which failure this is, by UDisks2's own error names — the
/// distinctions [`VolumeError`] exists to keep.
fn classify(e: zbus::Error) -> VolumeError {
    if let zbus::Error::MethodError(name, message, _) = &e {
        return classify_name(name.as_str(), message.clone().unwrap_or_default());
    }
    match e {
        zbus::Error::Address(_) | zbus::Error::InputOutput(_) => VolumeError::Unavailable,
        zbus::Error::FDO(fdo) => match *fdo {
            zbus::fdo::Error::ServiceUnknown(_) | zbus::fdo::Error::NameHasNoOwner(_) => VolumeError::Unavailable,
            zbus::fdo::Error::UnknownObject(_) | zbus::fdo::Error::UnknownMethod(_) | zbus::fdo::Error::UnknownInterface(_) => {
                VolumeError::Gone
            }
            other => VolumeError::Refused(other.to_string()),
        },
        other => VolumeError::Refused(other.to_string()),
    }
}

/// The name half of [`classify`], pure so the mapping is a test.
fn classify_name(name: &str, message: String) -> VolumeError {
    match name.rsplit('.').next().unwrap_or("") {
        "DeviceBusy" => VolumeError::Busy(message),
        "NotAuthorized" | "NotAuthorizedCanObtain" => VolumeError::NotAuthorized(message),
        "NotAuthorizedDismissed" | "Cancelled" => VolumeError::Dismissed,
        "ServiceUnknown" | "NameHasNoOwner" => VolumeError::Unavailable,
        "UnknownObject" | "UnknownMethod" | "UnknownInterface" => VolumeError::Gone,
        _ => VolumeError::Refused(message),
    }
}

/// The user this process runs as — `SetupByUID` is compared with it.
/// Read from `/proc/self`'s owner rather than a `libc` call this crate
/// would carry for one integer.
fn own_uid() -> u32 {
    use std::os::unix::fs::MetadataExt;
    std::fs::metadata("/proc/self").map(|m| m.uid()).unwrap_or(u32::MAX)
}

/// UDisks2, as this crate uses it.
pub struct UDisks2Backend {
    /// Connected on first use, and only a connection that worked is
    /// kept: a failed one is never cached, or "UDisks2 isn't running"
    /// would go on being said after the bus came back — the mistake
    /// CLAUDE.md says this project has made three times.
    connection: OnceCell<Connection>,
    uid: u32,
}

impl Default for UDisks2Backend {
    fn default() -> Self {
        Self::new()
    }
}

impl UDisks2Backend {
    /// A backend that connects when first asked something. Never fails:
    /// no bus is an answer to a question, not to construction.
    pub fn new() -> Self {
        UDisks2Backend { connection: OnceCell::new(), uid: own_uid() }
    }

    async fn connection(&self) -> Result<&Connection, VolumeError> {
        self.connection
            .get_or_try_init(|| async { bounded(TIMEOUT, Connection::system()).await })
            .await
    }

    async fn objects(&self) -> Result<ManagedObjects, VolumeError> {
        let connection = self.connection().await?;
        let manager = bounded(
            TIMEOUT,
            zbus::fdo::ObjectManagerProxy::builder(connection).destination(SERVICE)?.path(ROOT)?.build(),
        )
        .await?;
        match tokio::time::timeout(TIMEOUT, manager.get_managed_objects()).await {
            Err(_) => Err(VolumeError::TimedOut(TIMEOUT)),
            Ok(Ok(objects)) => Ok(objects),
            Ok(Err(e)) => Err(classify(e.into())),
        }
    }

    /// Every block device and drive UDisks2 lists, before
    /// [`inventory::assemble`] decides which are volumes — what the live
    /// tier checks the marshalling against.
    pub async fn raw(&self) -> Result<(Vec<RawBlock>, Vec<RawDrive>), VolumeError> {
        Ok(raw(&self.objects().await?))
    }

    async fn filesystem(&self, id: &VolumeId) -> Result<FilesystemProxy<'static>, VolumeError> {
        let connection = self.connection().await?;
        bounded(TIMEOUT, FilesystemProxy::builder(connection).path(id.0.clone())?.build()).await
    }
}

impl From<zbus::Error> for VolumeError {
    fn from(e: zbus::Error) -> Self {
        classify(e)
    }
}

/// Interaction left on — see the module doc.
fn options() -> HashMap<&'static str, Value<'static>> {
    HashMap::new()
}

#[async_trait::async_trait]
impl VolumeBackend for UDisks2Backend {
    async fn volumes(&self) -> Result<Vec<Volume>, VolumeError> {
        let (blocks, drives) = raw(&self.objects().await?);
        Ok(inventory::assemble(&blocks, &drives, self.uid))
    }

    async fn mount(&self, id: &VolumeId) -> Result<PathBuf, VolumeError> {
        let objects = self.objects().await?;
        let (blocks, _) = raw(&objects);
        let block = blocks.iter().find(|b| b.id == id.0).ok_or(VolumeError::Gone)?;
        if !block.has_filesystem {
            return Err(if block.encrypted { VolumeError::Locked } else { VolumeError::Refused("there is no filesystem on it to open".into()) });
        }
        // Mounted already — by an automounter, or another window: that
        // is the answer to "mount it", not an error about it.
        if let Some(point) = block.mount_points.first() {
            return Ok(point.clone());
        }
        let filesystem = self.filesystem(id).await?;
        match bounded(MOUNT_TIMEOUT, filesystem.mount(options())).await {
            Ok(point) => Ok(PathBuf::from(point)),
            Err(VolumeError::Refused(m)) if m.contains("already mounted") => {
                let (blocks, _) = raw(&self.objects().await?);
                blocks
                    .iter()
                    .find(|b| b.id == id.0)
                    .and_then(|b| b.mount_points.first().cloned())
                    .ok_or(VolumeError::Refused(m))
            }
            Err(e) => Err(e),
        }
    }

    async fn unmount(&self, id: &VolumeId) -> Result<(), VolumeError> {
        let filesystem = self.filesystem(id).await?;
        match bounded(UNMOUNT_TIMEOUT, filesystem.unmount(options())).await {
            // Not mounted is what was wanted.
            Err(VolumeError::Refused(m)) if m.contains("not mounted") => Ok(()),
            other => other,
        }
    }

    async fn eject(&self, id: &VolumeId) -> Result<(), VolumeError> {
        let objects = self.objects().await?;
        let (blocks, drives) = raw(&objects);
        let block = blocks.iter().find(|b| b.id == id.0).ok_or(VolumeError::Gone)?;
        let connection = self.connection().await?;
        // Everything mounted on the same drive goes first — powering off
        // a stick with its second partition still mounted would be the
        // unsafe removal Eject exists to prevent. A disk image has no
        // drive; only itself.
        let siblings: Vec<&RawBlock> = match &block.drive {
            Some(drive) => blocks.iter().filter(|b| b.drive.as_ref() == Some(drive)).collect(),
            None => vec![block],
        };
        for sibling in siblings.iter().filter(|b| !b.mount_points.is_empty()) {
            self.unmount(&VolumeId(sibling.id.clone())).await?;
        }
        if block.loop_setup_by.is_some() {
            let image = bounded(TIMEOUT, LoopProxy::builder(connection).path(id.as_str())?.build()).await?;
            return bounded(UNMOUNT_TIMEOUT, image.delete(options())).await;
        }
        let Some(drive_id) = &block.drive else { return Ok(()) };
        let drive = drives.iter().find(|d| &d.id == drive_id).ok_or(VolumeError::Gone)?;
        let proxy = bounded(TIMEOUT, DriveProxy::builder(connection).path(drive_id.as_str())?.build()).await?;
        if drive.can_power_off {
            bounded(UNMOUNT_TIMEOUT, proxy.power_off(options())).await
        } else if drive.ejectable {
            bounded(UNMOUNT_TIMEOUT, proxy.eject(options())).await
        } else {
            // Unmounted, which is as far as this drive goes.
            Ok(())
        }
    }

    async fn watch(&self) -> Result<mpsc::Receiver<()>, VolumeError> {
        let connection = self.connection().await?.clone();
        // Every signal UDisks2 sends from under its root — objects added
        // and removed, properties changed — and the bus's word when
        // UDisks2 itself starts or stops.
        let objects = zbus::MatchRule::builder()
            .msg_type(zbus::message::Type::Signal)
            .sender(SERVICE)?
            .path_namespace(ROOT)?
            .build();
        let owner = zbus::MatchRule::builder()
            .msg_type(zbus::message::Type::Signal)
            .sender("org.freedesktop.DBus")?
            .interface("org.freedesktop.DBus")?
            .member("NameOwnerChanged")?
            .add_arg(SERVICE)?
            .build();
        let mut objects = bounded(TIMEOUT, zbus::MessageStream::for_match_rule(objects, &connection, Some(64))).await?;
        let mut owner = bounded(TIMEOUT, zbus::MessageStream::for_match_rule(owner, &connection, Some(8))).await?;
        // Capacity one: a signal is "look again", so a second while one
        // is waiting adds nothing and is dropped.
        let (tx, rx) = mpsc::channel(1);
        tokio::spawn(async move {
            loop {
                let next = tokio::select! {
                    m = objects.next() => m,
                    m = owner.next() => m,
                };
                if next.is_none() || tx.is_closed() {
                    break;
                }
                let _ = tx.try_send(());
            }
        });
        Ok(rx)
    }
}

/// UDisks2's objects as the plain data [`crate::inventory`] reads.
///
/// Marshalling only, and forgiving: a property missing or of an
/// unexpected type reads as its empty value, so one odd device cannot
/// hide every other. Whether these names and types are UDisks2's is
/// what the live tier (`tests/live_udisks.rs`) asks the real one.
pub fn raw(objects: &ManagedObjects) -> (Vec<RawBlock>, Vec<RawDrive>) {
    let mut blocks = Vec::new();
    let mut drives = Vec::new();
    for (path, interfaces) in objects {
        let iface = |name: &str| interfaces.iter().find(|(k, _)| k.as_str() == name).map(|(_, v)| v);
        if let Some(block) = iface(BLOCK) {
            let filesystem = iface(FILESYSTEM);
            blocks.push(RawBlock {
                id: path.to_string(),
                device: PathBuf::from(nul_trimmed(bytes(block.get("Device")))),
                drive: object(block.get("Drive")),
                size: u64_of(block.get("Size")),
                id_label: string(block.get("IdLabel")),
                id_usage: string(block.get("IdUsage")),
                id_type: string(block.get("IdType")),
                hint_ignore: bool_of(block.get("HintIgnore")),
                hint_system: bool_of(block.get("HintSystem")),
                hint_name: string(block.get("HintName")),
                crypto_backing: object(block.get("CryptoBackingDevice")),
                has_filesystem: filesystem.is_some(),
                mount_points: filesystem
                    .map(|f| byte_arrays(f.get("MountPoints")).into_iter().map(|p| PathBuf::from(nul_trimmed(p))).collect())
                    .unwrap_or_default(),
                loop_setup_by: iface(LOOP).map(|l| u32_of(l.get("SetupByUID"))),
                encrypted: iface(ENCRYPTED).is_some(),
            });
        }
        if let Some(drive) = iface(DRIVE) {
            let compatibility = strings(drive.get("MediaCompatibility"));
            drives.push(RawDrive {
                id: path.to_string(),
                removable: bool_of(drive.get("Removable")),
                media_removable: bool_of(drive.get("MediaRemovable")),
                ejectable: bool_of(drive.get("Ejectable")),
                can_power_off: bool_of(drive.get("CanPowerOff")),
                optical: bool_of(drive.get("Optical")) || compatibility.iter().any(|c| c.starts_with("optical")),
                connection_bus: string(drive.get("ConnectionBus")),
            });
        }
    }
    (blocks, drives)
}

fn bool_of(v: Option<&OwnedValue>) -> bool {
    matches!(v.map(|v| &**v), Some(Value::Bool(true)))
}

fn u64_of(v: Option<&OwnedValue>) -> u64 {
    match v.map(|v| &**v) {
        Some(Value::U64(n)) => *n,
        _ => 0,
    }
}

fn u32_of(v: Option<&OwnedValue>) -> u32 {
    match v.map(|v| &**v) {
        Some(Value::U32(n)) => *n,
        _ => u32::MAX,
    }
}

fn string(v: Option<&OwnedValue>) -> String {
    match v.map(|v| &**v) {
        Some(Value::Str(s)) => s.as_str().to_string(),
        _ => String::new(),
    }
}

fn strings(v: Option<&OwnedValue>) -> Vec<String> {
    match v.map(|v| &**v) {
        Some(Value::Array(a)) => a
            .iter()
            .filter_map(|x| match x {
                Value::Str(s) => Some(s.as_str().to_string()),
                _ => None,
            })
            .collect(),
        _ => Vec::new(),
    }
}

/// An object path, `None` for UDisks2's "nothing", which is `/`.
fn object(v: Option<&OwnedValue>) -> Option<String> {
    match v.map(|v| &**v) {
        Some(Value::ObjectPath(p)) if p.as_str() != "/" => Some(p.as_str().to_string()),
        _ => None,
    }
}

fn array_bytes(v: &Value<'_>) -> Vec<u8> {
    match v {
        Value::Array(a) => a
            .iter()
            .filter_map(|x| match x {
                Value::U8(b) => Some(*b),
                _ => None,
            })
            .collect(),
        _ => Vec::new(),
    }
}

fn bytes(v: Option<&OwnedValue>) -> Vec<u8> {
    v.map(|v| array_bytes(v)).unwrap_or_default()
}

fn byte_arrays(v: Option<&OwnedValue>) -> Vec<Vec<u8>> {
    match v.map(|v| &**v) {
        Some(Value::Array(a)) => a.iter().map(array_bytes).collect(),
        _ => Vec::new(),
    }
}

/// UDisks2 sends paths as NUL-terminated byte strings (`ay`), so a path
/// that is not UTF-8 survives — and the NUL has to come off.
fn nul_trimmed(mut bytes: Vec<u8>) -> String {
    while bytes.last() == Some(&0) {
        bytes.pop();
    }
    use std::os::unix::ffi::OsStringExt;
    std::ffi::OsString::from_vec(bytes).to_string_lossy().into_owned()
}

#[cfg(test)]
mod tests {
    use super::*;
    use zbus::names::OwnedInterfaceName;
    use zbus::zvariant::{Array, ObjectPath, OwnedObjectPath};

    fn owned(v: Value<'static>) -> OwnedValue {
        OwnedValue::try_from(v).unwrap()
    }

    fn nul_bytes(s: &str) -> Value<'static> {
        let mut b = s.as_bytes().to_vec();
        b.push(0);
        Value::from(b)
    }

    /// The card and its drive as `udisksctl dump` printed them on the
    /// machine this was written on, rebuilt as the property maps
    /// `GetManagedObjects` carries — `ay` paths with their NUL, `o`
    /// drive paths, `/` for "no crypto backing".
    fn sample() -> ManagedObjects {
        let mut objects = ManagedObjects::new();
        let drive_path = "/org/freedesktop/UDisks2/drives/1TB_Card_500014a000000001";
        let mut block = HashMap::new();
        block.insert("Device".to_string(), owned(nul_bytes("/dev/sda1")));
        block.insert("Drive".to_string(), owned(Value::from(ObjectPath::try_from(drive_path).unwrap())));
        block.insert("Size".to_string(), owned(Value::U64(1_000_202_043_392)));
        block.insert("IdLabel".to_string(), owned(Value::from("Development")));
        block.insert("IdUsage".to_string(), owned(Value::from("filesystem")));
        block.insert("IdType".to_string(), owned(Value::from("ntfs")));
        block.insert("HintIgnore".to_string(), owned(Value::Bool(false)));
        block.insert("HintSystem".to_string(), owned(Value::Bool(false)));
        block.insert("HintName".to_string(), owned(Value::from("")));
        block.insert("CryptoBackingDevice".to_string(), owned(Value::from(ObjectPath::try_from("/").unwrap())));
        let mut filesystem = HashMap::new();
        let points: Array<'static> = Array::from(vec![{
            let mut b = b"/run/media/apost/Development".to_vec();
            b.push(0);
            b
        }]);
        filesystem.insert("MountPoints".to_string(), owned(Value::Array(points)));
        let mut interfaces = HashMap::new();
        interfaces.insert(OwnedInterfaceName::try_from(BLOCK).unwrap(), block);
        interfaces.insert(OwnedInterfaceName::try_from(FILESYSTEM).unwrap(), filesystem);
        objects.insert(OwnedObjectPath::try_from("/org/freedesktop/UDisks2/block_devices/sda1").unwrap(), interfaces);

        let mut drive = HashMap::new();
        drive.insert("Removable".to_string(), owned(Value::Bool(true)));
        drive.insert("MediaRemovable".to_string(), owned(Value::Bool(false)));
        drive.insert("Ejectable".to_string(), owned(Value::Bool(false)));
        drive.insert("CanPowerOff".to_string(), owned(Value::Bool(true)));
        drive.insert("Optical".to_string(), owned(Value::Bool(false)));
        drive.insert("ConnectionBus".to_string(), owned(Value::from("usb")));
        drive.insert("MediaCompatibility".to_string(), owned(Value::Array(Array::from(Vec::<String>::new()))));
        let mut interfaces = HashMap::new();
        interfaces.insert(OwnedInterfaceName::try_from(DRIVE).unwrap(), drive);
        objects.insert(OwnedObjectPath::try_from(drive_path).unwrap(), interfaces);
        objects
    }

    #[test]
    fn property_maps_become_raw_blocks_and_drives() {
        let (blocks, drives) = raw(&sample());
        assert_eq!(blocks.len(), 1);
        let b = &blocks[0];
        assert_eq!(b.device, PathBuf::from("/dev/sda1"), "the NUL comes off");
        assert_eq!(b.drive.as_deref(), Some("/org/freedesktop/UDisks2/drives/1TB_Card_500014a000000001"));
        assert_eq!(b.crypto_backing, None, "`/` is UDisks2's nothing");
        assert_eq!(b.mount_points, [PathBuf::from("/run/media/apost/Development")]);
        assert!(b.has_filesystem);
        assert_eq!(b.loop_setup_by, None);
        assert_eq!(drives.len(), 1);
        assert!(drives[0].can_power_off && drives[0].removable);
        assert_eq!(drives[0].connection_bus, "usb");
    }

    #[test]
    fn udisks_error_names_keep_their_distinctions() {
        let busy = classify_name("org.freedesktop.UDisks2.Error.DeviceBusy", "target is busy".into());
        assert_eq!(busy, VolumeError::Busy("target is busy".into()));
        assert!(matches!(classify_name("org.freedesktop.UDisks2.Error.NotAuthorizedCanObtain", String::new()), VolumeError::NotAuthorized(_)));
        assert_eq!(classify_name("org.freedesktop.UDisks2.Error.NotAuthorizedDismissed", String::new()), VolumeError::Dismissed);
        assert_eq!(classify_name("org.freedesktop.DBus.Error.ServiceUnknown", String::new()), VolumeError::Unavailable);
        assert_eq!(classify_name("org.freedesktop.DBus.Error.UnknownObject", String::new()), VolumeError::Gone);
        assert_eq!(classify_name("org.freedesktop.UDisks2.Error.Failed", "nope".into()), VolumeError::Refused("nope".into()));
    }

    #[test]
    fn a_missing_property_reads_as_empty_not_as_a_failure() {
        let mut objects = ManagedObjects::new();
        let mut interfaces = HashMap::new();
        interfaces.insert(OwnedInterfaceName::try_from(BLOCK).unwrap(), HashMap::new());
        objects.insert(OwnedObjectPath::try_from("/org/freedesktop/UDisks2/block_devices/odd").unwrap(), interfaces);
        let (blocks, _) = raw(&objects);
        assert_eq!(blocks[0].device, PathBuf::new());
        assert!(!blocks[0].has_filesystem);
    }
}
