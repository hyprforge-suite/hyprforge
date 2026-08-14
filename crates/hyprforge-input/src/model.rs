//! What the module owns, and what it deliberately leaves alone.
//!
//! The central idea is that a setting is either **ours** or **not present**.
//! `hl.config` updates only the keys it's passed (measured, not assumed —
//! see [`crate::codegen`]), so a settings file holding three keys overrides
//! exactly those three and leaves every other input option to Hyprland's
//! default or the user's own config. That is what makes this module safe to
//! adopt gradually rather than all at once.

use crate::catalog::{self, Kind, Setting};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

/// Categories Hyprland has but this module doesn't edit, so the UI can say
/// so instead of implying input config is fully covered.
///
/// `input:tablet` needs a `vec2` editor for its mapped-region and
/// active-area fields, and hardware to verify against. Listing it is
/// cheaper than pretending it doesn't exist and leaving a user hunting for
/// where their tablet settings went.
pub const UNSUPPORTED_CATEGORIES: &[(&str, &str)] = &[(
    "input:tablet",
    "Drawing tablet mapping isn't editable here yet — keep it in your own config.",
)];

/// One setting's value. The variant must match its catalog [`Kind`];
/// [`Settings::validate`] is what enforces that.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum Value {
    Bool(bool),
    Int(i64),
    Float(f64),
    Text(String),
}

impl Value {
    /// The value a catalog default describes, for seeding an editor.
    pub fn default_for(kind: &Kind) -> Value {
        match *kind {
            Kind::Bool { default } => Value::Bool(default),
            Kind::Int { default, .. } | Kind::IntEnum { default, .. } => Value::Int(default),
            Kind::Float { default, .. } => Value::Float(default),
            Kind::Text { default } | Kind::TextEnum { default, .. } => {
                Value::Text(default.to_string())
            }
        }
    }

    pub fn as_bool(&self) -> Option<bool> {
        match self {
            Value::Bool(b) => Some(*b),
            _ => None,
        }
    }

    pub fn as_int(&self) -> Option<i64> {
        match self {
            Value::Int(i) => Some(*i),
            _ => None,
        }
    }

    pub fn as_float(&self) -> Option<f64> {
        match self {
            Value::Float(f) => Some(*f),
            // An integer literal in a hand-edited TOML is a perfectly clear
            // way to write a float setting; refusing `scroll_factor = 1`
            // would be pedantry.
            Value::Int(i) => Some(*i as f64),
            _ => None,
        }
    }

    pub fn as_text(&self) -> Option<&str> {
        match self {
            Value::Text(s) => Some(s),
            _ => None,
        }
    }
}

/// The settings this module owns, keyed by Hyprland's colon-form option key.
///
/// A `BTreeMap` rather than a `HashMap` so the generated Lua is byte-stable
/// across runs: a file that reshuffles itself on every save produces noise
/// in version control and makes a real change impossible to spot in a diff.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Settings {
    #[serde(default, flatten)]
    pub values: BTreeMap<String, Value>,
}

/// A stored value the catalog can't accept, named precisely enough to fix by
/// hand.
#[derive(Debug, Clone, PartialEq)]
pub struct Invalid {
    pub key: String,
    pub problem: String,
}

impl Settings {
    /// A one-key set, for checking a single value without building a whole
    /// settings file around it.
    pub fn from_one(key: &str, value: Value) -> Settings {
        let mut s = Settings::default();
        s.set(key, value);
        s
    }

    pub fn is_empty(&self) -> bool {
        self.values.is_empty()
    }

    pub fn get(&self, key: &str) -> Option<&Value> {
        self.values.get(key)
    }

    /// Takes ownership of `key`, replacing any previous value.
    pub fn set(&mut self, key: &str, value: Value) {
        self.values.insert(key.to_string(), value);
    }

    /// Gives `key` back to Hyprland and the user's own config: the
    /// generated file stops mentioning it entirely.
    pub fn clear(&mut self, key: &str) {
        self.values.remove(key);
    }

