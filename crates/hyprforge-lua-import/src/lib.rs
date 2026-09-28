//! Reads a hand-written `hyprland.lua` by running it.
//!
//! Hyprforge-internal, published so the suite's applications can build
//! from crates.io; the API follows the suite, not semver. This is the one
//! crate in the suite that links a Lua interpreter (`mlua`), and keeping
//! it that way is the point of its shape: [`evaluate`] returns calls as
//! [`RecordedCall`]s carrying `serde_json::Value` arguments, and the
//! domain crates (`hyprforge-shortcuts`, `hyprforge-windowrules`) convert
//! those without ever depending on this crate — only the Settings app,
//! which orchestrates an import, does.
//!
//! # Why an interpreter and not a parser
//!
//! `hl.bind(mainMod .. " + Q", hl.dsp.window.close())` cannot be read as
//! a value from source text: `mainMod` is a variable, and only evaluating
//! the script — the user's own variables, in their own order, through
//! every `require()` — resolves it. So this crate builds a Lua state with
//! `hl.bind`, `hl.window_rule`, `hl.workspace_rule` and `hl.monitor`
//! stubbed to *record* their arguments, and `hl.dsp` as a proxy table
//! that turns any dispatcher path into a marker without knowing the
//! dispatcher names ahead of time. That is also what recovers the
//! *action* of a bind, which `hyprctl binds -j` reports only as `__lua`.
//!
//! # What "sandboxed" has to mean here
//!
//! The config is untrusted input, and `mlua::Lua::new()` — the "safe"
//! constructor — does **not** remove `os`, `io`, `package`, `load`,
//! `loadfile` or `dofile`. An early version of this crate let a test
//! config run `os.execute`. The globals are nil'd explicitly before
//! anything evaluates, `require()` is reimplemented to resolve against
//! the Hyprland config directory rather than Lua's `package.path`, and a
//! memory limit and an instruction limit bound a script that never
//! returns. A test asserts `os` and `io` stay
//! unreachable; anyone reusing `Lua::new()` on foreign Lua elsewhere
//! should not assume otherwise.
//!
//! # What comes back
//!
//! [`ImportResult`] never fails as a whole: every file that evaluated is
//! reported, and each one that did not is a `(path, reason)` in
//! `failures`. Each call carries its source file, so a caller can exclude
//! Hyprforge's own generated files (also `require()`d, so also
//! evaluated), and the line it starts on, so a line can be removed after
//! a successful import only when it is provably the whole call.
//!
//! [`check_syntax`] is the other export: compiling a fragment without
//! running it, for the Shortcuts editor's raw-Lua field, because a syntax
//! error there makes Hyprland reject the whole generated file.

mod eval;
mod record;
mod syntax;

pub use eval::evaluate;
pub use record::{ImportResult, RecordedCall};
pub use syntax::check_syntax;
