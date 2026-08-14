use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

/// A modifier, as Hyprland names it in a bind string and as it appears in the
/// bitmask `hyprctl binds` reports.
///
/// The bit values are X11's, which is what Hyprland reports — `SUPER` is 64,
/// not "the sixth thing in this list". They're written out rather than
/// derived from ordering so that inserting a variant here can't silently
/// renumber the others and turn every stored shortcut into a different one.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "UPPERCASE")]
pub enum Modifier {
    Shift,
    Caps,
    Ctrl,
    Alt,
    Mod2,
    Mod3,
    Super,
    Mod5,
}

impl Modifier {
    pub const ALL: [Modifier; 8] = [
        Modifier::Shift,
        Modifier::Caps,
        Modifier::Ctrl,
        Modifier::Alt,
        Modifier::Mod2,
        Modifier::Mod3,
        Modifier::Super,
        Modifier::Mod5,
    ];

    /// The bit this modifier occupies in Hyprland's `modmask`.
    pub fn bit(self) -> u32 {
        match self {
            Modifier::Shift => 1,
            Modifier::Caps => 2,
            Modifier::Ctrl => 4,
            Modifier::Alt => 8,
            Modifier::Mod2 => 16,
            Modifier::Mod3 => 32,
            Modifier::Super => 64,
            Modifier::Mod5 => 128,
        }
    }

    /// What Hyprland calls it inside a bind string.
    pub fn keyword(self) -> &'static str {
        match self {
            Modifier::Shift => "SHIFT",
            Modifier::Caps => "CAPS",
            Modifier::Ctrl => "CTRL",
            Modifier::Alt => "ALT",
            Modifier::Mod2 => "MOD2",
            Modifier::Mod3 => "MOD3",
            Modifier::Super => "SUPER",
            Modifier::Mod5 => "MOD5",
        }
    }

    /// The modifiers a `modmask` has set. Bits Hyprland reports that don't
    /// correspond to a known modifier are ignored rather than guessed at.
    pub fn from_mask(mask: u32) -> Vec<Modifier> {
        Modifier::ALL.into_iter().filter(|m| mask & m.bit() != 0).collect()
    }

    pub fn mask_of(mods: &[Modifier]) -> u32 {
        mods.iter().fold(0, |acc, m| acc | m.bit())
    }
}

/// A key combination: some modifiers plus one key.
///
/// Comparison is by *mask*, not by the modifier list, so `SUPER+SHIFT` and
/// `SHIFT+SUPER` are the same shortcut — which is what a user means, and what
/// Hyprland enforces anyway.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct KeyCombo {
    #[serde(default)]
    pub mods: Vec<Modifier>,
    /// The key name, e.g. `Q`, `Return`, `XF86AudioRaiseVolume`. Hyprland
    /// matches these case-insensitively for letters but not for the named
    /// keys, so it's stored exactly as given.
    pub key: String,
}

impl KeyCombo {
    pub fn mask(&self) -> u32 {
        Modifier::mask_of(&self.mods)
    }

    pub fn is_empty(&self) -> bool {
        self.key.trim().is_empty()
    }

    /// Hyprland's bind-string form, e.g. `SUPER + SHIFT + Q`. Modifiers are
    /// emitted in bit order so the same combination always renders the same
    /// way regardless of the order they were added in.
    pub fn to_bind_string(&self) -> String {
        let mut parts: Vec<&str> = Modifier::ALL
            .into_iter()
            .filter(|m| self.mask() & m.bit() != 0)
            .map(|m| m.keyword())
            .collect();
        let key = self.key.trim();
        parts.push(key);
        parts.join(" + ")
    }

    /// True when both would be the same physical shortcut. Keys compare
    /// case-insensitively because Hyprland treats `q` and `Q` alike, and a
    /// user who types one shouldn't be told the other is free.
    pub fn conflicts_with(&self, other: &KeyCombo) -> bool {
        self.mask() == other.mask() && self.key.trim().eq_ignore_ascii_case(other.key.trim())
    }
}

impl PartialEq for KeyCombo {
    fn eq(&self, other: &Self) -> bool {
        self.conflicts_with(other)
    }
}

impl Eq for KeyCombo {}

