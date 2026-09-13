//! Keeping the machine awake, what the battery is doing, and which power
//! profile is active — over `systemd-logind`, UPower and
//! `power-profiles-daemon`.
//!
//! All three are already daemons, so this crate is a client and never a
//! second one — the same choice `hyprforge-network` and
//! `hyprforge-bluetooth` made for NetworkManager and BlueZ. They are also
//! three *independent* daemons, so each gets its own backend trait
//! ([`backend::InhibitBackend`], [`backend::BatteryBackend`],
//! [`backend::PowerProfilesBackend`]) rather than one trait with three
//! kinds of methods on it: one can be down while the others answer fine,
//! and nothing above the trait should have to depend on a daemon it never
//! asked about.
//!
//! Each trait has a real implementation and a mock. The logic above that
//! line — what "keep awake" means as a [`types::WhatSet`], whether a
//! battery reading counts as low, which duration applies to which state,
//! whether a profile name is one of the three the daemon has ever
//! documented — is testable on a machine with none of these three
//! reachable, which is every machine this project's tier 1 runs on.
//!
//! # What does not live here
//!
//! The tray items and Settings screens that show and drive all of this.
//! Those are wired up against this crate separately; this crate is the
//! backend only.
//!
//! # The one fact `InhibitBackend` follows from
//!
//! `Inhibit` hands back a file descriptor, and the inhibit lasts exactly
//! as long as that descriptor is open. Dropping it releases the inhibit
//! silently — nothing on the bus reports an error, the machine simply
//! sleeps. [`backend::InhibitBackend`]'s doc comment explains how this
//! shapes the trait; [`logind::LogindBackend`] is where the descriptor is
//! actually held.
//!
//! # The one fact the battery and profile backends follow from
//!
//! A daemon that is not running is never allowed to look like a specific,
//! plausible answer: not an empty inhibitor list, not "no battery
//! present", not a made-up active profile. Each of
//! [`types::InhibitError`], [`types::BatteryError`] and
//! [`types::ProfileError`] keeps an `Unavailable` variant exactly so that
//! distinction survives all the way to a screen. The other side of it is
//! named too: a machine can genuinely have no battery
//! ([`backend::BatteryBackend::battery`] returning `Ok(None)`), and that
//! is a normal machine, not an error.

pub mod backend;
pub mod logind;
pub mod power_profiles;
pub mod types;
pub mod upower;

pub use backend::{BatteryBackend, InhibitBackend, PowerProfilesBackend};
pub use logind::LogindBackend;
pub use power_profiles::PowerProfilesDaemonBackend;
pub use types::{
    BatteryError, BatteryInfo, BatteryState, InhibitError, InhibitorInfo, PowerProfile,
    ProfileError, What, WhatSet, LOW_BATTERY_PERCENT,
};
pub use upower::UPowerBackend;
