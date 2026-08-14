//! Every input option Hyprforge can set, with its type, default and the
//! values it accepts.
//!
//! Like the shortcuts dispatcher catalog, this is the single place that
//! claims anything about Hyprland's input surface, and
//! `tests/live_input.rs` checks every claim against a running compositor:
//! a key that doesn't exist, or whose type this file gets wrong, fails
//! there rather than in someone's config.
//!
//! Keys are written in Hyprland's own colon form (`input:touchpad:flip_x`)
//! because that is what `hyprctl getoption` takes, which is how both the
//! live test and [`crate::import`] address them. [`crate::codegen`] turns
//! them back into the nested Lua tables `hl.config` wants.
//!
//! **`input:tablet:*` is deliberately absent.** Four of its nine fields are
//! `vec2`, a type nothing else here uses and which needs its own editor,
//! and there is no drawing tablet on hand to verify any of it against. An
//! unverified guess about hardware nobody here can test is worth less than
//! an honest gap — see [`crate::model::UNSUPPORTED_CATEGORIES`].

/// What a setting accepts, and what Hyprland does with it when nothing sets
/// it. The default matters for more than display: [`crate::model`] uses it
/// to tell "the user chose the default" from "the user chose nothing",
/// which is the difference between writing the key and staying out of the
/// way.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Kind {
    Bool {
        default: bool,
    },
    Int {
        default: i64,
        min: Option<i64>,
        max: Option<i64>,
    },
    /// An integer whose values are named states rather than a quantity —
    /// a dropdown, not a spinner.
    IntEnum {
        default: i64,
        choices: &'static [(i64, &'static str)],
    },
    Float {
        default: f64,
        min: Option<f64>,
        max: Option<f64>,
    },
    Text {
        default: &'static str,
    },
    /// Text with a closed set of accepted values. Only used where the set
    /// really is closed: `accel_profile` looks like one but also takes
    /// `custom <step> <points...>`, so it stays [`Kind::Text`].
    TextEnum {
        default: &'static str,
        choices: &'static [&'static str],
    },
}

impl Kind {
    /// The name `hyprctl getoption -j` uses for this type's value field.
    /// The live test compares against this, which is what stops the catalog
    /// claiming an int where Hyprland has a float.
    pub fn hyprctl_field(&self) -> &'static str {
        match self {
            Kind::Bool { .. } => "bool",
            Kind::Int { .. } | Kind::IntEnum { .. } => "int",
            Kind::Float { .. } => "float",
            Kind::Text { .. } | Kind::TextEnum { .. } => "str",
        }
    }
}

#[derive(Debug, Clone, Copy)]
pub struct Setting {
    /// Hyprland's own colon-separated key, e.g. `input:touchpad:flip_x`.
    pub key: &'static str,
    /// Short human label for the editor.
    pub label: &'static str,
    /// One sentence on what it does, shown next to the control. Kept to
    /// what a user needs to decide, not a restatement of the label.
    pub help: &'static str,
    pub kind: Kind,
}

impl Setting {
    /// The `input:touchpad`-style prefix this setting groups under.
    pub fn category(&self) -> &'static str {
        match self.key.rfind(':') {
            Some(i) => &self.key[..i],
            None => self.key,
        }
    }

    /// The last path segment — the name as written inside the Lua table.
    pub fn leaf(&self) -> &'static str {
        match self.key.rfind(':') {
            Some(i) => &self.key[i + 1..],
            None => self.key,
        }
    }
}

/// A group of settings, in the order the editor shows them.
#[derive(Debug, Clone, Copy)]
pub struct Category {
    pub key: &'static str,
    pub label: &'static str,
    pub help: &'static str,
}

pub const CATEGORIES: &[Category] = &[
    Category {
        key: "input",
        label: "Keyboard & pointer",
        help: "Applies to every input device unless a per-device setting overrides it.",
    },
    Category {
        key: "input:touchpad",
        label: "Touchpad",
        help: "Only affects devices Hyprland recognises as touchpads.",
    },
    Category {
        key: "input:touchdevice",
        label: "Touchscreen",
        help: "Touchscreens, including which display they map to.",
    },
    Category {
        key: "input:virtualkeyboard",
        label: "Virtual keyboards",
        help: "On-screen keyboards and input methods that type on your behalf.",
    },
    Category {
        key: "input:tablettool",
        label: "Tablet stylus",
        help: "Stylus buttons and pressure range. The tablet surface itself isn't editable here yet.",
    },
];

