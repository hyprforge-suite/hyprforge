//! Converts a call recorded by `hyprforge-lua-import` (evaluating a user's
//! own `hyprland.lua`) back into this crate's own `Matcher`/`Effects`/
//! `WorkspaceRule` shapes.
//!
//! Takes plain `(kind, args)` rather than
//! `hyprforge_lua_import::RecordedCall` directly, so this crate never has
//! to depend on `hyprforge-lua-import` — and therefore never on `mlua` —
//! itself; only the caller orchestrating an import (the Settings GUI)
//! does. `serde_json::Value` is the shared currency instead.
//!
//! This is `codegen.rs` in reverse: the captured JSON mirrors exactly what
//! `hl.window_rule({...})`/`hl.workspace_rule({...})` receive on the wire,
//! which is *not* the same shape as the TOML-facing `Matcher`/`Effects`
//! structs — `workspace`/`opacity` are single Hyprland strings there,
//! structured fields here. Every conversion below undoes one thing
//! `codegen.rs` does; see that file for the forward direction.

use crate::model::{Effects, Matcher, Opacity, Workspace, WorkspaceRule};
use serde_json::{Map, Value};

/// A `Rule`'s matcher/effects/enabled, recovered from a recorded
/// `hl.window_rule({...})` call — everything but `name`, which the caller
/// generates fresh via [`crate::model::generate_rule_name`], the same as
/// any other newly-added rule. `None` if the call has no matcher at all
/// (Hyprland requires one, so a call missing every match field couldn't
/// have been a real window rule).
pub fn rule_from_call(kind: &str, args: &[Value]) -> Option<ImportedRule> {
    if kind != "window_rule" {
        return None;
    }
    let table = args.first()?.as_object()?;
    let match_table = table.get("match")?.as_object()?;
    let matcher = matcher_from_json(match_table);

    let mut dropped = unknown_keys(match_table, MATCHER_KEYS);
    dropped.extend(unknown_keys(table, EFFECT_KEYS));
    dropped.sort();
    dropped.dedup();

    // An empty matcher used to mean "not a window rule". It also means
    // "every field this rule matched on is one we don't model" — a rule
    // like `match = { workspace = "3" }` is perfectly real, and reading
    // it as absent is what let the caller delete the source line for a
    // rule it was never going to regenerate.
    if matcher.is_empty() && dropped.is_empty() {
        return None;
    }

    let enabled = table.get("enabled").and_then(Value::as_bool).unwrap_or(true);
    Some(ImportedRule { matcher, effects: effects_from_json(table), enabled, dropped })
}

/// A window rule recovered from a call, and what could not be recovered
/// with it.
///
/// `dropped` is the whole point of the struct. This importer models a
/// fixed set of keys, Hyprland accepts more, and the caller *deletes the
/// user's original line* once a rule is imported. Without a record of
/// what was not understood, a rule is silently widened — `class` kept,
/// `workspace` dropped — and the line that said otherwise is removed. A
/// rule that floated one Steam window on workspace 3 then floats every
/// Steam window everywhere, and the review screen shows a clean import.
#[derive(Debug, Clone, PartialEq)]
pub struct ImportedRule {
    pub matcher: Matcher,
    pub effects: Effects,
    pub enabled: bool,
    /// Keys present in the call that this importer does not model, in
    /// sorted order. Empty means the rule round-trips exactly.
    pub dropped: Vec<String>,
}

impl ImportedRule {
    /// Whether regenerating this rule reproduces what the user wrote.
    ///
    /// Only a rule that does may have its hand-written source line
    /// removed — the same rule `hyprforge-shortcuts` applies through
    /// `is_complete`, for the same reason: codegen emits nothing for an
    /// empty matcher, so removing the original would delete a working
    /// rule and put nothing back.
    pub fn is_faithful(&self) -> bool {
        self.dropped.is_empty() && !self.matcher.is_empty()
    }
}

