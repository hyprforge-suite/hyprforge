//! Bluetooth adapters and devices, over BlueZ.
//!
//! `bluetoothd` is already a daemon, so this crate is a client and not a
//! second one — the same call `hyprforge-network` makes about
//! NetworkManager.
//!
//! Everything goes through [`backend::BluetoothBackend`], which has a real
//! implementation and a mock, so the logic above it is testable on a
//! machine with no adapter.
//!
//! # What this does not do yet
//!
//! **Pairing.** It needs an `org.bluez.Agent1` — a passkey or yes/no
//! confirmation exchanged with the remote device, registered with BlueZ
//! and driven from the UI — which is its own piece of work. Unpaired
//! devices are listed and say so; see [`types::Device::unsupported_reason`].

pub mod backend;
pub mod bluez;
pub mod types;

pub use backend::{for_display, BluetoothBackend};
pub use bluez::BlueZBackend;
pub use types::{Address, AdapterState, BluetoothError, Device, DeviceKind, Status};
