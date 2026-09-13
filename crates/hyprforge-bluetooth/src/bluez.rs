//! The real [`BluetoothBackend`], talking to BlueZ on the system bus.
//!
//! Everything here is D-Bus mechanics. The decisions — what a device
//! looks like once read, how the list is ordered, what an unpaired
//! device is told — live in [`crate::types`] and [`crate::backend`],
//! where they can be tested on a machine with no `bluetoothd` at all.
//! What is left here is the part that can only be checked against the
//! service itself, which is what the `#[ignore]`d live test in
//! `tests/live_bluez.rs` is for.
//!
//! # Nothing waits forever
//!
//! Every call is wrapped in a timeout. BlueZ blocks on real radio
//! hardware: `Connect()` negotiates a link with a remote device that may
//! be out of range or busy pairing with something else. It is exactly
//! the class `hyprforge_process::output` exists for, one transport
//! further out.

use crate::backend::BluetoothBackend;
use crate::types::{Address, AdapterState, BluetoothError, Device, DeviceKind, Status};
use std::collections::HashMap;
use std::future::Future;
use std::time::Duration;
use zbus::zvariant::{ObjectPath, OwnedObjectPath, OwnedValue};
use zbus::Connection;

/// Ordinary calls: a property read, a list, a method that does not touch
/// the radio.
pub const TIMEOUT: Duration = Duration::from_secs(5);

/// `Connect()`. Deliberately far longer than [`TIMEOUT`]: this covers a
/// real link negotiation with a remote radio, and a five-second cap
/// would report a working headset as broken on any slow pairing. It is
/// still a cap — a connect that never resolves is a spinner forever
/// otherwise.
pub const CONNECT_TIMEOUT: Duration = Duration::from_secs(30);

/// Pairing. Longer than everything else here because BlueZ does not
/// answer until the conversation finishes, and part of that conversation
/// is a person reading six digits off two screens. Still bounded: a far
/// end that walks away must not leave the screen waiting forever.
pub const PAIR_TIMEOUT: Duration = Duration::from_secs(120);

#[zbus::proxy(
    interface = "org.freedesktop.DBus.ObjectManager",
    default_service = "org.bluez",
    default_path = "/"
)]
trait ObjectManagerDbus {
    fn get_managed_objects(
        &self,
    ) -> zbus::Result<HashMap<OwnedObjectPath, HashMap<String, HashMap<String, OwnedValue>>>>;
}

#[zbus::proxy(
    interface = "org.bluez.Adapter1",
    default_service = "org.bluez"
)]
trait AdapterDbus {
    fn start_discovery(&self) -> zbus::Result<()>;
    fn stop_discovery(&self) -> zbus::Result<()>;
    fn remove_device(&self, device: &ObjectPath<'_>) -> zbus::Result<()>;

    #[zbus(property)]
    fn powered(&self) -> zbus::Result<bool>;
    #[zbus(property)]
    fn set_powered(&self, powered: bool) -> zbus::Result<()>;
    #[zbus(property)]
    fn discovering(&self) -> zbus::Result<bool>;
    #[zbus(property)]
    fn alias(&self) -> zbus::Result<String>;
    /// Documented as experimental and readonly, and genuinely absent on
    /// some BlueZ builds — read through `GetManagedObjects`'s property
    /// map rather than this proxy method, so its absence is a `None`
    /// and not a failed call. Kept here only to name the property.
    #[allow(dead_code)]
    #[zbus(property)]
    fn power_state(&self) -> zbus::Result<String>;
}

#[zbus::proxy(
    interface = "org.bluez.Device1",
    default_service = "org.bluez"
)]
trait DeviceDbus {
    fn connect(&self) -> zbus::Result<()>;
    fn disconnect(&self) -> zbus::Result<()>;
    fn pair(&self) -> zbus::Result<()>;
    fn cancel_pairing(&self) -> zbus::Result<()>;

    #[zbus(property)]
    fn set_trusted(&self, trusted: bool) -> zbus::Result<()>;
}