/// Keys of `match = { .. }` that [`matcher_from_json`] reads.
const MATCHER_KEYS: &[&str] = &[
    "class",
    "title",
    "initial_class",
    "initial_title",
    "fullscreen",
    "float",
    "xwayland",
    "tag",
    "content",
];

/// Top-level keys of `hl.window_rule({ .. })` that this importer reads.
/// `match` and `enabled` are structural rather than effects.
const EFFECT_KEYS: &[&str] = &[
    "match",
    "enabled",
    "workspace",
    "tag",
    "float",
    "move",
    "size",
    "opacity",
    "border_color",
    "no_blur",
    "rounding",
    "opaque",
    "no_anim",
    "no_focus",
    "stay_focused",
    "dim_around",
    "keep_aspect_ratio",
    "border_size",
    "min_size",
    "max_size",
    "animation",
    "idle_inhibit",
    "tile",
    "fullscreen",
    "maximize",
    "pin",
    "center",
    "no_initial_focus",
    "monitor",
    "suppress_event",
    "group",
    "no_close_for",
];

/// Keys of `hl.workspace_rule({ .. })` that this importer reads.
const WORKSPACE_KEYS: &[&str] = &["workspace", "monitor", "default", "persistent"];

/// Keys present in `t` that are not in `known`.
///
/// A `null` counts as absent: the evaluator records an unset Lua field
/// that way, and reporting it as unmodelled would make every rule look
/// lossy.
fn unknown_keys(t: &Map<String, Value>, known: &[&str]) -> Vec<String> {
    t.iter()
        .filter(|(k, v)| !v.is_null() && !known.contains(&k.as_str()))
        .map(|(k, _)| k.clone())
        .collect()
}

/// A `WorkspaceRule`, recovered from a recorded `hl.workspace_rule({...})`
/// call. `None` if it has no workspace (Hyprland requires one).
pub fn workspace_rule_from_call(kind: &str, args: &[Value]) -> Option<ImportedWorkspaceRule> {
    if kind != "workspace_rule" {
        return None;
    }
    let table = args.first()?.as_object()?;
    let workspace = table.get("workspace")?.as_str()?.trim();
    if workspace.is_empty() {
        return None;
    }
    Some(ImportedWorkspaceRule {
        rule: WorkspaceRule {
            workspace: workspace.to_string(),
            monitor: get_str(table, "monitor").unwrap_or_default(),
            default: get_bool(table, "default").unwrap_or(false),
            persistent: get_bool(table, "persistent").unwrap_or(false),
        },
        dropped: {
            let mut d = unknown_keys(table, WORKSPACE_KEYS);
            d.sort();
            d
        },
    })
}

/// A workspace rule recovered from a call, and what could not be
/// recovered with it. See [`ImportedRule`] for why `dropped` exists —
/// this importer reads four of Hyprland's keys and drops the rest
/// (`on_created_empty`, `gaps_in`, `gaps_out`, `decorate`, ...).
#[derive(Debug, Clone, PartialEq)]
pub struct ImportedWorkspaceRule {
    pub rule: WorkspaceRule,
    pub dropped: Vec<String>,
}

impl ImportedWorkspaceRule {
    /// Whether regenerating this rule reproduces what the user wrote.
    pub fn is_faithful(&self) -> bool {
        self.dropped.is_empty()
    }
}

fn get_str(t: &Map<String, Value>, k: &str) -> Option<String> {
    t.get(k).and_then(Value::as_str).map(str::to_string)
}

fn get_bool(t: &Map<String, Value>, k: &str) -> Option<bool> {
    t.get(k).and_then(Value::as_bool)
}

fn get_i32(t: &Map<String, Value>, k: &str) -> Option<i32> {
    t.get(k).and_then(Value::as_i64).map(|n| n as i32)
}

