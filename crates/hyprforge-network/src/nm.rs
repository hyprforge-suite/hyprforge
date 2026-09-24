//! The real [`NetworkBackend`], talking to NetworkManager on the system bus.
//!
//! Everything here is D-Bus mechanics. The decisions — what counts as
//! WPA3, which access point wins when a name is published from four
//! radios, whether a passphrase is long enough to be worth sending — live
//! in [`crate::types`] and [`crate::backend`], where they can be tested
//! on a machine with no NetworkManager at all. What is left here is the
//! part that can only be checked against the service itself, which is
//! what the `#[ignore]`d live test in `tests/live_networkmanager.rs` is
//! for.
//!
//! # Nothing waits forever
//!
//! Every call is wrapped in a timeout. NetworkManager blocks on real
//! radio hardware: a scan waits for the adapter, and an activation waits
//! for association and then DHCP from a server that may never answer. It
//! is exactly the class `hyprforge_process::output` exists for, one
//! transport further out.

use crate::backend::{no_such_wired_interface, NetworkBackend, SavedNetwork, Status, WiredState, WiredStatus};
use crate::secret::Psk;
use crate::types::{AccessPoint, NetworkError, RadioState, Security, Ssid};
use std::collections::HashMap;
use std::future::Future;
use std::time::Duration;
use zbus::zvariant::{ObjectPath, OwnedObjectPath, OwnedValue, Value};
use zbus::Connection;

/// Ordinary calls: a property read, a list, a delete.
pub const TIMEOUT: Duration = Duration::from_secs(5);

/// Joining a network. Deliberately far longer than [`TIMEOUT`]: this
/// covers association, the 4-way handshake and a DHCP lease, and a
/// five-second cap would report a working network as a failure on any
/// slow AP. It is still a cap — an activation that never resolves is a
/// spinner forever otherwise.
pub const ACTIVATION_TIMEOUT: Duration = Duration::from_secs(45);

/// `NM_DEVICE_TYPE_WIFI`.
const DEVICE_TYPE_WIFI: u32 = 2;

/// `NM_DEVICE_TYPE_ETHERNET`. Not a bridge (13), a veth (20) or anything
/// else with a cable-shaped name: container plumbing reports those, and
/// listing Docker's bridges as "wired" would put interfaces nobody
/// plugged in at the top of the menu.
const DEVICE_TYPE_ETHERNET: u32 = 1;

/// `NM_DEVICE_STATE_*`, the ones [`wired_state`] tells apart.
const DEVICE_STATE_UNKNOWN: u32 = 0;
const DEVICE_STATE_UNMANAGED: u32 = 10;
const DEVICE_STATE_PREPARE: u32 = 40;
const DEVICE_STATE_ACTIVATED: u32 = 100;

/// A wired device's state, from NetworkManager's device state and its
/// carrier — or `None` for a device NetworkManager does not manage, which
/// is not this crate's to show or to switch.
///
/// Carrier is asked separately rather than read off the state because
/// NetworkManager folds "no cable" into `UNAVAILABLE` together with other
/// reasons a device can't be used; the carrier says which it is.
pub(crate) fn wired_state(device_state: u32, carrier: bool) -> Option<WiredState> {
    match device_state {
        DEVICE_STATE_UNKNOWN | DEVICE_STATE_UNMANAGED => None,
        DEVICE_STATE_ACTIVATED => Some(WiredState::Connected),
        DEVICE_STATE_PREPARE..DEVICE_STATE_ACTIVATED => Some(WiredState::Connecting),
        _ if !carrier => Some(WiredState::CableUnplugged),
        _ => Some(WiredState::Disconnected),
    }
}

/// `NM_ACTIVE_CONNECTION_STATE_*`.
const ACTIVE_STATE_ACTIVATED: u32 = 2;
const ACTIVE_STATE_DEACTIVATED: u32 = 4;

#[zbus::proxy(
    interface = "org.freedesktop.NetworkManager",
    default_service = "org.freedesktop.NetworkManager",
    default_path = "/org/freedesktop/NetworkManager"
)]
trait NetworkManagerDbus {
    fn get_devices(&self) -> zbus::Result<Vec<OwnedObjectPath>>;

