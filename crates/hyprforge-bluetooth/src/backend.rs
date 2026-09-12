//! The boundary between this crate's logic and BlueZ.
//!
//! Same arrangement as `hyprforge-network`'s `NetworkBackend` and
//! `hyprforge-displayd`'s `OutputBackend`, for the same reason: the
//! machine running tier 1 has no guaranteed `bluetoothd`, no guaranteed
//! adapter, and certainly no headset to connect to. Everything above this
//! trait is testable without any of it.

use crate::types::{Address, BluetoothError, Device, Status};

#[async_trait::async_trait]
pub trait BluetoothBackend: Send + Sync {
    async fn status(&self) -> Result<Status, BluetoothError>;

    /// Every device BlueZ knows about — paired ones and anything seen
    /// during the current discovery session.
    async fn devices(&self) -> Result<Vec<Device>, BluetoothError>;

    /// Starts or stops a discovery session.
    ///
    /// Separate from [`BluetoothBackend::devices`] because discovery
    /// costs battery and airtime on both ends, and must not be implied by
    /// a screen repainting. It is also how BlueZ populates the device
    /// list in the first place, so the screen has to ask for it.
    async fn set_discovery(&self, on: bool) -> Result<(), BluetoothError>;

    async fn set_powered(&self, on: bool) -> Result<(), BluetoothError>;

    /// Pairs with a device, which is a *conversation*: BlueZ will call
    /// back into whatever agent is registered and wait for the user.
    ///
    /// Long-running by nature, and the one operation here whose duration
    /// is set by a person rather than a radio.
    async fn pair(&self, address: &Address) -> Result<(), BluetoothError>;

    /// Abandons a pairing in progress. Needed because the far end may
    /// never answer, and a dialog the user dismissed must not leave BlueZ
    /// waiting.
    async fn cancel_pairing(&self, address: &Address) -> Result<(), BluetoothError>;

    async fn connect(&self, address: &Address) -> Result<(), BluetoothError>;
    async fn disconnect(&self, address: &Address) -> Result<(), BluetoothError>;

    /// Trusting a device lets it reconnect on its own — a headset that
    /// comes back when you switch it on rather than needing a click.
    async fn set_trusted(&self, address: &Address, trusted: bool) -> Result<(), BluetoothError>;

    /// Removes the pairing entirely.
    async fn forget(&self, address: &Address) -> Result<(), BluetoothError>;
}

/// Orders the device list the way the screen shows it.
///
/// Connected first, then paired, then everything else — because the row a
/// user came to press is nearly always one of the first two groups, while
/// discovery is busily appending strangers to the bottom. Within a group,
/// by name, so a list that is being appended to while it is read does not
/// reorder under the pointer.
///
/// Deliberately *not* sorted by RSSI: a Bluetooth signal fluctuates far
/// more than a Wi-Fi one, and sorting by it makes the list jump every few
/// seconds. `hyprforge-network` sorts by strength because its list is
/// rebuilt from a scan every ten seconds; this one is a live object tree.
pub fn for_display(mut devices: Vec<Device>) -> Vec<Device> {
    devices.sort_by(|a, b| {
        fn rank(d: &Device) -> u8 {
            match (d.connected, d.paired) {
                (true, _) => 0,
                (_, true) => 1,
                _ => 2,
            }
        }
        rank(a)
            .cmp(&rank(b))
            .then_with(|| a.alias.to_lowercase().cmp(&b.alias.to_lowercase()))
            .then_with(|| a.address.cmp(&b.address))
    });
    devices
}

#[cfg(any(test, feature = "mock"))]
pub mod mock {
    use super::*;
    use crate::types::AdapterState;
    use std::sync::Mutex;

    /// A BlueZ that does what the test says.
    #[derive(Default)]
    pub struct MockBackend {
        pub devices: Mutex<Vec<Device>>,
        pub status: Mutex<Option<Status>>,
        /// Every call fails, for the "daemon isn't there" path.
        pub unavailable: Mutex<bool>,
        /// Every *action* fails with this, for the refusal paths.
        pub refuse: Mutex<Option<String>>,
        pub discovery_calls: Mutex<Vec<bool>>,
        pub connected: Mutex<Vec<Address>>,
        pub forgotten: Mutex<Vec<Address>>,
        pub paired: Mutex<Vec<Address>>,
        pub cancelled: Mutex<Vec<Address>>,
        pub trusted: Mutex<Vec<(Address, bool)>>,
    }

