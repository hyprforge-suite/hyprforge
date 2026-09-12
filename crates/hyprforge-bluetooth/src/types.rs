//! What a Bluetooth adapter and its devices look like to this crate,
//! independent of D-Bus.

use std::fmt;

/// A BDADDR, as BlueZ writes it: `AA:BB:CC:DD:EE:FF`.
///
/// A newtype rather than a `String` because it is also the identity a
/// device is addressed by — the object path is derived from it
/// (`/org/bluez/hci0/dev_AA_BB_CC_DD_EE_FF`) — and mixing it up with a
/// display name is the mistake worth making impossible.
#[derive(Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct Address(String);

impl Address {
    pub fn new(value: impl Into<String>) -> Self {
        Address(value.into())
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for Address {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl fmt::Debug for Address {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Address({})", self.0)
    }
}

/// What kind of thing this is, for the icon beside its name.
///
/// Derived from BlueZ's own `Icon` property rather than from the class-of
/// -device bits: BlueZ has already done that decoding, and re-deriving it
/// here would be a second, drifting copy of a mapping that is not ours.
/// `Icon` is documented as *optional*, so [`DeviceKind::Unknown`] is a
/// real answer and not a parse failure.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DeviceKind {
    Headset,
    Speaker,
    Headphones,
    Keyboard,
    Mouse,
    Gamepad,
    Phone,
    Computer,
    Watch,
    Printer,
    Unknown,
}

impl DeviceKind {
    /// Maps BlueZ's freedesktop icon names.
    pub fn from_icon(icon: Option<&str>) -> DeviceKind {
        match icon.unwrap_or_default() {
            "audio-headset" => DeviceKind::Headset,
            "audio-headphones" => DeviceKind::Headphones,
            "audio-speakers" | "audio-card" => DeviceKind::Speaker,
            "input-keyboard" => DeviceKind::Keyboard,
            "input-mouse" | "input-tablet" => DeviceKind::Mouse,
            "input-gaming" => DeviceKind::Gamepad,
            "phone" => DeviceKind::Phone,
            "computer" => DeviceKind::Computer,
            "watch" => DeviceKind::Watch,
            "printer" => DeviceKind::Printer,
            _ => DeviceKind::Unknown,
        }
    }

    /// A short word for the row's second line.
    pub fn label(self) -> &'static str {
        match self {
            DeviceKind::Headset => "Headset",
            DeviceKind::Headphones => "Headphones",
            DeviceKind::Speaker => "Speaker",
            DeviceKind::Keyboard => "Keyboard",
            DeviceKind::Mouse => "Mouse",
            DeviceKind::Gamepad => "Controller",
            DeviceKind::Phone => "Phone",
            DeviceKind::Computer => "Computer",
            DeviceKind::Watch => "Watch",
            DeviceKind::Printer => "Printer",
            DeviceKind::Unknown => "Device",
        }
    }
}

/// One remote device, as the list shows it.
#[derive(Debug, Clone, PartialEq)]
pub struct Device {
    pub address: Address,
    /// BlueZ's `Alias`, which always exists — it falls back to `Name`, and
    /// then to the address with colons turned to dashes.
    pub alias: String,
    /// `Name` is documented as optional and is genuinely absent for a
    /// device that has never been resolved. Kept separate from `alias` so
    /// "this device has not told us its name yet" stays distinguishable
    /// from "it is called this".
    pub name: Option<String>,
    pub kind: DeviceKind,
    pub paired: bool,
    pub trusted: bool,
    pub connected: bool,
    /// `RSSI` is optional: it is absent for a device that is known but not
    /// currently advertising. `None` means "not in range right now", which
    /// is not the same as a weak signal.
    pub rssi: Option<i16>,
}

impl Device {
    /// Whether this device has ever told us its name.
    ///
    /// A device that has not shows its address, and saying so is better
    /// than showing an address that looks like a name.
    pub fn is_unnamed(&self) -> bool {
        self.name.is_none()
    }

    /// Why a device is listed but cannot be acted on from here yet.
    ///
    /// Pairing needs an `org.bluez.Agent1` — a passkey or yes/no
    /// confirmation exchanged with the remote device — which is its own
    /// piece of work. An unpaired device is still *listed*, because a
    /// device missing from the list is a bug report and a device that
    /// says "not yet" is an answer. Same call as enterprise Wi-Fi.
    pub fn unsupported_reason(&self) -> Option<&'static str> {
        (!self.paired).then_some(
            "Pairing a new device isn't supported yet — it needs a passkey \
             confirmation Hyprforge can't show. Pair it with `bluetoothctl`, \
             and it will appear here ready to connect.",
        )
    }
}