    /// Brings up a connection that already exists, with the secret
    /// NetworkManager already stores. `specific_object` is `/` — let it
    /// pick the access point itself, since a saved connection may be
    /// reachable through several.
    fn activate_connection(
        &self,
        connection: &ObjectPath<'_>,
        device: &ObjectPath<'_>,
        specific_object: &ObjectPath<'_>,
    ) -> zbus::Result<OwnedObjectPath>;

    fn add_and_activate_connection(
        &self,
        connection: HashMap<&str, HashMap<&str, Value<'_>>>,
        device: &ObjectPath<'_>,
        specific_object: &ObjectPath<'_>,
    ) -> zbus::Result<(OwnedObjectPath, OwnedObjectPath)>;

    #[zbus(property)]
    fn wireless_enabled(&self) -> zbus::Result<bool>;
    #[zbus(property)]
    fn set_wireless_enabled(&self, enabled: bool) -> zbus::Result<()>;
    /// False means a rocker switch or an Fn key. No amount of D-Bus will
    /// change it, which is why it is a separate [`RadioState`].
    #[zbus(property)]
    fn wireless_hardware_enabled(&self) -> zbus::Result<bool>;
}

#[zbus::proxy(
    interface = "org.freedesktop.NetworkManager.Device",
    default_service = "org.freedesktop.NetworkManager"
)]
trait DeviceDbus {
    #[zbus(property)]
    fn device_type(&self) -> zbus::Result<u32>;
    #[zbus(property)]
    fn interface(&self) -> zbus::Result<String>;
    #[zbus(property)]
    fn state(&self) -> zbus::Result<u32>;
    /// `/` while nothing is active on the device.
    #[zbus(property)]
    fn active_connection(&self) -> zbus::Result<OwnedObjectPath>;
    fn disconnect(&self) -> zbus::Result<()>;
}

#[zbus::proxy(
    interface = "org.freedesktop.NetworkManager.Device.Wired",
    default_service = "org.freedesktop.NetworkManager"
)]
trait WiredDbus {
    #[zbus(property)]
    fn carrier(&self) -> zbus::Result<bool>;
    /// Mb/s, `0` when there is no link to have a speed.
    #[zbus(property)]
    fn speed(&self) -> zbus::Result<u32>;
}

#[zbus::proxy(
    interface = "org.freedesktop.NetworkManager.Device.Wireless",
    default_service = "org.freedesktop.NetworkManager"
)]
trait WirelessDbus {
    /// `GetAllAccessPoints`, not `GetAccessPoints`: the latter is
    /// deprecated and omits APs that hide their SSID, which this crate
    /// filters out itself and for its own reason.
    fn get_all_access_points(&self) -> zbus::Result<Vec<OwnedObjectPath>>;
    fn request_scan(&self, options: HashMap<&str, Value<'_>>) -> zbus::Result<()>;
    #[zbus(property)]
    fn active_access_point(&self) -> zbus::Result<OwnedObjectPath>;
}

#[zbus::proxy(
    interface = "org.freedesktop.NetworkManager.AccessPoint",
    default_service = "org.freedesktop.NetworkManager"
)]
trait AccessPointDbus {
    /// `ay`, and that is not an accident — see [`Ssid`].
    #[zbus(property)]
    fn ssid(&self) -> zbus::Result<Vec<u8>>;
    #[zbus(property)]
    fn strength(&self) -> zbus::Result<u8>;
    #[zbus(property)]
    fn frequency(&self) -> zbus::Result<u32>;
    #[zbus(property)]
    fn hw_address(&self) -> zbus::Result<String>;
    #[zbus(property)]
    fn flags(&self) -> zbus::Result<u32>;
    #[zbus(property)]
    fn wpa_flags(&self) -> zbus::Result<u32>;
    #[zbus(property)]
    fn rsn_flags(&self) -> zbus::Result<u32>;
}

#[zbus::proxy(
    interface = "org.freedesktop.NetworkManager.Settings",
    default_service = "org.freedesktop.NetworkManager",
    default_path = "/org/freedesktop/NetworkManager/Settings"
)]
trait SettingsDbus {
    fn list_connections(&self) -> zbus::Result<Vec<OwnedObjectPath>>;
}

#[zbus::proxy(
    interface = "org.freedesktop.NetworkManager.Settings.Connection",
    default_service = "org.freedesktop.NetworkManager"
)]
trait SettingsConnectionDbus {
    fn get_settings(&self) -> zbus::Result<HashMap<String, HashMap<String, OwnedValue>>>;
    fn delete(&self) -> zbus::Result<()>;
}

