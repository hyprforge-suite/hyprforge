//! The real [`BatteryBackend`], talking to UPower on the system bus.
//!
//! Everything here is D-Bus mechanics: which device is "the" battery, and
//! how its raw properties map to [`BatteryState`] and a rounded
//! percentage, are pure functions kept testable without a bus — see
//! [`crate::types`] for the ones that live above this line, and this
//! file's own `#[cfg(test)]` block for the two (`duration_from_upower_seconds`,
//! `percentage_from_upower`) that are D-Bus-adjacent enough to live
//! here instead.

use crate::backend::BatteryBackend;
use crate::types::{BatteryError, BatteryInfo, BatteryState};
use std::time::Duration;
use zbus::zvariant::OwnedObjectPath;
use zbus::Connection;

/// A property read or an `EnumerateDevices` call — nothing here waits on
/// hardware the way NetworkManager's activation does, so one bound
/// covers everything.
pub const TIMEOUT: Duration = Duration::from_secs(5);

/// `UP_DEVICE_KIND_BATTERY` from upower's own headers.
///
/// Not sufficient on its own to identify *the* battery: a wireless mouse
/// or a Bluetooth headset also reports `Type == Battery`. `power_supply`
/// and `is_present`, checked alongside it in
/// [`UPowerBackend::system_battery_device`], are what narrow it down to
/// the battery that keeps the machine itself running.
const DEVICE_TYPE_BATTERY: u32 = 2;

#[zbus::proxy(
    interface = "org.freedesktop.UPower",
    default_service = "org.freedesktop.UPower",
    default_path = "/org/freedesktop/UPower"
)]
trait UPowerManagerDbus {
    fn enumerate_devices(&self) -> zbus::Result<Vec<OwnedObjectPath>>;
}

#[zbus::proxy(
    interface = "org.freedesktop.UPower.Device",
    default_service = "org.freedesktop.UPower"
)]
trait UPowerDeviceDbus {
    #[zbus(property, name = "Type")]
    fn kind(&self) -> zbus::Result<u32>;
    #[zbus(property)]
    fn state(&self) -> zbus::Result<u32>;
    #[zbus(property)]
    fn percentage(&self) -> zbus::Result<f64>;
    #[zbus(property)]
    fn time_to_empty(&self) -> zbus::Result<i64>;
    #[zbus(property)]
    fn time_to_full(&self) -> zbus::Result<i64>;
    /// "if the power device is a battery that this machine cannot live
    /// without" — UPower's own words. A wireless mouse's battery answers
    /// `false` here; the laptop's own battery answers `true`.
    #[zbus(property)]
    fn power_supply(&self) -> zbus::Result<bool>;
    #[zbus(property)]
    fn is_present(&self) -> zbus::Result<bool>;
}

/// Bounds a call and turns a D-Bus failure into something a screen can
/// say out loud.
async fn bounded<T>(
    limit: Duration,
    call: impl std::future::Future<Output = zbus::Result<T>>,
) -> Result<T, BatteryError> {
    match tokio::time::timeout(limit, call).await {
        Err(_elapsed) => Err(BatteryError::TimedOut(limit)),
        Ok(Ok(value)) => Ok(value),
        Ok(Err(e)) => Err(classify(e)),
    }
}

/// Which failures mean "UPower isn't there" and which mean "UPower said
/// no". Collapsing the two would put a specific-looking battery reading
/// on screen for a machine whose UPower is simply not running — the
/// mistake this project keeps finding in every D-Bus-backed module.
fn classify(e: zbus::Error) -> BatteryError {
    if let zbus::Error::MethodError(name, _, _) = &e {
        let name = name.as_str();
        if name.ends_with(".ServiceUnknown") || name.ends_with(".NameHasNoOwner") {
            return BatteryError::Unavailable;
        }
        return BatteryError::Refused(e.to_string());
    }
    match e {
        zbus::Error::Address(_) | zbus::Error::InputOutput(_) => BatteryError::Unavailable,
        other => BatteryError::Refused(other.to_string()),
    }
}

/// UPower reports `TimeToEmpty`/`TimeToFull` as signed seconds, `0` while
/// it is still estimating. Neither a negative value (never documented,
/// and `Duration` cannot represent one) nor that estimating-`0` is a real
/// duration — both become "nothing to say yet" rather than a `Duration`
/// construction that would panic, or a claim of "zero time left" nobody
/// asked for.
fn duration_from_upower_seconds(seconds: i64) -> Option<Duration> {
    if seconds <= 0 {
        return None;
    }
    Some(Duration::from_secs(seconds as u64))
}

