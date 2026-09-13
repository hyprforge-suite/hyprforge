//! Does **power-profiles-daemon** agree?
//!
//! The unit tests check this crate against its own idea of the D-Bus
//! interface. That is the same trap the option catalogues were in before
//! tier 2 existed: code can be internally consistent and wrong about the
//! service it talks to. A renamed property or a different set of
//! profiles is invisible here until something asks the real thing.
//!
//! **Every test in this file is read-only, and `set_active_profile` is
//! never called.** This runs on a machine somebody is using, right now,
//! and changing the active profile changes how their machine performs
//! and how loud its fans are for as long as they don't notice and change
//! it back. Reading `ActiveProfile` costs nothing; writing it is a
//! decision this crate makes on the user's behalf without asking, which
//! is exactly what an unattended test must never do. See "The live tier
//! is read-only" in the project's house rules.
//!
//! Run with `cargo test -p hyprforge-power --test live_power_profiles -- --ignored`.

use hyprforge_power::backend::PowerProfilesBackend;
use hyprforge_power::power_profiles::PowerProfilesDaemonBackend;
use hyprforge_power::types::{PowerProfile, ProfileError};

/// Same convention as every other live tier: libtest has no skipped
/// state, so a check that could not run announces itself rather than
/// returning early and printing `ok` like one that passed.
const SKIP_MARKER: &str = "HYPRFORGE-SKIP:";

/// Connects, or says why it could not.
async fn backend() -> Option<PowerProfilesDaemonBackend> {
    match PowerProfilesDaemonBackend::connect().await {
        Ok(backend) => Some(backend),
        Err(e) => {
            eprintln!("{SKIP_MARKER} no system bus to reach power-profiles-daemon on ({e})");
            None
        }
    }
}

/// The claim every other test in this file rests on: power-profiles-daemon
/// is on the system bus under `net.hadess.PowerProfiles` and its
/// `Profiles` property answers in the shape this crate expects.
#[tokio::test]
#[ignore]
async fn power_profiles_daemon_answers_on_the_system_bus() {
    let Some(backend) = backend().await else { return };
    match backend.profiles().await {
        Ok(profiles) => println!("profiles offered: {profiles:?}"),
        Err(ProfileError::Unavailable) => {
            eprintln!("{SKIP_MARKER} power-profiles-daemon is not running")
        }
        Err(e) => panic!("power-profiles-daemon answered, but not in the shape expected: {e}"),
    }
}

/// power-profiles-daemon's whole contract is exactly these three names —
/// this is a claim about the real daemon's `Profiles` property, not
/// something the compiler can see: a daemon version that renamed or
/// dropped one would make this crate silently unable to show or set it.
#[tokio::test]
#[ignore]
async fn the_three_documented_profiles_are_all_offered() {
    let Some(backend) = backend().await else { return };
    match backend.profiles().await {
        Ok(profiles) => {
            for expected in
                [PowerProfile::PowerSaver, PowerProfile::Balanced, PowerProfile::Performance]
            {
                assert!(
                    profiles.contains(&expected),
                    "expected {expected} among the offered profiles {profiles:?}"
                );
            }
        }
        Err(ProfileError::Unavailable) => {
            eprintln!("{SKIP_MARKER} power-profiles-daemon is not running")
        }
        Err(e) => panic!("reading the offered profiles failed: {e}"),
    }
}

/// The one assertion this whole file exists for: `ActiveProfile` is
/// always a member of `Profiles`. Nothing in this crate's type system can
/// see that — `PowerProfile::parse` would happily accept an active
/// profile the real daemon never actually offers on this machine, and
/// only asking the live daemon both questions at once can catch it
/// drifting apart.
#[tokio::test]
#[ignore]
async fn the_active_profile_is_one_of_the_offered_profiles() {
    let Some(backend) = backend().await else { return };
    let profiles = match backend.profiles().await {
        Ok(profiles) => profiles,
        Err(ProfileError::Unavailable) => {
            eprintln!("{SKIP_MARKER} power-profiles-daemon is not running");
            return;
        }
        Err(e) => panic!("reading the offered profiles failed: {e}"),
    };
    match backend.active_profile().await {
        Ok(active) => {
            assert!(
                profiles.contains(&active),
                "active profile {active} is not among the offered profiles {profiles:?}"
            );
            println!("active profile: {active}");
        }
        Err(ProfileError::Unavailable) => {
            eprintln!("{SKIP_MARKER} power-profiles-daemon is not running")
        }
        Err(e) => panic!("reading the active profile failed: {e}"),
    }
}

/// `set_active_profile` is deliberately never named in this file except
/// here, as a compile-time reminder of what was left unexercised: this
/// pins that `PowerProfile::Performance` — the profile confirmed active
/// on the development machine this crate was built against — is a value
/// this crate's own type accepts, without ever sending it anywhere.
#[test]
fn setting_the_active_profile_is_deliberately_not_exercised_by_this_file() {
    let confirmed_active_during_development = PowerProfile::Performance;
    assert_eq!(confirmed_active_during_development.as_str(), "performance");
}
