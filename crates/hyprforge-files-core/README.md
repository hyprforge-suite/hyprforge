# hyprforge-files-core

The model layer of the Hyprforge file manager: the Browser update/view state shared with the portal's file dialog, the fuzzy path bar, preferences, clipboard, drag, undo and sidebar, over iced 0.14 widgets.

The model layer of the Files app: what a directory listing is, how it
is read, sorted and filtered, and what gets remembered between
launches.

And the browser view itself: `browser::Browser`, the
`update`/`view` state both the app window and the portal's open/save
dialog render — see that module's doc for the `Mode` seam that makes
sharing it possible. Everything below it stays pure model code,
testable without a window: `prefs` here, and `sort`, `filter` and
`backend`, which now live in `hyprforge-listing` (see below).

## Where this lives

Part of [Hyprforge](https://github.com/hyprforge-suite/hyprforge), a suite of native Hyprland desktop
applications. This crate is `crates/hyprforge-files-core` there; it is published to
crates.io so the suite's applications can be built from their own
repositories, and its API follows the suite. Issues and pull requests go
to the suite repository.

```
cargo add hyprforge-files-core
```

This README is rendered from the crate's `//!` documentation by
`tools/crate-readme.py`; edit `src/lib.rs`, not this file.

## Licence

MIT. See `LICENSE`.
