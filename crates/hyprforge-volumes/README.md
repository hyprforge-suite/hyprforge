# hyprforge-volumes

Drives over UDisks2 and network shares over gvfs and the mount table: what is plugged in, mounting, unmounting and ejecting it, and connecting to a server, behind backend traits with a mock.

Drives and network shares: what is plugged in, mounting, unmounting
and ejecting it over UDisks2, and connecting to a server through
gvfs — the sidebar's Devices and Remote sections in the Hyprforge
file manager.

UDisks2 and gvfs are both already running on a desktop, so this crate
is a client of each and never a second one — the choice
`hyprforge-network` made for NetworkManager. Mounting needs no root:
UDisks2 mounts on the session user's behalf under polkit, and gvfs
mounts into the user's own FUSE directory.

# The shape

The layering CLAUDE.md lays down for a D-Bus-backed module, in the
order it was built:

- **Plain data** — `types`: a `Volume`, a `Share`, and errors
  that keep apart the failures a person acts on differently (a busy
  drive, a policy that said no, a service that is not running).
- **Pure decisions** — `inventory` (which of UDisks2's block
  devices are volumes a person wants, and what to call them),
  `mountinfo` (which mounts are network shares), `gvfs` (what it
  can reach, what an address means, and answering `gio`'s prompts),
  and `types::sentence` (what to say when it fails).
- **The seams** — `backend`'s `VolumeBackend` and `ShareBackend`,
  each with a mock behind the `mock` feature, so a window's handling
  of a refused unmount or an unplugged stick is testable on a machine
  with no drives.
- **The clients** — `udisks` and `network`. Marshalling only;
  whether they agree with the real services is what the live tier
  (`tests/live_udisks.rs`) asks, read-only.

The data and decisions build without the `client` feature, which is
everything with a runtime in it: the file manager's browser view
needs to draw a drive, not talk to one.

# The one fact everything here follows from

A service that is not running is never allowed to look like an
answer. UDisks2 down is `VolumeError::Unavailable` — not an empty
list, which would claim every drive had been unplugged — and gvfs
missing is `Gvfs::Absent` with a sentence, never a Connect button
that silently does nothing.

# What is not here

Unlocking an encrypted drive (it shows, and says it is locked),
formatting, partitioning, and browsing a network for servers. The
last is gvfs's `network://`, and a list of whatever answered a
broadcast is a different feature from "connect to this address".

## Where this lives

Part of [Hyprforge](https://github.com/hyprforge-suite/hyprforge), a suite of native Hyprland desktop
applications. This crate is `crates/hyprforge-volumes` there; it is published to
crates.io so the suite's applications can be built from their own
repositories, and its API follows the suite. Issues and pull requests go
to the suite repository.

```
cargo add hyprforge-volumes
```

This README is rendered from the crate's `//!` documentation by
`tools/crate-readme.py`; edit `src/lib.rs`, not this file.

## Licence

MIT. See `LICENSE`.
