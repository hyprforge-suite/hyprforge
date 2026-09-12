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
//! Pairing is the exception to "everything goes through the backend": it
//! is a conversation BlueZ starts, over `org.bluez.Agent1`, not a call this
//! crate makes — see [`agent`].

pub mod agent;
pub mod backend;
pub mod pairing;
pub mod bluez;
pub mod types;

pub use agent::{register, AgentError, AgentHandle, PairingAgent, PairingRequest};
pub use pairing::{PairingPrompt, Passkey};
pub use backend::{for_display, BluetoothBackend};
pub use bluez::BlueZBackend;
pub use types::{Address, AdapterState, BluetoothError, Device, DeviceKind, Status};
