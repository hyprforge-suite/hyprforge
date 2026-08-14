//! Adopting the input settings a user already has, and reading what's
//! currently running.
//!
//! **Import comes from the user's config file**, evaluated by
//! `hyprforge-lua-import` the same way shortcuts and window rules import —
//! [`settings_from_call`] and [`candidates_from_config`].
//!
//! It used to come from the live compositor instead, keying off the `set`
//! flag in `hyprctl getoption -j`. That reads well and is wrong in a way
//! that bit a real user within a day: Hyprland reports *set*, not *set by
//! your config*, and it cannot tell the two apart. Anything that wrote an
//! option since the last reload — `hyprctl`, a script, a test suite —
//! is indistinguishable from a line in `hyprland.lua`. A stray
//! `input:touchdevice:enabled = false` from a test run got imported and
//! persisted that way, disabling touch input on a machine whose config had
//! never mentioned it.
//!
//! Evaluating the config has neither problem. The value is the one written
//! down, and `RecordedCall::source_path` lets the caller exclude
//! Hyprforge's own generated file, so importing can't echo Hyprforge's
//! values back as if the user had chosen them.
//!
//! [`live`] still reads the compositor, because "what is running right
//! now" is a genuinely different question and the editor needs it to show
//! unowned rows truthfully. It just isn't what import is built on.

use crate::catalog::{self, Kind};
use crate::model::{Settings, Value};
use std::collections::BTreeMap;
use std::process::Command;

