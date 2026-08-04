use serde::{Deserialize, Serialize};

/// Match criteria for `hl.window_rule`'s `match` table. All fields are
/// optional; Hyprland requires at least one to be set, which callers should
/// validate before generating (see [`crate::codegen::generate`]).
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct Matcher {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub class: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub initial_class: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub initial_title: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub fullscreen: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub floating: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub xwayland: Option<bool>,
}

impl Matcher {
    pub fn is_empty(&self) -> bool {
        self == &Matcher::default()
    }
}

/// Per-state opacity, mapping to Hyprland's
/// `"active inactive fullscreen"` string form, with an optional `override`
/// suffix per value (absolute instead of multiplicative).
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct Opacity {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub active: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub inactive: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub fullscreen: Option<f32>,
    /// When true, every set value is emitted with the ` override` suffix.
    #[serde(default)]
    pub is_override: bool,
}

impl Opacity {
    pub fn is_empty(&self) -> bool {
        self.active.is_none() && self.inactive.is_none() && self.fullscreen.is_none()
    }

    /// Renders to Hyprland's Lua opacity string, e.g. `"0.9 0.7 1.0 override"`.
    /// Missing leading values are not representable positionally in
    /// Hyprland's format, so unset values default to `1.0`.
    pub fn to_lua_value(&self) -> String {
        let suffix = if self.is_override { " override" } else { "" };
        let mut parts = Vec::new();
        if self.active.is_some() || self.inactive.is_some() || self.fullscreen.is_some() {
            parts.push(format!("{}{}", self.active.unwrap_or(1.0), suffix));
        }
        if self.inactive.is_some() || self.fullscreen.is_some() {
            parts.push(format!("{}{}", self.inactive.unwrap_or(1.0), suffix));
        }
        if self.fullscreen.is_some() {
            parts.push(format!("{}{}", self.fullscreen.unwrap_or(1.0), suffix));
        }
        parts.join(" ")
    }
}

/// Effects applied by the rule. Additive by design — new fields should be
/// `Option`s with `#[serde(skip_serializing_if = "Option::is_none")]`, never
/// a rewrite of this struct or the codegen match arms below it.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct Effects {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub float: Option<bool>,
    /// `[x, y]` — literal pixels or Hyprland move-expression strings.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub r#move: Option<[String; 2]>,
    /// `[width, height]` — literal pixels or Hyprland size-expression strings.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub size: Option<[String; 2]>,
    #[serde(skip_serializing_if = "Opacity::is_empty", default)]
    pub opacity: Opacity,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub border_color: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub no_blur: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub rounding: Option<i32>,
}

/// One ordered rule. Order is semantically meaningful in Hyprland (later
/// rules override earlier ones for the same window) and is preserved by
/// TOML array order and Lua emission order alike — never resorted.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Rule {
    /// Stable, auto-generated identity used as the Lua rule's `name` field.
    /// This is how the app finds "the rule it made for Discord" again on a
    /// later edit — never by matching table contents.
    pub name: String,
    #[serde(default = "default_true")]
    pub enabled: bool,
    pub matcher: Matcher,
    #[serde(default)]
    pub effects: Effects,
}

fn default_true() -> bool {
    true
}

/// Slugifies `label` (lowercased, non-alphanumerics collapsed to `-`) and
/// appends a numeric suffix to keep the result unique against `existing`
/// names, producing a name like `hyprforge-discord-1`.
pub fn generate_rule_name(label: &str, existing: &[String]) -> String {
    let slug: String = label
        .to_lowercase()
        .chars()
        .map(|c| if c.is_alphanumeric() { c } else { '-' })
        .collect::<String>()
        .split('-')
        .filter(|s| !s.is_empty())
        .collect::<Vec<_>>()
        .join("-");
    let slug = if slug.is_empty() {
        "rule".to_string()
    } else {
        slug
    };

    let mut n = 1u32;
    loop {
        let candidate = format!("hyprforge-{slug}-{n}");
        if !existing.iter().any(|e| e == &candidate) {
            return candidate;
        }
        n += 1;
    }
}
