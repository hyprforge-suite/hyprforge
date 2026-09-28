# hyprforge-session

Autostart, environment variables, touchpad gestures and app permissions in a Hyprland session, as one generated Lua file; Hyprforge-internal, published so the suite's apps build from crates.io.

The parts of a Hyprland session that aren't windows or appearance:
what starts with it, what environment it runs in, what the touchpad
does, and what applications are allowed to do.

All four are `hl.*` calls in the user's Lua config, so they share the
Lua plumbing the other modules use — one generated file, one
`require()` line, placed last.

`gestures` is the exception worth knowing about: Hyprland *refuses*
a gesture that duplicates an earlier one instead of overriding it, so
that module detects conflicts before writing rather than relying on
last-one-wins.

## Where this lives

Part of [Hyprforge](https://github.com/hyprforge-suite/hyprforge), a suite of native Hyprland desktop
applications. This crate is `crates/hyprforge-session` there; it is published to
crates.io so the suite's applications can be built from their own
repositories, and its API follows the suite. Issues and pull requests go
to the suite repository.

```
cargo add hyprforge-session
```

This README is rendered from the crate's `//!` documentation by
`tools/crate-readme.py`; edit `src/lib.rs`, not this file.

## Licence

MIT. See `LICENSE`.
