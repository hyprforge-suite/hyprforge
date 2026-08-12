use std::path::PathBuf;

/// One `hl.bind`/`hl.window_rule`/`hl.workspace_rule`/`hl.monitor` call
/// recorded while evaluating a user's config, in whatever form the real
/// interpreter resolved its arguments to — never the literal Lua source.
#[derive(Debug, Clone, PartialEq)]
pub struct RecordedCall {
    /// `"bind"` | `"window_rule"` | `"workspace_rule"` | `"monitor"`.
    pub kind: String,
    /// Which `.lua` file this call came from, so a caller can exclude
    /// Hyprforge's own generated files (also `require()`d from
    /// `hyprland.lua`, and therefore evaluated right alongside the user's
    /// own) from import candidates.
    pub source_path: PathBuf,
    /// The 1-indexed source line the call *starts* on (`Lua::inspect_stack`
    /// at the caller's level), if the interpreter reported one. For a
    /// call written entirely on one line, this is also where it *ends* —
    /// which a caller can check for directly (does this line, trimmed,
    /// both start with `hl.` and end with `)`?) before treating it as
    /// safe to remove after a successful import. A multi-line call's
    /// start line fails that check on its own, so the same test doubles
    /// as "don't touch anything whose full span isn't known."
    pub line: Option<usize>,
    pub args: Vec<serde_json::Value>,
}

/// The outcome of evaluating a user's `hyprland.lua` and everything it
/// `require()`s.
#[derive(Debug, Clone, Default)]
pub struct ImportResult {
    pub calls: Vec<RecordedCall>,
    /// `(file, reason)` for anything that didn't evaluate — a syntax/API
    /// the sandbox doesn't support, a `require()` target that doesn't
    /// exist, or the instruction/memory limit. Never a hard failure for
    /// the whole import: everything else still evaluates and is reported
    /// (vision pillar #3 — no dead ends, no silent partial results).
    pub failures: Vec<(PathBuf, String)>,
}
