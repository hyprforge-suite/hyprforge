//! Does **BlueZ** agree?
//!
//! The unit tests check this crate against its own idea of the D-Bus
//! interface. That is the same trap the option catalogues were in before
//! tier 2 existed: code can be internally consistent and wrong about the
//! service it talks to. A renamed property or a changed type is invisible
//! here until something asks the real thing.
//!
//! **Every test in this file is strictly read-only.** Nothing powers the
//! adapter on or off, starts or stops discovery, connects, disconnects,
//! trusts, or removes anything. Discovery in particular costs battery on
//! every device it can see, not just this machine's — starting it from a
//! test that runs unattended is not a cost worth paying for coverage.
//! This runs on a machine somebody is using.
//!
//! Run with `cargo test -p hyprforge-bluetooth -- --ignored`.

use hyprforge_bluetooth::backend::BluetoothBackend;
use hyprforge_bluetooth::bluez::BlueZBackend;
use hyprforge_bluetooth::types::BluetoothError;

/// Same convention as the ecosystem parse tests and the NetworkManager
/// live tests: libtest has no skipped state, so a check that could not
/// run announces itself rather than returning early and printing `ok`
/// like one that passed.
const SKIP_MARKER: &str = "HYPRFORGE-SKIP:";

/// Connects, or says why it could not.
async fn backend() -> Option<BlueZBackend> {
    match BlueZBackend::connect().await {
        Ok(backend) => Some(backend),
        Err(e) => {
            eprintln!("{SKIP_MARKER} no system bus to reach BlueZ on ({e})");
            None
        }
    }
}

/// The claim every other call rests on: BlueZ is on the system bus under
/// this name, has at least one `Adapter1`, and answers a property read
/// with the types this crate expects (`Powered`/`Discovering` as `b`,
/// `Alias` as `s`).
#[tokio::test]
#[ignore]
async fn bluez_answers_on_the_system_bus() {
    let Some(backend) = backend().await else { return };
    match backend.status().await {
        Ok(status) => {
            // Nothing is asserted about *which* state: a machine with the
            // adapter off is a legitimate machine. What is asserted is
            // that the properties exist and have the types this crate
            // claims.
            println!(
                "state: {:?}, discovering: {}, alias: {}",
                status.state, status.discovering, status.alias
            );
        }
        Err(BluetoothError::Unavailable) => {
            eprintln!("{SKIP_MARKER} bluetoothd is not running");
        }
        Err(BluetoothError::NoAdapter) => {
            eprintln!("{SKIP_MARKER} this machine has no Bluetooth adapter BlueZ can see");
        }
        Err(e) => panic!("BlueZ answered, but not in the shape this crate expects: {e}"),
    }
}

/// Every `Device1` under the adapter enumerates without the
/// optional-property handling (`Name`, `Icon`, `RSSI`) blowing up on a
/// device that has never resolved one of them — the single most likely
/// bug this crate could have.
#[tokio::test]
#[ignore]
async fn every_device_enumerates_even_when_its_optional_properties_are_missing() {
    let Some(backend) = backend().await else { return };
    match backend.devices().await {
        Ok(devices) => {
            if devices.is_empty() {
                eprintln!(
                    "{SKIP_MARKER} no devices are known to BlueZ to check properties against"
                );
                return;
            }
            for device in &devices {
                assert!(
                    !device.alias.is_empty(),
                    "every device has an Alias, even one derived only from its address"
                );
                // RSSI, when present, is a plausible dBm value: negative,
                // and not so far negative it could not be a real reading.
                if let Some(rssi) = device.rssi {
                    assert!(
                        (-100..0).contains(&rssi),
                        "RSSI {rssi} for {:?} is outside a plausible dBm range",
                        device.address
                    );
                }
            }
            println!(
                "{} devices read ({} unnamed, {} with RSSI)",
                devices.len(),
                devices.iter().filter(|d| d.is_unnamed()).count(),
                devices.iter().filter(|d| d.rssi.is_some()).count()
            );
        }
        Err(BluetoothError::Unavailable) => eprintln!("{SKIP_MARKER} bluetoothd is not running"),
        Err(BluetoothError::NoAdapter) => {
            eprintln!("{SKIP_MARKER} this machine has no Bluetooth adapter BlueZ can see")
        }
        Err(e) => panic!("reading devices failed: {e}"),
    }
}

/// A machine the kernel reports a Bluetooth controller for must not come
/// back as `NoAdapter` — that would mean BlueZ's `Adapter1` discovery in
/// this crate has drifted from what BlueZ actually exposes.
#[tokio::test]
#[ignore]
async fn a_machine_with_a_bluetooth_controller_is_not_reported_as_having_no_adapter() {
    let Some(backend) = backend().await else { return };
    let has_bluetooth_controller = std::path::Path::new("/sys/class/bluetooth")
        .read_dir()
        .map(|entries| entries.flatten().count() > 0)
        .unwrap_or(false);
    if !has_bluetooth_controller {
        eprintln!("{SKIP_MARKER} the kernel reports no Bluetooth controller to find");
        return;
    }
    match backend.status().await {
        Err(BluetoothError::NoAdapter) => panic!(
            "the kernel reports a Bluetooth controller but BlueZ's object tree has no \
             Adapter1 — this crate's adapter discovery is probably wrong"
        ),
        Err(BluetoothError::Unavailable) => eprintln!("{SKIP_MARKER} bluetoothd is not running"),
        _ => {}
    }
}
