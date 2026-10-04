# hyprforge-setup

The steps that finish a Hyprforge install — services, keybinds, idle lock, notification blur, default apps, the open/save dialog — each checked, applied and undone; Hyprforge-internal, published so the suite's apps build from crates.io.

The steps that finish a Hyprforge install, each one checked, applied
and undone — the page of instructions the installer used to print,
done instead of described.

Hyprforge-internal, published so the suite's applications can build
from crates.io; the API follows the suite, not semver. The Settings
app is the one consumer: its Set up page renders `ITEMS` with their
`State`s, and `hyprforge-settings --setup` is the same thing for a
terminal and for the installer.

# What an item is

An `Item` is plain data — an id, a label, a one-line why, whether it
is ticked by default, which programs it needs — with three operations
behind it, reached through `check`, `apply` and `undo`:

- **check** reports a `State`: done, to do (and what it would
  change), unavailable (and why), or unknown. A check that could not
  run is `State::Unknown`, never done — the installer's three-way
  rule, and CLAUDE.md's "could not read is not empty".
- **apply** runs only on an item that just checked as to do, and
  records in `setup.toml` what it changed and what was there before.
- **undo** puts back exactly what was recorded, and nothing setup did
  not do: a service a package enabled for every user is not "disabled"
  by an undo, because setup never enabled it.

Every item writes through the module that already owns its file —
`shortcuts.toml` through `hyprforge-shortcuts`, `idle.toml` through
`hyprforge-ecosystem`, `mimeapps.list` through `hyprforge-mime` — so
the Settings page for that file shows what setup did and can change
it.

# Two decisions someone would otherwise reverse

**A chord that is taken is skipped and named, never overwritten.**
Keybinds are required last, so a bind written over the user's own
would silently win. The item reports `State::Unavailable` with what
holds the chord.

**Only an item that is to do is applied, and an unreadable record
refuses everything.** `apply` re-checks each item first; `--yes`
never touches something unknown. A `setup.toml` that will not parse
stops both apply and undo — undoing "nothing" would leave every change
in place, and applying would overwrite the only record of what was
there before.

# What is where

`Env` is every path, explicit so a test can point all of it at a
temp directory; `System` is every other process (`systemctl --user`,
`hyprctl`, `pgrep`, the session bus), with `RealSystem` bounding each
call and `mock::MockSystem` (feature `mock`) standing in for all of
it; `record` is `setup.toml`; `state` is the four states and the
`--porcelain` line the installer parses.

## Where this lives

Part of [Hyprforge](https://github.com/hyprforge-suite/hyprforge), a suite of native Hyprland desktop
applications. This crate is `crates/hyprforge-setup` there; it is published to
crates.io so the suite's applications can be built from their own
repositories, and its API follows the suite. Issues and pull requests go
to the suite repository.

```
cargo add hyprforge-setup
```

This README is rendered from the crate's `//!` documentation by
`tools/crate-readme.py`; edit `src/lib.rs`, not this file.

## Licence

MIT. See `LICENSE`.