/// One value inside a dispatcher's argument table.
///
/// Only the Lua types Hyprland's dispatchers actually accept. Untagged so
/// TOML stores them as what they are — `direction = "left"`, `workspace = 3`,
/// `follow = true` — rather than as tagged wrappers.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(untagged)]
pub enum ParamValue {
    Bool(bool),
    Int(i64),
    Float(f64),
    Str(String),
}

impl ParamValue {
    /// The value as a user would type it into a text field. The inverse of
    /// the editor's parse step, so a round-trip through the form doesn't
    /// change a stored value's type.
    pub fn as_text(&self) -> String {
        match self {
            ParamValue::Bool(b) => b.to_string(),
            ParamValue::Int(i) => i.to_string(),
            ParamValue::Float(f) => f.to_string(),
            ParamValue::Str(s) => s.clone(),
        }
    }
}

/// What a shortcut does: a dispatcher path under `hl.dsp` plus the arguments
/// it's called with.
///
/// `params` is the normal case — a structured table the
/// [`catalog`](crate::catalog) knows the shape of, which is what lets the
/// editor render real fields and lets codegen emit a call that Hyprland can
/// always parse.
///
/// `raw` is the escape hatch, and it's load-bearing rather than vestigial:
/// Hyprland has dispatchers and argument shapes this crate's catalog doesn't
/// model, and a config full of them still has to import, edit and re-emit
/// without losing anything. When `raw` is set it wins, and `params` is
/// ignored.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct Action {
    /// A dispatcher path relative to `hl.dsp`, e.g. `window.close` or
    /// `exec_cmd`.
    pub dispatcher: String,
    /// The dispatcher's argument table, by key.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub params: BTreeMap<String, ParamValue>,
    /// A Lua argument expression emitted verbatim inside the call's
    /// parentheses.
    ///
    /// The `argument` alias is what an older `shortcuts.toml` used for this
    /// same field back when it was the *only* way to store an argument, so
    /// a store written before the catalog existed still loads — and still
    /// generates byte-identical Lua.
    #[serde(default, alias = "argument", skip_serializing_if = "Option::is_none")]
    pub raw: Option<String>,
}

impl Action {
    pub fn is_empty(&self) -> bool {
        self.dispatcher.trim().is_empty()
    }

    /// A dispatcher called with a single structured table.
    pub fn with_params(
        dispatcher: impl Into<String>,
        params: BTreeMap<String, ParamValue>,
    ) -> Self {
        Action { dispatcher: dispatcher.into(), params, raw: None }
    }

    /// A dispatcher whose argument this crate isn't modelling — see `raw`.
    pub fn with_raw(dispatcher: impl Into<String>, raw: impl Into<String>) -> Self {
        let raw = raw.into();
        Action {
            dispatcher: dispatcher.into(),
            params: BTreeMap::new(),
            // An empty `raw` and no `raw` mean the same thing (a call with
            // no arguments), so they're stored the same way rather than
            // leaving `Some("")` around to be special-cased downstream.
            raw: (!raw.trim().is_empty()).then_some(raw),
        }
    }
}

/// The flags in `hl.bind`'s third argument, beside the description.
///
/// All default-false, and skipped entirely when none are set, so a shortcut
/// that uses none of them stores and reads exactly as it did before these
/// existed.
///
/// Spelling matters here: Hyprland's repeat flag is `repeating`, not
/// `repeat` — verified against the wiki's Binds page for 0.56.
#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(default)]
pub struct BindFlags {
    /// Required for `mouse:272`-style binds to work at all.
    pub mouse: bool,
    /// Fires even while an input inhibitor (e.g. a lockscreen) is active.
    pub locked: bool,
    /// Fires repeatedly while the key is held.
    pub repeating: bool,
    /// Fires on key release instead of press.
    pub release: bool,
    /// Fires only on a long press.
    pub long_press: bool,
    /// The key event still reaches the focused window.
    pub non_consuming: bool,
}

impl BindFlags {
    pub fn is_default(&self) -> bool {
        *self == BindFlags::default()
    }

