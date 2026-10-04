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

    // --- Static effects: applied as the window opens, rather than being
    // toggled on a window already up.
    /// Open tiled. The counterpart to `float`; setting both is contradictory
    /// and Hyprland resolves it, so the UI offers them as separate opt-ins
    /// rather than pretending to know which wins.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tile: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub fullscreen: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub maximize: Option<bool>,
    /// Floating windows only — Hyprland ignores it otherwise.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub pin: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub center: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub no_initial_focus: Option<bool>,
    /// Which monitor the window opens on. Same `desc:<description>` form as
    /// [`WorkspaceRule::monitor`], and preferred for the same reason: a
    /// connector name can move between boots.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub monitor: Option<String>,
    /// Events from the window to ignore, e.g. `maximize`, `fullscreen`,
    /// `activate`, `activatefocus`.
    ///
    /// Hyprland accepts *any* string here without complaint, so an
    /// unrecognised value is not an error — it simply does nothing. That
    /// makes a typo invisible, which is why the UI shows the known values.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub suppress_event: Option<String>,
    /// Grouping behaviour, e.g. `new`, `lock`, `deny`, `barred`. Unvalidated
    /// in the same way as `suppress_event`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub group: Option<String>,
    /// Refuse close requests for this long after the window opens. Hyprland
    /// takes a bare integer; the units are its own.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub no_close_for: Option<i32>,
}

/// Values `suppress_event` is known to act on. Hyprland does *not* reject
/// anything else, so this list is a convenience rather than a constraint —
/// treat it as "the ones that do something", not "the ones that parse".
pub const SUPPRESS_EVENTS: [&str; 5] = [
    "fullscreen",
    "maximize",
    "maximizefullscreen",
    "activate",
    "activatefocus",
];

/// Values `group` is known to act on. Unvalidated by Hyprland, as above.
pub const GROUP_MODES: [&str; 4] = ["new", "lock", "deny", "barred"];

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

/// A rule for a layer-shell surface — a bar, a launcher, a notification
/// popup — via Hyprland's `hl.layer_rule`.
///
/// Deliberately minimal: a namespace and blur, because that is the one
/// thing anything in the suite needs (blur behind notif's toasts and its
/// centre panel) and the one shape checked against a live Hyprland 0.56.
/// `hl.layer_rule({ name = …, match = { namespace = "^notif$" }, blur =
/// true })` is accepted there, and a field it does not know is *rejected*
/// — which aborts the whole generated file, window rules included. So a
/// field is added here only after it has been loaded into a real
/// compositor, never from memory of hyprlang's `layerrule = …` names,
/// which are not the Lua ones.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct LayerRule {
    /// Stable identity, the Lua rule's `name`, as with [`Rule::name`]:
    /// how Setup finds the rule it added again to take it back out.
    pub name: String,
    /// Off means not emitted. Whether `hl.layer_rule` takes an `enabled`
    /// field the way `hl.window_rule` does has not been checked, and an
    /// unknown field fails the whole file, so "off" is spelled "absent".
    #[serde(default = "default_true")]
    pub enabled: bool,
    /// The layer surface's namespace, as a regex — `^notif$`, anchored,
    /// because an unanchored `notif` would also match `notif-center` and
    /// anything else that happens to contain it.
    pub namespace: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub blur: Option<bool>,
}

impl LayerRule {
    /// A rule that matches nothing or does nothing is skipped rather than
    /// emitted — the same treatment an empty window-rule matcher gets.
    pub fn is_empty(&self) -> bool {
        self.namespace.trim().is_empty() || self.blur.is_none()
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
