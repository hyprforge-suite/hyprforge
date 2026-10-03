# hyprforge-ui

The iced widget layer built on hyprforge-look's shared theme, giving every Hyprforge app the same spacing scale, palette and widgets.

What Hyprforge apps are built out of.

The iced half of the shared look: one spacing scale, one palette, one
set of widgets, so a sidebar in Settings and a sidebar in the file
manager breathe the same amount and use the same accent.

Deliberately knows nothing about Hyprland. A calculator should be
able to use these widgets without inheriting Lua codegen, which is
why the one widget that *did* know — the "your config can't take a
require line" banner — lives in the Settings app instead.

`DESIGN-SYSTEM.md`, beside this crate's manifest, is the catalogue:
every token with where it comes from, every size, mark and widget,
and the rule for what belongs here rather than in an app.

## Where this lives

Part of [Hyprforge](https://github.com/hyprforge-suite/hyprforge), a suite of native Hyprland desktop
applications. This crate is `crates/hyprforge-ui` there; it is published to
crates.io so the suite's applications can be built from their own
repositories, and its API follows the suite. Issues and pull requests go
to the suite repository.

```
cargo add hyprforge-ui
```

This README is rendered from the crate's `//!` documentation by
`tools/crate-readme.py`; edit `src/lib.rs`, not this file.

## Licence

MIT. See `LICENSE`.
