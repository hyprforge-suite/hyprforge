# hyprforge-power

Keep-awake inhibitors, battery state and power profiles over systemd-logind, UPower and power-profiles-daemon, each behind its own backend trait with a mock.

Keeping the machine awake, what the battery is doing, and which power
profile is active — over `systemd-logind`, UPower and
`power-profiles-daemon`.

All three are already daemons, so this crate is a client and never a
second one — the same choice `hyprforge-network` and
`hyprforge-bluetooth` made for NetworkManager and BlueZ. They are also
three *independent* daemons, so each gets its own backend trait
(`backend::InhibitBackend`, `backend::BatteryBackend`,
`backend::PowerProfilesBackend`) rather than one trait with three
kinds of methods on it: one can be down while the others answer fine,
and nothing above the trait should have to depend on a daemon it never
asked about.

Each trait has a real implementation and a mock. The logic above that
line — what "keep awake" means as a `types::WhatSet`, whether a
battery reading counts as low, which duration applies to which state,
whether a profile name is one of the three the daemon has ever
documented — is testable on a machine with none of these three
reachable, which is every machine this project's tier 1 runs on.

# What does not live here

The tray items and Settings screens that show and drive all of this.
Those are wired up against this crate separately; this crate is the
backend only.

# The one fact `InhibitBackend` follows from

`Inhibit` hands back a file descriptor, and the inhibit lasts exactly
as long as that descriptor is open. Dropping it releases the inhibit
silently — nothing on the bus reports an error, the machine simply
sleeps. `backend::InhibitBackend`'s doc comment explains how this
shapes the trait; `logind::LogindBackend` is where the descriptor is
actually held.

Which process holds it decides how long keep awake lasts. Held by the
Settings window it ended when the window closed, and held by the tray
daemon it ended whenever the daemon restarted — "keep awake does
nothing", with nothing anywhere saying why. `keep_awake` gives the
descriptor a process of its own, found again through logind's own list
of inhibitors, so it lasts until it is turned off and every program
that shows it agrees whether it is on.

# The one fact the battery and profile backends follow from

A daemon that is not running is never allowed to look like a specific,
plausible answer: not an empty inhibitor list, not "no battery
present", not a made-up active profile. Each of
`types::InhibitError`, `types::BatteryError` and
`types::ProfileError` keeps an `Unavailable` variant exactly so that
distinction survives all the way to a screen. The other side of it is
named too: a machine can genuinely have no battery
(`backend::BatteryBackend::battery` returning `Ok(None)`), and that
is a normal machine, not an error.

## Where this lives

Part of [Hyprforge](https://github.com/hyprforge-suite/hyprforge), a suite of native Hyprland desktop
applications. This crate is `crates/hyprforge-power` there; it is published to
crates.io so the suite's applications can be built from their own
repositories, and its API follows the suite. Issues and pull requests go
to the suite repository.

```
cargo add hyprforge-power
```

This README is rendered from the crate's `//!` documentation by
`tools/crate-readme.py`; edit `src/lib.rs`, not this file.

## Licence

MIT. See `LICENSE`.
