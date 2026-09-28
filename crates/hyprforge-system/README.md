# hyprforge-system

The behaviour and platform hl.config catalog (misc, binds, xwayland, opengl, render, quirks) and its require line; Hyprforge-internal, published so the suite's apps build from crates.io.

Behaviour and platform settings: the `hl.config` categories that
aren't about how things look.

Everything here is `hyprforge_core::hlconfig` — this crate is the
`catalog` and the require line, nothing more. `misc`, `binds`,
`xwayland`, `opengl`, `render`, `ecosystem` and `quirks`.

`render` in particular is where the settings that can make the screen
stop working live. They are catalogued rather than hidden, because a
user who needs `direct_scanout` off has no other way to reach it from
a GUI — but the ones that bite are marked in their help text.

## Where this lives

Part of [Hyprforge](https://github.com/hyprforge-suite/hyprforge), a suite of native Hyprland desktop
applications. This crate is `crates/hyprforge-system` there; it is published to
crates.io so the suite's applications can be built from their own
repositories, and its API follows the suite. Issues and pull requests go
to the suite repository.

```
cargo add hyprforge-system
```

This README is rendered from the crate's `//!` documentation by
`tools/crate-readme.py`; edit `src/lib.rs`, not this file.

## Licence

MIT. See `LICENSE`.