    impl MockBackend {
        pub fn new() -> Self {
            Self::default()
        }

        pub fn with_devices(devices: Vec<Device>) -> Self {
            let backend = Self::new();
            *backend.devices.lock().unwrap() = devices;
            backend
        }

        pub fn set_status(&self, status: Status) {
            *self.status.lock().unwrap() = Some(status);
        }

        fn guard(&self) -> Result<(), BluetoothError> {
            if *self.unavailable.lock().unwrap() {
                return Err(BluetoothError::Unavailable);
            }
            Ok(())
        }

        fn guard_action(&self) -> Result<(), BluetoothError> {
            self.guard()?;
            if let Some(reason) = self.refuse.lock().unwrap().clone() {
                return Err(BluetoothError::Refused(reason));
            }
            Ok(())
        }

        fn with_device<F>(&self, address: &Address, f: F)
        where
            F: FnOnce(&mut Device),
        {
            let mut devices = self.devices.lock().unwrap();
            if let Some(d) = devices.iter_mut().find(|d| &d.address == address) {
                f(d);
            }
        }
    }

    #[async_trait::async_trait]
    impl BluetoothBackend for MockBackend {
        async fn status(&self) -> Result<Status, BluetoothError> {
            self.guard()?;
            Ok(self.status.lock().unwrap().clone().unwrap_or(Status {
                state: AdapterState::On,
                discovering: false,
                alias: "mock-adapter".to_string(),
            }))
        }

        async fn devices(&self) -> Result<Vec<Device>, BluetoothError> {
            self.guard()?;
            Ok(self.devices.lock().unwrap().clone())
        }

        async fn set_discovery(&self, on: bool) -> Result<(), BluetoothError> {
            self.guard_action()?;
            self.discovery_calls.lock().unwrap().push(on);
            let mut status = self.status.lock().unwrap();
            let current = status.clone().unwrap_or(Status {
                state: AdapterState::On,
                discovering: false,
                alias: "mock-adapter".to_string(),
            });
            *status = Some(Status { discovering: on, ..current });
            Ok(())
        }

        async fn set_powered(&self, on: bool) -> Result<(), BluetoothError> {
            self.guard_action()?;
            let mut status = self.status.lock().unwrap();
            let current = status.clone().unwrap_or(Status {
                state: AdapterState::On,
                discovering: false,
                alias: "mock-adapter".to_string(),
            });
            *status = Some(Status {
                state: if on { AdapterState::On } else { AdapterState::Off },
                // Powering off ends discovery; leaving it true would show a
                // spinner on an adapter that is off.
                discovering: on && current.discovering,
                ..current
            });
            Ok(())
        }

        async fn pair(&self, address: &Address) -> Result<(), BluetoothError> {
            self.guard_action()?;
            self.paired.lock().unwrap().push(address.clone());
            self.with_device(address, |d| d.paired = true);
            Ok(())
        }

        async fn cancel_pairing(&self, address: &Address) -> Result<(), BluetoothError> {
            self.guard_action()?;
            self.cancelled.lock().unwrap().push(address.clone());
            Ok(())
        }

        async fn connect(&self, address: &Address) -> Result<(), BluetoothError> {
            self.guard_action()?;
            self.connected.lock().unwrap().push(address.clone());
            self.with_device(address, |d| d.connected = true);
            Ok(())
        }

        async fn disconnect(&self, address: &Address) -> Result<(), BluetoothError> {
            self.guard_action()?;
            self.with_device(address, |d| d.connected = false);
            Ok(())
        }

        async fn set_trusted(&self, address: &Address, trusted: bool) -> Result<(), BluetoothError> {
            self.guard_action()?;
            self.trusted.lock().unwrap().push((address.clone(), trusted));
            self.with_device(address, |d| d.trusted = trusted);
            Ok(())
        }