    /// Why this combination of flags won't load, if it won't.
    ///
    /// Hyprland rejects `repeating` together with `release` or `long_press`
    /// — "long_press / release is incompatible with repeat" — and a rejected
    /// bind fails the whole generated file, not just itself. Verified
    /// against 0.56.1 in `tests/live_lua.rs`, which also pins that the
    /// compositor still refuses it, so this guard can't outlive the rule.
    pub fn conflict(&self) -> Option<&'static str> {
        (self.repeating && (self.release || self.long_press))
            .then_some("Repeating can't be combined with release or long press.")
    }

    /// `(lua key, value)` for each flag that's on, in a fixed order so the
    /// generated file doesn't churn between saves.
    pub fn set_flags(&self) -> Vec<&'static str> {
        [
            ("mouse", self.mouse),
            ("locked", self.locked),
            ("repeating", self.repeating),
            ("release", self.release),
            ("long_press", self.long_press),
            ("non_consuming", self.non_consuming),
        ]
        .into_iter()
        .filter_map(|(name, on)| on.then_some(name))
        .collect()
    }
}

/// One shortcut Hyprforge owns.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct Shortcut {
    /// Stable identity, as with window rules — how the app finds the shortcut
    /// it made on a later edit, never by matching the key combination.
    pub name: String,
    #[serde(default = "default_true")]
    pub enabled: bool,
    pub combo: KeyCombo,
    pub action: Action,
    /// Shown in `hyprctl binds` and in any Hyprland keybind overlay. Always
    /// set, and always prefixed — see [`description_for`].
    #[serde(default)]
    pub description: String,
    #[serde(default, skip_serializing_if = "BindFlags::is_default")]
    pub flags: BindFlags,
}

fn default_true() -> bool {
    true
}

/// The prefix every Hyprforge description carries.
///
/// Hyprland reports a bind's description in `hyprctl binds` but not which
/// file defined it, so this is what lets the app recognise its own binds at
/// runtime — no config parsing, and no separate bookkeeping to fall out of
/// sync with reality. It also means a shortcut is self-describing to anything
/// else that reads the compositor's binds.
pub const DESCRIPTION_PREFIX: &str = "hyprforge: ";

/// The description to emit for a shortcut, prefixed so it's identifiable.
pub fn description_for(shortcut: &Shortcut) -> String {
    let text = shortcut.description.trim();
    if text.is_empty() {
        format!("{DESCRIPTION_PREFIX}{}", shortcut.name)
    } else {
        format!("{DESCRIPTION_PREFIX}{text}")
    }
}