/// Bounds a call and turns a D-Bus failure into something a screen can
/// say out loud.
async fn bounded<T>(
    limit: Duration,
    call: impl Future<Output = zbus::Result<T>>,
) -> Result<T, BluetoothError> {
    match tokio::time::timeout(limit, call).await {
        Err(_elapsed) => Err(BluetoothError::TimedOut(limit)),
        Ok(Ok(value)) => Ok(value),
        Ok(Err(e)) => Err(classify(e)),
    }
}

/// Which failures mean "BlueZ isn't there" and which mean "BlueZ said
/// no".
///
/// Collapsing the two would put "no devices nearby" on screen for a
/// machine whose `bluetooth.service` is stopped, which is the mistake
/// `hyprforge-network`'s equivalent function exists to avoid making
/// twice. Everything unrecognised stays [`BluetoothError::Refused`]
/// carrying the original text, because a message we did not anticipate
/// is still more use than one we invented.
fn classify(e: zbus::Error) -> BluetoothError {
    if let zbus::Error::MethodError(name, _, _) = &e {
        let name = name.as_str();
        if name.ends_with(".ServiceUnknown") || name.ends_with(".NameHasNoOwner") {
            return BluetoothError::Unavailable;
        }
        if name.starts_with("org.bluez.Error.") {
            return BluetoothError::Refused(e.to_string());
        }
        return BluetoothError::Refused(e.to_string());
    }
    match e {
        // No bus to talk to at all.
        zbus::Error::Address(_) | zbus::Error::InputOutput(_) => BluetoothError::Unavailable,
        other => BluetoothError::Refused(other.to_string()),
    }
}

/// Reads one property out of `GetManagedObjects`'s already-fetched map,
/// as a concrete type.
///
/// `TryFrom<&OwnedValue>` returning `Err` is treated the same as the key
/// being absent: BlueZ's own optional properties (`Name`, `Icon`,
/// `RSSI`) are simply missing from the map for a device that has never
/// reported them, rather than present with some placeholder value, but
/// reading defensively here means a surprising variant type degrades to
/// "not known" instead of failing the whole device.
fn prop<T>(props: &HashMap<String, OwnedValue>, key: &str) -> Option<T>
where
    T: TryFrom<OwnedValue>,
{
    props.get(key).and_then(|v| T::try_from(v.clone()).ok())
}

/// BlueZ, as this crate uses it.
pub struct BlueZBackend {
    connection: Connection,
}

impl BlueZBackend {
    /// Opens the system bus.
    ///
    /// Failing here is [`BluetoothError::Unavailable`] rather than a
    /// panic or an empty backend: a machine without `bluetoothd` is a
    /// supported machine, and the screen's job is to say so.
    pub async fn connect() -> Result<Self, BluetoothError> {
        let connection = tokio::time::timeout(TIMEOUT, Connection::system())
            .await
            .map_err(|_| BluetoothError::TimedOut(TIMEOUT))?
            .map_err(classify)?;
        Ok(BlueZBackend { connection })
    }

    async fn managed_objects(
        &self,
    ) -> Result<HashMap<OwnedObjectPath, HashMap<String, HashMap<String, OwnedValue>>>, BluetoothError>
    {
        let manager = bounded(TIMEOUT, ObjectManagerDbusProxy::new(&self.connection)).await?;
        bounded(TIMEOUT, manager.get_managed_objects()).await
    }

    /// The path of the first `Adapter1` BlueZ reports.
    ///
    /// Resolved per call rather than cached, because a USB dongle can
    /// appear and disappear while the screen is open and a cached path
    /// would outlive the adapter it names.
    async fn adapter_path(&self) -> Result<OwnedObjectPath, BluetoothError> {
        let objects = self.managed_objects().await?;
        objects
            .into_iter()
            .find(|(_, ifaces)| ifaces.contains_key("org.bluez.Adapter1"))
            .map(|(path, _)| path)
            .ok_or(BluetoothError::NoAdapter)
    }