#[zbus::proxy(
    interface = "org.freedesktop.NetworkManager.Connection.Active",
    default_service = "org.freedesktop.NetworkManager"
)]
trait ActiveConnectionDbus {
    #[zbus(property)]
    fn state(&self) -> zbus::Result<u32>;
    /// The connection's name, as NetworkManager lists it.
    #[zbus(property)]
    fn id(&self) -> zbus::Result<String>;
}

/// Bounds a call and turns a D-Bus failure into something a screen can
/// say out loud.
async fn bounded<T>(
    limit: Duration,
    call: impl Future<Output = zbus::Result<T>>,
) -> Result<T, NetworkError> {
    match tokio::time::timeout(limit, call).await {
        Err(_elapsed) => Err(NetworkError::TimedOut(limit)),
        Ok(Ok(value)) => Ok(value),
        Ok(Err(e)) => Err(classify(e)),
    }
}

/// Which failures mean "NetworkManager isn't there" and which mean
/// "NetworkManager said no".
///
/// Collapsing the two would put "there are no networks nearby" on screen
/// for a machine whose network service is stopped, which is the mistake
/// this project keeps finding. Everything unrecognised stays
/// [`NetworkError::Refused`] carrying the original text, because a
/// message we did not anticipate is still more use than one we invented.
fn classify(e: zbus::Error) -> NetworkError {
    if let zbus::Error::MethodError(name, _, _) = &e {
        let name = name.as_str();
        if name.ends_with(".ServiceUnknown") || name.ends_with(".NameHasNoOwner") {
            return NetworkError::Unavailable;
        }
        if name.contains("NoSecrets") || name.contains("SecretsRequired") {
            return NetworkError::BadPassphrase;
        }
        return NetworkError::Refused(e.to_string());
    }
    match e {
        // No bus to talk to at all.
        zbus::Error::Address(_) | zbus::Error::InputOutput(_) => NetworkError::Unavailable,
        other => NetworkError::Refused(other.to_string()),
    }
}

/// The settings dictionary handed to `AddAndActivateConnection`.
///
/// Split out as a plain function so it can be asserted on without a bus,
/// which matters more here than anywhere else in this file: it is the one
/// place the passphrase is written into a structure, and the test that
/// proves it lands in the security section and nowhere else is the only
/// mechanical check that it is not also being written into a field that
/// gets logged.
///
/// No `uuid` and no `autoconnect`: NetworkManager generates the first and
/// defaults the second to true, which is the behaviour wanted anyway —
/// a network joined once should come back on its own.
pub(crate) fn connection_settings(
    ssid: &Ssid,
    security: Security,
    psk: Option<&Psk>,
) -> HashMap<&'static str, HashMap<&'static str, Value<'static>>> {
    let mut settings: HashMap<&'static str, HashMap<&'static str, Value<'static>>> = HashMap::new();

    let mut connection: HashMap<&'static str, Value<'static>> = HashMap::new();
    // The name in NetworkManager's own lists. Lossy is right here and
    // only here: this is a label, while the `ssid` below is the bytes to
    // actually associate with.
    connection.insert("id", Value::from(ssid.to_display_string()));
    connection.insert("type", Value::from("802-11-wireless"));
    settings.insert("connection", connection);

    let mut wireless: HashMap<&'static str, Value<'static>> = HashMap::new();
    wireless.insert("ssid", Value::from(ssid.as_bytes().to_vec()));
    wireless.insert("mode", Value::from("infrastructure"));
    settings.insert("802-11-wireless", wireless);

    // An open network gets no security section at all. Sending one with
    // `key-mgmt = none` would describe a WEP network and fail against an
    // AP that has no encryption whatsoever.
    if let Some(key_mgmt) = security.key_mgmt() {
        let mut sec: HashMap<&'static str, Value<'static>> = HashMap::new();
        sec.insert("key-mgmt", Value::from(key_mgmt));
        if let Some(psk) = psk {
            sec.insert("psk", Value::from(psk.expose().to_string()));
        }
        settings.insert("802-11-wireless-security", sec);
    }

    settings
}

/// NetworkManager, as this crate uses it.
pub struct NetworkManagerBackend {
    connection: Connection,
}

