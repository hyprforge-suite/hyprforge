# hyprforge-appearance

The Appearance model: the hl.config look catalog, animations, the gsettings bridge and look::resolve() into the shared Theme; Hyprforge-internal, published so the suite's apps build from crates.io.

One screen for how the desktop looks.

Three things a user thinks of as one decision, which the system keeps
in three unrelated places:

- **Hyprland settings** — borders, gaps, rounding, blur, shadows,
  cursor. An `hl.config` overlay, handled by
  `hyprforge_core::hlconfig`; this crate supplies only the
  `catalog`.
- **`animations`** — speed and curve per animation. Looks
  append-shaped but is last-wins per leaf, so it's the same overlay in
  the same generated file.
- **`desktop`** — the GTK/icon/cursor/font/dark-mode settings that
  live in gsettings, which no file here generates and which owns
  itself differently. See that module for why.

## Where this lives

Part of [Hyprforge](https://github.com/hyprforge-suite/hyprforge), a suite of native Hyprland desktop
applications. This crate is `crates/hyprforge-appearance` there; it is published to
crates.io so the suite's applications can be built from their own
repositories, and its API follows the suite. Issues and pull requests go
to the suite repository.

```
cargo add hyprforge-appearance
```

This README is rendered from the crate's `//!` documentation by
`tools/crate-readme.py`; edit `src/lib.rs`, not this file.

## Licence

MIT. See `LICENSE`.