pub fn category(key: &str) -> Option<&'static Category> {
    CATEGORIES.iter().find(|c| c.key == key)
}

pub fn get(key: &str) -> Option<&'static Setting> {
    SETTINGS.iter().find(|s| s.key == key)
}

/// Settings in `category`, in catalog order.
pub fn in_category(category: &str) -> impl Iterator<Item = &'static Setting> + use<'_> {
    SETTINGS.iter().filter(move |s| s.category() == category)
}

const FOLLOW_MOUSE: &[(i64, &str)] = &[
    (0, "Never change focus"),
    (1, "Focus whatever is under the cursor"),
    (2, "Cursor focus separate, clicking moves keyboard focus"),
    (3, "Cursor focus fully separate from keyboard focus"),
];

const FOCUS_ON_CLOSE: &[(i64, &str)] = &[
    (0, "Next window in line"),
    (1, "Window under the cursor"),
    (2, "Most recently used window"),
];

const FLOAT_SWITCH: &[(i64, &str)] = &[
    (0, "Never"),
    (1, "On tiled/floating switches"),
    (2, "Also on float-to-float switches"),
];

const OFF_WINDOW_AXIS: &[(i64, &str)] = &[
    (0, "Ignore them"),
    (1, "Send out-of-bounds coordinates"),
    (2, "Fake the closest point inside the window"),
    (3, "Warp the cursor inside the window"),
];

const DISCRETE_SCROLL: &[(i64, &str)] = &[
    (0, "Off"),
    (1, "Non-standard events only"),
    (2, "All scroll wheel events"),
];

const DRAG_LOCK: &[(i64, &str)] = &[
    (0, "Off"),
    (1, "On, with a timeout"),
    (2, "On, sticky"),
];

const DRAG_3FG: &[(i64, &str)] = &[
    (0, "Off"),
    (1, "Three fingers"),
    (2, "Four fingers"),
];

const SHARE_STATES: &[(i64, &str)] = &[
    (0, "No"),
    (1, "Yes"),
    (2, "Yes, unless an input method is active"),
];

const ERASER_MODE: &[(i64, &str)] = &[
    (0, "The tool's own hardware behaviour"),
    (1, "Send a button event instead"),
];

/// The transform values are the monitor ones; only the four rotations are
/// offered because the flipped variants need a preview to pick correctly
/// and mis-setting one leaves a touchscreen unusable in a way that is hard
/// to attribute.
const TOUCH_TRANSFORM: &[(i64, &str)] = &[
    (0, "None"),
    (1, "90° clockwise"),
    (2, "180°"),
    (3, "270° clockwise"),
];