#[derive(Debug, thiserror::Error)]
pub enum ImportError {
    #[error("couldn't run hyprctl: {0}")]
    Hyprctl(#[source] std::io::Error),
    #[error("hyprctl returned nothing — is Hyprland running?")]
    NoCompositor,
}

/// What the compositor currently has for one option, whether or not
/// anyone set it.
///
/// The editor needs this for every option, not just the set ones: a row
/// showing the catalog default for a key the user's own config sets to
/// something else is a screen that lies about what's running. `set` is what
/// lets a row say *where* the value it's showing came from.
#[derive(Debug, Clone, PartialEq)]
pub struct Live {
    pub key: &'static str,
    pub value: Value,
    /// The user's config writes this key, as opposed to it being
    /// Hyprland's own default.
    pub set: bool,
}

/// One option the compositor reports as explicitly set.
#[derive(Debug, Clone, PartialEq)]
pub struct Discovered {
    pub key: &'static str,
    pub label: &'static str,
    pub value: Value,
    /// This module already owns the key. Shown so the review list can say
    /// "already imported" rather than offering the same thing again.
    pub already_owned: bool,
    /// The stored value differs from what's live — the case worth the
    /// user's attention, since importing overwrites theirs.
    pub differs: bool,
}

/// Hyprland's placeholders for "nothing here", returned in the value field
/// for unset string options. They are display text, not values: importing
/// one literally would write `kb_file = [[[[EMPTY]]]]` into the config.
const SENTINELS: &[&str] = &["[[EMPTY]]", "[[Auto]]"];

/// Every catalog option the running compositor reports as explicitly set.
pub fn discover(current: &Settings) -> Result<Vec<Discovered>, ImportError> {
    let out = run_hyprctl()?;
    Ok(parse(&out, current))
}

/// What the compositor currently has for every catalogued option, set or
/// not — one `hyprctl` call, the same one [`discover`] makes.
pub fn live() -> Result<Vec<Live>, ImportError> {
    let out = run_hyprctl()?;
    Ok(parse_live(&out))
}

fn run_hyprctl() -> Result<String, ImportError> {
    let batch = catalog::SETTINGS
        .iter()
        .map(|s| format!("getoption {}", s.key))
        .collect::<Vec<_>>()
        .join(";");
    let out = Command::new("hyprctl")
        .arg("-j")
        .arg("--batch")
        .arg(batch)
        .output()
        .map_err(ImportError::Hyprctl)?;
    let body = String::from_utf8_lossy(&out.stdout).to_string();
    if body.trim().is_empty() {
        return Err(ImportError::NoCompositor);
    }
    Ok(body)
}

/// Split out from [`discover`] so the parsing is testable without a
/// compositor — the part that can actually be wrong.
pub fn parse(hyprctl_json: &str, current: &Settings) -> Vec<Discovered> {
    parse_live(hyprctl_json)
        .into_iter()
        .filter(|l| l.set)
        .map(|l| {
            let stored = current.get(l.key);
            Discovered {
                key: l.key,
                label: catalog::get(l.key).map(|s| s.label).unwrap_or(l.key),
                already_owned: stored.is_some(),
                differs: stored.is_some_and(|s| *s != l.value),
                value: l.value,
            }
        })
        .collect()
}

/// Every catalogued option in `hyprctl_json`, set or not.
pub fn parse_live(hyprctl_json: &str) -> Vec<Live> {
    let mut found = Vec::new();
    for line in hyprctl_json.lines() {
        let line = line.trim();
        if !line.starts_with('{') {
            continue;
        }
        let Ok(obj) = serde_json::from_str::<serde_json::Value>(line) else {
            continue;
        };
        let Some(key) = obj.get("option").and_then(|v| v.as_str()) else {
            continue;
        };
        let Some(setting) = catalog::get(key) else {
            continue;
        };
        let Some(value) = read_value(&obj, &setting.kind) else {
            continue;
        };
        found.push(Live {
            key: setting.key,
            value,
            set: obj.get("set").and_then(|v| v.as_bool()) == Some(true),
        });
    }
    found
}

/// Reads the value out of a `getoption` object using the type the catalog
/// claims, so a catalog type that disagrees with the compositor yields
/// nothing rather than a silently wrong value. The live test is what stops
/// that disagreement existing in the first place.
fn read_value(obj: &serde_json::Value, kind: &Kind) -> Option<Value> {
    match kind {
        Kind::Bool { .. } => obj.get("bool")?.as_bool().map(Value::Bool),
        Kind::Int { .. } | Kind::IntEnum { .. } => obj.get("int")?.as_i64().map(Value::Int),
        Kind::Float { .. } => obj.get("float")?.as_f64().map(Value::Float),
        Kind::Text { .. } | Kind::TextEnum { .. } => {
            let s = obj.get("str")?.as_str()?;
            if SENTINELS.contains(&s) {
                return None;
            }
            Some(Value::Text(s.to_string()))
        }
    }
}

/// Flattens one recorded `hl.config({...})` call into catalog keys.
///
/// Takes plain `(kind, args)` rather than a
/// `hyprforge_lua_import::RecordedCall`, so this crate never depends on the
/// evaluator — and therefore never on `mlua`. `serde_json::Value` is the
/// shared currency, the same arrangement window rules and shortcuts use.
///
/// This is [`crate::codegen`] in reverse: nested tables under `input`
/// become the colon keys the catalog is written in.
pub fn settings_from_call(kind: &str, args: &[serde_json::Value]) -> Vec<(&'static str, Value)> {
    if kind != "config" {
        return Vec::new();
    }
    let Some(input) = args.first().and_then(|a| a.get("input")) else {
        return Vec::new();
    };
    let mut out = Vec::new();
    flatten(input, "input", &mut out);
    out
}

fn flatten(node: &serde_json::Value, prefix: &str, out: &mut Vec<(&'static str, Value)>) {
    let Some(table) = node.as_object() else {
        return;
    };
    for (name, value) in table {
        let key = format!("{prefix}:{name}");
        if value.is_object() {
            flatten(value, &key, out);
            continue;
        }
        // Looked up rather than trusted: a config can set input options
        // this module doesn't cover (`input:tablet:*`) or that Hyprland
        // added since, and neither belongs in the store.
        let Some(setting) = catalog::get(&key) else {
            continue;
        };
        if let Some(v) = coerce(value, &setting.kind) {
            out.push((setting.key, v));
        }
    }
}

/// Reads a JSON value as the type the catalog declares.
///
/// The coercion that matters is integer-to-float: a config writing
/// `sensitivity = 0` produces a JSON integer, while the same setting is a
/// float to Hyprland. Storing it as an integer would render `sensitivity =
/// 0` into the generated Lua and hand an int to a float option.
fn coerce(value: &serde_json::Value, kind: &Kind) -> Option<Value> {
    match kind {
        Kind::Bool { .. } => value.as_bool().map(Value::Bool),
        Kind::Int { .. } | Kind::IntEnum { .. } => value.as_i64().map(Value::Int),
        Kind::Float { .. } => value.as_f64().map(Value::Float),
        Kind::Text { .. } | Kind::TextEnum { .. } => {
            value.as_str().map(|s| Value::Text(s.to_string()))
        }
    }
}

/// Turns flattened config settings into review candidates.
///
/// `found` must be in evaluation order, with Hyprforge's own generated file
/// already excluded by the caller — it has the `source_path` to do it, and
/// including it would echo Hyprforge's own values back as if the user had
/// written them. Later calls override earlier ones for the same key, which
/// is what Hyprland does.
pub fn candidates_from_config(
    found: &[(&'static str, Value)],
    current: &Settings,
) -> Vec<Discovered> {
    let mut resolved: BTreeMap<&'static str, Value> = BTreeMap::new();
    for (key, value) in found {
        resolved.insert(key, value.clone());
    }
    resolved
        .into_iter()
        .map(|(key, value)| {
            let stored = current.get(key);
            Discovered {
                key,
                label: catalog::get(key).map(|s| s.label).unwrap_or(key),
                already_owned: stored.is_some(),
                differs: stored.is_some_and(|s| *s != value),
                value,
            }
        })
        .collect()
}

/// Folds the chosen options into `settings`. Only the keys passed are
/// touched — importing is additive, so a user can adopt their keyboard
/// settings without the app also claiming their touchpad.
pub fn merge(settings: &mut Settings, chosen: &[Discovered]) {
    for d in chosen {
        settings.set(d.key, d.value.clone());
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Real `hyprctl -j --batch` output, blank lines and all.
    const SAMPLE: &str = r#"{"option": "input:kb_layout", "str": "us", "set": true }

{"option": "input:repeat_rate", "int": 25, "set": false }

{"option": "input:sensitivity", "float": 0.000000, "set": true }

{"option": "input:touchpad:natural_scroll", "bool": false, "set": true }

{"option": "input:kb_file", "str": "[[EMPTY]]", "set": false }
"#;

    #[test]
    fn only_options_the_user_actually_set_are_offered() {
        let found = parse(SAMPLE, &Settings::default());
        let keys: Vec<&str> = found.iter().map(|d| d.key).collect();
        assert_eq!(
            keys,
            vec![
                "input:kb_layout",
                "input:sensitivity",
                "input:touchpad:natural_scroll"
            ]
        );
    }

    #[test]
    fn each_type_is_read_as_the_catalog_declares_it() {
        let found = parse(SAMPLE, &Settings::default());
        assert_eq!(found[0].value, Value::Text("us".into()));
        assert_eq!(found[1].value, Value::Float(0.0));
        assert_eq!(found[2].value, Value::Bool(false));
    }

    /// A sentinel is display text for "unset". Importing it literally would
    /// write `[[EMPTY]]` into the config as if it were a filename.
    #[test]
    fn a_sentinel_placeholder_is_never_imported_as_a_value() {
        let json = r#"{"option": "input:kb_file", "str": "[[EMPTY]]", "set": true }"#;
        assert_eq!(parse(json, &Settings::default()), vec![]);
    }

    #[test]
    fn an_already_owned_key_is_marked_rather_than_hidden() {
        let mut current = Settings::default();
        current.set("input:kb_layout", Value::Text("us".into()));
        let found = parse(SAMPLE, &current);
        let layout = found.iter().find(|d| d.key == "input:kb_layout").unwrap();
        assert!(layout.already_owned);
        assert!(!layout.differs, "same value, so nothing to warn about");
    }

    /// The case that matters: importing would replace what the app already
    /// has, so the review list has to be able to say so.
    #[test]
    fn a_conflicting_value_is_flagged_as_different() {
        let mut current = Settings::default();
        current.set("input:kb_layout", Value::Text("de".into()));
        let found = parse(SAMPLE, &current);
        let layout = found.iter().find(|d| d.key == "input:kb_layout").unwrap();
        assert!(layout.already_owned && layout.differs);
    }

    #[test]
    fn merging_is_additive_and_leaves_other_keys_alone() {
        let mut settings = Settings::default();
        settings.set("input:repeat_delay", Value::Int(400));
        let found = parse(SAMPLE, &settings);
        merge(&mut settings, &found);
        assert_eq!(settings.get("input:repeat_delay"), Some(&Value::Int(400)));
        assert_eq!(
            settings.get("input:kb_layout"),
            Some(&Value::Text("us".into()))
        );
    }

    #[test]
    fn merging_a_subset_imports_only_that_subset() {
        let found = parse(SAMPLE, &Settings::default());
        let mut settings = Settings::default();
        merge(&mut settings, &found[..1]);
        assert_eq!(settings.values.len(), 1);
        assert!(settings.get("input:kb_layout").is_some());
    }

    /// The editor needs every option's live value, not just the set ones —
    /// a row falling back to the catalog default would show `false` for a
    /// setting the user's own config turned on.
    #[test]
    fn live_reports_unset_options_too_with_their_source_marked() {
        let live = parse_live(SAMPLE);
        let repeat = live.iter().find(|l| l.key == "input:repeat_rate").unwrap();
        assert_eq!(repeat.value, Value::Int(25));
        assert!(!repeat.set, "Hyprland's own default");
        let layout = live.iter().find(|l| l.key == "input:kb_layout").unwrap();
        assert!(layout.set, "the user's config writes this one");
    }

    /// A sentinel is still not a value, whichever way it's read.
    #[test]
    fn live_drops_sentinel_placeholders() {
        assert!(parse_live(SAMPLE).iter().all(|l| l.key != "input:kb_file"));
    }

    fn call(json: serde_json::Value) -> Vec<(&'static str, Value)> {
        settings_from_call("config", &[json])
    }

    /// The real shape, taken from this machine's `hyprland.lua`.
    #[test]
    fn a_config_call_flattens_into_catalog_keys() {
        let found = call(serde_json::json!({
            "input": {
                "kb_layout": "us",
                "numlock_by_default": true,
                "follow_mouse": 1,
                "sensitivity": 0,
                "touchpad": { "natural_scroll": false }
            }
        }));
        let map: BTreeMap<_, _> = found.into_iter().collect();
        assert_eq!(map["input:kb_layout"], Value::Text("us".into()));
        assert_eq!(map["input:numlock_by_default"], Value::Bool(true));
        assert_eq!(map["input:follow_mouse"], Value::Int(1));
        assert_eq!(map["input:touchpad:natural_scroll"], Value::Bool(false));
    }

    /// `sensitivity = 0` in a config is a JSON integer, but the setting is
    /// a float. Stored as an integer it would render `sensitivity = 0` and
    /// hand an int to a float option.
    #[test]
    fn an_integer_written_for_a_float_setting_becomes_a_float() {
        let map: BTreeMap<_, _> = call(serde_json::json!({ "input": { "sensitivity": 0 } }))
            .into_iter()
            .collect();
        assert_eq!(map["input:sensitivity"], Value::Float(0.0));
    }

    /// A config sets far more than input; nothing else may leak in.
    #[test]
    fn other_categories_are_ignored() {
        assert_eq!(
            call(serde_json::json!({
                "decoration": { "rounding": 10 },
                "general": { "border_size": 2 }
            })),
            vec![]
        );
    }

    /// Options outside the catalog — `input:tablet:*`, or anything
    /// Hyprland adds later — must not enter the store.
    #[test]
    fn uncatalogued_input_options_are_skipped() {
        let found = call(serde_json::json!({
            "input": {
                "tablet": { "left_handed": true },
                "some_new_option": 3,
                "kb_layout": "de"
            }
        }));
        assert_eq!(found, vec![("input:kb_layout", Value::Text("de".into()))]);
    }

    #[test]
    fn a_non_config_call_yields_nothing() {
        assert_eq!(settings_from_call("bind", &[serde_json::json!({})]), vec![]);
        assert_eq!(settings_from_call("config", &[]), vec![]);
    }

    /// Hyprland applies the last call for a key, so import must resolve the
    /// same way.
    #[test]
    fn a_later_call_wins_for_the_same_key() {
        let mut found = call(serde_json::json!({ "input": { "repeat_rate": 20 } }));
        found.extend(call(serde_json::json!({ "input": { "repeat_rate": 40 } })));
        let candidates = candidates_from_config(&found, &Settings::default());
        assert_eq!(candidates.len(), 1);
        assert_eq!(candidates[0].value, Value::Int(40));
    }

    /// The defect this replaced the live import to fix: a value only
    /// Hyprforge's own generated file sets must never come back as
    /// something the user configured. The caller drops that file by
    /// `source_path`; this pins that dropping it is sufficient.
    #[test]
    fn excluding_the_generated_file_removes_values_only_it_set() {
        let user = call(serde_json::json!({ "input": { "kb_layout": "us" } }));
        let generated = call(serde_json::json!({
            "input": { "kb_layout": "us", "touchdevice": { "enabled": false } }
        }));

        let with_generated: Vec<_> = user.iter().chain(generated.iter()).cloned().collect();
        assert!(
            candidates_from_config(&with_generated, &Settings::default())
                .iter()
                .any(|c| c.key == "input:touchdevice:enabled"),
            "including the generated file is what let the phantom value in"
        );

        let candidates = candidates_from_config(&user, &Settings::default());
        assert_eq!(candidates.len(), 1);
        assert_eq!(candidates[0].key, "input:kb_layout");
    }

    #[test]
    fn config_candidates_mark_what_is_already_owned() {
        let mut current = Settings::default();
        current.set("input:kb_layout", Value::Text("de".into()));
        let found = call(serde_json::json!({ "input": { "kb_layout": "us" } }));
        let candidates = candidates_from_config(&found, &current);
        assert!(candidates[0].already_owned && candidates[0].differs);
    }

    /// Everything imported from a config has to be storable, or import
    /// hands the user a file its own validator rejects.
    #[test]
    fn config_imported_values_pass_validation() {
        let found = call(serde_json::json!({
            "input": {
                "kb_layout": "us", "sensitivity": 0, "repeat_rate": 25,
                "touchpad": { "natural_scroll": true }
            }
        }));
        let mut settings = Settings::default();
        let candidates = candidates_from_config(&found, &settings);
        merge(&mut settings, &candidates);
        assert_eq!(settings.validate(), vec![]);
    }

    #[test]
    fn garbage_lines_are_skipped_rather_than_fatal() {
        let json = "no such option\n\n{\"option\": \"input:kb_layout\", \"str\": \"us\", \"set\": true }\nnot json at all\n";
        assert_eq!(parse(json, &Settings::default()).len(), 1);
    }

    /// Everything imported has to be storable, or import would hand the
    /// user a settings file its own validator rejects.
    #[test]
    fn imported_values_pass_validation() {
        let found = parse(SAMPLE, &Settings::default());
        let mut settings = Settings::default();
        merge(&mut settings, &found);
        assert_eq!(settings.validate(), vec![]);
    }
}
