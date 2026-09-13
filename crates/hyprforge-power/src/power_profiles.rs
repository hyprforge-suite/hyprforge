//! The real [`PowerProfilesBackend`], talking to `power-profiles-daemon`
//! on the system bus.
//!
//! # Which bus name
//!
//! `power-profiles-daemon` publishes its interface under two names: its
//! own `net.hadess.PowerProfiles`, and an alias,
//! `org.freedesktop.UPower.PowerProfiles`, that exists so desktop
//! environments already expecting everything power-related under the
//! `UPower` umbrella can find it there too. This crate uses
//! `net.hadess.PowerProfiles` — it is the daemon's own name rather than a
//! compatibility alias grafted on for someone else's convenience, so it
//! is the one guaranteed to keep working if the alias is ever dropped,
//! and it is what the daemon's own CLI, `powerprofilesctl`, talks to.
//!
//! # Nothing here writes during a test
//!
//! [`PowerProfilesBackend::set_active_profile`] is a real write — it
//! changes how the machine behaves for whoever is using it — and
//! `tests/live_power_profiles.rs` never calls it. See that file's module
//! doc for the read-only rule this crate's live tier follows everywhere.

use crate::backend::PowerProfilesBackend;
use crate::types::{PowerProfile, ProfileError};
use std::collections::HashMap;
use std::time::Duration;
use zbus::zvariant::OwnedValue;
use zbus::Connection;

pub const TIMEOUT: Duration = Duration::from_secs(5);

#[zbus::proxy(
    interface = "net.hadess.PowerProfiles",
    default_service = "net.hadess.PowerProfiles",
    default_path = "/net/hadess/PowerProfiles"
)]
trait PowerProfilesDbus {
    /// `aa{sv}`: one dict per offered profile, each carrying at least a
    /// `Profile` key. `Driver` and `PerformanceDegraded` also live in
    /// each dict but are not read here — this crate only needs the name.
    #[zbus(property)]
    fn profiles(&self) -> zbus::Result<Vec<HashMap<String, OwnedValue>>>;
    #[zbus(property)]
    fn active_profile(&self) -> zbus::Result<String>;
    #[zbus(property)]
    fn set_active_profile(&self, value: &str) -> zbus::Result<()>;
}

/// Bounds a call and turns a D-Bus failure into something a screen can
/// say out loud.
async fn bounded<T>(
    limit: Duration,
    call: impl std::future::Future<Output = zbus::Result<T>>,
) -> Result<T, ProfileError> {
    match tokio::time::timeout(limit, call).await {
        Err(_elapsed) => Err(ProfileError::TimedOut(limit)),
        Ok(Ok(value)) => Ok(value),
        Ok(Err(e)) => Err(classify(e)),
    }
}

/// Which failures mean "power-profiles-daemon isn't there" and which mean
/// "power-profiles-daemon said no". Collapsing the two would put a
/// specific-looking active profile on screen for a machine whose daemon
/// is simply not running — the mistake this project keeps finding in
/// every D-Bus-backed module.
fn classify(e: zbus::Error) -> ProfileError {
    if let zbus::Error::MethodError(name, _, _) = &e {
        let name = name.as_str();
        if name.ends_with(".ServiceUnknown") || name.ends_with(".NameHasNoOwner") {
            return ProfileError::Unavailable;
        }
        return ProfileError::Refused(e.to_string());
    }
    match e {
        zbus::Error::Address(_) | zbus::Error::InputOutput(_) => ProfileError::Unavailable,
        other => ProfileError::Refused(other.to_string()),
    }
}

/// Pulls the `Profile` name out of one entry of the `Profiles` property.
///
/// Kept free of `zbus::Connection` so it can be tested against a
/// hand-built dictionary without a bus — the one place in this file the
/// parsing is worth separating from the call that fetches it.
fn profile_name(entry: &HashMap<String, OwnedValue>) -> Option<String> {
    entry.get("Profile").and_then(|v| String::try_from(v.clone()).ok())
}

/// `power-profiles-daemon`, as this crate uses it.
pub struct PowerProfilesDaemonBackend {
    connection: Connection,
}

impl PowerProfilesDaemonBackend {
    /// Opens the system bus.
    ///
    /// Failing here is [`ProfileError::Unavailable`] rather than a panic:
    /// a machine without `power-profiles-daemon` running is a state the
    /// screen has to describe, not crash on.
    pub async fn connect() -> Result<Self, ProfileError> {
        let connection = tokio::time::timeout(TIMEOUT, Connection::system())
            .await
            .map_err(|_| ProfileError::TimedOut(TIMEOUT))?
            .map_err(classify)?;
        Ok(PowerProfilesDaemonBackend { connection })
    }

    async fn proxy(&self) -> Result<PowerProfilesDbusProxy<'_>, ProfileError> {
        bounded(TIMEOUT, PowerProfilesDbusProxy::new(&self.connection)).await
    }
}

#[async_trait::async_trait]
impl PowerProfilesBackend for PowerProfilesDaemonBackend {
    async fn profiles(&self) -> Result<Vec<PowerProfile>, ProfileError> {
        let proxy = self.proxy().await?;
        let raw = bounded(TIMEOUT, proxy.profiles()).await?;
        raw.iter()
            .map(|entry| {
                let name = profile_name(entry).ok_or_else(|| {
                    ProfileError::Refused("a profile entry had no Profile key".to_string())
                })?;
                PowerProfile::parse(&name).ok_or(ProfileError::UnknownProfile(name))
            })
            .collect()
    }

    async fn active_profile(&self) -> Result<PowerProfile, ProfileError> {
        let proxy = self.proxy().await?;
        let raw = bounded(TIMEOUT, proxy.active_profile()).await?;
        PowerProfile::parse(&raw).ok_or(ProfileError::UnknownProfile(raw))
    }

    async fn set_active_profile(&self, profile: PowerProfile) -> Result<(), ProfileError> {
        let proxy = self.proxy().await?;
        bounded(TIMEOUT, proxy.set_active_profile(profile.as_str())).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use zbus::zvariant::Value;

    fn entry(profile: &str) -> HashMap<String, OwnedValue> {
        let mut map = HashMap::new();
        map.insert(
            "Profile".to_string(),
            OwnedValue::try_from(Value::from(profile)).unwrap(),
        );
        map
    }

    #[test]
    fn a_profile_entrys_name_is_read_from_its_profile_key() {
        assert_eq!(profile_name(&entry("balanced")).as_deref(), Some("balanced"));
    }

    /// A dict with no `Profile` key is not something this crate invented
    /// a name for — it is a parse failure, reported as one.
    #[test]
    fn an_entry_with_no_profile_key_reports_nothing_rather_than_a_guess() {
        assert_eq!(profile_name(&HashMap::new()), None);
    }
}
