# hyprforge-displayd

A monitor-arrangement daemon for Hyprland, and `hyprforge-displayctl` to
drive it from a script.

It watches `wlr-output-management`, recognises a set of displays it has
seen before, and applies the layout you saved for that set. Plug the same
dock in tomorrow and the windows come back where you left them.

Part of [Hyprforge](https://github.com/adamrpostjr/hyprforge), a suite of
native Hyprland desktop applications. This repository is a split of the
`crates/hyprforge-displayd` directory there; development happens in the
monorepo and `sync.sh` keeps this copy in step.

## Two things worth knowing before reading the code

**Scales are 120ths, and most of the ones you would want do not exist.**
The protocol carries a scale as an integer number of 120ths, so 1.5 is
180 and 1.6 is 192 — but a fractional scale that does not divide the
mode cleanly is rejected by the compositor, and the failure is a layout
that silently does not apply. The monorepo's README has the full account
under "Scales are 120ths".

**A saved layout is keyed on a fingerprint, not on connector names.**
`DP-1` is whichever port something is plugged into today. Two identical
monitors report identical EXIF-style metadata, so the fingerprint has to
include position to tell them apart — see `fingerprint.rs`.

## Installing

```
cargo install --path .
```

Arch users can build the `hyprforge-displayd` package from the monorepo's
`packaging/arch` instead, which also installs the systemd user service.

## Licence

MIT. See `LICENSE`.
