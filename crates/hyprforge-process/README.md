# hyprforge-process

A bounded replacement for Command::output() so a hung subprocess like hyprctl, gsettings or fc-list never blocks the caller forever.

Running a subprocess that might not come back.

`Command::output()` waits forever. Every external program this
project talks to — `hyprctl`, `gsettings`, `fc-list` — is one that
can stop answering: a wedged compositor, a hung dconf, an NFS home
directory. Waiting forever on any of them means an app that has to
be killed, and for a login screen it would mean nobody can log in.

That is the same shape of mistake as waiting forever for a
compositor to grant a session lock, which cost five visible seconds
before anyone noticed. Unbounded waits on another process are worth
treating as a category.

Deliberately not in `hyprforge-paths`, which has no dependencies and
is about where files live: this is about talking to another process,
not about the filesystem. It used to live in `hyprforge-core`, on the
theory that every caller already depended on that crate — but
`hyprforge-network`, `hyprforge-bluetooth` and `hyprforge-power` are
clients of other daemons (NetworkManager, BlueZ, systemd-logind) with
no Hyprland config to read or write, and pulling in 4,000+ lines of
`hlconfig`/`hyprlang`/`lua` machinery for a bounded-subprocess helper
made the layering a lie. This crate has no workspace dependencies at
all, same as `hyprforge-paths`, so a non-Hyprland app can use it
without dragging in Hyprland at all. `hyprforge-core` re-exports this
module as `hyprforge_core::command` so existing call sites keep
working; new code should depend on `hyprforge-process` directly.

## Where this lives

Part of [Hyprforge](https://github.com/hyprforge-suite/hyprforge), a suite of native Hyprland desktop
applications. This crate is `crates/hyprforge-process` there; it is published to
crates.io so the suite's applications can be built from their own
repositories, and its API follows the suite. Issues and pull requests go
to the suite repository.

```
cargo add hyprforge-process
```

This README is rendered from the crate's `//!` documentation by
`tools/crate-readme.py`; edit `src/lib.rs`, not this file.

## Licence

MIT. See `LICENSE`.
