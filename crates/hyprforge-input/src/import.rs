//! Adopting the input settings a user already has.
//!
//! Unlike shortcuts and window rules, this module doesn't evaluate the
//! user's `hyprland.lua` to find out what they configured. It asks the
//! running compositor, because `hyprctl getoption -j` reports something no
//! amount of config parsing can: a `set` flag distinguishing "the user
//! chose this" from "this is Hyprland's default".
//!
//! That distinction is the whole feature. Importing every option would put
//! 51 keys under Hyprforge's ownership, most of which the user never had an
//! opinion about, and every one of those would then override anything they
//! later wrote by hand. Importing only what `set` reports gives exactly the
//! keys they actually configured — verified against this machine's config,
//! where the nine `set` options are precisely the nine lines in its
//! `hl.config` block.
//!
//! The tradeoff is that this reads the *live* compositor, so it reflects a
//! reload rather than the file on disk. For adoption that is the better
//! answer: what's running is what the user is actually experiencing.

use crate::catalog::{self, Kind};
use crate::model::{Settings, Value};
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