pub const SETTINGS: &[Setting] = &[
    // ---- input ----
    Setting {
        key: "input:kb_layout",
        label: "Keyboard layout",
        help: "XKB layout name, e.g. \"us\". Comma-separate several to switch between them.",
        kind: Kind::Text { default: "us" },
    },
    Setting {
        key: "input:kb_variant",
        label: "Layout variant",
        help: "XKB variant, e.g. \"dvorak\". One per layout, comma-separated, blanks allowed.",
        kind: Kind::Text { default: "" },
    },
    Setting {
        key: "input:kb_model",
        label: "Keyboard model",
        help: "XKB model name. Rarely needed; leave empty unless a key doesn't work.",
        kind: Kind::Text { default: "" },
    },
    Setting {
        key: "input:kb_options",
        label: "Layout options",
        help: "XKB options, e.g. \"grp:alt_shift_toggle\" to switch layouts, \"caps:escape\" to remap Caps Lock.",
        kind: Kind::Text { default: "" },
    },
    Setting {
        key: "input:kb_rules",
        label: "Layout rules",
        help: "XKB rules file. Almost always left empty.",
        kind: Kind::Text { default: "" },
    },
    Setting {
        key: "input:kb_file",
        label: "Custom keymap file",
        help: "Path to your own .xkb file, replacing the settings above entirely.",
        kind: Kind::Text { default: "" },
    },
    Setting {
        key: "input:numlock_by_default",
        label: "Num Lock on at startup",
        help: "Turns Num Lock on when Hyprland starts.",
        kind: Kind::Bool { default: false },
    },
    Setting {
        key: "input:resolve_binds_by_sym",
        label: "Match shortcuts by symbol",
        help: "With several layouts, match shortcuts against the symbol you actually type rather than always the first layout.",
        kind: Kind::Bool { default: false },
    },
    Setting {
        key: "input:repeat_rate",
        label: "Key repeat rate",
        help: "Repeats per second while a key is held.",
        kind: Kind::Int { default: 25, min: Some(1), max: Some(255) },
    },
    Setting {
        key: "input:repeat_delay",
        label: "Key repeat delay",
        help: "Milliseconds a key must be held before it starts repeating.",
        kind: Kind::Int { default: 600, min: Some(1), max: Some(10_000) },
    },
    Setting {
        key: "input:sensitivity",
        label: "Pointer sensitivity",
        help: "Mouse speed, from -1.0 (slowest) to 1.0 (fastest). 0 leaves the device as-is.",
        kind: Kind::Float { default: 0.0, min: Some(-1.0), max: Some(1.0) },
    },
    Setting {
        key: "input:accel_profile",
        label: "Acceleration profile",
        help: "\"adaptive\", \"flat\", or \"custom <step> <points...>\". Empty uses libinput's default for each device.",
        kind: Kind::Text { default: "" },
    },
    Setting {
        key: "input:force_no_accel",
        label: "Bypass pointer processing",
        help: "Sends the rawest possible pointer signal, overriding the settings above. Can desynchronise the cursor; not recommended.",
        kind: Kind::Bool { default: false },
    },
    Setting {
        key: "input:rotation",
        label: "Pointer rotation",
        help: "Rotates pointer movement, in degrees clockwise (0–359).",
        kind: Kind::Int { default: 0, min: Some(0), max: Some(359) },
    },
    Setting {
        key: "input:left_handed",
        label: "Left-handed buttons",
        help: "Swaps the left and right mouse buttons.",
        kind: Kind::Bool { default: false },
    },
    Setting {
        key: "input:natural_scroll",
        label: "Natural scrolling",
        help: "Scrolling moves the content rather than the scrollbar. Touchpads have their own setting.",
        kind: Kind::Bool { default: false },
    },
    Setting {
        key: "input:scroll_factor",
        label: "Scroll speed",
        help: "Multiplier for scroll movement on external mice.",
        kind: Kind::Float { default: 1.0, min: Some(0.0), max: Some(20.0) },
    },
    Setting {
        key: "input:scroll_method",
        label: "Scroll method",
        help: "How scrolling is produced on devices without a wheel. Empty uses the device default.",
        kind: Kind::TextEnum {
            default: "",
            choices: &["", "2fg", "edge", "on_button_down", "no_scroll"],
        },
    },
    Setting {
        key: "input:scroll_button",
        label: "Scroll button",
        help: "Button code that scrolls while held, for \"on_button_down\". 0 uses the device default.",
        kind: Kind::Int { default: 0, min: Some(0), max: Some(0x2ff) },
    },
    Setting {
        key: "input:scroll_button_lock",
        label: "Scroll button latches",
        help: "Press and release the scroll button to lock it on, instead of holding it.",
        kind: Kind::Bool { default: false },
    },
    Setting {
        key: "input:scroll_points",
        label: "Custom scroll curve",
        help: "\"<step> <points...>\" scroll curve. Only used when the acceleration profile is \"custom\".",
        kind: Kind::Text { default: "" },
    },
    Setting {
        key: "input:follow_mouse",
        label: "Focus follows mouse",
        help: "Whether moving the cursor over a window focuses it.",
        kind: Kind::IntEnum { default: 1, choices: FOLLOW_MOUSE },
    },
    Setting {
        key: "input:follow_mouse_shrink",
        label: "Focus dead zone",
        help: "Shrinks unfocused windows' focus hitbox by this many pixels, creating a dead zone in the gaps. Only with focus-follows-mouse on.",
        kind: Kind::Int { default: 0, min: Some(0), max: Some(500) },
    },
    Setting {
        key: "input:follow_mouse_threshold",
        label: "Focus movement threshold",
        help: "How far the cursor must travel before the window under it takes focus. Only with focus-follows-mouse on.",
        kind: Kind::Float { default: 0.0, min: Some(0.0), max: Some(500.0) },
    },
    Setting {
        key: "input:mouse_refocus",
        label: "Refocus without crossing a border",
        help: "When off, focus only follows the mouse when it actually crosses a window boundary.",
        kind: Kind::Bool { default: true },
    },
    Setting {
        key: "input:focus_on_close",
        label: "Focus after closing a window",
        help: "Which window takes focus when the focused one closes.",
        kind: Kind::IntEnum { default: 0, choices: FOCUS_ON_CLOSE },
    },
    Setting {
        key: "input:float_switch_override_focus",
        label: "Refocus on float/tile switch",
        help: "Whether focus jumps to the window under the cursor when a window changes between tiled and floating.",
        kind: Kind::IntEnum { default: 1, choices: FLOAT_SWITCH },
    },
    Setting {
        key: "input:special_fallthrough",
        label: "See through empty special workspaces",
        help: "A special workspace holding only floating windows no longer blocks focusing the regular ones behind it.",
        kind: Kind::Bool { default: false },
    },
    Setting {
        key: "input:off_window_axis_events",
        label: "Scrolling outside a window",
        help: "What to do with scroll events landing in the gap or border around the focused window.",
        kind: Kind::IntEnum { default: 1, choices: OFF_WINDOW_AXIS },
    },
    Setting {
        key: "input:emulate_discrete_scroll",
        label: "Emulate stepped scrolling",
        help: "Turns smooth high-resolution scrolling into discrete steps for apps that expect them.",
        kind: Kind::IntEnum { default: 1, choices: DISCRETE_SCROLL },
    },
    // ---- input:touchpad ----
    Setting {
        key: "input:touchpad:tap_to_click",
        label: "Tap to click",
        help: "Tapping with one, two or three fingers sends left, right and middle click.",
        kind: Kind::Bool { default: true },
    },
    Setting {
        key: "input:touchpad:natural_scroll",
        label: "Natural scrolling",
        help: "Two-finger scrolling moves the content rather than the scrollbar.",
        kind: Kind::Bool { default: false },
    },
    Setting {
        key: "input:touchpad:disable_while_typing",
        label: "Disable while typing",
        help: "Ignores the touchpad briefly after a keystroke, so your palm doesn't move the cursor.",
        kind: Kind::Bool { default: true },
    },
    Setting {
        key: "input:touchpad:scroll_factor",
        label: "Scroll speed",
        help: "Multiplier for how far a two-finger scroll moves.",
        kind: Kind::Float { default: 1.0, min: Some(0.0), max: Some(20.0) },
    },
    Setting {
        key: "input:touchpad:tap_and_drag",
        label: "Tap and drag",
        help: "Tap then hold to start dragging without pressing the pad down.",
        kind: Kind::Bool { default: true },
    },
    Setting {
        key: "input:touchpad:drag_lock",
        label: "Drag lock",
        help: "Lifting your finger mid-drag doesn't drop what you're dragging.",
        kind: Kind::IntEnum { default: 0, choices: DRAG_LOCK },
    },
    Setting {
        key: "input:touchpad:drag_3fg",
        label: "Multi-finger drag",
        help: "Drag with three or four fingers instead of holding a click.",
        kind: Kind::IntEnum { default: 0, choices: DRAG_3FG },
    },
    Setting {
        key: "input:touchpad:clickfinger_behavior",
        label: "Click by finger count",
        help: "One, two or three fingers pressing down send left, right and middle click, ignoring where on the pad you press.",
        kind: Kind::Bool { default: false },
    },
    Setting {
        key: "input:touchpad:middle_button_emulation",
        label: "Middle-click emulation",
        help: "Pressing left and right together counts as a middle click.",
        kind: Kind::Bool { default: false },
    },
    Setting {
        key: "input:touchpad:tap_button_map",
        label: "Tap button order",
        help: "\"lrm\" maps two-finger tap to right click, \"lmr\" to middle click.",
        kind: Kind::TextEnum { default: "", choices: &["", "lrm", "lmr"] },
    },
    Setting {
        key: "input:touchpad:flip_x",
        label: "Invert horizontally",
        help: "Reverses left/right touchpad movement.",
        kind: Kind::Bool { default: false },
    },
    Setting {
        key: "input:touchpad:flip_y",
        label: "Invert vertically",
        help: "Reverses up/down touchpad movement.",
        kind: Kind::Bool { default: false },
    },
    // ---- input:touchdevice ----
    Setting {
        key: "input:touchdevice:enabled",
        label: "Touchscreen enabled",
        help: "Turn off to ignore touch input entirely.",
        kind: Kind::Bool { default: true },
    },
    Setting {
        key: "input:touchdevice:output",
        label: "Mapped display",
        help: "Which display touches land on. Empty stops auto-detection; leave as-is to let Hyprland choose.",
        kind: Kind::Text { default: "" },
    },
    Setting {
        key: "input:touchdevice:transform",
        label: "Touch rotation",
        help: "Rotates touch coordinates to match a rotated screen.",
        kind: Kind::IntEnum { default: 0, choices: TOUCH_TRANSFORM },
    },
    // ---- input:virtualkeyboard ----
    Setting {
        key: "input:virtualkeyboard:share_states",
        label: "Share key and modifier state",
        help: "Whether an on-screen keyboard shares held keys and modifiers with your physical one.",
        kind: Kind::IntEnum { default: 2, choices: SHARE_STATES },
    },
    Setting {
        key: "input:virtualkeyboard:release_pressed_on_close",
        label: "Release keys when closed",
        help: "Releases anything a virtual keyboard was holding down when it disappears, so a key can't stick.",
        kind: Kind::Bool { default: false },
    },
    // ---- input:tablettool ----
    Setting {
        key: "input:tablettool:eraser_button_mode",
        label: "Eraser button",
        help: "Whether the stylus eraser erases, or sends a button press you can bind.",
        kind: Kind::IntEnum { default: 0, choices: ERASER_MODE },
    },
    Setting {
        key: "input:tablettool:eraser_button_override",
        label: "Eraser button code",
        help: "Which button the eraser sends, when it's set to send one. 0 uses the default.",
        kind: Kind::Int { default: 0, min: Some(0), max: Some(0x2ff) },
    },
    Setting {
        key: "input:tablettool:pressure_range_min",
        label: "Minimum pressure",
        help: "Lowest pressure the stylus reports, 0.0–1.0. Negative uses the tool's own minimum.",
        kind: Kind::Float { default: -1.0, min: Some(-1.0), max: Some(1.0) },
    },
    Setting {
        key: "input:tablettool:pressure_range_max",
        label: "Maximum pressure",
        help: "Highest pressure the stylus reports, 0.0–1.0. Negative uses the tool's own maximum.",
        kind: Kind::Float { default: -1.0, min: Some(-1.0), max: Some(1.0) },
    },
];

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;

    #[test]
    fn keys_are_unique() {
        let mut seen = HashSet::new();
        for s in SETTINGS {
            assert!(seen.insert(s.key), "duplicate catalog key: {}", s.key);
        }
    }

    #[test]
    fn every_setting_belongs_to_a_declared_category() {
        for s in SETTINGS {
            assert!(
                category(s.category()).is_some(),
                "{} is in undeclared category {}",
                s.key,
                s.category()
            );
        }
    }

    /// An empty category would render as a heading with nothing under it.
    #[test]
    fn every_category_has_settings() {
        for c in CATEGORIES {
            assert!(
                in_category(c.key).next().is_some(),
                "category {} has no settings",
                c.key
            );
        }
    }

    /// A default outside its own declared range would make the editor open
    /// showing a value it then refuses to save.
    #[test]
    fn defaults_sit_inside_their_declared_range() {
        for s in SETTINGS {
            match s.kind {
                Kind::Int { default, min, max } => {
                    assert!(min.is_none_or(|m| default >= m), "{}", s.key);
                    assert!(max.is_none_or(|m| default <= m), "{}", s.key);
                }
                Kind::Float { default, min, max } => {
                    assert!(min.is_none_or(|m| default >= m), "{}", s.key);
                    assert!(max.is_none_or(|m| default <= m), "{}", s.key);
                }
                Kind::IntEnum { default, choices } => {
                    assert!(choices.iter().any(|(v, _)| *v == default), "{}", s.key);
                }
                Kind::TextEnum { default, choices } => {
                    assert!(choices.contains(&default), "{}", s.key);
                }
                _ => {}
            }
        }
    }

    #[test]
    fn category_and_leaf_split_the_key() {
        let s = get("input:touchpad:flip_x").unwrap();
        assert_eq!(s.category(), "input:touchpad");
        assert_eq!(s.leaf(), "flip_x");
        let top = get("input:kb_layout").unwrap();
        assert_eq!(top.category(), "input");
        assert_eq!(top.leaf(), "kb_layout");
    }
}
