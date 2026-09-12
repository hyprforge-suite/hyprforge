//! The boundary between this crate's logic and NetworkManager.
//!
//! Exactly the arrangement `hyprforge-displayd` uses for
//! `wlr-output-management-v1`, and for the same reason: the machine
//! running the tests has no guaranteed NetworkManager, no guaranteed
//! Wi-Fi adapter, and certainly no access point to join. Everything above
//! this trait — classification, sorting, the screen's state machine — is
//! then testable without any of that, and only [`nm`] has to be believed
//! on the strength of a live test.
//!
//! [`nm`]: crate::nm

use crate::types::{AccessPoint, NetworkError, RadioState, Ssid};
use crate::secret::Psk;

/// A saved network. NetworkManager owns these; this crate never stores
/// credentials of its own.
#[derive(Debug, Clone, PartialEq)]
pub struct SavedNetwork {
    pub ssid: Ssid,
    /// NetworkManager's connection path, as the handle to forget it by.
    pub id: String,
    pub autoconnect: bool,
}

/// What the screen is looking at right now.
#[derive(Debug, Clone, PartialEq)]
pub struct Status {
    pub radio: RadioState,
    /// The SSID currently associated, if any.
    pub connected_to: Option<Ssid>,
}

#[async_trait::async_trait]
pub trait NetworkBackend: Send + Sync {
    async fn status(&self) -> Result<Status, NetworkError>;

    /// Access points currently visible. Does **not** trigger a scan —
    /// [`NetworkBackend::request_scan`] does, and it is separate because
    /// scanning briefly drops throughput on some adapters and should not
    /// happen every time a screen repaints.
    async fn access_points(&self) -> Result<Vec<AccessPoint>, NetworkError>;

    async fn request_scan(&self) -> Result<(), NetworkError>;

    async fn saved_networks(&self) -> Result<Vec<SavedNetwork>, NetworkError>;

    /// Joins a network, saving it so it reconnects on its own.
    ///
    /// `psk` is `None` for open and OWE networks. Implementations must
    /// not log it, and must not include it in an error message — a failed
    /// activation reports [`NetworkError::BadPassphrase`], which says the
    /// password was wrong without repeating it.
    async fn connect(
        &self,
        ap: &AccessPoint,
        psk: Option<&Psk>,
    ) -> Result<(), NetworkError>;

    async fn disconnect(&self) -> Result<(), NetworkError>;

    /// Deletes a saved network, so it stops reconnecting.
    async fn forget(&self, id: &str) -> Result<(), NetworkError>;

    async fn set_radio(&self, on: bool) -> Result<(), NetworkError>;
}

/// Sorts access points the way the list shows them: strongest first, and
/// one row per SSID.
///
/// Deduplication is the point. A mesh or a dual-band router publishes the
/// same SSID from several BSSIDs, and listing each one gives a user five
/// rows called "home" that all do the same thing. The strongest is kept
/// because it is the one NetworkManager will pick anyway.
///
/// Hidden networks are dropped rather than collapsed into one anonymous
/// row: they cannot be joined by name from this screen, so a list of
/// identical "(hidden network)" entries is noise.
pub fn for_display(mut points: Vec<AccessPoint>) -> Vec<AccessPoint> {
    points.retain(|ap| !ap.ssid.is_hidden());
    // Strongest first, then by name so equal-strength rows don't shuffle
    // between refreshes — a list that reorders under the cursor is its
    // own bug.
    points.sort_by(|a, b| {
        b.strength
            .cmp(&a.strength)
            .then_with(|| a.ssid.cmp(&b.ssid))
            .then_with(|| a.bssid.cmp(&b.bssid))
    });
    let mut seen = std::collections::HashSet::new();
    points.retain(|ap| seen.insert(ap.ssid.clone()));
    points
}

