# hyprforge-paths

XDG config paths, the user's own Pictures/Downloads directories, and crash-safe atomic file writes shared by every app in the Hyprforge desktop suite.

Where Hyprforge keeps things, and how it writes them.

Every app in the suite needs these two answers and nothing else from
them: which directory is mine, and how do I replace a file without
ever leaving a half-written one on disk. Neither question has
anything to do with Hyprland, a GUI toolkit, or Lua, so neither does
this crate — it has no dependencies at all.

Paths that *are* Hyprland-specific (`hypr/hyprforge/*.lua`, the
generated config artifacts) live in `hyprforge-core` instead.

`user_dirs` is the one module with a third answer — where *your*
Pictures and Downloads are — and it belongs here for the same reason:
two apps ask it, and it needs nothing but `std`.

## Where this lives

Part of [Hyprforge](https://github.com/hyprforge-suite/hyprforge), a suite of native Hyprland desktop
applications. This crate is `crates/hyprforge-paths` there; it is published to
crates.io so the suite's applications can be built from their own
repositories, and its API follows the suite. Issues and pull requests go
to the suite repository.

```
cargo add hyprforge-paths
```

This README is rendered from the crate's `//!` documentation by
`tools/crate-readme.py`; edit `src/lib.rs`, not this file.

## Licence

MIT. See `LICENSE`.
