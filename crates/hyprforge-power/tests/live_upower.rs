//! Does **UPower** agree?
//!
//! The unit tests check this crate against its own idea of the D-Bus
//! interface. That is the same trap the option catalogues were in before
//! tier 2 existed: code can be internally consistent and wrong about the
//! service it talks to. A renamed property or a changed type is invisible
//! here until something asks the real thing.
//!
//! **Every test in this file is read-only.** Nothing here writes a
//! property or calls a method that changes anything — UPower's own
//! interface is read-only for battery status anyway, so there is nothing
//! to accidentally trigger, but the convention is worth stating alongside
//! the other live tiers in this project.
//!
//! Run with `cargo test -p hyprforge-power --test live_upower -- --ignored`.

use hyprforge_power::backend::BatteryBackend;
use hyprforge_power::types::{BatteryError, BatteryState};
use hyprforge_power::upower::UPowerBackend;

/// Same convention as every other live tier: libtest has no skipped
/// state, so a check that could not run announces itself rather than
/// returning early and printing `ok` like one that passed.
const SKIP_MARKER: &str = "HYPRFORGE-SKIP:";

/// Connects, or says why it could not.
async fn backend() -> Option<UPowerBackend> {
    match UPowerBackend::connect().await {
        Ok(backend) => Some(backend),
        Err(e) => {
            eprintln!("{SKIP_MARKER} no system bus to reach UPower on ({e})");
            None
        }
    }
}

/// The claim every other test in this file rests on: UPower is on the
/// system bus under its well-known name and `EnumerateDevices` answers
/// without error.
#[tokio::test]
#[ignore]
async fn upower_answers_on_the_system_bus() {
    let Some(backend) = backend().await else { return };
    match backend.battery().await {
        Ok(_) => {} // Shape asserted by the tests below; this just proves the round trip works.
        Err(BatteryError::Unavailable) => eprintln!("{SKIP_MARKER} UPower is not running"),
        Err(e) => panic!("UPower answered, but not in the shape this crate expects: {e}"),
    }
}

/// The property types this crate reads — `Percentage` as a `d` that
/// lands in 0..=100 once rounded, and `State` as a `u` this crate's
/// classifier recognises — checked against whichever device this crate
/// picks out as "the" battery.
///
/// `Ok(None)` is a real, passing outcome: a desktop with no battery is a
/// normal machine, and this test only has something to check against
/// when one exists. It is not skipped in that case (there is nothing
/// UPower failed to answer), it simply has nothing further to assert.
#[tokio::test]
#[ignore]
async fn the_system_battery_if_present_has_a_percentage_and_state_this_crate_understands() {
    let Some(backend) = backend().await else { return };
    match backend.battery().await {
        Ok(Some(info)) => {
            assert!(
                info.percentage <= 100,
                "Percentage is documented as 0..=100 once rounded; got {}",
                info.percentage
            );
            assert_ne!(
                info.state,
                BatteryState::Unknown,
                "UPower's own State values are all mapped by BatteryState::from_upower — an \
                 Unknown here means either the classifier fell out of date with upower's enum, \
                 or the device reported something genuinely undocumented"
            );
            println!(
                "battery: {}% state={:?} time_remaining={:?}",
                info.percentage,
                info.state,
                info.time_remaining_words()
            );
        }
        Ok(None) => println!("this machine reports no system battery — a normal, AC-only machine"),
        Err(BatteryError::Unavailable) => eprintln!("{SKIP_MARKER} UPower is not running"),
        Err(e) => panic!("reading the battery failed: {e}"),
    }
}

/// `TimeToEmpty`/`TimeToFull` are signed seconds on the wire. This
/// asserts the guard in `upower::duration_from_upower_seconds` is
/// actually meeting a real, non-negative, non-absurd value rather than
/// only the values the unit tests made up — a battery estimate longer
/// than a week would mean either the guard stopped working or UPower
/// sent something this crate has never seen.
#[tokio::test]
#[ignore]
async fn a_reported_time_remaining_is_never_negative_or_absurdly_long() {
    let Some(backend) = backend().await else { return };
    match backend.battery().await {
        Ok(Some(info)) => match info.time_remaining() {
            Some(d) => {
                assert!(
                    d.as_secs() < 7 * 24 * 3600,
                    "a week-plus battery estimate is not something this crate has seen \
                     documented; got {d:?}"
                );
                println!("time remaining: {d:?}");
            }
            None => {
                eprintln!(
                    "{SKIP_MARKER} UPower has no time estimate for the current state right now"
                );
            }
        },
        Ok(None) => eprintln!("{SKIP_MARKER} this machine has no system battery to time"),
        Err(BatteryError::Unavailable) => eprintln!("{SKIP_MARKER} UPower is not running"),
        Err(e) => panic!("reading the battery failed: {e}"),
    }
}