impl NetworkManagerBackend {
    /// Opens the system bus.
    ///
    /// Failing here is [`NetworkError::Unavailable`] rather than a
    /// panic or an empty backend: a machine without NetworkManager is a
    /// supported machine, and the screen's job is to say so.
    pub async fn connect() -> Result<Self, NetworkError> {
        let connection = tokio::time::timeout(TIMEOUT, Connection::system())
            .await
            .map_err(|_| NetworkError::TimedOut(TIMEOUT))?
            .map_err(classify)?;
        Ok(NetworkManagerBackend { connection })
    }

    async fn root(&self) -> Result<NetworkManagerDbusProxy<'_>, NetworkError> {
        bounded(TIMEOUT, NetworkManagerDbusProxy::new(&self.connection)).await
    }

    /// The first Wi-Fi device NetworkManager reports.
    ///
    /// Resolved per call rather than cached, because a USB adapter can
    /// appear and disappear while the screen is open and a cached path
    /// would outlive the device it names.
    async fn wifi_device(&self) -> Result<OwnedObjectPath, NetworkError> {
        let devices = bounded(TIMEOUT, self.root().await?.get_devices()).await?;
        for path in devices {
            let device = bounded(
                TIMEOUT,
                DeviceDbusProxy::builder(&self.connection)
                    .path(path.clone())
                    .map_err(classify)?
                    .build(),
            )
            .await?;
            if bounded(TIMEOUT, device.device_type()).await? == DEVICE_TYPE_WIFI {
                return Ok(path);
            }
        }
        Err(NetworkError::NoWifiDevice)
    }

    async fn device(&self, path: OwnedObjectPath) -> Result<DeviceDbusProxy<'_>, NetworkError> {
        bounded(
            TIMEOUT,
            DeviceDbusProxy::builder(&self.connection)
                .path(path)
                .map_err(classify)?
                .build(),
        )
        .await
    }

    /// One Ethernet device's status, or `None` when it is not an
    /// Ethernet device NetworkManager manages.
    async fn read_wired(&self, path: OwnedObjectPath) -> Result<Option<WiredStatus>, NetworkError> {
        let device = self.device(path.clone()).await?;
        if bounded(TIMEOUT, device.device_type()).await? != DEVICE_TYPE_ETHERNET {
            return Ok(None);
        }
        let wired = bounded(
            TIMEOUT,
            WiredDbusProxy::builder(&self.connection)
                .path(path)
                .map_err(classify)?
                .build(),
        )
        .await?;
        let carrier = bounded(TIMEOUT, wired.carrier()).await?;
        let Some(state) = wired_state(bounded(TIMEOUT, device.state()).await?, carrier) else {
            return Ok(None);
        };

        let connection = match state {
            WiredState::Connected | WiredState::Connecting => {
                let active = bounded(TIMEOUT, device.active_connection()).await?;
                if active.as_str() == "/" {
                    None
                } else {
                    let active = bounded(
                        TIMEOUT,
                        ActiveConnectionDbusProxy::builder(&self.connection)
                            .path(active)
                            .map_err(classify)?
                            .build(),
                    )
                    .await?;
                    Some(bounded(TIMEOUT, active.id()).await?)
                }
            }
            WiredState::CableUnplugged | WiredState::Disconnected => None,
        };
        let speed_mbps = match state {
            WiredState::Connected => Some(bounded(TIMEOUT, wired.speed()).await?).filter(|s| *s > 0),
            _ => None,
        };

        Ok(Some(WiredStatus {
            interface: bounded(TIMEOUT, device.interface()).await?,
            state,
            connection,
            speed_mbps,
        }))
    }

    /// The object path of the Ethernet device called `interface`.
    async fn wired_device(&self, interface: &str) -> Result<OwnedObjectPath, NetworkError> {
        for path in bounded(TIMEOUT, self.root().await?.get_devices()).await? {
            let device = self.device(path.clone()).await?;
            if bounded(TIMEOUT, device.device_type()).await? == DEVICE_TYPE_ETHERNET
                && bounded(TIMEOUT, device.interface()).await? == interface
            {
                return Ok(path);
            }
        }
        Err(no_such_wired_interface(interface))
    }

    async fn wireless(&self) -> Result<WirelessDbusProxy<'_>, NetworkError> {
        let path = self.wifi_device().await?;
        bounded(
            TIMEOUT,
            WirelessDbusProxy::builder(&self.connection)
                .path(path)
                .map_err(classify)?
                .build(),
        )
        .await
    }

    async fn read_access_point(
        &self,
        path: OwnedObjectPath,
    ) -> Result<AccessPoint, NetworkError> {
        let ap = bounded(
            TIMEOUT,
            AccessPointDbusProxy::builder(&self.connection)
                .path(path)
                .map_err(classify)?
                .build(),
        )
        .await?;
        Ok(AccessPoint {
            ssid: Ssid::new(bounded(TIMEOUT, ap.ssid()).await?),
            bssid: bounded(TIMEOUT, ap.hw_address()).await?,
            strength: bounded(TIMEOUT, ap.strength()).await?,
            frequency_mhz: bounded(TIMEOUT, ap.frequency()).await?,
            security: Security::classify(
                bounded(TIMEOUT, ap.flags()).await?,
                bounded(TIMEOUT, ap.wpa_flags()).await?,
                bounded(TIMEOUT, ap.rsn_flags()).await?,
            ),
        })
    }

    /// The object path of the AP with this BSSID.
    ///
    /// `AccessPoint` carries no D-Bus path on purpose — keeping the model
    /// free of bus types is what lets the mock exist — so the path is
    /// resolved again at the point it is needed. A BSSID that has since
    /// vanished is a real outcome: the network went away between listing
    /// and clicking.
    async fn access_point_path(&self, bssid: &str) -> Result<OwnedObjectPath, NetworkError> {
        let wireless = self.wireless().await?;
        for path in bounded(TIMEOUT, wireless.get_all_access_points()).await? {
            let ap = bounded(
                TIMEOUT,
                AccessPointDbusProxy::builder(&self.connection)
                    .path(path.clone())
                    .map_err(classify)?
                    .build(),
            )
            .await?;
            if bounded(TIMEOUT, ap.hw_address()).await?.eq_ignore_ascii_case(bssid) {
                return Ok(path);
            }
        }
        Err(NetworkError::Refused(
            "That network is no longer in range.".to_string(),
        ))
    }

    /// Waits for an activation to settle, and says which way it went.
    async fn await_activation(
        &self,
        active: OwnedObjectPath,
        had_passphrase: bool,
    ) -> Result<(), NetworkError> {
        let proxy = bounded(
            TIMEOUT,
            ActiveConnectionDbusProxy::builder(&self.connection)
                .path(active)
                .map_err(classify)?
                .build(),
        )
        .await?;

        let deadline = tokio::time::Instant::now() + ACTIVATION_TIMEOUT;
        loop {
            match bounded(TIMEOUT, proxy.state()).await {
                Ok(ACTIVE_STATE_ACTIVATED) => return Ok(()),
                Ok(ACTIVE_STATE_DEACTIVATED) => {
                    // NetworkManager does not tell us "wrong password" in
                    // so many words here, and guessing from the reason
                    // code means tracking an enum that has grown twice.
                    // A deactivation on an attempt that carried a
                    // passphrase is overwhelmingly a wrong one, and the
                    // message says what to do either way.
                    return Err(if had_passphrase {
                        NetworkError::BadPassphrase
                    } else {
                        NetworkError::Refused(
                            "The network refused the connection.".to_string(),
                        )
                    });
                }
                // The object disappears when NetworkManager tears a
                // failed activation down, which is a terminal answer and
                // not a reason to keep polling.
                Err(NetworkError::Refused(_)) => {
                    return Err(if had_passphrase {
                        NetworkError::BadPassphrase
                    } else {
                        NetworkError::Refused("The connection was dropped.".to_string())
                    })
                }
                Err(e) => return Err(e),
                Ok(_still_activating) => {}
            }
            if tokio::time::Instant::now() >= deadline {
                return Err(NetworkError::TimedOut(ACTIVATION_TIMEOUT));
            }
            tokio::time::sleep(Duration::from_millis(250)).await;
        }
    }
}

