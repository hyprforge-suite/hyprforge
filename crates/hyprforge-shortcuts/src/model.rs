use serde::{Deserialize, Serialize};

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

/// What a shortcut does.
///
/// Hyprland's Lua dispatchers live under `hl.dsp.*` and take a table, so this
/// stores the call rather than trying to model every dispatcher's arguments —
/// there are dozens, they take different shapes, and a wrong guess produces a
/// bind that silently does nothing.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct Action {
    /// A dispatcher path relative to `hl.dsp`, e.g. `window.close` or
    /// `exec_cmd`.
    pub dispatcher: String,
    /// The Lua argument expression, emitted verbatim inside the call's
    /// parentheses. Empty means no arguments.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub argument: String,
}

impl Action {
    pub fn is_empty(&self) -> bool {
        self.dispatcher.trim().is_empty()
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

    #[test]
    fn generated_names_avoid_collisions() {
        let existing = vec!["hyprforge-launch-terminal-1".to_string()];
        assert_eq!(
            generate_shortcut_name("Launch terminal", &existing),
            "hyprforge-launch-terminal-2"
        );
    }
}
