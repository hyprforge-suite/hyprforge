//! Touchpad gestures.
//!
//! `hl.gesture({ fingers, direction, action, … })`.
//!
//! **Gestures do not behave like the rest of Hyprland's config, and the
//! difference decides the design.** A repeated `hl.bind` or `hl.config`
//! key takes the last one; a repeated gesture is *refused*:
//!
//! ```text
//! hl.gesture: Gesture will be overshadowed by a previous gesture.
//!             Previous UP shadows new UP
//! ```
//!
//! So the overlay model everything else uses — write ours last and win —
//! does not work here. A gesture that duplicates one the user already has
//! is rejected by Hyprland, and because a rejected call aborts the chunk
//! it would take every other line in the generated file with it. Conflicts
//! are therefore detected *before* writing, against the gestures found in
//! the user's own config, and reported rather than emitted.
//!
//! Every direction and action below was checked against Hyprland 0.56.1,
//! reloading between each so an earlier registration couldn't shadow the
//! next and make a valid name look invalid.

use hyprforge_core::lua::lua_string;
use serde::{Deserialize, Serialize};

/// Swipe and pinch directions, verified live.
pub const DIRECTIONS: &[(&str, &str)] = &[
    ("swipe", "Any swipe"),
    ("horizontal", "Swipe left or right"),
    ("vertical", "Swipe up or down"),
    ("left", "Swipe left"),
    ("right", "Swipe right"),
    ("up", "Swipe up"),
    ("down", "Swipe down"),
    ("pinch", "Any pinch"),
    ("pinchin", "Pinch in"),
    ("pinchout", "Pinch out"),
];

/// What a gesture does, and the extra field it takes, verified live.
pub const ACTIONS: &[(&str, &str, Option<&str>)] = &[
    ("workspace", "Switch workspaces", None),
    ("move", "Move the active window", None),
    ("resize", "Resize the active window", None),
    ("close", "Close the active window", None),
    ("fullscreen", "Fullscreen the active window", Some("mode")),
    ("float", "Float the active window", Some("mode")),
    ("special", "Toggle a special workspace", Some("workspace_name")),
    ("cursor_zoom", "Zoom into the cursor", Some("zoom_level")),
    ("scroll_move", "Scroll the tape, in the scrolling layout", None),
];

pub fn action_argument(action: &str) -> Option<&'static str> {
    ACTIONS
        .iter()
        .find(|(name, _, _)| *name == action.trim())
        .and_then(|(_, _, arg)| *arg)
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Gesture {
    /// How many fingers. Hyprland's own examples use 3 and 4.
    pub fingers: u32,
    pub direction: String,
    pub action: String,
    /// Modifier keys held during the gesture, e.g. `SUPER`. Empty for
    /// none.
    #[serde(default)]
    pub mods: String,
    /// The action's extra argument, when it takes one — a workspace name,
    /// a zoom level, a mode.
    #[serde(default)]
    pub argument: String,
    #[serde(default)]
    pub scale: Option<f64>,
    #[serde(default = "yes")]
    pub enabled: bool,
}

fn yes() -> bool {
    true
}

impl Default for Gesture {
    fn default() -> Self {
        Gesture {
            fingers: 3,
            direction: "horizontal".to_string(),
            action: "workspace".to_string(),
            mods: String::new(),
            argument: String::new(),
            scale: None,
            enabled: true,
        }
    }
}

impl Gesture {
    /// What Hyprland compares when deciding one gesture shadows another:
    /// the finger count, the direction and the modifiers.
    pub fn shadow_key(&self) -> (u32, String, String) {
        (
            self.fingers,
            self.direction.trim().to_lowercase(),
            self.mods.trim().to_uppercase(),
        )
    }
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Settings {
    #[serde(default, rename = "gesture")]
    pub gestures: Vec<Gesture>,
}

impl Settings {
    pub fn is_empty(&self) -> bool {
        self.gestures.is_empty()
    }

