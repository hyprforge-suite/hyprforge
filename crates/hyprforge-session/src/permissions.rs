//! What applications are allowed to do.
//!
//! `hl.permission({ binary, type, mode })`, where `binary` is a **regex**
//! matched against the executable's path — not a plain path. This
//! machine's own config relies on that:
//!
//! ```lua
//! hl.permission({ binary = "/usr/(bin|local/bin)/grim", type = "screencopy", mode = "allow" })
//! ```
//!
//! Treating it as a literal path would quietly stop matching, and the
//! failure mode is a screenshot tool that returns a black image with
//! "permission denied" written on it.
//!
//! These are security rules, so the module is deliberately conservative:
//! a rule is never reordered (order decides which matches first for
//! keyboards especially), and `deny` and `ask` are never turned into
//! `allow` by anything other than the user saying so.

use hyprforge_core::lua::lua_string;
use serde::{Deserialize, Serialize};

/// The permission types Hyprland knows, with their defaults and why
/// someone would restrict them.
pub const TYPES: &[(&str, &str, &str)] = &[
    (
        "screencopy",
        "Screen capture",
        "Reading your screen without going through the desktop portal — grim, wf-recorder. Denied, they get a black image.",
    ),
    (
        "plugin",
        "Loading plugins",
        "Loading code into Hyprland. Leave hyprctl out of this: anything that can run hyprctl could load a plugin of its choosing.",
    ),
    (
        "keyboard",
        "New keyboards",
        "Connecting a keyboard. Order matters here — a deny rule for .* belongs last.",
    ),
    (
        "cursorpos",
        "Cursor position",
        "Reading where your pointer is, directly over Wayland.",
    ),
    (
        "input-capture",
        "Capturing all input",
        "Capturing every key, click and touch from the compositor.",
    ),
];

/// Hyprland's three modes.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Mode {
    Allow,
    /// Prompt each time. Hyprland's default for most types.
    #[default]
    Ask,
    Deny,
}

impl Mode {
    pub const ALL: [Mode; 3] = [Mode::Allow, Mode::Ask, Mode::Deny];

    pub fn as_str(self) -> &'static str {
        match self {
            Mode::Allow => "allow",
            Mode::Ask => "ask",
            Mode::Deny => "deny",
        }
    }

    pub fn parse(s: &str) -> Option<Mode> {
        Mode::ALL.into_iter().find(|m| m.as_str() == s.trim())
    }
}