    async fn adapter(&self) -> Result<AdapterDbusProxy<'_>, BluetoothError> {
        let path = self.adapter_path().await?;
        bounded(
            TIMEOUT,
            AdapterDbusProxy::builder(&self.connection)
                .path(path)
                .map_err(classify)?
                .build(),
        )
        .await
    }

    async fn device_proxy(&self, address: &Address) -> Result<DeviceDbusProxy<'_>, BluetoothError> {
        let path = self.device_path(address).await?;
        bounded(
            TIMEOUT,
            DeviceDbusProxy::builder(&self.connection)
                .path(path)
                .map_err(classify)?
                .build(),
        )
        .await
    }

    /// The object path of the `Device1` with this address, under the
    /// current adapter.
    ///
    /// `Device` carries no D-Bus path on purpose — keeping the model
    /// free of bus types is what lets the mock exist — so the path is
    /// resolved again at the point it is needed. An address that has
    /// since vanished is a real outcome: BlueZ's object tree churns
    /// during discovery.
    async fn device_path(&self, address: &Address) -> Result<OwnedObjectPath, BluetoothError> {
        let adapter_path = self.adapter_path().await?;
        let objects = self.managed_objects().await?;
        for (path, ifaces) in objects {
            if !path.as_str().starts_with(adapter_path.as_str()) {
                continue;
            }
            let Some(dev) = ifaces.get("org.bluez.Device1") else {
                continue;
            };
            if prop::<String>(dev, "Address").as_deref() == Some(address.as_str()) {
                return Ok(path);
            }
        }
        Err(BluetoothError::Refused(
            "That device is no longer known to BlueZ.".to_string(),
        ))
    }

    /// Turns one `Device1` property map into a [`Device`].
    ///
    /// `Name`, `Icon` and `RSSI` are documented as optional and are
    /// genuinely absent for a device BlueZ has not yet resolved or is
    /// not currently advertising — reading them with [`prop`] rather
    /// than an infallible accessor is what keeps one such device from
    /// failing the whole list.
    fn read_device(props: &HashMap<String, OwnedValue>) -> Option<Device> {
        let address = prop::<String>(props, "Address")?;
        // `Alias` always exists — BlueZ falls back to `Name` and then to
        // the address itself — but is read defensively anyway, because a
        // device with a missing `Alias` is still worth listing under its
        // address rather than dropping.
        let alias = prop::<String>(props, "Alias").unwrap_or_else(|| address.clone());
        Some(Device {
            address: Address::new(address),
            alias,
            name: prop::<String>(props, "Name"),
            kind: DeviceKind::from_icon(prop::<String>(props, "Icon").as_deref()),
            paired: prop::<bool>(props, "Paired").unwrap_or(false),
            trusted: prop::<bool>(props, "Trusted").unwrap_or(false),
            connected: prop::<bool>(props, "Connected").unwrap_or(false),
            rssi: prop::<i16>(props, "RSSI"),
        })
    }
}

#[async_trait::async_trait]
impl BluetoothBackend for BlueZBackend {
    async fn status(&self) -> Result<Status, BluetoothError> {
        let adapter = self.adapter().await?;

        // `PowerState` is documented experimental and may not exist on
        // every BlueZ build, so its absence falls back to `Powered`
        // rather than being treated as an error.
        let state = match bounded(TIMEOUT, adapter.power_state()).await {
            Ok(power_state) => match power_state.as_str() {
                "off-blocked" => AdapterState::HardwareBlocked,
                "off-enabling" | "on-disabling" => AdapterState::Changing,
                "on" => AdapterState::On,
                _ => AdapterState::Off,
            },
            Err(_) => {
                if bounded(TIMEOUT, adapter.powered()).await? {
                    AdapterState::On
                } else {
                    AdapterState::Off
                }
            }
        };

        Ok(Status {
            state,
            discovering: bounded(TIMEOUT, adapter.discovering()).await?,
            alias: bounded(TIMEOUT, adapter.alias()).await?,
        })
    }

    async fn devices(&self) -> Result<Vec<Device>, BluetoothError> {
        let adapter_path = self.adapter_path().await?;
        let objects = self.managed_objects().await?;
        let mut devices = Vec::new();
        for (path, ifaces) in objects {
            if !path.as_str().starts_with(adapter_path.as_str()) {
                continue;
            }
            let Some(dev_props) = ifaces.get("org.bluez.Device1") else {
                continue;
            };
            // A device that vanished mid-enumeration, or whose required
            // fields are missing outright, is skipped rather than
            // failing the whole list — BlueZ's object tree churns
            // constantly during discovery. Optional fields are already
            // handled inside `read_device` via `prop`.
            if let Some(device) = Self::read_device(dev_props) {
                devices.push(device);
            }
        }
        Ok(devices)
    }

