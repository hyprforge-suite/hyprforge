# hyprforge-core

Hyprland config machinery: the hl.config overlay, hyprlang files, Lua quoting, require() setup and reload-or-roll-back; Hyprforge-internal, published so the suite's apps build from crates.io.

Hyprland config machinery: everything the suite needs to own a slice
of a user's Hyprland configuration without owning the rest of it.

Hyprforge-internal, published so the suite's applications can build
from crates.io; the API follows the suite, not semver. A new app
should never need this crate — see the layering note in the
repository's `CLAUDE.md` — and one that talks to NetworkManager,
BlueZ or a file system wants `hyprforge-paths`, `hyprforge-process`
or `hyprforge-look` instead, none of which know Hyprland exists.

# The one arrangement everything here serves

Hyprforge never rewrites `hyprland.lua`. Each module owns one
generated file under `hypr/hyprforge/`, and the user's own config
pulls it in with a single `require()` that `lua_setup` inserts once
and never touches again. The ecosystem daemons (hyprpaper,
hyprsunset, hypridle) still read hyprlang, so `hyprlang` does the
same with a `source =` line. Both are last-one-wins formats, which is
where the two decisions someone would otherwise get wrong live:

- **Where the line goes is not a style choice.** An `hl.config`
  overlay or a keybind sourced *first* is silently beaten by the
  user's own block, and the app reports saves that changed nothing.
  `hlconfig` documents the measured semantics; each module's own
  `setup` picks a `lua_setup::Placement` and says why.
- **In a last-one-wins list, the duplicate to flag is the last one.**
  Every generator skips what its validation reports, so marking the
  winner drops the value that applies. `supersede` owns that rule
  because two crates got it backwards the same way.

# What is where

| Question | Module |
|---|---|
| Editing an `hl.config` category: catalog, storage, codegen, import | `hlconfig` |
| Owning a slice of a daemon's hyprlang file | `hyprlang` |
| Quoting a value as Lua source | `lua` |
| Inserting the `require()` line, once | `lua_setup` |
| Write, reload, verify, roll back if Hyprland refuses | `apply_lua` |
| Which duplicate is dead | `supersede` |
| A monitor's pixel size in the compositor's 1/120th-scale layout | `geometry` |
| Connector names, for settings that take `eDP-2` and not `desc:` | `monitors` |
| Where the generated files live | `paths` |

The D-Bus proxy for `hyprforge-displayd` (`displayd_proxy`) sits
behind the `dbus` feature so the Lua-only consumers never link zbus.
`command` is a compatibility re-export of `hyprforge-process`; new
code depends on that crate directly.

## Where this lives

Part of [Hyprforge](https://github.com/hyprforge-suite/hyprforge), a suite of native Hyprland desktop
applications. This crate is `crates/hyprforge-core` there; it is published to
crates.io so the suite's applications can be built from their own
repositories, and its API follows the suite. Issues and pull requests go
to the suite repository.

```
cargo add hyprforge-core
```

This README is rendered from the crate's `//!` documentation by
`tools/crate-readme.py`; edit `src/lib.rs`, not this file.

## Licence

MIT. See `LICENSE`.