/// Whether the adapter is on, and whether it is ours to turn on.
///
/// `HardwareBlocked` is rfkill — a switch or an Fn key — and BlueZ reports
/// it as `PowerState = "off-blocked"`. Telling the user to flip it is the
/// only useful answer, and is not the same message as "Bluetooth is off".
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AdapterState {
    On,
    Off,
    HardwareBlocked,
    /// Transitioning. Shown as busy rather than as either end state, so a
    /// toggle does not flick back under the pointer.
    Changing,
}

/// The adapter and what it is doing.
#[derive(Debug, Clone, PartialEq)]
pub struct Status {
    pub state: AdapterState,
    pub discovering: bool,
    /// The adapter's own friendly name, for "Visible as ..." .
    pub alias: String,
}

/// What went wrong, in terms a screen can show without saying "check the
/// logs" (vision pillar #3).
///
/// [`BluetoothError::Unavailable`] is separate from an empty device list
/// for the same reason `NetworkError` makes that split: a stopped
/// `bluetooth.service` must never render as "no devices nearby".
#[derive(Debug, thiserror::Error)]
pub enum BluetoothError {
    #[error("Bluetooth isn't running, so devices can't be listed or connected. Start it with `systemctl start bluetooth`.")]
    Unavailable,
    #[error("This machine has no Bluetooth adapter that BlueZ can see.")]
    NoAdapter,
    #[error("Bluetooth is blocked by a hardware switch. Turn it back on with the switch or key on your machine.")]
    HardwareBlocked,
    #[error("BlueZ didn't answer within {0:?}.")]
    TimedOut(std::time::Duration),
    #[error("{0}")]
    Refused(String),
}

#[cfg(test)]
mod tests {
    use super::*;

    fn device(paired: bool, name: Option<&str>) -> Device {
        Device {
            address: Address::new("AA:BB:CC:DD:EE:FF"),
            alias: name.unwrap_or("AA-BB-CC-DD-EE-FF").to_string(),
            name: name.map(str::to_string),
            kind: DeviceKind::Headset,
            paired,
            trusted: false,
            connected: false,
            rssi: Some(-60),
        }
    }

    /// BlueZ has already decoded the class-of-device bits into `Icon`.
    /// Re-deriving the mapping here would be a second copy of something
    /// that is not ours to own.
    #[test]
    fn a_device_kind_comes_from_bluez_own_icon_name() {
        assert_eq!(DeviceKind::from_icon(Some("audio-headset")), DeviceKind::Headset);
        assert_eq!(DeviceKind::from_icon(Some("input-keyboard")), DeviceKind::Keyboard);
    }

    /// `Icon` is optional, so an unrecognised or missing one is a real
    /// answer rather than something to report as broken.
    #[test]
    fn a_device_with_no_icon_is_still_a_device() {
        assert_eq!(DeviceKind::from_icon(None), DeviceKind::Unknown);
        assert_eq!(DeviceKind::from_icon(Some("something-new")), DeviceKind::Unknown);
        assert_eq!(DeviceKind::Unknown.label(), "Device");
    }

    /// An unpaired device is listed and told why it cannot be connected,
    /// rather than being hidden or offered a button that fails.
    #[test]
    fn an_unpaired_device_is_listed_but_says_why_it_cannot_be_used_yet() {
        let d = device(false, Some("WH-1000XM4"));
        assert!(d.unsupported_reason().is_some_and(|r| r.contains("bluetoothctl")));
        assert!(device(true, Some("WH-1000XM4")).unsupported_reason().is_none());
    }

    /// A device that has never resolved its name has no `Name`, only an
    /// `Alias` derived from the address. Showing that address as though
    /// it were a name is worse than saying the name is not known.
    #[test]
    fn a_device_that_has_not_given_its_name_is_distinguishable_from_one_that_has() {
        assert!(device(true, None).is_unnamed());
        assert!(!device(true, Some("WH-1000XM4")).is_unnamed());
    }

    /// The distinction this crate exists to keep.
    #[test]
    fn an_absent_bluez_says_so_rather_than_reporting_no_devices() {
        let message = BluetoothError::Unavailable.to_string();
        assert!(message.contains("isn't running"));
        assert!(
            message.contains("systemctl"),
            "an error with no way out of it is the dead end pillar 3 forbids"
        );
    }

    /// rfkill is not the same as "switched off in software", and a toggle
    /// that cannot work must not be offered for it.
    #[test]
    fn a_hardware_block_is_its_own_state_not_merely_off() {
        assert_ne!(AdapterState::HardwareBlocked, AdapterState::Off);
        assert!(BluetoothError::HardwareBlocked.to_string().contains("switch"));
    }
}
