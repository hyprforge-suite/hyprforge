//! Drives, phones and network shares: what is plugged in, mounting,
//! unmounting and ejecting it over UDisks2, phones and cameras through
//! gvfs's MTP and gPhoto2 backends, and connecting to a server through
//! gvfs — the sidebar's Devices and Remote sections in the Hyprforge
//! file manager.
//!
//! UDisks2 and gvfs are both already running on a desktop, so this crate
//! is a client of each and never a second one — the choice
//! `hyprforge-network` made for NetworkManager. Mounting needs no root:
//! UDisks2 mounts on the session user's behalf under polkit, and gvfs
//! mounts into the user's own FUSE directory.
//!
//! # The shape
//!
//! The layering CLAUDE.md lays down for a D-Bus-backed module, in the
//! order it was built:
//!
//! - **Plain data** — [`types`]: a [`Volume`], a [`Share`], and errors
//!   that keep apart the failures a person acts on differently (a busy
//!   drive, a policy that said no, a service that is not running).
//! - **Pure decisions** — [`inventory`] (which of UDisks2's block
//!   devices are volumes a person wants, and what to call them),
//!   [`mountinfo`] (which mounts are network shares), [`gvfs`] (what it
//!   can reach, what an address means, and answering `gio`'s prompts),
//!   [`gadgets`] (phones and cameras: what gvfs's MTP and gPhoto2
//!   monitors list, and what is on USB that no installed backend reads),
//!   and [`types::sentence`] (what to say when it fails).
//! - **The seams** — `backend`'s `VolumeBackend` and `ShareBackend`,
//!   each with a mock behind the `mock` feature, so a window's handling
//!   of a refused unmount or an unplugged stick is testable on a machine
//!   with no drives.
//! - **The clients** — `udisks` and `network`. Marshalling only;
//!   whether they agree with the real services is what the live tier
//!   (`tests/live_udisks.rs`) asks, read-only.
//!
//! The data and decisions build without the `client` feature, which is
//! everything with a runtime in it: the file manager's browser view
//! needs to draw a drive, not talk to one.
//!
//! # The one fact everything here follows from
//!
//! A service that is not running is never allowed to look like an
//! answer. UDisks2 down is [`VolumeError::Unavailable`] — not an empty
//! list, which would claim every drive had been unplugged — and gvfs
//! missing is [`Gvfs::Absent`] with a sentence, never a Connect button
//! that silently does nothing.
//!
//! # What is not here
//!
//! Unlocking an encrypted drive (it shows, and says it is locked),
//! formatting, partitioning, and browsing a network for servers. The
//! last is gvfs's `network://`, and a list of whatever answered a
//! broadcast is a different feature from "connect to this address".

pub mod gadgets;
pub mod gvfs;
pub mod inventory;
pub mod mountinfo;
pub mod types;

#[cfg(feature = "client")]
pub mod backend;
#[cfg(feature = "client")]
pub mod network;
#[cfg(feature = "client")]
pub mod settle;
#[cfg(feature = "client")]
pub mod udisks;

pub use gadgets::{Gadget, GadgetKind, Gadgets, Unreadable};
pub use types::{
    sentence, Answers, ConnectError, Detach, Gvfs, Login, Operation, Question, Share, ShareKind, Volume, VolumeError,
    VolumeId, VolumeKind,
};

#[cfg(feature = "client")]
pub use backend::{ShareBackend, VolumeBackend};
#[cfg(feature = "client")]
pub use network::SystemShares;
#[cfg(feature = "client")]
pub use udisks::UDisks2Backend;