/// UPower's `Percentage` is a `d` and can, on some hardware, briefly
/// report a shade outside 0..=100 while the fuel gauge settles. Rounding
/// and clamping once, here, is what lets [`BatteryInfo::percentage`]
/// promise 0..=100 to everything above it without every caller re-doing
/// the check.
fn percentage_from_upower(value: f64) -> u8 {
    value.round().clamp(0.0, 100.0) as u8
}

/// UPower, as this crate uses it.
pub struct UPowerBackend {
    connection: Connection,
}

impl UPowerBackend {
    /// Opens the system bus.
    ///
    /// Failing here is [`BatteryError::Unavailable`] rather than a panic:
    /// UPower is D-Bus-activated on most systems, so this can fail simply
    /// because the system bus itself is unreachable, which is a state the
    /// screen has to describe, not crash on.
    pub async fn connect() -> Result<Self, BatteryError> {
        let connection = tokio::time::timeout(TIMEOUT, Connection::system())
            .await
            .map_err(|_| BatteryError::TimedOut(TIMEOUT))?
            .map_err(classify)?;
        Ok(UPowerBackend { connection })
    }

    /// The one device this crate calls "the battery": `Type == Battery`,
    /// `PowerSupply == true`, `IsPresent == true`. `Ok(None)` when no
    /// device matches — a desktop with only `AC`, which is a normal
    /// machine and not a UPower failure.
    async fn system_battery_device(&self) -> Result<Option<UPowerDeviceDbusProxy<'_>>, BatteryError> {
        let manager = bounded(TIMEOUT, UPowerManagerDbusProxy::new(&self.connection)).await?;
        let paths = bounded(TIMEOUT, manager.enumerate_devices()).await?;
        for path in paths {
            let device = bounded(
                TIMEOUT,
                UPowerDeviceDbusProxy::builder(&self.connection)
                    .path(path)
                    .map_err(classify)?
                    .build(),
            )
            .await?;
            if bounded(TIMEOUT, device.kind()).await? != DEVICE_TYPE_BATTERY {
                continue;
            }
            if !bounded(TIMEOUT, device.power_supply()).await? {
                continue;
            }
            if !bounded(TIMEOUT, device.is_present()).await? {
                continue;
            }
            return Ok(Some(device));
        }
        Ok(None)
    }
}

#[async_trait::async_trait]
impl BatteryBackend for UPowerBackend {
    async fn battery(&self) -> Result<Option<BatteryInfo>, BatteryError> {
        let Some(device) = self.system_battery_device().await? else {
            return Ok(None);
        };
        let percentage = percentage_from_upower(bounded(TIMEOUT, device.percentage()).await?);
        let state = BatteryState::from_upower(bounded(TIMEOUT, device.state()).await?);
        let time_to_empty =
            duration_from_upower_seconds(bounded(TIMEOUT, device.time_to_empty()).await?);
        let time_to_full =
            duration_from_upower_seconds(bounded(TIMEOUT, device.time_to_full()).await?);
        Ok(Some(BatteryInfo { percentage, state, time_to_empty, time_to_full }))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_zero_or_negative_upower_duration_is_treated_as_still_estimating() {
        assert_eq!(duration_from_upower_seconds(0), None);
        assert_eq!(duration_from_upower_seconds(-1), None, "UPower has never documented a negative value, and Duration cannot hold one");
    }

    #[test]
    fn a_positive_upower_duration_converts_directly_to_seconds() {
        assert_eq!(duration_from_upower_seconds(90), Some(Duration::from_secs(90)));
    }

    #[test]
    fn percentage_rounds_to_the_nearest_whole_number() {
        assert_eq!(percentage_from_upower(42.4), 42);
        assert_eq!(percentage_from_upower(42.5), 43);
    }

    /// The fuel gauge briefly reporting outside 0..=100 must not become a
    /// percentage nothing above this line can trust to be in range.
    #[test]
    fn percentage_is_clamped_to_0_through_100_even_if_upower_reports_outside_it() {
        assert_eq!(percentage_from_upower(-3.0), 0);
        assert_eq!(percentage_from_upower(103.2), 100);
    }

    #[test]
    fn device_type_battery_is_two() {
        // Pinned so nobody "simplifies" it back to a guess: this is
        // `UP_DEVICE_KIND_BATTERY` from upower's own enum, checked live in
        // `tests/live_upower.rs` against a real UPower.
        assert_eq!(DEVICE_TYPE_BATTERY, 2);
    }
}