    /// Every stored value the catalog rejects: unknown keys, values of the
    /// wrong type, out-of-range numbers, and text outside a closed set.
    ///
    /// Nothing here is a crash, because every one of these is reachable by
    /// hand-editing the TOML, and a settings app that dies on a typo in its
    /// own config file is worse than one that points at the line.
    pub fn validate(&self) -> Vec<Invalid> {
        let mut out = Vec::new();
        for (key, value) in &self.values {
            let Some(setting) = catalog::get(key) else {
                out.push(Invalid {
                    key: key.clone(),
                    problem: unknown_key_problem(key),
                });
                continue;
            };
            if let Some(problem) = check(setting, value) {
                out.push(Invalid {
                    key: key.clone(),
                    problem,
                });
            }
        }
        out
    }
}

/// Why a key isn't in the catalog — separated because "this belongs to a
/// category we don't edit" is a completely different situation from "this
/// is a typo", and telling a user their tablet setting is a typo would send
/// them looking in the wrong place.
fn unknown_key_problem(key: &str) -> String {
    for (category, why) in UNSUPPORTED_CATEGORIES {
        if key.starts_with(&format!("{category}:")) {
            return (*why).to_string();
        }
    }
    "not an input option Hyprforge knows about".to_string()
}

fn check(setting: &Setting, value: &Value) -> Option<String> {
    match setting.kind {
        Kind::Bool { .. } => value.as_bool().map(|_| ()).ok_or("expected true or false".to_string()),
        Kind::Int { min, max, .. } => value
            .as_int()
            .ok_or("expected a whole number".to_string())
            .and_then(|i| in_range(i as f64, min.map(|m| m as f64), max.map(|m| m as f64))),
        Kind::IntEnum { choices, .. } => value
            .as_int()
            .ok_or("expected a whole number".to_string())
            .and_then(|i| {
                if choices.iter().any(|(v, _)| *v == i) {
                    Ok(())
                } else {
                    let allowed: Vec<String> =
                        choices.iter().map(|(v, _)| v.to_string()).collect();
                    Err(format!("expected one of {}", allowed.join(", ")))
                }
            }),
        Kind::Float { min, max, .. } => value
            .as_float()
            .ok_or("expected a number".to_string())
            .and_then(|f| in_range(f, min, max)),
        Kind::Text { .. } => value.as_text().map(|_| ()).ok_or("expected text".to_string()),
        Kind::TextEnum { choices, .. } => value
            .as_text()
            .ok_or("expected text".to_string())
            .and_then(|s| {
                if choices.contains(&s) {
                    Ok(())
                } else {
                    let allowed: Vec<String> = choices
                        .iter()
                        .map(|c| if c.is_empty() { "\"\"".to_string() } else { format!("\"{c}\"") })
                        .collect();
                    Err(format!("expected one of {}", allowed.join(", ")))
                }
            }),
    }
    .err()
}

