//! The boundary between this crate's logic and `systemd-logind`.
//!
//! Exactly the arrangement `hyprforge-network` uses for NetworkManager,
//! and for the same reason: the machine running tier 1 has a guaranteed
//! logind (it is part of systemd), but no test may actually hold a
//! system-wide inhibit while it runs unattended, so the seam has to exist
//! before the D-Bus client does.
//!
//! # The one fact this whole module exists to respect
//!
//! An inhibit lasts exactly as long as the file descriptor `Inhibit`
//! returns stays open. A backend that returns `Ok(())` from
//! [`InhibitBackend::take`] and then drops that descriptor has inhibited
//! nothing — and nothing anywhere reports an error when that happens, the
//! machine just sleeps. So the descriptor is not a return value here at
//! all: it is state the backend holds for as long as the caller wants to
//! stay awake, released only by [`InhibitBackend::release`] or by the
//! backend itself going away.

use crate::types::{
    BatteryError, BatteryInfo, InhibitError, InhibitorInfo, PowerProfile, ProfileError, WhatSet,
};

#[async_trait::async_trait]
pub trait InhibitBackend: Send + Sync {
    /// Takes an inhibit covering `what`, holding it internally.
    ///
    /// A no-op — not an error, and not a second file descriptor — when
    /// this backend already holds one: the "keep awake" toggle this crate
    /// is for has exactly one on/off state, and taking a second lock on
    /// top of the first would only make releasing it require closing two
    /// descriptors instead of one for no benefit.
    async fn take(&self, what: WhatSet, who: &str, why: &str) -> Result<(), InhibitError>;

    /// Releases the inhibit this backend holds, if any.
    ///
    /// A no-op, not an error, when nothing is held — a "keep awake"
    /// toggle turned off twice is not a bug report.
    async fn release(&self) -> Result<(), InhibitError>;

    /// What this backend currently holds, if anything.
    async fn held(&self) -> Result<Option<WhatSet>, InhibitError>;

    /// Everyone else — every other process — currently holding an
    /// inhibit, as logind reports it. Read-only: this never takes or
    /// releases anything on another process's behalf.
    async fn list_inhibitors(&self) -> Result<Vec<InhibitorInfo>, InhibitError>;
}

/// The boundary between this crate's logic and UPower.
///
/// A separate trait from [`InhibitBackend`], not a method added to it:
/// UPower and `systemd-logind` are different daemons with independent
/// lifetimes — one can be down while the other answers fine — and a
/// caller asking "what's the battery doing" has no need to also depend on
/// logind being reachable, or vice versa.
#[async_trait::async_trait]
pub trait BatteryBackend: Send + Sync {
    /// `Ok(None)` when this machine has no system battery — a desktop
    /// with only `AC` is a normal machine, not a failure. `Err(..)` is
    /// reserved for UPower itself not answering; the two must never be
    /// confused, which is the whole reason this returns an `Option`
    /// inside a `Result` rather than an empty/default `BatteryInfo`.
    async fn battery(&self) -> Result<Option<BatteryInfo>, BatteryError>;
}

/// The boundary between this crate's logic and `power-profiles-daemon`.
///
/// Also its own trait, for the same reason [`BatteryBackend`] is: this is
/// a third daemon, independent of both `systemd-logind` and UPower.
#[async_trait::async_trait]
pub trait PowerProfilesBackend: Send + Sync {
    /// The profiles this machine can be switched between — normally all
    /// three, but read rather than assumed, since a machine could in
    /// principle offer fewer.
    async fn profiles(&self) -> Result<Vec<PowerProfile>, ProfileError>;

    async fn active_profile(&self) -> Result<PowerProfile, ProfileError>;

    /// Changes the active profile. A write, and the one call in this
    /// crate a live test must never make — see the doc comment on
    /// `tests/live_power_profiles.rs`.
    async fn set_active_profile(&self, profile: PowerProfile) -> Result<(), ProfileError>;
}

// The settings/tray code that drives the "keep awake" toggle is generic
// over `InhibitBackend` so its own tests can use this instead of a real
// D-Bus connection. That means the mock has to compile as ordinary
// (non-test) code under this crate's `mock` feature, not just under
// `#[cfg(test)]` inside this one — see `hyprforge-network::backend::mock`
// for the same arrangement.
#[cfg(any(test, feature = "mock"))]
pub mod mock {
    use super::*;
    use std::sync::Mutex;

    /// A logind that does what the test says.
    #[derive(Default)]
    pub struct MockBackend {
        held: Mutex<Option<WhatSet>>,
        /// Set to make every call fail, for the "logind isn't there" path.
        pub unavailable: Mutex<bool>,
        /// What `list_inhibitors` reports — other processes' locks, not
        /// this backend's own.
        pub others: Mutex<Vec<InhibitorInfo>>,
        /// How many times `take` actually opened a new lock, as opposed
        /// to finding one already held. A test asserting the first
        /// handle was not lost checks this stays at 1 across repeated
        /// calls.
        pub takes: Mutex<usize>,
    }

