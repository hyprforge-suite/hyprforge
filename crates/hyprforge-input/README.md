# hyprforge-input

The keyboard, pointer and touchpad hl.config catalog, XKB layout discovery and the require line for the generated file; Hyprforge-internal, published so the suite's apps build from crates.io.

Keyboard, pointer and touchpad settings.

Almost everything here lives in `hyprforge_core::hlconfig`, the
generic `hl.config` overlay machinery every settings category shares.
This crate is the part that is actually about input: the `catalog` of
options, and the `setup` require line that decides where the
generated file is sourced.

## Where this lives

Part of [Hyprforge](https://github.com/hyprforge-suite/hyprforge), a suite of native Hyprland desktop
applications. This crate is `crates/hyprforge-input` there; it is published to
crates.io so the suite's applications can be built from their own
repositories, and its API follows the suite. Issues and pull requests go
to the suite repository.

```
cargo add hyprforge-input
```

This README is rendered from the crate's `//!` documentation by
`tools/crate-readme.py`; edit `src/lib.rs`, not this file.

## Licence

MIT. See `LICENSE`.
