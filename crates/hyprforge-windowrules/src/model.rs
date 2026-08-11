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
    /// Matches windows carrying a tag. `term` matches the bare tag and any
    /// dynamic `term*`; `term*` matches only the dynamic form.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tag: Option<String>,
    /// Content type the window declares, e.g. `game`, `video`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub content: Option<String>,
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

/// Which workspace a matching window opens on.
///
/// Hyprland takes this as one string, so the `silent` flag isn't a separate
/// field there — it's a suffix. Modelling it as a bool keeps the GUI a
/// checkbox and keeps the suffix from being something a user has to know to
/// type.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct Workspace {
    /// A workspace ID (`3`), a name (`name:coding`), a special workspace
    /// (`special:scratchpad`), or the literal `unset`.
    pub name: String,
    /// Open the window there without following it. Worth defaulting on for
    /// anything that launches in the background, which is most of what
    /// people write this rule for.
    #[serde(default)]
    pub silent: bool,
}

impl Workspace {
    pub fn is_empty(&self) -> bool {
        self.name.trim().is_empty()
    }

    /// Renders Hyprland's single-string form, e.g. `3 silent`. `unset` is a
    /// keyword rather than a workspace, so it never takes the suffix.
    pub fn to_lua_value(&self) -> String {
        let name = self.name.trim();
        if self.silent && name != "unset" {
            format!("{name} silent")
        } else {
            name.to_string()
        }
    }
}

/// Effects applied by the rule. Additive by design — new fields should be
/// `Option`s with `#[serde(skip_serializing_if = "Option::is_none")]`, never
/// a rewrite of this struct or the codegen match arms below it.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct Effects {
    /// Where the window opens. The rule people reach for first — "Discord on
    /// workspace 3" — and the one this module went longest without.
    #[serde(skip_serializing_if = "Workspace::is_empty", default)]
    pub workspace: Workspace,
    /// Applies a tag: `+name` sets, `-name` unsets, bare toggles.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tag: Option<String>,
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
    #[serde(skip_serializing_if = "Option::is_none")]
    pub opaque: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub no_anim: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub no_focus: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub stay_focused: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub dim_around: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub keep_aspect_ratio: Option<bool>,
    /// Hyprland has no `no_border` effect — this set to 0 is how that's said.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub border_size: Option<i32>,
    /// `[width, height]` in pixels. Unlike `size`, these take no expressions.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub min_size: Option<[i32; 2]>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_size: Option<[i32; 2]>,
    /// An animation style, optionally with a percentage: `popin`, `popin 80%`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub animation: Option<String>,
    /// One of [`IDLE_INHIBIT_MODES`]. Hyprland validates this and rejects
    /// anything else outright — which aborts the whole generated file — so it
    /// must never be free text in the UI.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub idle_inhibit: Option<String>,
}

/// The only values `idle_inhibit` accepts, confirmed against Hyprland 0.56.1
/// — anything else fails with `idle_inhibit rule has unknown mode`.
pub const IDLE_INHIBIT_MODES: [&str; 4] = ["none", "always", "focus", "fullscreen"];

/// Pins a workspace to a monitor, via Hyprland's `hl.workspace_rule`.
///
/// This is how "Steam on the external display" is actually expressed: a
/// window rule puts the window on a workspace, and this puts the workspace on
/// a monitor. Kept separate from [`Rule`] because it's a different Lua call
/// with different semantics — merging the two into one ordered list would
/// make that ordering meaningless, since only window rules override each
/// other.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct WorkspaceRule {
    /// A workspace ID (`3`), a name (`name:coding`), or a special workspace.
    pub workspace: String,
    /// How Hyprland should find the monitor. Prefer `desc:<description>` over
    /// a connector name: connectors are assigned in probe order and can move
    /// between boots or replugs, while the description comes from the EDID
    /// and identifies the physical panel. Must be the description exactly as
    /// `hyprctl monitors` reports it, since that's what Hyprland compares
    /// against.
    pub monitor: String,
    /// Make this the workspace that opens on that monitor by default.
    #[serde(default)]
    pub default: bool,
    /// Keep the workspace alive even while empty.
    #[serde(default)]
    pub persistent: bool,
}

impl WorkspaceRule {
    /// A rule with no workspace names nothing and is skipped rather than
    /// emitted — the same treatment an empty matcher gets.
    pub fn is_empty(&self) -> bool {
        self.workspace.trim().is_empty()
    }
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
