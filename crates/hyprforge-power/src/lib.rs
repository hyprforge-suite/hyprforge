//! Keeping the machine awake, over `systemd-logind`.
//!
//! logind is already a daemon, so this crate is a client and not a
//! second one — the same choice `hyprforge-network` and
//! `hyprforge-bluetooth` made for NetworkManager and BlueZ.
//!
//! Everything here goes through [`backend::InhibitBackend`], which has a
//! real implementation ([`logind::LogindBackend`]) and a mock. The logic
//! above that line — what "keep awake" means as a [`types::WhatSet`] —
//! is testable on a machine with no logind reachable, which cannot
//! happen in practice (it is part of systemd) but matters anyway because
//! nothing here should need a live bus connection to prove it typos
//! `"idel"` into a compile error rather than a silent no-op.
//!
//! # What does not live here
//!
//! The tray item that flips this on and off. That is wired up against
//! this crate separately, and this crate is the backend only.
//!
//! # The one fact everything here follows from
//!
//! `Inhibit` hands back a file descriptor, and the inhibit lasts exactly
//! as long as that descriptor is open. Dropping it releases the inhibit
//! silently — nothing on the bus reports an error, the machine simply
//! sleeps. [`backend::InhibitBackend`]'s doc comment explains how this
//! shapes the trait; [`logind::LogindBackend`] is where the descriptor is
//! actually held.

pub mod backend;
pub mod logind;
pub mod types;

pub use backend::InhibitBackend;
pub use logind::LogindBackend;
pub use types::{InhibitError, InhibitorInfo, What, WhatSet};