    /// Gestures that can't be written, with the reason.
    ///
    /// `existing` is the set already declared in the user's own config.
    /// Hyprland refuses a gesture shadowed by an earlier one, and a
    /// refused call aborts the whole generated file — so a conflict has
    /// to be caught here rather than discovered at reload.
    pub fn invalid(&self, existing: &[Gesture]) -> Vec<(usize, String)> {
        let mut out = Vec::new();
        for (i, gesture) in self.gestures.iter().enumerate() {
            if gesture.fingers < 2 {
                out.push((i, "a gesture needs at least 2 fingers".to_string()));
            } else if !DIRECTIONS.iter().any(|(d, _)| *d == gesture.direction.trim()) {
                out.push((i, format!("\"{}\" isn't a direction", gesture.direction.trim())));
            } else if !ACTIONS.iter().any(|(a, _, _)| *a == gesture.action.trim()) {
                out.push((i, format!("\"{}\" isn't an action", gesture.action.trim())));
            }
        }

        let mut claimed: Vec<(u32, String, String)> =
            existing.iter().map(Gesture::shadow_key).collect();
        for (i, gesture) in self.gestures.iter().enumerate() {
            if !gesture.enabled || out.iter().any(|(bad, _)| *bad == i) {
                continue;
            }
            let key = gesture.shadow_key();
            if claimed.contains(&key) {
                out.push((
                    i,
                    format!(
                        "{} fingers {} is already taken — Hyprland refuses a second one, \
                         so remove the other before adding this",
                        gesture.fingers,
                        gesture.direction.trim()
                    ),
                ));
            }
            claimed.push(key);
        }
        out
    }
}

/// Renders the `hl.gesture` calls.
pub fn generate(settings: &Settings, existing: &[Gesture]) -> String {
    let bad: Vec<usize> = settings.invalid(existing).into_iter().map(|(i, _)| i).collect();
    let mut out = String::new();
    for (i, gesture) in settings.gestures.iter().enumerate() {
        if !gesture.enabled || bad.contains(&i) {
            continue;
        }
        out.push_str(&render_one(gesture));
    }
    out
}

/// One `hl.gesture` line, exactly as [`generate`] writes it.
pub fn render_one(gesture: &Gesture) -> String {
    let mut fields = vec![
        format!("fingers = {}", gesture.fingers),
        format!("direction = {}", lua_string(gesture.direction.trim())),
        format!("action = {}", lua_string(gesture.action.trim())),
    ];
    if !gesture.mods.trim().is_empty() {
        fields.push(format!("mods = {}", lua_string(gesture.mods.trim())));
    }
    if let Some(scale) = gesture.scale {
        fields.push(format!("scale = {scale}"));
    }
    // Only the argument this action actually takes: `workspace_name` on a
    // `close` gesture is a key Hyprland doesn't know for that action.
    if let Some(name) = action_argument(&gesture.action) {
        let value = gesture.argument.trim();
        if !value.is_empty() {
            // Numeric arguments go unquoted; a quoted zoom level is a
            // string where Hyprland wants a number.
            if value.parse::<f64>().is_ok() {
                fields.push(format!("{name} = {value}"));
            } else {
                fields.push(format!("{name} = {}", lua_string(value)));
            }
        }
    }
    format!("hl.gesture({{ {} }})\n", fields.join(", "))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn gesture(fingers: u32, direction: &str, action: &str) -> Gesture {
        Gesture {
            fingers,
            direction: direction.into(),
            action: action.into(),
            ..Gesture::default()
        }
    }

    /// The exact shape from this machine's own config.
    #[test]
    fn a_gesture_renders_the_way_a_hand_written_one_looks() {
        let out = render_one(&gesture(3, "horizontal", "workspace"));
        assert_eq!(
            out,
            "hl.gesture({ fingers = 3, direction = [[horizontal]], action = [[workspace]] })\n"
        );
    }

    #[test]
    fn mods_and_scale_are_written_only_when_set() {
        let mut g = gesture(3, "up", "fullscreen");
        assert!(!render_one(&g).contains("mods"));
        assert!(!render_one(&g).contains("scale"));
        g.mods = "SUPER".into();
        g.scale = Some(1.5);
        let out = render_one(&g);
        assert!(out.contains("mods = [[SUPER]]"), "{out}");
        assert!(out.contains("scale = 1.5"), "{out}");
    }

    /// A quoted zoom level is a string where Hyprland wants a number.
    #[test]
    fn a_numeric_argument_is_written_unquoted() {
        let mut g = gesture(2, "pinch", "cursor_zoom");
        g.argument = "2".into();
        assert!(render_one(&g).contains("zoom_level = 2"), "{}", render_one(&g));

        let mut g = gesture(3, "up", "special");
        g.argument = "magic".into();
        assert!(render_one(&g).contains("workspace_name = [[magic]]"));
    }

    /// `workspace_name` on a `close` gesture is a key Hyprland doesn't
    /// know for that action.
    #[test]
    fn an_argument_is_only_written_for_an_action_that_takes_one() {
        let mut g = gesture(3, "down", "close");
        g.argument = "stray".into();
        let out = render_one(&g);
        assert!(!out.contains("stray"), "{out}");
        assert_eq!(action_argument("close"), None);
        assert_eq!(action_argument("special"), Some("workspace_name"));
    }

    /// The rule that makes gestures different: Hyprland *refuses* a
    /// duplicate rather than overriding it, and a refused call aborts the
    /// whole generated file.
    #[test]
    fn a_gesture_shadowed_by_the_users_own_is_reported_not_written() {
        let existing = vec![gesture(3, "horizontal", "workspace")];
        let settings = Settings { gestures: vec![gesture(3, "horizontal", "move")] };
        let problems = settings.invalid(&existing);
        assert_eq!(problems.len(), 1);
        assert!(problems[0].1.contains("already taken"), "{}", problems[0].1);
        assert_eq!(generate(&settings, &existing), "", "it must not be written");
    }

    /// Two of ours shadowing each other is the same failure.
    #[test]
    fn two_of_our_own_gestures_cannot_shadow_each_other() {
        let settings = Settings {
            gestures: vec![gesture(3, "up", "close"), gesture(3, "up", "move")],
        };
        assert_eq!(settings.invalid(&[]).len(), 1);
        assert_eq!(generate(&settings, &[]).matches("hl.gesture").count(), 1);
    }

    /// Different modifiers make a different gesture, so they don't clash.
    #[test]
    fn modifiers_distinguish_otherwise_identical_gestures() {
        let mut with_mod = gesture(3, "up", "move");
        with_mod.mods = "SUPER".into();
        let settings = Settings { gestures: vec![gesture(3, "up", "close"), with_mod] };
        assert_eq!(settings.invalid(&[]), vec![]);
        assert_eq!(generate(&settings, &[]).matches("hl.gesture").count(), 2);
    }

    #[test]
    fn finger_count_distinguishes_gestures() {
        let settings = Settings {
            gestures: vec![gesture(3, "up", "close"), gesture(4, "up", "move")],
        };
        assert_eq!(settings.invalid(&[]), vec![]);
    }

    #[test]
    fn an_unknown_direction_or_action_is_reported() {
        let settings = Settings { gestures: vec![gesture(3, "sideways", "workspace")] };
        assert!(settings.invalid(&[])[0].1.contains("direction"));

        let settings = Settings { gestures: vec![gesture(3, "up", "explode")] };
        assert!(settings.invalid(&[])[0].1.contains("action"));
    }

    #[test]
    fn a_one_finger_gesture_is_refused() {
        let settings = Settings { gestures: vec![gesture(1, "up", "close")] };
        assert!(settings.invalid(&[])[0].1.contains("2 fingers"));
    }

    /// Case and spacing must not let a clash slip through.
    #[test]
    fn shadow_keys_ignore_case_and_spacing() {
        let mut a = gesture(3, "Up", "close");
        a.mods = "super".into();
        let mut b = gesture(3, " up ", "move");
        b.mods = " SUPER ".into();
        assert_eq!(a.shadow_key(), b.shadow_key());
    }

    #[test]
    fn a_disabled_gesture_is_kept_but_not_written() {
        let mut g = gesture(3, "up", "close");
        g.enabled = false;
        let settings = Settings { gestures: vec![g] };
        assert_eq!(generate(&settings, &[]), "");
    }
}