impl std::fmt::Display for Mode {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Rule {
    /// A **regex** matched against the binary's path.
    pub binary: String,
    pub r#type: String,
    pub mode: Mode,
    #[serde(default = "yes")]
    pub enabled: bool,
}

fn yes() -> bool {
    true
}

/// By hand for `enabled`, as elsewhere. The mode stays `Ask`: defaulting
/// a new security rule to `allow` would quietly widen a permission.
impl Default for Rule {
    fn default() -> Self {
        Rule {
            binary: String::new(),
            r#type: String::new(),
            mode: Mode::Ask,
            enabled: true,
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Settings {
    #[serde(default, rename = "rule")]
    pub rules: Vec<Rule>,
}

impl Settings {
    pub fn is_empty(&self) -> bool {
        self.rules.is_empty()
    }

    pub fn invalid(&self) -> Vec<(usize, String)> {
        let mut out = Vec::new();
        for (i, rule) in self.rules.iter().enumerate() {
            if rule.binary.trim().is_empty() {
                out.push((i, "needs a binary path or pattern".to_string()));
            } else if !TYPES.iter().any(|(t, _, _)| *t == rule.r#type.trim()) {
                out.push((i, format!("\"{}\" isn't a permission type", rule.r#type.trim())));
            } else if let Some(problem) = check_pattern(&rule.binary) {
                out.push((i, problem));
            }
        }
        out
    }
}

/// Catches the regex mistakes that turn a rule into one that matches
/// nothing — which, for a permission, means the app is silently governed
/// by the default instead.
///
/// Deliberately not a full regex parser. It refuses the two shapes that
/// are always wrong and leaves everything else to Hyprland, because
/// wrongly rejecting a valid pattern would be worse than not checking.
pub fn check_pattern(binary: &str) -> Option<String> {
    let pattern = binary.trim();
    let opens = pattern.matches('(').count();
    let closes = pattern.matches(')').count();
    if opens != closes {
        return Some("unbalanced brackets in the pattern".to_string());
    }
    if !pattern.starts_with('/') && !pattern.starts_with('^') && !pattern.starts_with('.') {
        return Some("should be a full path or pattern, starting with /".to_string());
    }
    None
}

/// Renders the `hl.permission` calls.
///
/// Order is preserved exactly. Hyprland takes keyboard rules in order —
/// a catch-all `deny` belongs last — so reordering them, even to sort
/// them tidily, would change what the rules mean.
pub fn generate(settings: &Settings) -> String {
    let bad: Vec<usize> = settings.invalid().into_iter().map(|(i, _)| i).collect();
    let mut out = String::new();
    for (i, rule) in settings.rules.iter().enumerate() {
        if !rule.enabled || bad.contains(&i) {
            continue;
        }
        out.push_str(&format!(
            "hl.permission({{ binary = {}, type = {}, mode = {} }})\n",
            lua_string(rule.binary.trim()),
            lua_string(rule.r#type.trim()),
            lua_string(rule.mode.as_str())
        ));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rule(binary: &str, kind: &str, mode: Mode) -> Rule {
        Rule { binary: binary.into(), r#type: kind.into(), mode, enabled: true }
    }

    /// The exact shape from this machine's own config.
    #[test]
    fn a_rule_renders_the_way_a_hand_written_one_looks() {
        let settings = Settings {
            rules: vec![rule("/usr/(bin|local/bin)/grim", "screencopy", Mode::Allow)],
        };
        assert_eq!(
            generate(&settings),
            "hl.permission({ binary = [[/usr/(bin|local/bin)/grim]], type = [[screencopy]], mode = [[allow]] })\n"
        );
    }

    /// Hyprland takes keyboard rules in order — a catch-all deny belongs
    /// last — so reordering them changes what they mean.
    #[test]
    fn order_is_preserved_exactly() {
        let settings = Settings {
            rules: vec![
                rule("/usr/bin/trusted-kb", "keyboard", Mode::Allow),
                rule(".*", "keyboard", Mode::Deny),
            ],
        };
        let out = generate(&settings);
        assert!(out.find("trusted-kb") < out.find(".*"), "{out}");
    }

    #[test]
    fn every_mode_round_trips_through_its_name() {
        for mode in Mode::ALL {
            assert_eq!(Mode::parse(mode.as_str()), Some(mode));
        }
        assert_eq!(Mode::parse("maybe"), None);
    }

    /// Ask is Hyprland's default for most types, so it's ours too —
    /// defaulting to allow would quietly widen a permission.
    #[test]
    fn the_default_mode_is_ask_not_allow() {
        assert_eq!(Mode::default(), Mode::Ask);
        assert_eq!(Rule::default().mode, Mode::Ask);
    }

    #[test]
    fn an_unknown_type_is_reported_and_skipped() {
        let settings = Settings { rules: vec![rule("/usr/bin/x", "telepathy", Mode::Allow)] };
        assert!(settings.invalid()[0].1.contains("permission type"));
        assert_eq!(generate(&settings), "");
    }

    /// A pattern that matches nothing means the app is silently governed
    /// by the default instead of the rule the user wrote.
    #[test]
    fn a_broken_pattern_is_reported() {
        let settings = Settings { rules: vec![rule("/usr/(bin/grim", "screencopy", Mode::Allow)] };
        assert!(settings.invalid()[0].1.contains("brackets"));

        let settings = Settings { rules: vec![rule("grim", "screencopy", Mode::Allow)] };
        assert!(settings.invalid()[0].1.contains("full path"));
    }

    /// Real patterns from the wiki and this machine must all pass —
    /// wrongly rejecting a valid one is worse than not checking.
    #[test]
    fn real_patterns_are_accepted() {
        for good in [
            "/usr/bin/grim",
            "/usr/(bin|local/bin)/grim",
            "/usr/(lib|libexec|lib64)/xdg-desktop-portal-hyprland",
            "/nix/store/[a-z0-9]{32}-grim-[0-9.]*/bin/grim",
            ".*",
            "^/usr/bin/.*",
        ] {
            assert_eq!(check_pattern(good), None, "{good} was refused");
        }
    }

    #[test]
    fn a_disabled_rule_is_kept_but_not_written() {
        let mut r = rule("/usr/bin/grim", "screencopy", Mode::Allow);
        r.enabled = false;
        assert_eq!(generate(&Settings { rules: vec![r] }), "");
    }

    #[test]
    fn every_type_has_a_label_and_an_explanation() {
        for (name, label, why) in TYPES {
            assert!(!name.is_empty() && !label.is_empty() && !why.is_empty());
        }
    }

    #[test]
    fn a_new_rule_is_enabled_and_asks() {
        assert!(Rule::default().enabled);
        assert_eq!(Rule::default().mode, Mode::Ask);
    }

    #[test]
    fn an_empty_set_generates_nothing() {
        assert_eq!(generate(&Settings::default()), "");
    }
}