    impl MockBackend {
        pub fn new() -> Self {
            Self::default()
        }

        fn guard(&self) -> Result<(), InhibitError> {
            if *self.unavailable.lock().unwrap() {
                return Err(InhibitError::Unavailable);
            }
            Ok(())
        }
    }

    #[async_trait::async_trait]
    impl InhibitBackend for MockBackend {
        async fn take(&self, what: WhatSet, _who: &str, _why: &str) -> Result<(), InhibitError> {
            self.guard()?;
            let mut held = self.held.lock().unwrap();
            if held.is_some() {
                // Already holding one: this must not replace it, because
                // replacing it is exactly the "drop the fd, lose the
                // inhibit" bug this module exists to prevent.
                return Ok(());
            }
            *held = Some(what);
            *self.takes.lock().unwrap() += 1;
            Ok(())
        }

        async fn release(&self) -> Result<(), InhibitError> {
            self.guard()?;
            *self.held.lock().unwrap() = None;
            Ok(())
        }

        async fn held(&self) -> Result<Option<WhatSet>, InhibitError> {
            self.guard()?;
            Ok(self.held.lock().unwrap().clone())
        }

        async fn list_inhibitors(&self) -> Result<Vec<InhibitorInfo>, InhibitError> {
            self.guard()?;
            Ok(self.others.lock().unwrap().clone())
        }
    }

    /// A UPower that does what the test says.
    #[derive(Default)]
    pub struct BatteryMockBackend {
        pub battery: Mutex<Option<BatteryInfo>>,
        /// Set to make every call fail, for the "UPower isn't there"
        /// path — kept entirely separate from `battery` being `None`,
        /// which means "no battery on this machine".
        pub unavailable: Mutex<bool>,
    }

    impl BatteryMockBackend {
        pub fn new() -> Self {
            Self::default()
        }

        fn guard(&self) -> Result<(), BatteryError> {
            if *self.unavailable.lock().unwrap() {
                return Err(BatteryError::Unavailable);
            }
            Ok(())
        }
    }

    #[async_trait::async_trait]
    impl BatteryBackend for BatteryMockBackend {
        async fn battery(&self) -> Result<Option<BatteryInfo>, BatteryError> {
            self.guard()?;
            Ok(*self.battery.lock().unwrap())
        }
    }

    /// A `power-profiles-daemon` that does what the test says.
    #[derive(Default)]
    pub struct PowerProfilesMockBackend {
        pub profiles: Mutex<Vec<PowerProfile>>,
        pub active: Mutex<Option<PowerProfile>>,
        /// Set to make every call fail, for the "daemon isn't there"
        /// path.
        pub unavailable: Mutex<bool>,
        /// Every profile `set_active_profile` was actually asked to
        /// switch to — a test asserting the live tier never writes reads
        /// this on the real backend's mock stand-in, not the daemon.
        pub set_calls: Mutex<Vec<PowerProfile>>,
    }

    impl PowerProfilesMockBackend {
        pub fn new() -> Self {
            Self::default()
        }

        /// The usual case: all three profiles offered, one active.
        pub fn with_active(active: PowerProfile) -> Self {
            let backend = Self::new();
            *backend.profiles.lock().unwrap() = vec![
                PowerProfile::PowerSaver,
                PowerProfile::Balanced,
                PowerProfile::Performance,
            ];
            *backend.active.lock().unwrap() = Some(active);
            backend
        }

        fn guard(&self) -> Result<(), ProfileError> {
            if *self.unavailable.lock().unwrap() {
                return Err(ProfileError::Unavailable);
            }
            Ok(())
        }
    }

    #[async_trait::async_trait]
    impl PowerProfilesBackend for PowerProfilesMockBackend {
        async fn profiles(&self) -> Result<Vec<PowerProfile>, ProfileError> {
            self.guard()?;
            Ok(self.profiles.lock().unwrap().clone())
        }

        async fn active_profile(&self) -> Result<PowerProfile, ProfileError> {
            self.guard()?;
            self.active
                .lock()
                .unwrap()
                .ok_or_else(|| ProfileError::Refused("no active profile set".to_string()))
        }