        async fn forget(&self, address: &Address) -> Result<(), BluetoothError> {
            self.guard_action()?;
            self.forgotten.lock().unwrap().push(address.clone());
            self.devices.lock().unwrap().retain(|d| &d.address != address);
            Ok(())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::mock::MockBackend;
    use super::*;
    use crate::types::DeviceKind;

    fn device(alias: &str, addr: &str, paired: bool, connected: bool) -> Device {
        Device {
            address: Address::new(addr),
            alias: alias.to_string(),
            name: Some(alias.to_string()),
            kind: DeviceKind::Headset,
            paired,
            trusted: false,
            connected,
            rssi: Some(-55),
        }
    }

    /// The row someone came to press is almost always one they already
    /// own, and discovery appends strangers to the list while they look
    /// for it.
    #[test]
    fn the_devices_already_in_use_sort_above_the_ones_just_discovered() {
        let listed = for_display(vec![
            device("Zeta stranger", "00:00:00:00:00:03", false, false),
            device("Alpha paired", "00:00:00:00:00:02", true, false),
            device("Zulu connected", "00:00:00:00:00:01", true, true),
        ]);
        let names: Vec<_> = listed.iter().map(|d| d.alias.as_str()).collect();
        assert_eq!(names, ["Zulu connected", "Alpha paired", "Zeta stranger"]);
    }

    /// A Bluetooth RSSI swings far more than a Wi-Fi one. Sorting by it
    /// would make the list reorder every few seconds under the pointer,
    /// which is why this list is grouped and named rather than ranked.
    #[test]
    fn a_changing_signal_does_not_reorder_the_list() {
        let weak = device("Headset", "00:00:00:00:00:01", true, false);
        let mut strong = weak.clone();
        strong.rssi = Some(-20);
        let other = device("Alpha", "00:00:00:00:00:02", true, false);

        let with_weak = for_display(vec![weak, other.clone()]);
        let with_strong = for_display(vec![strong, other]);
        let names = |v: Vec<Device>| v.iter().map(|d| d.alias.clone()).collect::<Vec<_>>();
        assert_eq!(names(with_weak), names(with_strong));
    }

    #[tokio::test]
    async fn an_unavailable_bluez_is_an_error_on_every_call_not_an_empty_list() {
        let backend = MockBackend::with_devices(vec![device("H", "00:00:00:00:00:01", true, false)]);
        *backend.unavailable.lock().unwrap() = true;
        assert!(matches!(backend.devices().await, Err(BluetoothError::Unavailable)));
        assert!(matches!(backend.status().await, Err(BluetoothError::Unavailable)));
    }

    /// Discovery is an explicit request, not something a list refresh
    /// turns on behind the user's back — it costs battery at both ends.
    #[tokio::test]
    async fn listing_devices_does_not_start_a_discovery_session() {
        let backend = MockBackend::with_devices(vec![device("H", "00:00:00:00:00:01", true, false)]);
        let _ = backend.devices().await.unwrap();
        assert!(backend.discovery_calls.lock().unwrap().is_empty());
    }

    /// An adapter that is off cannot be discovering, and a screen showing
    /// a spinner over a powered-off adapter is showing a lie.
    #[tokio::test]
    async fn powering_the_adapter_off_also_ends_discovery() {
        let backend = MockBackend::new();
        backend.set_discovery(true).await.unwrap();
        assert!(backend.status().await.unwrap().discovering);

        backend.set_powered(false).await.unwrap();
        let status = backend.status().await.unwrap();
        assert_eq!(status.state, crate::types::AdapterState::Off);
        assert!(!status.discovering, "an adapter that is off is not discovering");
    }

    /// Pairing is what turns a listed stranger into something usable, so
    /// a paired device must actually read back as paired — the list
    /// groups on exactly that.
    #[tokio::test]
    async fn a_paired_device_reads_back_as_paired() {
        let addr = Address::new("00:00:00:00:00:01");
        let backend =
            MockBackend::with_devices(vec![device("Headset", addr.as_str(), false, false)]);
        assert!(!backend.devices().await.unwrap()[0].paired);

        backend.pair(&addr).await.unwrap();

        let device = backend.devices().await.unwrap().remove(0);
        assert!(device.paired);
        assert!(device.paired);
    }

    #[tokio::test]
    async fn forgetting_a_device_removes_it_from_the_list() {
        let addr = Address::new("00:00:00:00:00:01");
        let backend = MockBackend::with_devices(vec![device("H", addr.as_str(), true, false)]);
        backend.forget(&addr).await.unwrap();
        assert!(backend.devices().await.unwrap().is_empty());
    }

    #[tokio::test]
    async fn trusting_a_device_is_recorded_so_it_can_reconnect_on_its_own() {
        let addr = Address::new("00:00:00:00:00:01");
        let backend = MockBackend::with_devices(vec![device("H", addr.as_str(), true, false)]);
        backend.set_trusted(&addr, true).await.unwrap();
        assert!(backend.devices().await.unwrap()[0].trusted);
    }
}