// The settings app's Network screen is generic over `NetworkBackend` so its
// tests can drive it with this mock instead of a real D-Bus connection —
// see `hyprforge-settings/src/modules/network.rs`. That means the mock has
// to compile as ordinary (non-test) code for that crate's `mock` feature,
// not just under `#[cfg(test)]` inside this one.
#[cfg(any(test, feature = "mock"))]
pub mod mock {
    use super::*;
    use std::sync::Mutex;

    /// A NetworkManager that does what the test says.
    #[derive(Default)]
    pub struct MockBackend {
        pub points: Mutex<Vec<AccessPoint>>,
        pub saved: Mutex<Vec<SavedNetwork>>,
        pub status: Mutex<Option<Status>>,
        /// Set to make every call fail, for the "daemon isn't there" path.
        pub unavailable: Mutex<bool>,
        pub scans: Mutex<usize>,
        /// What `connect` was called with — the passphrase is recorded so
        /// a test can assert it was passed through, which is the one
        /// place that is legitimate.
        pub connect_calls: Mutex<Vec<(Ssid, Option<String>)>>,
        pub forgotten: Mutex<Vec<String>>,
        /// When set, `connect` reports a rejected passphrase.
        pub reject_psk: Mutex<bool>,
    }

    impl MockBackend {
        pub fn new() -> Self {
            Self::default()
        }

        pub fn with_points(points: Vec<AccessPoint>) -> Self {
            let backend = Self::new();
            *backend.points.lock().unwrap() = points;
            backend
        }

        fn guard(&self) -> Result<(), NetworkError> {
            if *self.unavailable.lock().unwrap() {
                return Err(NetworkError::Unavailable);
            }
            Ok(())
        }
    }

    #[async_trait::async_trait]
    impl NetworkBackend for MockBackend {
        async fn status(&self) -> Result<Status, NetworkError> {
            self.guard()?;
            Ok(self.status.lock().unwrap().clone().unwrap_or(Status {
                radio: RadioState::On,
                connected_to: None,
            }))
        }

        async fn access_points(&self) -> Result<Vec<AccessPoint>, NetworkError> {
            self.guard()?;
            Ok(self.points.lock().unwrap().clone())
        }

        async fn request_scan(&self) -> Result<(), NetworkError> {
            self.guard()?;
            *self.scans.lock().unwrap() += 1;
            Ok(())
        }

        async fn saved_networks(&self) -> Result<Vec<SavedNetwork>, NetworkError> {
            self.guard()?;
            Ok(self.saved.lock().unwrap().clone())
        }

        async fn connect(&self, ap: &AccessPoint, psk: Option<&Psk>) -> Result<(), NetworkError> {
            self.guard()?;
            self.connect_calls
                .lock()
                .unwrap()
                .push((ap.ssid.clone(), psk.map(|p| p.expose().to_string())));
            if *self.reject_psk.lock().unwrap() {
                return Err(NetworkError::BadPassphrase);
            }
            self.saved.lock().unwrap().push(SavedNetwork {
                ssid: ap.ssid.clone(),
                id: format!("mock-{}", ap.ssid),
                autoconnect: true,
            });
            *self.status.lock().unwrap() = Some(Status {
                radio: RadioState::On,
                connected_to: Some(ap.ssid.clone()),
            });
            Ok(())
        }

        async fn disconnect(&self) -> Result<(), NetworkError> {
            self.guard()?;
            *self.status.lock().unwrap() = Some(Status {
                radio: RadioState::On,
                connected_to: None,
            });
            Ok(())
        }

        async fn forget(&self, id: &str) -> Result<(), NetworkError> {
            self.guard()?;
            self.forgotten.lock().unwrap().push(id.to_string());
            self.saved.lock().unwrap().retain(|s| s.id != id);
            Ok(())
        }

