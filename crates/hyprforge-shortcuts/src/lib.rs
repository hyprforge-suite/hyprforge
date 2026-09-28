//! Keyboard shortcuts for Hyprland: the model behind the Settings app's
//! Shortcuts screen, and the `keybinds.lua` it generates.
//!
//! Hyprforge-internal, published so the suite's applications can build
//! from crates.io; the API follows the suite, not semver. A shortcut is
//! a [`KeyCombo`] (modifiers plus a key, in Hyprland's own bit values), an
//! [`Action`] (a dispatcher and its arguments), and [`BindFlags`]; the set
//! is stored as TOML and rendered as `hl.bind(...)` lines into one file
//! that the user's `hyprland.lua` `require()`s. Nothing here draws.
//!
//! # Two decisions someone would otherwise reverse
//!
//! **Ask the compositor what is bound; do not parse the config.** A real
//! config binds `mainMod .. " + C"`, and static text cannot resolve the
//! variable. `hyprctl binds -j` reports what Hyprland actually resolved,
//! so [`binds`] reads that for conflict detection. Its limit is that a
//! Lua-defined bind's dispatcher comes back as opaque `__lua` — enough to
//! know a chord is taken, never what by. Recovering the *action* is
//! [`import`]'s job, fed by `hyprforge-lua-import` evaluating the config
//! for real; this crate takes the recorded call as plain JSON so it never
//! links a Lua VM itself.
//!
//! **Keybinds go last.** Hyprland resolves a repeated bind last-one-wins,
//! so the generated file is `require()`d after the user's own — a
//! shortcut changed in Settings takes effect over a line in a file the
//! user may not have opened in months. Window rules are placed the other
//! way round, for a reason that is theirs; see [`setup`].
//!
//! # What is where
//!
//! [`model`] is the data; [`catalog`] is which dispatchers exist and what
//! each takes, one entry per *meaning* rather than per dispatcher, every
//! entry loaded into a live Hyprland by a test; [`codegen`] and [`lua`]
//! render Lua; [`storage`] is the TOML file; [`apply`] writes, reloads,
//! and rolls back if Hyprland refuses; [`setup`] installs the `require()`
//! line through `hyprforge_core::lua_setup`.

pub mod apply;
pub mod binds;
pub mod catalog;
pub mod codegen;
pub mod import;
pub mod lua;
pub mod model;
pub mod setup;
pub mod storage;

pub use model::{Action, BindFlags, KeyCombo, Modifier, ParamValue, Shortcut};