#[async_trait::async_trait]
impl NetworkBackend for NetworkManagerBackend {
    async fn status(&self) -> Result<Status, NetworkError> {
        let root = self.root().await?;
        let radio = if !bounded(TIMEOUT, root.wireless_hardware_enabled()).await? {
            RadioState::HardwareOff
        } else if bounded(TIMEOUT, root.wireless_enabled()).await? {
            RadioState::On
        } else {
            RadioState::Off
        };

        // No device is not an error for a status read — it is a machine
        // with no Wi-Fi, which the radio state above already describes.
        let connected_to = match self.wireless().await {
            Ok(wireless) => {
                let path = bounded(TIMEOUT, wireless.active_access_point()).await?;
                // "/" is NetworkManager's way of saying "none".
                if path.as_str() == "/" {
                    None
                } else {
                    Some(self.read_access_point(path).await?.ssid)
                }
            }
            Err(NetworkError::NoWifiDevice) => None,
            Err(e) => return Err(e),
        };

        Ok(Status { radio, connected_to })
    }

    async fn wired(&self) -> Result<Vec<WiredStatus>, NetworkError> {
        let mut wired = Vec::new();
        for path in bounded(TIMEOUT, self.root().await?.get_devices()).await? {
            match self.read_wired(path).await {
                Ok(Some(status)) => wired.push(status),
                Ok(None) => {}
                // A USB adapter unplugged between the list and the read:
                // the same churn `access_points` tolerates.
                Err(NetworkError::Refused(_)) => continue,
                Err(e) => return Err(e),
            }
        }
        Ok(wired)
    }

