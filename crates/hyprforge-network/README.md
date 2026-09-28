# hyprforge-network

Wi-Fi, wired status and the radio toggle over NetworkManager, behind a backend trait with a mock so everything above the D-Bus client is testable without an adapter or access point.

Wi-Fi, wired status and the radio toggle, over NetworkManager.

NetworkManager is already a daemon, so this crate is a client and not
a second one — the same choice `hyprforge-displayd` did *not* get to
make, because nothing owns `wlr-output-management-v1` on a user's
behalf.

Everything here goes through `backend::NetworkBackend`, which has a
real implementation and a mock. The logic above that line is testable
on a machine with no NetworkManager, no adapter and no access point,
which is every machine this project's tier 1 runs on.

# What does not live here

Credentials. NetworkManager stores them, and this crate hands a
passphrase over once and keeps no copy — see `secret::Psk`, which
also refuses to print itself.

## Where this lives

Part of [Hyprforge](https://github.com/hyprforge-suite/hyprforge), a suite of native Hyprland desktop
applications. This crate is `crates/hyprforge-network` there; it is published to
crates.io so the suite's applications can be built from their own
repositories, and its API follows the suite. Issues and pull requests go
to the suite repository.

```
cargo add hyprforge-network
```

This README is rendered from the crate's `//!` documentation by
`tools/crate-readme.py`; edit `src/lib.rs`, not this file.

## Licence

MIT. See `LICENSE`.
