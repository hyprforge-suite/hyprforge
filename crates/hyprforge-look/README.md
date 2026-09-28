# hyprforge-look

The Hyprforge suite's shared Color and Theme types, with no GUI toolkit dependency so the lock screen and greeter can paint the same look as every iced app.

What every Hyprforge app looks like.

One crate rather than one per app, because the alternative already
happened: the Settings app and the lock screen each grew their own
palette and drifted to different accents, different backgrounds and
different text colours — two products from one project, which is the
exact failure this suite exists to avoid.

Deliberately free of any GUI toolkit. Some of its users have no window
at all — `hyprforge-core` reads colours out of a Hyprland config and
`hyprforge-appearance` resolves and publishes the theme — and a toolkit
here would be carried by each of them for nothing. The rule began with
the lock screen painting into a raw Wayland buffer with no toolkit;
the lock screen and greeter both draw through iced's software renderer
now, and like every other iced host they convert at their own boundary
(`hyprforge_ui::color`).

## Where this lives

Part of [Hyprforge](https://github.com/hyprforge-suite/hyprforge), a suite of native Hyprland desktop
applications. This crate is `crates/hyprforge-look` there; it is published to
crates.io so the suite's applications can be built from their own
repositories, and its API follows the suite. Issues and pull requests go
to the suite repository.

```
cargo add hyprforge-look
```

This README is rendered from the crate's `//!` documentation by
`tools/crate-readme.py`; edit `src/lib.rs`, not this file.

## Licence

MIT. See `LICENSE`.
