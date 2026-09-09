//! What the module owns, and what it deliberately leaves alone.
//!
//! The central idea is that a setting is either **ours** or **not present**.
//! `hl.config` updates only the keys it's passed (measured, not assumed —
//! see [`crate::codegen`]), so a settings file holding three keys overrides
//! exactly those three and leaves every other input option to Hyprland's
//! default or the user's own config. That is what makes this module safe to
//! adopt gradually rather than all at once.

use super::{Catalog, Kind, Setting};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

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
            Kind::Int { default, .. }
            | Kind::IntEnum { default, .. }
            | Kind::Gaps { default, .. } => Value::Int(default),
            Kind::Float { default, .. } => Value::Float(default),
            Kind::Text { default }
            | Kind::TextEnum { default, .. }
            | Kind::Color { default }
            | Kind::ColorInt { default } => Value::Text(default.to_string()),
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
    pub fn validate(&self, catalog: &Catalog) -> Vec<Invalid> {
        let mut out = Vec::new();
        for (key, value) in &self.values {
            let Some(setting) = catalog.get(key) else {
                out.push(Invalid {
                    key: key.clone(),
                    problem: catalog.unknown_key_reason(key),
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

fn check(setting: &Setting, value: &Value) -> Option<String> {
    match setting.kind {
        Kind::Bool { .. } => value.as_bool().map(|_| ()).ok_or("expected true or false".to_string()),
        Kind::Int { min, max, .. } | Kind::Gaps { min, max, .. } => value
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
        Kind::Color { .. } | Kind::ColorInt { .. } => value
            .as_text()
            .ok_or("expected a colour".to_string())
            .and_then(check_color),
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

/// Hyprland's colour literals: `rgba(rrggbbaa)` and `rgb(rrggbb)`.
///
/// Checked rather than passed through, because a malformed colour is
/// refused by Hyprland — and a refused value in a generated file takes
/// every other setting in that file down with it, not just this one.
///
/// Deliberately not exhaustive: Hyprland also accepts `0xAARRGGBB` and
/// named-gradient forms. Those are accepted as written rather than
/// rejected, since refusing a form Hyprland takes would be worse than
/// letting it through — a wrong "invalid" is a dead end, an unchecked
/// valid value is merely unchecked.
fn check_color(s: &str) -> Result<(), String> {
    // Only things that claim to be an rgb()/rgba() call are checked.
    // Anything else is left alone — see the doc comment: refusing a form
    // Hyprland accepts is worse than leaving one unchecked, so this is
    // the one place the strict parser must not spread to.
    if !s.starts_with("rgba(") && !s.starts_with("rgb(") {
        return Ok(());
    }
    hyprforge_look::Color::parse(s)
        .map(|_| ())
        .map_err(|_| "expected rgba(rrggbbaa) or rgb(rrggbb) in hex".to_string())
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

/// A small catalog for the tests in this module and its siblings —
/// enough shapes to exercise every branch without pinning the generic
/// machinery to any real module's option list.
#[cfg(test)]
pub(super) mod test_catalog {
    use super::super::{Catalog, Category, Kind, Setting};

    pub const SETTINGS: &[Setting] = &[
        Setting {
            key: "demo:count",
            label: "Count",
            help: "a whole number",
            kind: Kind::Int { default: 5, min: Some(1), max: Some(10) },
        },
        Setting {
            key: "demo:ratio",
            label: "Ratio",
            help: "a number",
            kind: Kind::Float { default: 1.0, min: Some(-1.0), max: Some(2.0) },
        },
        Setting {
            key: "demo:enabled",
            label: "Enabled",
            help: "on or off",
            kind: Kind::Bool { default: false },
        },
        Setting {
            key: "demo:name",
            label: "Name",
            help: "free text",
            kind: Kind::Text { default: "" },
        },
        Setting {
            key: "demo:mode",
            label: "Mode",
            help: "a closed set",
            kind: Kind::TextEnum { default: "", choices: &["", "fast", "slow"] },
        },
        Setting {
            key: "demo:level",
            label: "Level",
            help: "named numbers",
            kind: Kind::IntEnum { default: 0, choices: &[(0, "Off"), (1, "On")] },
        },
        Setting {
            key: "demo:tint",
            label: "Tint",
            help: "a colour",
            kind: Kind::Color { default: "rgba(000000ff)" },
        },
        Setting {
            key: "demo:nested:deep",
            label: "Deep",
            help: "a subcategory",
            kind: Kind::Bool { default: true },
        },
    ];

    pub const CATEGORIES: &[Category] = &[
        Category { key: "demo", label: "Demo", help: "" },
        Category { key: "demo:nested", label: "Nested", help: "" },
    ];

    pub const CATALOG: Catalog = Catalog {
        settings: SETTINGS,
        categories: CATEGORIES,
        unsupported: &[("demo:untouched", "Not editable here yet.")],
    };
}

#[cfg(test)]
mod tests {
    use super::test_catalog::CATALOG;
    use super::*;

    #[test]
    fn a_valid_set_has_nothing_to_report() {
        let mut s = Settings::default();
        s.set("demo:name", Value::Text("hello".into()));
        s.set("demo:count", Value::Int(3));
        s.set("demo:enabled", Value::Bool(true));
        s.set("demo:ratio", Value::Float(-0.3));
        s.set("demo:tint", Value::Text("rgba(bd93f9ff)".into()));
        assert_eq!(s.validate(&CATALOG), vec![]);
    }

    #[test]
    fn an_out_of_range_number_is_named_with_its_limit() {
        let s = Settings::from_one("demo:ratio", Value::Float(9.0));
        let bad = s.validate(&CATALOG);
        assert_eq!(bad.len(), 1);
        assert!(bad[0].problem.contains("at most 2"), "{}", bad[0].problem);
    }

    #[test]
    fn a_wrong_type_is_rejected_rather_than_coerced() {
        let s = Settings::from_one("demo:enabled", Value::Text("yes".into()));
        assert_eq!(s.validate(&CATALOG).len(), 1);
    }

    /// Writing `ratio = 1` in the TOML is unambiguous, so it's accepted
    /// where a float is wanted. The reverse is not: a fractional count is
    /// a real mistake.
    #[test]
    fn an_integer_is_accepted_for_a_float_but_not_the_reverse() {
        assert_eq!(
            Settings::from_one("demo:ratio", Value::Int(1)).validate(&CATALOG),
            vec![]
        );
        assert_eq!(
            Settings::from_one("demo:count", Value::Float(2.5))
                .validate(&CATALOG)
                .len(),
            1
        );
    }

    #[test]
    fn a_value_outside_a_closed_set_lists_what_is_allowed() {
        let s = Settings::from_one("demo:mode", Value::Text("sideways".into()));
        let bad = s.validate(&CATALOG);
        assert!(bad[0].problem.contains("\"fast\""), "{}", bad[0].problem);
    }

    /// A key from a part the module knowingly doesn't edit is a real
    /// setting, not a typo, and saying so is what stops someone hunting
    /// for a misspelling that isn't there.
    #[test]
    fn an_unsupported_category_says_so_instead_of_calling_it_a_typo() {
        let s = Settings::from_one("demo:untouched:thing", Value::Bool(true));
        let bad = s.validate(&CATALOG);
        assert_eq!(bad.len(), 1);
        assert!(bad[0].problem.contains("Not editable"), "{}", bad[0].problem);
    }

    #[test]
    fn an_unknown_key_is_reported_as_unknown() {
        let s = Settings::from_one("demo:cuont", Value::Int(1));
        let bad = s.validate(&CATALOG);
        assert_eq!(bad.len(), 1);
        assert!(bad[0].problem.contains("know"), "{}", bad[0].problem);
    }

    /// `nan` and `inf` are valid TOML floats, so this is reachable by
    /// hand-editing. NaN slips past a naive range check — every
    /// comparison against it is false — and then renders as `NaN.0`, a
    /// syntax error that takes down the entire generated file.
    #[test]
    fn a_non_finite_float_is_refused_rather_than_passing_the_range_check() {
        for bad in [f64::NAN, f64::INFINITY, f64::NEG_INFINITY] {
            let problems = Settings::from_one("demo:ratio", Value::Float(bad)).validate(&CATALOG);
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
        s.set("demo:ratio", Value::Float(f64::NAN));
        s.set("demo:count", Value::Int(3));
        let lua = crate::hlconfig::codegen::generate(&s, &CATALOG, "");
        assert!(!lua.contains("NaN"), "{lua}");
        assert!(!lua.contains("ratio"), "{lua}");
        assert!(lua.contains("count = 3"), "{lua}");
    }

    /// A malformed colour is refused by Hyprland, and a refused value in
    /// a generated file takes every other setting in it down too.
    #[test]
    fn a_malformed_colour_is_refused() {
        for bad in ["rgba(bd93f9)", "rgb(bd93f9ff)", "rgba(zzzzzzzz)"] {
            let problems =
                Settings::from_one("demo:tint", Value::Text(bad.into())).validate(&CATALOG);
            assert_eq!(problems.len(), 1, "{bad} was accepted");
        }
        for good in ["rgba(bd93f9ff)", "rgb(282a36)", "0xffbd93f9"] {
            assert_eq!(
                Settings::from_one("demo:tint", Value::Text(good.into())).validate(&CATALOG),
                vec![],
                "{good} was refused"
            );
        }
    }

    #[test]
    fn clearing_a_key_removes_it_entirely() {
        let mut s = Settings::from_one("demo:count", Value::Int(3));
        s.clear("demo:count");
        assert!(s.is_empty());
    }

    #[test]
    fn defaults_come_back_with_the_catalog_type() {
        for setting in CATALOG.settings {
            let s = Settings::from_one(setting.key, Value::default_for(&setting.kind));
            assert_eq!(
                s.validate(&CATALOG),
                vec![],
                "{} rejected its own default",
                setting.key
            );
        }
    }
}