    async fn wired_connect(&self, interface: &str) -> Result<(), NetworkError> {
        let device = self.wired_device(interface).await?;
        // `/` for the connection lets NetworkManager pick the best saved
        // one for this device — its documented meaning, and the choice it
        // makes by itself when a cable goes in.
        let any = ObjectPath::try_from("/").expect("/ is a valid object path");
        let root = self.root().await?;
        let active = bounded(
            ACTIVATION_TIMEOUT,
            root.activate_connection(&any, &device.as_ref(), &any),
        )
        .await?;
        self.await_activation(active, false).await
    }

    async fn wired_disconnect(&self, interface: &str) -> Result<(), NetworkError> {
        let path = self.wired_device(interface).await?;
        bounded(TIMEOUT, self.device(path).await?.disconnect()).await
    }

    async fn access_points(&self) -> Result<Vec<AccessPoint>, NetworkError> {
        let wireless = self.wireless().await?;
        let paths = bounded(TIMEOUT, wireless.get_all_access_points()).await?;
        let mut points = Vec::with_capacity(paths.len());
        for path in paths {
            match self.read_access_point(path).await {
                Ok(ap) => points.push(ap),
                // One AP that vanished mid-read must not empty the list:
                // scan results churn constantly, and a network going away
                // between the list and the read is normal.
                Err(NetworkError::Refused(_)) => continue,
                Err(e) => return Err(e),
            }
        }
        Ok(points)
    }

    async fn request_scan(&self) -> Result<(), NetworkError> {
        let wireless = self.wireless().await?;
        bounded(TIMEOUT, wireless.request_scan(HashMap::new())).await
    }

    async fn saved_networks(&self) -> Result<Vec<SavedNetwork>, NetworkError> {
        let settings = bounded(TIMEOUT, SettingsDbusProxy::new(&self.connection)).await?;
        let mut saved = Vec::new();
        for path in bounded(TIMEOUT, settings.list_connections()).await? {
            let conn = bounded(
                TIMEOUT,
                SettingsConnectionDbusProxy::builder(&self.connection)
                    .path(path.clone())
                    .map_err(classify)?
                    .build(),
            )
            .await?;
            let all = match bounded(TIMEOUT, conn.get_settings()).await {
                Ok(all) => all,
                // A connection readable only by root — a system
                // connection with stored secrets — is not ours to list.
                Err(NetworkError::Refused(_)) => continue,
                Err(e) => return Err(e),
            };
            let Some(wireless) = all.get("802-11-wireless") else {
                continue; // Not Wi-Fi.
            };
            let Some(ssid) = wireless.get("ssid").and_then(|v| Vec::<u8>::try_from(v.clone()).ok())
            else {
                continue;
            };
            let autoconnect = all
                .get("connection")
                .and_then(|c| c.get("autoconnect"))
                .and_then(|v| bool::try_from(v.clone()).ok())
                // NetworkManager's own default when the key is absent.
                .unwrap_or(true);
            saved.push(SavedNetwork {
                ssid: Ssid::new(ssid),
                id: path.as_str().to_string(),
                autoconnect,
            });
        }
        Ok(saved)
    }