fn in_range(v: f64, min: Option<f64>, max: Option<f64>) -> Result<(), String> {
    // Checked before the bounds, because NaN passes them: every comparison
    // against NaN is false, so `v < lo` and `v > hi` are both false and it
    // would sail through as in-range.
    //
    // `nan` and `inf` are both valid TOML floats, so a hand-edited file
    // reaches here. Neither survives codegen: NaN renders as `NaN.0`, a Lua
    // syntax error that takes the whole generated file — and therefore the
    // user's whole config — down, and `inf` renders as an undefined Lua
    // global that silently evaluates to nil.
    if !v.is_finite() {
        return Err("must be an ordinary number".to_string());
    }
    match (min, max) {
        (Some(lo), _) if v < lo => Err(format!("must be at least {lo}")),
        (_, Some(hi)) if v > hi => Err(format!("must be at most {hi}")),
        _ => Ok(()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_valid_set_has_nothing_to_report() {
        let mut s = Settings::default();
        s.set("input:kb_layout", Value::Text("us,cz".into()));
        s.set("input:repeat_rate", Value::Int(30));
        s.set("input:touchpad:tap_to_click", Value::Bool(true));
        s.set("input:sensitivity", Value::Float(-0.3));
        assert_eq!(s.validate(), vec![]);
    }

    #[test]
    fn an_out_of_range_number_is_named_with_its_limit() {
        let mut s = Settings::default();
        s.set("input:sensitivity", Value::Float(2.5));
        let bad = s.validate();
        assert_eq!(bad.len(), 1);
        assert_eq!(bad[0].key, "input:sensitivity");
        assert!(bad[0].problem.contains("at most 1"), "{}", bad[0].problem);
    }

    #[test]
    fn a_wrong_type_is_rejected_rather_than_coerced() {
        let mut s = Settings::default();
        s.set("input:touchpad:tap_to_click", Value::Text("yes".into()));
        assert_eq!(s.validate().len(), 1);
    }

    /// Writing `scroll_factor = 1` in the TOML is unambiguous, so it's
    /// accepted where a float is wanted. The reverse is not: a fractional
    /// repeat rate is a real mistake.
    #[test]
    fn an_integer_is_accepted_for_a_float_but_not_the_reverse() {
        let mut s = Settings::default();
        s.set("input:touchpad:scroll_factor", Value::Int(1));
        assert_eq!(s.validate(), vec![]);

        let mut s = Settings::default();
        s.set("input:repeat_rate", Value::Float(25.5));
        assert_eq!(s.validate().len(), 1);
    }

    #[test]
    fn a_value_outside_a_closed_set_lists_what_is_allowed() {
        let mut s = Settings::default();
        s.set("input:touchpad:tap_button_map", Value::Text("rlm".into()));
        let bad = s.validate();
        assert!(bad[0].problem.contains("\"lrm\""), "{}", bad[0].problem);
    }

    /// A tablet key in a hand-edited file is a real setting this module
    /// doesn't cover, not a typo, and saying so is what stops someone
    /// hunting for a misspelling that isn't there.
    #[test]
    fn an_unsupported_category_says_so_instead_of_calling_it_a_typo() {
        let mut s = Settings::default();
        s.set("input:tablet:left_handed", Value::Bool(true));
        let bad = s.validate();
        assert_eq!(bad.len(), 1);
        assert!(bad[0].problem.contains("tablet"), "{}", bad[0].problem);
        assert!(!bad[0].problem.contains("know"), "{}", bad[0].problem);
    }

    #[test]
    fn an_unknown_key_is_reported_as_unknown() {
        let mut s = Settings::default();
        s.set("input:kb_layuot", Value::Text("us".into()));
        let bad = s.validate();
        assert_eq!(bad.len(), 1);
        assert!(bad[0].problem.contains("know"), "{}", bad[0].problem);
    }

    /// `nan` and `inf` are valid TOML floats, so this is reachable by
    /// hand-editing. NaN slips past a naive range check — every comparison
    /// against it is false — and then renders as `NaN.0`, a syntax error
    /// that takes down the entire generated file.
    #[test]
    fn a_non_finite_float_is_refused_rather_than_passing_the_range_check() {
        for bad in [f64::NAN, f64::INFINITY, f64::NEG_INFINITY] {
            let mut s = Settings::default();
            s.set("input:touchpad:scroll_factor", Value::Float(bad));
            let problems = s.validate();
            assert_eq!(problems.len(), 1, "{bad} was accepted");
            assert!(
                problems[0].problem.contains("ordinary number"),
                "{}",
                problems[0].problem
            );
        }
    }

    /// The whole point of catching it: nothing non-finite may reach the
    /// generated Lua.
    #[test]
    fn a_non_finite_float_never_reaches_the_generated_file() {
        let mut s = Settings::default();
        s.set("input:touchpad:scroll_factor", Value::Float(f64::NAN));
        s.set("input:repeat_rate", Value::Int(30));
        let lua = crate::codegen::generate(&s);
        assert!(!lua.contains("NaN"), "{lua}");
        assert!(!lua.contains("scroll_factor"), "{lua}");
        assert!(lua.contains("repeat_rate = 30"), "{lua}");
    }

    #[test]
    fn clearing_a_key_removes_it_entirely() {
        let mut s = Settings::default();
        s.set("input:repeat_rate", Value::Int(30));
        s.clear("input:repeat_rate");
        assert!(s.is_empty());
    }

    #[test]
    fn defaults_come_back_with_the_catalog_type() {
        for setting in catalog::SETTINGS {
            let v = Value::default_for(&setting.kind);
            let mut s = Settings::default();
            s.set(setting.key, v);
            assert_eq!(s.validate(), vec![], "{} rejected its own default", setting.key);
        }
    }
}