        async fn set_radio(&self, on: bool) -> Result<(), NetworkError> {
            self.guard()?;
            let mut status = self.status.lock().unwrap();
            let connected = status.as_ref().and_then(|s| s.connected_to.clone());
            *status = Some(Status {
                radio: if on { RadioState::On } else { RadioState::Off },
                connected_to: if on { connected } else { None },
            });
            Ok(())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::mock::MockBackend;
    use super::*;
    use crate::types::Security;

    fn ap(ssid: &str, bssid: &str, strength: u8) -> AccessPoint {
        AccessPoint {
            ssid: Ssid::new(ssid),
            bssid: bssid.to_string(),
            strength,
            frequency_mhz: 2412,
            security: Security::Wpa2Personal,
        }
    }

    /// A dual-band router is one network to a person and several access
    /// points to the protocol. Showing the protocol's view gives five
    /// rows called "home", all of which do the same thing.
    #[test]
    fn one_network_published_from_several_radios_is_a_single_row() {
        let listed = for_display(vec![
            ap("home", "aa", 40),
            ap("home", "bb", 82),
            ap("cafe", "cc", 55),
        ]);
        assert_eq!(listed.len(), 2);
        assert_eq!(listed[0].ssid.to_display_string(), "home");
        assert_eq!(
            listed[0].strength, 82,
            "the row should carry the strongest radio's signal, which is the one \
             NetworkManager will associate with"
        );
    }

    /// Equal signal is common — two APs both pegged at 100 — and a list
    /// that reorders itself under the pointer between refreshes is its
    /// own bug.
    #[test]
    fn equally_strong_networks_keep_a_stable_order() {
        let once = for_display(vec![ap("beta", "aa", 70), ap("alpha", "bb", 70)]);
        let twice = for_display(vec![ap("alpha", "bb", 70), ap("beta", "aa", 70)]);
        let names = |v: Vec<AccessPoint>| {
            v.iter().map(|a| a.ssid.to_display_string()).collect::<Vec<_>>()
        };
        assert_eq!(names(once), names(twice));
    }

    #[test]
    fn hidden_networks_are_left_out_rather_than_listed_as_a_row_of_blanks() {
        let listed = for_display(vec![ap("home", "aa", 60), ap("", "bb", 90)]);
        assert_eq!(listed.len(), 1);
        assert_eq!(listed[0].ssid.to_display_string(), "home");
    }

    #[tokio::test]
    async fn an_unavailable_daemon_is_an_error_on_every_call_not_an_empty_list() {
        let backend = MockBackend::with_points(vec![ap("home", "aa", 60)]);
        *backend.unavailable.lock().unwrap() = true;

        assert!(matches!(
            backend.access_points().await,
            Err(NetworkError::Unavailable)
        ));
        assert!(matches!(
            backend.saved_networks().await,
            Err(NetworkError::Unavailable)
        ));
        assert!(matches!(backend.status().await, Err(NetworkError::Unavailable)));
    }

    #[tokio::test]
    async fn joining_a_network_saves_it_so_it_comes_back_on_its_own() {
        let backend = MockBackend::new();
        let home = ap("home", "aa", 80);
        backend
            .connect(&home, Some(&Psk::new("correcthorse")))
            .await
            .unwrap();

        let saved = backend.saved_networks().await.unwrap();
        assert_eq!(saved.len(), 1);
        assert!(saved[0].autoconnect);
        assert_eq!(backend.status().await.unwrap().connected_to, Some(Ssid::new("home")));
    }

    #[tokio::test]
    async fn turning_the_radio_off_reports_nothing_connected() {
        let backend = MockBackend::new();
        backend.connect(&ap("home", "aa", 80), Some(&Psk::new("correcthorse"))).await.unwrap();
        backend.set_radio(false).await.unwrap();

        let status = backend.status().await.unwrap();
        assert_eq!(status.radio, RadioState::Off);
        assert_eq!(status.connected_to, None);
    }

    #[tokio::test]
    async fn forgetting_a_network_stops_it_reconnecting() {
        let backend = MockBackend::new();
        backend.connect(&ap("home", "aa", 80), Some(&Psk::new("correcthorse"))).await.unwrap();
        let id = backend.saved_networks().await.unwrap()[0].id.clone();

        backend.forget(&id).await.unwrap();
        assert!(backend.saved_networks().await.unwrap().is_empty());
    }
}
