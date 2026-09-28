# hyprforge-windowrules

Window and workspace rules: TOML storage, window-rules.lua codegen, import, and live windows and monitors to build a rule from; Hyprforge-internal, published so the suite's apps build from crates.io.

Window rules for Hyprland: the model behind the Settings app's Window
Rules screen, and the `window-rules.lua` it generates.

Hyprforge-internal, published so the suite's applications can build
from crates.io; the API follows the suite, not semver. A `Rule` is a
`Matcher` (which windows) and `Effects` (what happens to them); a
`WorkspaceRule` pins a workspace to a monitor. Both are stored in one
TOML file and rendered as `hl.window_rule` / `hl.workspace_rule` calls
into a file the user's `hyprland.lua` `require()`s. Nothing here draws.

# Two decisions someone would otherwise reverse

**Hyprforge's rules go first, not last.** Hyprland evaluates named
rules before anonymous ones and lets the last match win. Every rule
here is named, so placing the `require()` *before* the user's own
makes theirs override ours by default; appending would let a rule
saved in Settings silently beat a rule they wrote by hand. This is
the opposite of keybinds, and `setup` says why.

**Take names from the thing that compares them.** A mistyped class
or monitor produces a rule that never matches and nothing reports it,
so `clients` lists the windows actually open and `monitors` the
monitors Hyprland itself describes — `BOE 0x0BC9`, not displayd's
`BOE 0x0BC9  (eDP-2)`, because the string has to match what Hyprland
compares against.

# What is where

`model` is the data; `codegen` renders it as Lua and `import`
undoes that, one conversion per thing codegen does, for calls
`hyprforge-lua-import` recorded from a hand-written config (taken as
plain JSON, so this crate never links a Lua VM); `storage` is the
TOML file; `apply` writes, reloads and rolls back if Hyprland refuses
the file; `setup` installs the `require()` line through
`hyprforge_core::lua_setup`.

## Where this lives

Part of [Hyprforge](https://github.com/hyprforge-suite/hyprforge), a suite of native Hyprland desktop
applications. This crate is `crates/hyprforge-windowrules` there; it is published to
crates.io so the suite's applications can be built from their own
repositories, and its API follows the suite. Issues and pull requests go
to the suite repository.

```
cargo add hyprforge-windowrules
```

This README is rendered from the crate's `//!` documentation by
`tools/crate-readme.py`; edit `src/lib.rs`, not this file.

## Licence

MIT. See `LICENSE`.