fn matcher_from_json(m: &Map<String, Value>) -> Matcher {
    Matcher {
        class: get_str(m, "class"),
        title: get_str(m, "title"),
        initial_class: get_str(m, "initial_class"),
        initial_title: get_str(m, "initial_title"),
        fullscreen: get_bool(m, "fullscreen"),
        // Hyprland's matcher field is `float`, not `floating` — same
        // rename `render_matcher` in codegen.rs applies going the other
        // way.
        floating: get_bool(m, "float"),
        xwayland: get_bool(m, "xwayland"),
        tag: get_str(m, "tag"),
        content: get_str(m, "content"),
    }
}

/// `move`/`size` entries are a literal number for a pixel value or a
/// string for a Hyprland expression — `lua_expr` in codegen.rs is what
/// produces that split going the other way.
fn value_to_expr_string(v: &Value) -> Option<String> {
    match v {
        Value::String(s) => Some(s.clone()),
        Value::Number(n) => Some(n.to_string()),
        _ => None,
    }
}

fn pair_from_json(v: Option<&Value>) -> Option<[String; 2]> {
    let arr = v?.as_array()?;
    let a = value_to_expr_string(arr.first()?)?;
    let b = value_to_expr_string(arr.get(1)?)?;
    Some([a, b])
}

fn int_pair_from_json(v: Option<&Value>) -> Option<[i32; 2]> {
    let arr = v?.as_array()?;
    let a = arr.first()?.as_i64()? as i32;
    let b = arr.get(1)?.as_i64()? as i32;
    Some([a, b])
}

/// Inverse of [`Opacity::to_lua_value`]: that renders each set value as
/// `"<number><space><override>"` (when overriding) and joins with spaces,
/// so filtering the whitespace-split tokens down to the ones that parse as
/// numbers recovers the up-to-3 positional values regardless of whether
/// `override` appears between them.
fn parse_opacity(s: &str) -> Opacity {
    let is_override = s.contains("override");
    let numbers: Vec<f32> = s.split_whitespace().filter_map(|tok| tok.parse::<f32>().ok()).collect();
    Opacity {
        active: numbers.first().copied(),
        inactive: numbers.get(1).copied(),
        fullscreen: numbers.get(2).copied(),
        is_override,
    }
}

/// Inverse of [`Workspace::to_lua_value`].
fn parse_workspace(s: &str) -> Workspace {
    let s = s.trim();
    match s.strip_suffix(" silent") {
        Some(name) => Workspace { name: name.to_string(), silent: true },
        None => Workspace { name: s.to_string(), silent: false },
    }
}