    async fn connect(&self, ap: &AccessPoint, psk: Option<&Psk>) -> Result<(), NetworkError> {
        if let Some(reason) = ap.security.unsupported_reason() {
            return Err(NetworkError::Refused(reason.to_string()));
        }
        // Refused here rather than sent, because NetworkManager reports a
        // too-short passphrase several seconds later as a failed
        // activation, which looks exactly like a wrong one.
        if ap.security.needs_passphrase() {
            match psk {
                None => {
                    return Err(NetworkError::Refused(
                        "This network needs a password.".to_string(),
                    ))
                }
                Some(psk) if !psk.is_plausible_wpa() => {
                    return Err(NetworkError::Refused(
                        "A Wi-Fi password is between 8 and 63 characters.".to_string(),
                    ))
                }
                Some(_) => {}
            }
        }

        let device = self.wifi_device().await?;
        let ap_path = self.access_point_path(&ap.bssid).await?;
        let settings = connection_settings(&ap.ssid, ap.security, psk);

        let root = self.root().await?;
        let (added, active) = bounded(
            ACTIVATION_TIMEOUT,
            root.add_and_activate_connection(settings, &device.as_ref(), &ap_path.as_ref()),
        )
        .await?;

        match self.await_activation(active, psk.is_some()).await {
            Ok(()) => Ok(()),
            Err(e) => {
                // NetworkManager keeps the connection it was asked to
                // add even when activating it failed, so a mistyped
                // password otherwise leaves a saved network that retries
                // forever and shadows the next attempt. Best effort: the
                // activation failure is the answer either way.
                let _ = self.forget(added.as_str()).await;
                Err(e)
            }
        }
    }

    async fn connect_saved(&self, id: &str) -> Result<(), NetworkError> {
        let device = self.wifi_device().await?;
        let connection = ObjectPath::try_from(id.to_string())
            .map_err(|_| NetworkError::Refused(format!("not a connection path: {id}")))?;
        let any_access_point = ObjectPath::try_from("/").expect("/ is a valid object path");

        let root = self.root().await?;
        let active = bounded(
            ACTIVATION_TIMEOUT,
            root.activate_connection(&connection, &device.as_ref(), &any_access_point),
        )
        .await?;

        // `had_passphrase` is false: nothing was typed, so a failure here
        // is not the user's password being wrong. It is more likely the
        // network being out of range, and saying "the password wasn't
        // accepted" would send them to change a password that is fine.
        self.await_activation(active, false).await
    }

    async fn disconnect(&self) -> Result<(), NetworkError> {
        let path = self.wifi_device().await?;
        let device = bounded(
            TIMEOUT,
            DeviceDbusProxy::builder(&self.connection)
                .path(path)
                .map_err(classify)?
                .build(),
        )
        .await?;
        bounded(TIMEOUT, device.disconnect()).await
    }

    async fn forget(&self, id: &str) -> Result<(), NetworkError> {
        let conn = bounded(
            TIMEOUT,
            SettingsConnectionDbusProxy::builder(&self.connection)
                .path(id.to_string())
                .map_err(classify)?
                .build(),
        )
        .await?;
        bounded(TIMEOUT, conn.delete()).await
    }