        async fn set_active_profile(&self, profile: PowerProfile) -> Result<(), ProfileError> {
            self.guard()?;
            self.set_calls.lock().unwrap().push(profile);
            *self.active.lock().unwrap() = Some(profile);
            Ok(())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::mock::MockBackend;
    use super::*;
    use crate::types::What;

    #[tokio::test]
    async fn taking_an_inhibit_twice_does_not_lose_the_first_handle() {
        let backend = MockBackend::new();
        backend.take(WhatSet::keep_awake(), "hyprforge", "keep awake").await.unwrap();
        backend
            .take(WhatSet::new([What::Sleep]), "hyprforge", "a second request")
            .await
            .unwrap();

        // Only one lock was ever actually opened — the second `take`
        // found one already held and left it alone.
        assert_eq!(*backend.takes.lock().unwrap(), 1);
        // And what is held is still the first request's set, not the
        // second one silently overwriting it.
        assert_eq!(backend.held().await.unwrap(), Some(WhatSet::keep_awake()));
    }

    #[tokio::test]
    async fn releasing_an_inhibit_that_was_never_taken_is_a_no_op_not_an_error() {
        let backend = MockBackend::new();
        assert!(backend.held().await.unwrap().is_none());
        backend.release().await.expect("releasing nothing is not an error");
        assert!(backend.held().await.unwrap().is_none());
    }

    #[tokio::test]
    async fn taking_then_releasing_leaves_nothing_held() {
        let backend = MockBackend::new();
        backend.take(WhatSet::keep_awake(), "hyprforge", "keep awake").await.unwrap();
        assert!(backend.held().await.unwrap().is_some());
        backend.release().await.unwrap();
        assert!(backend.held().await.unwrap().is_none());
    }

    /// The distinction this crate exists to keep, exercised through the
    /// trait rather than just the error type: a daemon that is not there
    /// must fail every call, not report an empty inhibitor list.
    #[tokio::test]
    async fn an_unavailable_logind_is_an_error_on_every_call_not_an_empty_list() {
        let backend = MockBackend::new();
        *backend.unavailable.lock().unwrap() = true;

        assert!(matches!(backend.held().await, Err(InhibitError::Unavailable)));
        assert!(matches!(
            backend.list_inhibitors().await,
            Err(InhibitError::Unavailable)
        ));
        assert!(matches!(
            backend.take(WhatSet::keep_awake(), "hyprforge", "why").await,
            Err(InhibitError::Unavailable)
        ));
    }

    #[tokio::test]
    async fn list_inhibitors_reports_other_holders_without_touching_our_own_state() {
        let backend = MockBackend::new();
        *backend.others.lock().unwrap() = vec![InhibitorInfo {
            what: "sleep:handle-lid-switch".to_string(),
            who: "systemd-inhibit".to_string(),
            why: "working".to_string(),
            mode: "block".to_string(),
            uid: 1000,
            pid: 4242,
        }];

        let others = backend.list_inhibitors().await.unwrap();
        assert_eq!(others.len(), 1);
        assert_eq!(others[0].why, "working");
        // Reading who else holds a lock must not be mistaken for holding
        // one ourselves.
        assert!(backend.held().await.unwrap().is_none());
    }

    // --- BatteryBackend --------------------------------------------------

    use super::mock::{BatteryMockBackend, PowerProfilesMockBackend};
    use crate::types::BatteryState;

    #[tokio::test]
    async fn a_machine_with_no_battery_is_not_confused_with_upower_being_down() {
        let backend = BatteryMockBackend::new();
        // Nothing set: this is the "desktop, no battery" case, and it
        // must read as `Ok(None)`, not as an error.
        assert_eq!(backend.battery().await.unwrap(), None);
    }

    #[tokio::test]
    async fn an_unavailable_upower_is_an_error_not_a_missing_battery() {
        let backend = BatteryMockBackend::new();
        *backend.unavailable.lock().unwrap() = true;
        assert!(matches!(backend.battery().await, Err(BatteryError::Unavailable)));
    }

    #[tokio::test]
    async fn a_present_battery_is_reported_as_read() {
        let info = BatteryInfo {
            percentage: 90,
            state: BatteryState::FullyCharged,
            time_to_empty: None,
            time_to_full: None,
        };
        let backend = BatteryMockBackend::new();
        *backend.battery.lock().unwrap() = Some(info);
        assert_eq!(backend.battery().await.unwrap(), Some(info));
    }

    // --- PowerProfilesBackend --------------------------------------------

    #[tokio::test]
    async fn an_unavailable_power_profiles_daemon_is_an_error_on_every_call() {
        let backend = PowerProfilesMockBackend::new();
        *backend.unavailable.lock().unwrap() = true;
        assert!(matches!(backend.profiles().await, Err(ProfileError::Unavailable)));
        assert!(matches!(backend.active_profile().await, Err(ProfileError::Unavailable)));
        assert!(matches!(
            backend.set_active_profile(PowerProfile::Balanced).await,
            Err(ProfileError::Unavailable)
        ));
    }

    #[tokio::test]
    async fn the_active_profile_is_always_one_of_the_offered_profiles() {
        let backend = PowerProfilesMockBackend::with_active(PowerProfile::Performance);
        let offered = backend.profiles().await.unwrap();
        let active = backend.active_profile().await.unwrap();
        assert!(offered.contains(&active));
    }

    #[tokio::test]
    async fn setting_the_active_profile_changes_what_is_read_back() {
        let backend = PowerProfilesMockBackend::with_active(PowerProfile::Balanced);
        backend.set_active_profile(PowerProfile::PowerSaver).await.unwrap();
        assert_eq!(backend.active_profile().await.unwrap(), PowerProfile::PowerSaver);
        assert_eq!(*backend.set_calls.lock().unwrap(), vec![PowerProfile::PowerSaver]);
    }
}