fn effects_from_json(t: &Map<String, Value>) -> Effects {
    Effects {
        workspace: t
            .get("workspace")
            .and_then(Value::as_str)
            .map(parse_workspace)
            .unwrap_or_default(),
        tag: get_str(t, "tag"),
        float: get_bool(t, "float"),
        r#move: pair_from_json(t.get("move")),
        size: pair_from_json(t.get("size")),
        opacity: t
            .get("opacity")
            .and_then(Value::as_str)
            .map(parse_opacity)
            .unwrap_or_default(),
        border_color: get_str(t, "border_color"),
        no_blur: get_bool(t, "no_blur"),
        rounding: get_i32(t, "rounding"),
        opaque: get_bool(t, "opaque"),
        no_anim: get_bool(t, "no_anim"),
        no_focus: get_bool(t, "no_focus"),
        stay_focused: get_bool(t, "stay_focused"),
        dim_around: get_bool(t, "dim_around"),
        keep_aspect_ratio: get_bool(t, "keep_aspect_ratio"),
        border_size: get_i32(t, "border_size"),
        min_size: int_pair_from_json(t.get("min_size")),
        max_size: int_pair_from_json(t.get("max_size")),
        animation: get_str(t, "animation"),
        idle_inhibit: get_str(t, "idle_inhibit"),
        tile: get_bool(t, "tile"),
        fullscreen: get_bool(t, "fullscreen"),
        maximize: get_bool(t, "maximize"),
        pin: get_bool(t, "pin"),
        center: get_bool(t, "center"),
        no_initial_focus: get_bool(t, "no_initial_focus"),
        monitor: get_str(t, "monitor"),
        suppress_event: get_str(t, "suppress_event"),
        group: get_str(t, "group"),
        no_close_for: get_i32(t, "no_close_for"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::generate_rule_name;

    /// Runs `lua_source` as `hyprland.lua` through the real sandboxed
    /// evaluator, in a throwaway directory. Exercising `import.rs` against
    /// the actual evaluator (rather than hand-built JSON) is what proves
    /// the two sides of the crate boundary — `hl.window_rule({...})`'s
    /// real captured shape and what this file expects — actually agree.
    fn evaluated(lua_source: &str) -> hyprforge_lua_import::ImportResult {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("hyprland.lua"), lua_source).unwrap();
        let result = hyprforge_lua_import::evaluate(dir.path());
        assert!(result.failures.is_empty(), "unexpected failures: {:?}", result.failures);
        result
    }

    #[test]
    fn a_plain_literal_rule_round_trips() {
        let result = evaluated(r#"hl.window_rule({ match = { class = "^discord$" }, float = true, workspace = "name:coding silent" })"#);
        let call = &result.calls[0];
        let ImportedRule { matcher, effects, enabled, .. } =
            rule_from_call(&call.kind, &call.args).expect("should parse as a rule");
        assert_eq!(matcher.class.as_deref(), Some("^discord$"));
        assert_eq!(effects.float, Some(true));
        assert_eq!(effects.workspace.name, "name:coding");
        assert!(effects.workspace.silent);
        assert!(enabled);
        // Never trusts a captured `name` field — always freshly generated.
        let _ = generate_rule_name("discord", &[]);
    }

    #[test]
    fn the_matcher_float_key_maps_to_the_floating_field() {
        let result = evaluated(r#"hl.window_rule({ match = { float = false } })"#);
        let matcher = rule_from_call(&result.calls[0].kind, &result.calls[0].args).unwrap().matcher;
        assert_eq!(matcher.floating, Some(false));
    }

    #[test]
    fn opacity_round_trips_with_override() {
        let result = evaluated(
            r#"hl.window_rule({ match = { class = "x" }, opacity = "0.9 override 0.7 override 1 override" })"#,
        );
        let effects = rule_from_call(&result.calls[0].kind, &result.calls[0].args).unwrap().effects;
        assert_eq!(effects.opacity.active, Some(0.9));
        assert_eq!(effects.opacity.inactive, Some(0.7));
        assert_eq!(effects.opacity.fullscreen, Some(1.0));
        assert!(effects.opacity.is_override);
    }

    #[test]
    fn a_move_expression_and_a_literal_pixel_both_round_trip() {
        let result = evaluated(
            r#"hl.window_rule({ match = { class = "x" }, move = { "cursor_x-(window_w*0.5)", 40 } })"#,
        );
        let effects = rule_from_call(&result.calls[0].kind, &result.calls[0].args).unwrap().effects;
        assert_eq!(
            effects.r#move,
            Some(["cursor_x-(window_w*0.5)".to_string(), "40".to_string()])
        );
    }

    #[test]
    fn a_call_with_no_matcher_is_not_a_rule() {
        let result = evaluated(r#"hl.window_rule({ float = true })"#);
        assert!(rule_from_call(&result.calls[0].kind, &result.calls[0].args).is_none());
    }

    #[test]
    fn a_workspace_rule_round_trips() {
        let result =
            evaluated(r#"hl.workspace_rule({ workspace = "name:gaming", monitor = "desc:BOE 0x0BC9", default = true })"#);
        let wr = workspace_rule_from_call(&result.calls[0].kind, &result.calls[0].args).unwrap();
        assert_eq!(wr.rule.workspace, "name:gaming");
        assert_eq!(wr.rule.monitor, "desc:BOE 0x0BC9");
        assert!(wr.rule.default);
        assert!(!wr.rule.persistent);
    }

    #[test]
    fn a_bind_call_is_not_mistaken_for_a_rule() {
        let result = evaluated(r#"hl.bind("SUPER + Q", hl.dsp.window.close())"#);
        assert!(rule_from_call(&result.calls[0].kind, &result.calls[0].args).is_none());
    }
}

#[cfg(test)]
mod fidelity {
    use super::*;

    fn imported(lua: &str) -> ImportedRule {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("hyprland.lua"), lua).unwrap();
        let result = hyprforge_lua_import::evaluate(dir.path());
        assert!(result.failures.is_empty(), "unexpected failures: {:?}", result.failures);
        let call = &result.calls[0];
        rule_from_call(&call.kind, &call.args).expect("should parse as a rule")
    }

    /// The scenario that motivated all of this: a rule narrowed by a key
    /// we do not model. Importing it is fine; reporting it as faithful is
    /// not, because the caller deletes the source line for anything
    /// faithful and this rule comes back matching every Steam window on
    /// every workspace.
    #[test]
    fn a_matcher_key_we_cannot_read_makes_the_rule_unfaithful() {
        let rule = imported(
            r#"hl.window_rule({ match = { class = "^steam$", workspace = "3" }, float = true })"#,
        );
        assert_eq!(rule.matcher.class.as_deref(), Some("^steam$"));
        assert_eq!(rule.dropped, vec!["workspace".to_string()]);
        assert!(
            !rule.is_faithful(),
            "a widened rule must never earn the removal of its original line"
        );
    }

    /// A rule matched *only* by an unmodelled key used to read as "not a
    /// window rule at all", which is how it reached the caller as
    /// something safe to delete.
    #[test]
    fn a_rule_matched_only_by_an_unmodelled_key_is_still_a_rule() {
        let rule = imported(r#"hl.window_rule({ match = { workspace = "3" }, float = true })"#);
        assert!(rule.matcher.is_empty(), "nothing in the matcher is readable");
        assert!(!rule.dropped.is_empty(), "but the call said something we dropped");
        assert!(!rule.is_faithful());
    }

    /// The other half of the contract: a rule we read completely must
    /// stay faithful, or the gate would keep every original line forever
    /// and the import would never clean anything up.
    #[test]
    fn a_fully_understood_rule_is_faithful() {
        let rule = imported(
            r#"hl.window_rule({ match = { class = "^steam$" }, float = true, rounding = 4 })"#,
        );
        assert!(rule.dropped.is_empty(), "dropped: {:?}", rule.dropped);
        assert!(rule.is_faithful());
    }

    /// An unset Lua field arrives as JSON null. Counting it as dropped
    /// would make every rule look lossy and the gate would never open.
    #[test]
    fn an_unset_field_is_not_a_dropped_one() {
        let rule = imported(
            r#"hl.window_rule({ match = { class = "^steam$", title = nil }, float = true })"#,
        );
        assert!(rule.dropped.is_empty(), "dropped: {:?}", rule.dropped);
    }

    #[test]
    fn a_workspace_rule_reports_the_keys_it_cannot_read() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(
            dir.path().join("hyprland.lua"),
            r#"hl.workspace_rule({ workspace = "3", monitor = "eDP-1", on_created_empty = "kitty" })"#,
        )
        .unwrap();
        let result = hyprforge_lua_import::evaluate(dir.path());
        let call = &result.calls[0];
        let wr = workspace_rule_from_call(&call.kind, &call.args).unwrap();
        assert_eq!(wr.rule.workspace, "3");
        assert_eq!(wr.dropped, vec!["on_created_empty".to_string()]);
        assert!(!wr.is_faithful());
    }
}
