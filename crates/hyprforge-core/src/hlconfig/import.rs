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

use super::{Catalog, Kind, Settings, Value};
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
pub fn discover(catalog: &Catalog, current: &Settings) -> Result<Vec<Discovered>, ImportError> {
    let out = run_hyprctl(catalog)?;
    Ok(parse(&out, catalog, current))
}

/// What the compositor currently has for every catalogued option, set or
/// not — one `hyprctl` call, the same one [`discover`] makes.
pub fn live(catalog: &Catalog) -> Result<Vec<Live>, ImportError> {
    let out = run_hyprctl(catalog)?;
    Ok(parse_live(&out, catalog))
}

fn run_hyprctl(catalog: &Catalog) -> Result<String, ImportError> {
    let batch = catalog.settings
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
pub fn parse(hyprctl_json: &str, catalog: &Catalog, current: &Settings) -> Vec<Discovered> {
    parse_live(hyprctl_json, catalog)
        .into_iter()
        .filter(|l| l.set)
        .map(|l| {
            let stored = current.get(l.key);
            Discovered {
                key: l.key,
                label: catalog.get(l.key).map(|s| s.label).unwrap_or(l.key),
                already_owned: stored.is_some(),
                differs: stored.is_some_and(|s| *s != l.value),
                value: l.value,
            }
        })
        .collect()
}

/// Every catalogued option in `hyprctl_json`, set or not.
pub fn parse_live(hyprctl_json: &str, catalog: &Catalog) -> Vec<Live> {
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
        let Some(setting) = catalog.get(key) else {
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
        Kind::Color { .. } => read_gradient(obj.get("gradient")?.as_str()?),
        Kind::Gaps { .. } => read_gaps(obj.get("css")?.as_str()?),
    }
}

/// Converts a live gradient back into a form Hyprland would accept as
/// input.
///
/// `hyprctl` reports `ffbd93f9 0deg` — `AARRGGBB` plus an angle — for a
/// colour written as `rgba(bd93f9ff)`. Copying that string into a config
/// would be refused, so it is converted rather than passed through.
///
/// Anything that is a real gradient (several stops, or a non-zero angle)
/// returns `None`: it can't be represented as the single colour this
/// editor offers, and showing one stop of it would be a quiet lie about
/// what is running.
pub fn read_gradient(raw: &str) -> Option<Value> {
    let mut parts = raw.split_whitespace();
    let argb = parts.next()?;
    match parts.next() {
        None | Some("0deg") => {}
        // An angle means a real gradient, and so does a second colour.
        Some(_) => return None,
    }
    if parts.next().is_some() || argb.len() != 8 || !argb.chars().all(|c| c.is_ascii_hexdigit()) {
        return None;
    }
    let (alpha, rgb) = argb.split_at(2);
    Some(Value::Text(format!("rgba({rgb}{alpha})")))
}

/// Converts a live `css` gap back into the integer Hyprland accepts.
///
/// Reported as four numbers (`5 5 5 5`) whatever was written. Only a
/// uniform gap round-trips: per-side gaps need the table form, which this
/// editor doesn't offer, and collapsing them to one number would silently
/// change three of the four sides.
pub fn read_gaps(raw: &str) -> Option<Value> {
    let parts: Vec<&str> = raw.split_whitespace().collect();
    let first = parts.first()?.parse::<i64>().ok()?;
    if parts.iter().any(|p| p.parse::<i64>().ok() != Some(first)) {
        return None;
    }
    Some(Value::Int(first))
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
pub fn settings_from_call(
    kind: &str,
    args: &[serde_json::Value],
    catalog: &Catalog,
) -> Vec<(&'static str, Value)> {
    if kind != "config" {
        return Vec::new();
    }
    let Some(table) = args.first().and_then(|a| a.as_object()) else {
        return Vec::new();
    };
    // Every top-level category at once. A config sets far more than any
    // one module owns; the catalog lookup below is what filters.
    let mut out = Vec::new();
    for (name, value) in table {
        flatten(value, name, catalog, &mut out);
    }
    out
}

fn flatten(
    node: &serde_json::Value,
    prefix: &str,
    catalog: &Catalog,
    out: &mut Vec<(&'static str, Value)>,
) {
    let Some(table) = node.as_object() else {
        return;
    };
    for (name, value) in table {
        let key = format!("{prefix}:{name}");
        if value.is_object() {
            flatten(value, &key, catalog, out);
            continue;
        }
        // Looked up rather than trusted: a config can set input options
        // this module doesn't cover (`input:tablet:*`) or that Hyprland
        // added since, and neither belongs in the store.
        let Some(setting) = catalog.get(&key) else {
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
        Kind::Text { .. } | Kind::TextEnum { .. } | Kind::Color { .. } => {
            value.as_str().map(|s| Value::Text(s.to_string()))
        }
        // A config writes a gap as an integer; the table form is a
        // per-side gap this editor can't represent, and it arrives here
        // as an object, which `flatten` has already recursed into.
        Kind::Gaps { .. } => value.as_i64().map(Value::Int),
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
    catalog: &Catalog,
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
                label: catalog.get(key).map(|s| s.label).unwrap_or(key),
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
    use crate::hlconfig::model::test_catalog::CATALOG;

    /// Real `hyprctl -j --batch` output, blank lines and all.
    const SAMPLE: &str = r#"{"option": "demo:name", "str": "us", "set": true }

{"option": "demo:count", "int": 25, "set": false }

{"option": "demo:ratio", "float": 0.000000, "set": true }

{"option": "demo:nested:deep", "bool": false, "set": true }

{"option": "demo:mode", "str": "[[EMPTY]]", "set": false }
"#;

    fn parsed(current: &Settings) -> Vec<Discovered> {
        parse(SAMPLE, &CATALOG, current)
    }

    #[test]
    fn only_options_the_user_actually_set_are_offered() {
        let keys: Vec<&str> = parsed(&Settings::default()).iter().map(|d| d.key).collect();
        assert_eq!(keys, vec!["demo:name", "demo:ratio", "demo:nested:deep"]);
    }

    #[test]
    fn each_type_is_read_as_the_catalog_declares_it() {
        let found = parsed(&Settings::default());
        assert_eq!(found[0].value, Value::Text("us".into()));
        assert_eq!(found[1].value, Value::Float(0.0));
        assert_eq!(found[2].value, Value::Bool(false));
    }

    /// A sentinel is display text for "unset". Importing it literally
    /// would write `[[EMPTY]]` into the config as if it were a value.
    #[test]
    fn a_sentinel_placeholder_is_never_imported_as_a_value() {
        let json = r#"{"option": "demo:mode", "str": "[[EMPTY]]", "set": true }"#;
        assert_eq!(parse(json, &CATALOG, &Settings::default()), vec![]);
    }

    #[test]
    fn live_reports_unset_options_too_with_their_source_marked() {
        let live = parse_live(SAMPLE, &CATALOG);
        let count = live.iter().find(|l| l.key == "demo:count").unwrap();
        assert_eq!(count.value, Value::Int(25));
        assert!(!count.set, "Hyprland's own default");
        assert!(live.iter().find(|l| l.key == "demo:name").unwrap().set);
    }

    #[test]
    fn live_drops_sentinel_placeholders() {
        assert!(parse_live(SAMPLE, &CATALOG).iter().all(|l| l.key != "demo:mode"));
    }

    #[test]
    fn an_already_owned_key_is_marked_rather_than_hidden() {
        let current = Settings::from_one("demo:name", Value::Text("us".into()));
        let found = parsed(&current);
        let name = found.iter().find(|d| d.key == "demo:name").unwrap();
        assert!(name.already_owned);
        assert!(!name.differs, "same value, so nothing to warn about");
    }

    #[test]
    fn a_conflicting_value_is_flagged_as_different() {
        let current = Settings::from_one("demo:name", Value::Text("de".into()));
        let found = parsed(&current);
        let name = found.iter().find(|d| d.key == "demo:name").unwrap();
        assert!(name.already_owned && name.differs);
    }

    #[test]
    fn merging_is_additive_and_leaves_other_keys_alone() {
        let mut settings = Settings::from_one("demo:count", Value::Int(4));
        let found = parsed(&settings);
        merge(&mut settings, &found);
        assert_eq!(settings.get("demo:count"), Some(&Value::Int(4)));
        assert_eq!(settings.get("demo:name"), Some(&Value::Text("us".into())));
    }

    #[test]
    fn merging_a_subset_imports_only_that_subset() {
        let found = parsed(&Settings::default());
        let mut settings = Settings::default();
        merge(&mut settings, &found[..1]);
        assert_eq!(settings.values.len(), 1);
        assert!(settings.get("demo:name").is_some());
    }

    #[test]
    fn garbage_lines_are_skipped_rather_than_fatal() {
        let json = "no such option\n\n{\"option\": \"demo:name\", \"str\": \"us\", \"set\": true }\nnot json at all\n";
        assert_eq!(parse(json, &CATALOG, &Settings::default()).len(), 1);
    }

    fn call(json: serde_json::Value) -> Vec<(&'static str, Value)> {
        settings_from_call("config", &[json], &CATALOG)
    }

    #[test]
    fn a_config_call_flattens_into_catalog_keys() {
        let map: BTreeMap<_, _> = call(serde_json::json!({
            "demo": {
                "name": "us",
                "enabled": true,
                "level": 1,
                "ratio": 0,
                "nested": { "deep": false }
            }
        }))
        .into_iter()
        .collect();
        assert_eq!(map["demo:name"], Value::Text("us".into()));
        assert_eq!(map["demo:enabled"], Value::Bool(true));
        assert_eq!(map["demo:level"], Value::Int(1));
        assert_eq!(map["demo:nested:deep"], Value::Bool(false));
    }

    /// `ratio = 0` in a config is a JSON integer, but the setting is a
    /// float. Stored as an integer it would render `ratio = 0` and hand
    /// an int to a float option.
    #[test]
    fn an_integer_written_for_a_float_setting_becomes_a_float() {
        let map: BTreeMap<_, _> = call(serde_json::json!({ "demo": { "ratio": 0 } }))
            .into_iter()
            .collect();
        assert_eq!(map["demo:ratio"], Value::Float(0.0));
    }

    /// A config sets far more than any one module owns; only catalogued
    /// keys may enter the store.
    #[test]
    fn uncatalogued_options_are_skipped() {
        let found = call(serde_json::json!({
            "general": { "border_size": 2 },
            "demo": { "untouched": { "x": 1 }, "brand_new": 3, "name": "de" }
        }));
        assert_eq!(found, vec![("demo:name", Value::Text("de".into()))]);
    }

    #[test]
    fn a_non_config_call_yields_nothing() {
        assert_eq!(settings_from_call("bind", &[serde_json::json!({})], &CATALOG), vec![]);
        assert_eq!(settings_from_call("config", &[], &CATALOG), vec![]);
    }

    /// Hyprland applies the last call for a key, so import must resolve
    /// the same way.
    #[test]
    fn a_later_call_wins_for_the_same_key() {
        let mut found = call(serde_json::json!({ "demo": { "count": 2 } }));
        found.extend(call(serde_json::json!({ "demo": { "count": 4 } })));
        let candidates = candidates_from_config(&found, &CATALOG, &Settings::default());
        assert_eq!(candidates.len(), 1);
        assert_eq!(candidates[0].value, Value::Int(4));
    }

    /// The defect that made import read the config instead of the live
    /// compositor. Hyprforge's own generated file evaluates alongside the
    /// user's own, so a value only *it* sets must not come back as
    /// something the user configured — the caller drops it by
    /// `source_path`, and this pins that doing so is sufficient.
    #[test]
    fn excluding_the_generated_file_removes_values_only_it_set() {
        let user = call(serde_json::json!({ "demo": { "name": "us" } }));
        let generated = call(serde_json::json!({
            "demo": { "name": "us", "nested": { "deep": false } }
        }));

        let both: Vec<_> = user.iter().chain(generated.iter()).cloned().collect();
        assert!(
            candidates_from_config(&both, &CATALOG, &Settings::default())
                .iter()
                .any(|c| c.key == "demo:nested:deep"),
            "including the generated file is what let the phantom value in"
        );

        let candidates = candidates_from_config(&user, &CATALOG, &Settings::default());
        assert_eq!(candidates.len(), 1);
        assert_eq!(candidates[0].key, "demo:name");
    }

    #[test]
    fn config_candidates_mark_what_is_already_owned() {
        let current = Settings::from_one("demo:name", Value::Text("de".into()));
        let found = call(serde_json::json!({ "demo": { "name": "us" } }));
        let candidates = candidates_from_config(&found, &CATALOG, &current);
        assert!(candidates[0].already_owned && candidates[0].differs);
    }

    /// The exact string `hyprctl` reports for a colour written as
    /// `rgba(bd93f9ff)`. Copying it into a config would be refused, so it
    /// has to be converted back, not passed through.
    #[test]
    fn a_live_gradient_converts_back_to_the_form_hyprland_accepts() {
        assert_eq!(
            read_gradient("ffbd93f9 0deg"),
            Some(Value::Text("rgba(bd93f9ff)".into()))
        );
        assert_eq!(
            read_gradient("ee1a1a1a"),
            Some(Value::Text("rgba(1a1a1aee)".into()))
        );
    }

    /// A real gradient can't be shown as the single colour this editor
    /// offers, and showing one stop of it would misreport what's running.
    #[test]
    fn a_real_gradient_is_not_flattened_to_one_colour() {
        assert_eq!(read_gradient("ffbd93f9 45deg"), None);
        assert_eq!(read_gradient("ffbd93f9 ff282a36 0deg"), None);
        assert_eq!(read_gradient("nonsense"), None);
    }

    /// Gaps are reported as four numbers whatever was written.
    #[test]
    fn a_uniform_live_gap_reads_back_as_its_integer() {
        assert_eq!(read_gaps("5 5 5 5"), Some(Value::Int(5)));
        assert_eq!(read_gaps("0 0 0 0"), Some(Value::Int(0)));
    }

    /// Collapsing per-side gaps to one number would silently change three
    /// of the four sides.
    #[test]
    fn per_side_gaps_are_not_collapsed_into_one_number() {
        assert_eq!(read_gaps("5 10 5 10"), None);
        assert_eq!(read_gaps(""), None);
    }

    /// Everything imported has to be storable, or import hands the user a
    /// file its own validator rejects.
    #[test]
    fn imported_values_pass_validation() {
        let found = call(serde_json::json!({
            "demo": { "name": "us", "ratio": 0, "count": 3, "nested": { "deep": true } }
        }));
        let mut settings = Settings::default();
        let candidates = candidates_from_config(&found, &CATALOG, &settings);
        merge(&mut settings, &candidates);
        assert_eq!(settings.validate(&CATALOG), vec![]);
    }
}
