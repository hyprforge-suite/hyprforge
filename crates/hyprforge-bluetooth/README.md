# hyprforge-bluetooth

Bluetooth adapters, devices and pairing over BlueZ, behind a backend trait with a mock so everything above the D-Bus client is testable on a machine with no adapter.

Bluetooth adapters and devices, over BlueZ.

`bluetoothd` is already a daemon, so this crate is a client and not a
second one — the same call `hyprforge-network` makes about
NetworkManager.

Everything goes through `backend::BluetoothBackend`, which has a real
implementation and a mock, so the logic above it is testable on a
machine with no adapter.

Pairing is the exception to "everything goes through the backend": it
is a conversation BlueZ starts, over `org.bluez.Agent1`, not a call this
crate makes — see `agent`.

## Where this lives

Part of [Hyprforge](https://github.com/hyprforge-suite/hyprforge), a suite of native Hyprland desktop
applications. This crate is `crates/hyprforge-bluetooth` there; it is published to
crates.io so the suite's applications can be built from their own
repositories, and its API follows the suite. Issues and pull requests go
to the suite repository.

```
cargo add hyprforge-bluetooth
```

This README is rendered from the crate's `//!` documentation by
`tools/crate-readme.py`; edit `src/lib.rs`, not this file.

## Licence

MIT. See `LICENSE`.
