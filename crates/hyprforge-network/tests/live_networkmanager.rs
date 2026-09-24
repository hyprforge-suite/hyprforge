//! Does **NetworkManager** agree?
//!
//! The unit tests check this crate against its own idea of the D-Bus
//! interface. That is the same trap the option catalogues were in before
//! tier 2 existed: code can be internally consistent and wrong about the
//! service it talks to. A renamed property or a changed type is invisible
//! here until something asks the real thing.
//!
//! **Every test in this file is read-only.** Nothing joins a network,
//! forgets one, disconnects, or touches the radio — this runs on a
//! machine somebody is using, quite possibly over the connection being
//! inspected. A test that can drop your Wi-Fi is not worth the coverage.
//!
//! Run with `cargo test -p hyprforge-network -- --ignored`.

use hyprforge_network::backend::NetworkBackend;
use hyprforge_network::nm::NetworkManagerBackend;
use hyprforge_network::types::NetworkError;

/// Same convention as the ecosystem parse tests: libtest has no skipped
/// state, so a check that could not run announces itself rather than
/// returning early and printing `ok` like one that passed.
const SKIP_MARKER: &str = "HYPRFORGE-SKIP:";

/// Connects, or says why it could not.
async fn backend() -> Option<NetworkManagerBackend> {
    match NetworkManagerBackend::connect().await {
        Ok(backend) => Some(backend),
        Err(e) => {
            eprintln!("{SKIP_MARKER} no system bus to reach NetworkManager on ({e})");
            None
        }
    }
}

/// The claim every other call rests on: NetworkManager is on the system
/// bus under this name, and answers a property read.
#[tokio::test]
#[ignore]
async fn networkmanager_answers_on_the_system_bus() {
    let Some(backend) = backend().await else { return };
    match backend.status().await {
        Ok(status) => {
            // Nothing is asserted about *which* state: a machine with the
            // radio off is a legitimate machine. What is asserted is that
            // the properties exist and have the types this crate claims.
            println!("radio: {:?}, connected: {:?}", status.radio, status.connected_to);
        }
        Err(NetworkError::Unavailable) => {
            eprintln!("{SKIP_MARKER} NetworkManager is not running");
        }
        Err(e) => panic!("NetworkManager answered, but not in the shape this crate expects: {e}"),
    }
}

/// `GetAllAccessPoints`, `Ssid` as `ay`, `Strength` as `y`, `Frequency`
/// as `u`, and the three flag words — all read in one pass, because the
/// point is the property names and types rather than any value.
#[tokio::test]
#[ignore]
async fn every_access_point_property_this_crate_reads_exists_and_has_the_type_it_expects() {
    let Some(backend) = backend().await else { return };
    match backend.access_points().await {
        Ok(points) => {
            if points.is_empty() {
                eprintln!("{SKIP_MARKER} no access points are visible to check properties against");
                return;
            }
            for ap in &points {
                assert!(
                    ap.strength <= 100,
                    "Strength is documented as a percentage; got {} for {:?}",
                    ap.strength,
                    ap.ssid
                );
                assert!(
                    !ap.bssid.is_empty(),
                    "every access point has a HwAddress to identify it by"
                );
            }
            println!("{} access points read", points.len());
        }
        Err(NetworkError::Unavailable) => eprintln!("{SKIP_MARKER} NetworkManager is not running"),
        Err(NetworkError::NoWifiDevice) => {
            eprintln!("{SKIP_MARKER} this machine has no Wi-Fi device")
        }
        Err(e) => panic!("reading access points failed: {e}"),
    }
}

/// `NM_DEVICE_TYPE_WIFI` is 2. A hard-coded protocol constant is exactly
/// the kind of claim that is right until it isn't, and the symptom would
/// be "this machine has no Wi-Fi adapter" on a machine holding one.
#[tokio::test]
#[ignore]
async fn a_machine_with_a_wifi_adapter_is_not_reported_as_having_none() {
    let Some(backend) = backend().await else { return };
    let has_wireless_interface = std::path::Path::new("/sys/class/net")
        .read_dir()
        .map(|entries| {
            entries.flatten().any(|e| e.path().join("wireless").exists())
        })
        .unwrap_or(false);
    if !has_wireless_interface {
        eprintln!("{SKIP_MARKER} the kernel reports no wireless interface to find");
        return;
    }
    match backend.access_points().await {
        Err(NetworkError::NoWifiDevice) => panic!(
            "the kernel reports a wireless interface but NetworkManager's device scan found \
             none — NM_DEVICE_TYPE_WIFI is probably no longer 2"
        ),
        Err(NetworkError::Unavailable) => eprintln!("{SKIP_MARKER} NetworkManager is not running"),
        _ => {}
    }
}

/// `ListConnections`, `GetSettings`, and the shape this crate reads out
/// of them — `802-11-wireless.ssid` as bytes and the `autoconnect`
/// default.
#[tokio::test]
#[ignore]
async fn saved_networks_are_read_without_needing_their_secrets() {
    let Some(backend) = backend().await else { return };
    match backend.saved_networks().await {
        Ok(saved) => {
            // An empty list is a real answer here — a machine may have
            // joined nothing — so this asserts the call shape, not a count.
            for network in &saved {
                assert!(!network.id.is_empty(), "a saved network is addressed by its path");
            }
            println!("{} saved networks read", saved.len());
        }
        Err(NetworkError::Unavailable) => eprintln!("{SKIP_MARKER} NetworkManager is not running"),
        Err(e) => panic!("listing saved networks failed: {e}"),
    }
}

/// `Device.Wired`'s `Carrier` and `Speed`, `Device`'s `Interface`,
/// `State` and `ActiveConnection`, and the active connection's `Id` —
/// every property the wired status reads, against the real service.
///
/// Read-only like everything here: it lists, and connects or
/// disconnects nothing.
#[tokio::test]
#[ignore]
async fn every_wired_property_this_crate_reads_exists_and_has_the_type_it_expects() {
    use hyprforge_network::WiredState;
    let Some(backend) = backend().await else { return };
    match backend.wired().await {
        Ok(wired) if wired.is_empty() => {
            eprintln!("{SKIP_MARKER} no Ethernet interface NetworkManager manages on this machine");
        }
        Ok(wired) => {
            for port in &wired {
                println!("{port:?}");
                assert!(!port.interface.is_empty(), "every device has an interface name");
                if port.state != WiredState::Connected {
                    assert_eq!(port.speed_mbps, None, "a speed is only reported with a link");
                }
                if matches!(port.state, WiredState::CableUnplugged | WiredState::Disconnected) {
                    assert_eq!(port.connection, None, "no connection is named on a port with none active");
                }
            }
        }
        Err(NetworkError::Unavailable) => eprintln!("{SKIP_MARKER} NetworkManager is not running"),
        Err(e) => panic!("NetworkManager answered, but not in the shape this crate expects: {e}"),
    }
}
