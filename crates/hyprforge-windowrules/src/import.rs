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
pub fn rule_from_call(kind: &str, args: &[Value]) -> Option<(Matcher, Effects, bool)> {
    if kind != "window_rule" {
        return None;
    }
    let table = args.first()?.as_object()?;
    let matcher = matcher_from_json(table.get("match")?.as_object()?);
    if matcher.is_empty() {
        return None;
    }
    let enabled = table.get("enabled").and_then(Value::as_bool).unwrap_or(true);
    Some((matcher, effects_from_json(table), enabled))
}

/// A `WorkspaceRule`, recovered from a recorded `hl.workspace_rule({...})`
/// call. `None` if it has no workspace (Hyprland requires one).
pub fn workspace_rule_from_call(kind: &str, args: &[Value]) -> Option<WorkspaceRule> {
    if kind != "workspace_rule" {
        return None;
    }
    let table = args.first()?.as_object()?;
    let workspace = table.get("workspace")?.as_str()?.trim();
    if workspace.is_empty() {
        return None;
    }
    Some(WorkspaceRule {
        workspace: workspace.to_string(),
        monitor: get_str(table, "monitor").unwrap_or_default(),
        default: get_bool(table, "default").unwrap_or(false),
        persistent: get_bool(table, "persistent").unwrap_or(false),
    })
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
        let (matcher, effects, enabled) =
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
        let (matcher, _, _) = rule_from_call(&result.calls[0].kind, &result.calls[0].args).unwrap();
        assert_eq!(matcher.floating, Some(false));
    }

    #[test]
    fn opacity_round_trips_with_override() {
        let result = evaluated(
            r#"hl.window_rule({ match = { class = "x" }, opacity = "0.9 override 0.7 override 1 override" })"#,
        );
        let (_, effects, _) = rule_from_call(&result.calls[0].kind, &result.calls[0].args).unwrap();
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
        let (_, effects, _) = rule_from_call(&result.calls[0].kind, &result.calls[0].args).unwrap();
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
        assert_eq!(wr.workspace, "name:gaming");
        assert_eq!(wr.monitor, "desc:BOE 0x0BC9");
        assert!(wr.default);
        assert!(!wr.persistent);
    }

    #[test]
    fn a_bind_call_is_not_mistaken_for_a_rule() {
        let result = evaluated(r#"hl.bind("SUPER + Q", hl.dsp.window.close())"#);
        assert!(rule_from_call(&result.calls[0].kind, &result.calls[0].args).is_none());
    }
}
