//! Wi-Fi, wired status and the radio toggle, over NetworkManager.
//!
//! NetworkManager is already a daemon, so this crate is a client and not
//! a second one — the same choice `hyprforge-displayd` did *not* get to
//! make, because nothing owns `wlr-output-management-v1` on a user's
//! behalf.
//!
//! Everything here goes through [`backend::NetworkBackend`], which has a
//! real implementation and a mock. The logic above that line is testable
//! on a machine with no NetworkManager, no adapter and no access point,
//! which is every machine this project's tier 1 runs on.
//!
//! # What does not live here
//!
//! Credentials. NetworkManager stores them, and this crate hands a
//! passphrase over once and keeps no copy — see [`secret::Psk`], which
//! also refuses to print itself.

pub mod backend;
pub mod nm;
pub mod secret;
pub mod types;

pub use nm::NetworkManagerBackend;
pub use backend::{NetworkBackend, SavedNetwork, Status, WiredState, WiredStatus};
pub use secret::Psk;
pub use types::{AccessPoint, NetworkError, RadioState, Security, Ssid};
