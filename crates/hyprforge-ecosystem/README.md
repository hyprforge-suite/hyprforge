# hyprforge-ecosystem

hyprpaper, hyprsunset, hypridle and portal settings: one generated hyprlang file per daemon, and how a change reaches it; Hyprforge-internal, published so the suite's apps build from crates.io.

The Hypr ecosystem daemons' settings: wallpaper, colour temperature,
idle behaviour, and screen sharing.

These are not Hyprland itself. Each is a separate daemon with its own
hyprlang config file, so none of them goes through `hl.config` or
`require()` — the shared plumbing is `hyprforge_core::hyprlang`,
which owns one generated file per daemon and joins it to the user's
own config with a single `source =` line.

How a change reaches a running daemon differs, and the difference is
visible to the user:

- **hyprpaper** takes `hyprctl hyprpaper wallpaper` live.
- **hyprsunset** takes `hyprctl hyprsunset temperature` live.
- **hypridle** has no IPC at all (`hyprctl hypridle` → "unknown
  request") and has to be restarted for a change to take effect.
- **xdg-desktop-portal-hyprland** reads `xdph.conf` only at startup,
  and is restarted only on request — see `portal`.

## Where this lives

Part of [Hyprforge](https://github.com/hyprforge-suite/hyprforge), a suite of native Hyprland desktop
applications. This crate is `crates/hyprforge-ecosystem` there; it is published to
crates.io so the suite's applications can be built from their own
repositories, and its API follows the suite. Issues and pull requests go
to the suite repository.

```
cargo add hyprforge-ecosystem
```

This README is rendered from the crate's `//!` documentation by
`tools/crate-readme.py`; edit `src/lib.rs`, not this file.

## Licence

MIT. See `LICENSE`.