    async fn set_discovery(&self, on: bool) -> Result<(), BluetoothError> {
        let adapter = self.adapter().await?;
        if on {
            bounded(TIMEOUT, adapter.start_discovery()).await
        } else {
            bounded(TIMEOUT, adapter.stop_discovery()).await
        }
    }

    async fn set_powered(&self, on: bool) -> Result<(), BluetoothError> {
        let adapter = self.adapter().await?;
        bounded(TIMEOUT, adapter.set_powered(on)).await
    }

    async fn pair(&self, address: &Address) -> Result<(), BluetoothError> {
        let device = self.device_proxy(address).await?;
        // `PAIR_TIMEOUT`, not `CONNECT_TIMEOUT`: BlueZ does not answer
        // this until the pairing conversation is over, and that includes
        // however long a person takes to look at two screens and decide
        // the digits match. A radio's timeout is the wrong unit.
        bounded(PAIR_TIMEOUT, device.pair()).await
    }

    async fn cancel_pairing(&self, address: &Address) -> Result<(), BluetoothError> {
        let device = self.device_proxy(address).await?;
        // Short: this is the *abandon* path, and it must not itself wait
        // out a pairing that has already stopped answering.
        bounded(TIMEOUT, device.cancel_pairing()).await
    }

    async fn connect(&self, address: &Address) -> Result<(), BluetoothError> {
        let device = self.device_proxy(address).await?;
        bounded(CONNECT_TIMEOUT, device.connect()).await
    }

    async fn disconnect(&self, address: &Address) -> Result<(), BluetoothError> {
        let device = self.device_proxy(address).await?;
        bounded(TIMEOUT, device.disconnect()).await
    }

    async fn set_trusted(&self, address: &Address, trusted: bool) -> Result<(), BluetoothError> {
        let device = self.device_proxy(address).await?;
        bounded(TIMEOUT, device.set_trusted(trusted)).await
    }

    async fn forget(&self, address: &Address) -> Result<(), BluetoothError> {
        // Forgetting is `Adapter1.RemoveDevice(o)`, not a method on
        // `Device1` — BlueZ models "stop remembering this device" as
        // something the adapter does, not the device.
        let device_path = self.device_path(address).await?;
        let adapter = self.adapter().await?;
        bounded(TIMEOUT, adapter.remove_device(&device_path.as_ref())).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use zbus::zvariant::Value;

    fn owned(v: Value<'_>) -> OwnedValue {
        OwnedValue::try_from(v).expect("test value converts")
    }

    #[test]
    fn connecting_is_given_longer_than_a_property_read_but_is_still_bounded() {
        assert!(CONNECT_TIMEOUT > TIMEOUT);
        assert!(CONNECT_TIMEOUT <= Duration::from_secs(60));
    }

    /// The mechanical check for the single most likely bug in this file:
    /// an optional property missing from the map must read as `None`,
    /// not fail the whole device.
    #[test]
    fn a_device_missing_every_optional_property_still_reads() {
        let mut props: HashMap<String, OwnedValue> = HashMap::new();
        props.insert("Address".into(), owned(Value::from("AA:BB:CC:DD:EE:FF")));
        props.insert("Alias".into(), owned(Value::from("AA-BB-CC-DD-EE-FF")));
        props.insert("Paired".into(), OwnedValue::from(true));
        props.insert("Trusted".into(), OwnedValue::from(false));
        props.insert("Connected".into(), OwnedValue::from(false));
        // Deliberately no Name, Icon or RSSI.

        let device = BlueZBackend::read_device(&props).expect("a device with the required fields reads");
        assert_eq!(device.name, None);
        assert_eq!(device.kind, DeviceKind::Unknown);
        assert_eq!(device.rssi, None);
        assert!(device.paired);
    }

    /// A device with no `Address` cannot be identified at all, and is
    /// the one case `read_device` genuinely drops rather than defaulting.
    #[test]
    fn a_device_with_no_address_is_not_readable() {
        let props: HashMap<String, OwnedValue> = HashMap::new();
        assert!(BlueZBackend::read_device(&props).is_none());
    }
}
