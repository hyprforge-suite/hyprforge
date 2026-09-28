# hyprforge-fileops

Trash, copy, move and rename per the freedesktop specifications, aware of which filesystem a path is on, as step-driven operations with no async runtime or D-Bus.

Trash, copy, move and rename, per the freedesktop.org specification
where one applies — syscalls and arithmetic, no async runtime, no
D-Bus. See `fs` for why it needs to know which filesystem a path lives
on before it can trash or move it correctly, `trash` for the spec
itself, and `ops` for copy/move/rename's step-driven design.

## Where this lives

Part of [Hyprforge](https://github.com/hyprforge-suite/hyprforge), a suite of native Hyprland desktop
applications. This crate is `crates/hyprforge-fileops` there; it is published to
crates.io so the suite's applications can be built from their own
repositories, and its API follows the suite. Issues and pull requests go
to the suite repository.

```
cargo add hyprforge-fileops
```

This README is rendered from the crate's `//!` documentation by
`tools/crate-readme.py`; edit `src/lib.rs`, not this file.

## Licence

MIT. See `LICENSE`.