    async fn set_radio(&self, on: bool) -> Result<(), NetworkError> {
        let root = self.root().await?;
        bounded(TIMEOUT, root.set_wireless_enabled(on)).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn text(section: &HashMap<&'static str, Value<'static>>, key: &str) -> Option<String> {
        section.get(key).and_then(|v| String::try_from(v.clone()).ok())
    }

    #[test]
    fn a_wpa2_network_is_described_by_its_key_management_and_passphrase() {
        let settings = connection_settings(
            &Ssid::new("home"),
            Security::Wpa2Personal,
            Some(&Psk::new("correcthorse")),
        );
        let sec = settings
            .get("802-11-wireless-security")
            .expect("a WPA2 network needs a security section");
        assert_eq!(text(sec, "key-mgmt").as_deref(), Some("wpa-psk"));
        assert_eq!(text(sec, "psk").as_deref(), Some("correcthorse"));
        assert_eq!(
            text(&settings["connection"], "type").as_deref(),
            Some("802-11-wireless")
        );
    }

    #[test]
    fn a_wpa3_network_asks_for_sae() {
        let settings = connection_settings(
            &Ssid::new("home"),
            Security::Wpa3Personal,
            Some(&Psk::new("correcthorse")),
        );
        assert_eq!(
            text(&settings["802-11-wireless-security"], "key-mgmt").as_deref(),
            Some("sae")
        );
    }

    /// A security section with `key-mgmt = none` describes a WEP network,
    /// and sending one to an access point with no encryption at all fails
    /// rather than connecting openly.
    #[test]
    fn an_open_network_is_sent_with_no_security_section_at_all() {
        let settings = connection_settings(&Ssid::new("cafe"), Security::Open, None);
        assert!(!settings.contains_key("802-11-wireless-security"));
    }

    #[test]
    fn an_owe_network_carries_key_management_but_no_passphrase() {
        let settings = connection_settings(&Ssid::new("cafe"), Security::Owe, None);
        let sec = &settings["802-11-wireless-security"];
        assert_eq!(text(sec, "key-mgmt").as_deref(), Some("owe"));
        assert!(!sec.contains_key("psk"));
    }

    /// The mechanical check that the passphrase goes in one place.
    ///
    /// `Psk`'s `Debug` protects it right up to the moment it is written
    /// into this dictionary, where it has to become a plain string
    /// because NetworkManager needs it. What it must not do is end up in
    /// a second field as well — `connection.id` is the obvious hazard,
    /// since that is a free-text label and the sort of thing a later
    /// change might helpfully fill in. Every other section is searched
    /// for the value.
    #[test]
    fn the_passphrase_is_written_into_the_security_section_and_nowhere_else() {
        let secret = "correct-horse-battery";
        let settings = connection_settings(
            &Ssid::new("home"),
            Security::Wpa2Personal,
            Some(&Psk::new(secret)),
        );

        for (name, section) in &settings {
            for (key, value) in section {
                if *name == "802-11-wireless-security" && *key == "psk" {
                    continue;
                }
                let rendered = format!("{value:?}");
                assert!(
                    !rendered.contains(secret),
                    "the passphrase reached {name}.{key}, which is not where it is meant to be"
                );
            }
        }
    }

    /// The label is lossy text; the thing associated with is bytes. An
    /// SSID that is not valid UTF-8 must survive the round trip into the
    /// settings dictionary, or the network simply cannot be joined.
    #[test]
    fn the_ssid_is_sent_as_its_original_bytes_not_as_its_label() {
        let raw = vec![b'c', b'a', b'f', 0xE9];
        let settings = connection_settings(&Ssid::new(raw.clone()), Security::Open, None);
        let sent = Vec::<u8>::try_from(settings["802-11-wireless"]["ssid"].clone())
            .expect("the ssid is sent as a byte array");
        assert_eq!(sent, raw);
        // The human-facing label is separate and may be lossy.
        assert!(text(&settings["connection"], "id").is_some());
    }

    /// The mapping everything wired rests on. `UNAVAILABLE` (20) is the
    /// state an unplugged cable puts a device in — and not the only
    /// reason a device can be unavailable, which is why the carrier
    /// decides between "unplugged" and "not connected".
    #[test]
    fn a_wired_devices_state_and_carrier_map_to_the_four_states_people_act_on() {
        assert_eq!(wired_state(100, true), Some(WiredState::Connected));
        for connecting in [40, 50, 60, 70, 80, 90] {
            assert_eq!(wired_state(connecting, true), Some(WiredState::Connecting), "{connecting}");
        }
        assert_eq!(wired_state(20, false), Some(WiredState::CableUnplugged));
        assert_eq!(wired_state(30, false), Some(WiredState::CableUnplugged));
        assert_eq!(wired_state(30, true), Some(WiredState::Disconnected));
        assert_eq!(wired_state(120, true), Some(WiredState::Disconnected), "failed");
        assert_eq!(wired_state(110, true), Some(WiredState::Disconnected), "deactivating");
    }

    /// Unmanaged is NetworkManager saying "not mine" — a device someone
    /// configured by hand. Showing it with a connect button would offer
    /// to take it over.
    #[test]
    fn a_device_networkmanager_does_not_manage_is_not_listed() {
        assert_eq!(wired_state(10, true), None);
        assert_eq!(wired_state(0, true), None);
    }

    /// Timeouts exist, and the join one is not the read one: capping an
    /// activation at five seconds would report a working network as
    /// broken on any AP with a slow DHCP server.
    #[test]
    fn joining_is_given_longer_than_a_property_read_but_is_still_bounded() {
        assert!(ACTIVATION_TIMEOUT > TIMEOUT);
        assert!(ACTIVATION_TIMEOUT <= Duration::from_secs(60));
    }
}