/// Slugifies `label` and appends a numeric suffix to keep it unique — the
/// same identity scheme window rules use.
pub fn generate_shortcut_name(label: &str, existing: &[String]) -> String {
    let slug: String = label
        .to_lowercase()
        .chars()
        .map(|c| if c.is_alphanumeric() { c } else { '-' })
        .collect::<String>()
        .split('-')
        .filter(|s| !s.is_empty())
        .collect::<Vec<_>>()
        .join("-");
    let slug = if slug.is_empty() { "shortcut".to_string() } else { slug };

    let mut n = 1u32;
    loop {
        let candidate = format!("hyprforge-{slug}-{n}");
        if !existing.iter().any(|e| e == &candidate) {
            return candidate;
        }
        n += 1;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The masks this machine's own binds report: 64 is SUPER, 65 is
    /// SUPER+SHIFT, 12 is CTRL+ALT. Getting these wrong would make every
    /// conflict check meaningless.
    #[test]
    fn masks_match_what_hyprctl_reports() {
        assert_eq!(Modifier::from_mask(64), vec![Modifier::Super]);
        assert_eq!(Modifier::from_mask(65), vec![Modifier::Shift, Modifier::Super]);
        assert_eq!(Modifier::from_mask(12), vec![Modifier::Ctrl, Modifier::Alt]);
        assert_eq!(Modifier::from_mask(0), vec![]);
    }

    #[test]
    fn a_mask_round_trips_through_the_modifier_list() {
        for mask in [0u32, 1, 4, 12, 64, 65, 77] {
            assert_eq!(Modifier::mask_of(&Modifier::from_mask(mask)), mask);
        }
    }

    /// An unknown bit must not invent a modifier.
    #[test]
    fn unknown_bits_are_ignored() {
        assert_eq!(Modifier::from_mask(256), vec![]);
        assert_eq!(Modifier::from_mask(256 | 64), vec![Modifier::Super]);
    }

    #[test]
    fn renders_hyprlands_bind_string() {
        let combo = KeyCombo {
            mods: vec![Modifier::Super, Modifier::Shift],
            key: "Q".to_string(),
        };
        assert_eq!(combo.to_bind_string(), "SHIFT + SUPER + Q");
    }

    /// Order shouldn't change identity — the user means the same chord.
    #[test]
    fn modifier_order_does_not_change_the_shortcut() {
        let a = KeyCombo { mods: vec![Modifier::Super, Modifier::Shift], key: "Q".into() };
        let b = KeyCombo { mods: vec![Modifier::Shift, Modifier::Super], key: "Q".into() };
        assert_eq!(a, b);
        assert_eq!(a.to_bind_string(), b.to_bind_string());
    }

    /// Hyprland treats `q` and `Q` alike, so claiming one is free while the
    /// other is taken would be a lie.
    #[test]
    fn key_case_does_not_change_the_shortcut() {
        let a = KeyCombo { mods: vec![Modifier::Super], key: "q".into() };
        let b = KeyCombo { mods: vec![Modifier::Super], key: "Q".into() };
        assert!(a.conflicts_with(&b));
    }

    #[test]
    fn different_modifiers_do_not_conflict() {
        let a = KeyCombo { mods: vec![Modifier::Super], key: "Q".into() };
        let b = KeyCombo { mods: vec![Modifier::Super, Modifier::Shift], key: "Q".into() };
        assert!(!a.conflicts_with(&b));
    }

    #[test]
    fn a_combo_with_no_modifiers_is_still_a_shortcut() {
        let combo = KeyCombo { mods: vec![], key: "XF86AudioPlay".into() };
        assert_eq!(combo.to_bind_string(), "XF86AudioPlay");
        assert!(!combo.is_empty());
    }

    /// The prefix is how the app finds its own binds in `hyprctl binds`.
    #[test]
    fn descriptions_are_always_prefixed() {
        let mut s = Shortcut {
            name: "hyprforge-launch-1".into(),
            description: "Launch terminal".into(),
            ..Default::default()
        };
        assert_eq!(description_for(&s), "hyprforge: Launch terminal");
        s.description = "   ".into();
        assert_eq!(description_for(&s), "hyprforge: hyprforge-launch-1");
    }

    /// The compatibility promise for a `shortcuts.toml` written before the
    /// catalog existed: its `argument` string still loads, as `raw`.
    #[test]
    fn a_legacy_argument_field_loads_as_raw() {
        let toml = r#"
            name = "hyprforge-run-1"
            enabled = true
            description = "Run something"

            [combo]
            mods = ["SUPER"]
            key = "R"

            [action]
            dispatcher = "exec_cmd"
            argument = "[[ghostty]]"
        "#;
        let shortcut: Shortcut = toml::from_str(toml).unwrap();
        assert_eq!(shortcut.action.raw.as_deref(), Some("[[ghostty]]"));
        assert!(shortcut.action.params.is_empty());
        assert!(shortcut.flags.is_default());
    }

    /// An empty argument and no argument both mean "called with nothing", so
    /// they must not be two different stored states.
    #[test]
    fn an_empty_raw_argument_is_stored_as_none() {
        assert_eq!(Action::with_raw("exit", "   ").raw, None);
    }

    /// A rejected bind fails the whole generated file, so this combination
    /// has to be caught in the editor rather than discovered on reload.
    #[test]
    fn repeating_conflicts_with_release_and_long_press() {
        let repeating = BindFlags { repeating: true, ..BindFlags::default() };
        assert!(repeating.conflict().is_none());
        assert!(BindFlags { release: true, ..repeating }.conflict().is_some());
        assert!(BindFlags { long_press: true, ..repeating }.conflict().is_some());
        // Without `repeating` they're fine together.
        assert!(BindFlags { release: true, long_press: true, ..BindFlags::default() }
            .conflict()
            .is_none());
    }

    #[test]
    fn flags_render_in_a_fixed_order() {
        let flags = BindFlags { locked: true, mouse: true, ..BindFlags::default() };
        assert_eq!(flags.set_flags(), vec!["mouse", "locked"]);
        assert!(!flags.is_default());
        assert!(BindFlags::default().set_flags().is_empty());
    }

    #[test]
    fn generated_names_avoid_collisions() {
        let existing = vec!["hyprforge-launch-terminal-1".to_string()];
        assert_eq!(
            generate_shortcut_name("Launch terminal", &existing),
            "hyprforge-launch-terminal-2"
        );
    }
}
