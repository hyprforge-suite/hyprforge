//! Every behaviour and platform option Hyprforge can set.
//!
//! `tests/live_system.rs` checks each one against a running compositor.
//! That has already paid: the wiki documents `misc:bell_sound`,
//! `misc:float_force_onscreen`, `misc:new_float_force_onscreen` and
//! `render:not_shown_fifo_lock`, and Hyprland 0.56.1 answers "no such
//! option" for all four. They are declared unsupported rather than
//! catalogued, so a config using one is explained instead of being called
//! a typo.
//!
//! **`render` is the dangerous category.** Several of these can leave the
//! screen blank or unusable on particular hardware, and unlike a bad
//! colour there is no visible clue why. They are still offered — a user
//! who needs `direct_scanout` off has no other route to it from a GUI —
//! but the ones that bite say so in their help text.

pub use hyprforge_core::hlconfig::{Catalog, Category, Kind, Setting};

pub const CATEGORIES: &[Category] = &[
    Category {
        key: "misc",
        label: "Behaviour",
        help: "Focus, workspaces, window swallowing, and the bits that don't belong anywhere else.",
    },
    Category {
        key: "misc:col",
        label: "Splash colour",
        help: "The colour of the splash text over the wallpaper.",
    },
    Category {
        key: "binds",
        label: "Shortcut behaviour",
        help: "How keybindings behave — not which keys are bound. That's the Shortcuts screen.",
    },
    Category {
        key: "xwayland",
        label: "X11 applications",
        help: "How apps that don't speak Wayland are handled.",
    },
    Category {
        key: "render",
        label: "Rendering",
        help: "How frames are produced. Several of these can blank the screen on some hardware — read before changing.",
    },
    Category {
        key: "opengl",
        label: "OpenGL",
        help: "Driver-specific rendering workarounds.",
    },
    Category {
        key: "ecosystem",
        label: "Hyprland notices",
        help: "The popups Hyprland shows about itself.",
    },
    Category {
        key: "quirks",
        label: "Hardware quirks",
        help: "Workarounds for specific displays and drivers.",
    },
];

pub const CATALOG: Catalog = Catalog {
    settings: SETTINGS,
    categories: CATEGORIES,
    unsupported: &[
        (
            "misc:bell_sound",
            "A custom bell sound isn't in Hyprland 0.56 — the wiki documents it ahead of the release.",
        ),
        (
            "misc:float_force_onscreen",
            "Forcing floating windows on-screen isn't in Hyprland 0.56 — the wiki documents it ahead of the release.",
        ),
        (
            "misc:new_float_force_onscreen",
            "Forcing new floating windows on-screen isn't in Hyprland 0.56 — the wiki documents it ahead of the release.",
        ),
        (
            "render:not_shown_fifo_lock",
            "Fifo locking for hidden surfaces isn't in Hyprland 0.56 — the wiki documents it ahead of the release.",
        ),
    ],
};

pub fn get(key: &str) -> Option<&'static Setting> {
    CATALOG.get(key)
}

const VRR: &[(i64, &str)] = &[
    (0, "Off"),
    (1, "On"),
    (2, "On, but not for video and games"),
    (3, "On for fullscreen only"),
];

const DEFAULT_WALLPAPER: &[(i64, &str)] = &[
    (-1, "Random"),
    (0, "Plain"),
    (1, "The first one"),
    (2, "The second one"),
];

const FULLSCREEN_FOCUS: &[(i64, &str)] = &[
    (0, "Leave fullscreen"),
    (1, "Keep fullscreen and focus behind it"),
    (2, "Keep fullscreen"),
];

const WORKSPACE_TRACKING: &[(i64, &str)] = &[
    (0, "Off"),
    (1, "Only the first window"),
    (2, "Every window the app opens"),
];

const CENTER_ON: &[(i64, &str)] = &[(0, "Never"), (1, "Only with no windows"), (2, "Always")];

const FOCUS_METHOD: &[(i64, &str)] = &[(0, "Historical order"), (1, "Length of shared edge")];

const DIRECT_SCANOUT: &[(i64, &str)] = &[
    (0, "Off"),
    (1, "On"),
    (2, "On, only for content marked as a game"),
];

const CTM_ANIMATION: &[(i64, &str)] = &[(0, "Off"), (1, "On"), (2, "Automatic")];

const AUTO_HDR: &[(i64, &str)] = &[(0, "Off"), (1, "For HDR content"), (2, "Always")];

const NON_SHADER_CM: &[(i64, &str)] = &[
    (0, "Off"),
    (1, "Whenever possible"),
    (2, "For fullscreen only"),
    (3, "Automatic"),
];

const OFF_ON_AUTO: &[(i64, &str)] = &[(0, "Off"), (1, "On"), (2, "Automatic")];

const PREFER_HDR: &[(i64, &str)] = &[(0, "Off"), (1, "Always"), (2, "For games")];

pub const SETTINGS: &[Setting] = &[
    // ---- misc: focus and windows ----
    Setting {
        key: "misc:focus_on_activate",
        label: "Let apps take focus",
        help: "An app asking to be focused gets focus, instead of only being highlighted.",
        kind: Kind::Bool { default: false },
    },
    Setting {
        key: "misc:mouse_move_focuses_monitor",
        label: "Moving the mouse focuses a screen",
        help: "Moving the pointer onto another monitor focuses it.",
        kind: Kind::Bool { default: true },
    },
    Setting {
        key: "misc:always_follow_on_dnd",
        label: "Focus follows a drag",
        help: "Focus follows the pointer while dragging and dropping between windows.",
        kind: Kind::Bool { default: true },
    },
    Setting {
        key: "misc:layers_hog_keyboard_focus",
        label: "Panels keep keyboard focus",
        help: "Keyboard-interactive panels and launchers hold focus until dismissed.",
        kind: Kind::Bool { default: true },
    },
    Setting {
        key: "misc:on_focus_under_fullscreen",
        label: "Focusing behind a fullscreen window",
        help: "What happens when something behind a fullscreen window takes focus.",
        kind: Kind::IntEnum { default: 2, choices: FULLSCREEN_FOCUS },
    },
    Setting {
        key: "misc:exit_window_retains_fullscreen",
        label: "Stay fullscreen when one closes",
        help: "Closing a fullscreen window leaves the next one fullscreen too.",
        kind: Kind::Bool { default: false },
    },
    Setting {
        key: "misc:close_special_on_empty",
        label: "Close empty special workspaces",
        help: "A special workspace closes once its last window is gone.",
        kind: Kind::Bool { default: true },
    },
    Setting {
        key: "misc:initial_workspace_tracking",
        label: "Open windows where they were launched",
        help: "A window opens on the workspace its launcher was on, even if you've moved since.",
        kind: Kind::IntEnum { default: 1, choices: WORKSPACE_TRACKING },
    },
    Setting {
        key: "misc:initial_workspace_token_timeout",
        label: "How long to wait for it",
        help: "Seconds a window has to appear before it opens wherever you are instead.",
        kind: Kind::Int { default: 10, min: Some(0), max: Some(600) },
    },
    Setting {
        key: "misc:size_limits_tiled",
        label: "Apply size rules to tiled windows",
        help: "Minimum and maximum size window rules also constrain tiled windows.",
        kind: Kind::Bool { default: false },
    },
    Setting {
        key: "misc:middle_click_paste",
        label: "Middle-click paste",
        help: "Middle-clicking pastes the primary selection.",
        kind: Kind::Bool { default: true },
    },
    // ---- misc: window swallowing ----
    Setting {
        key: "misc:enable_swallow",
        label: "Swallow terminals",
        help: "A terminal hides itself while a program it launched is open.",
        kind: Kind::Bool { default: false },
    },
    Setting {
        key: "misc:swallow_regex",
        label: "Which windows swallow",
        help: "Class pattern for the windows that hide, e.g. \"^(kitty)$\".",
        kind: Kind::Text { default: "" },
    },
    Setting {
        key: "misc:swallow_exception_regex",
        label: "Except these titles",
        help: "Title pattern for windows that should not cause swallowing.",
        kind: Kind::Text { default: "" },
    },
    // ---- misc: display and power ----
    Setting {
        key: "misc:vrr",
        label: "Variable refresh rate",
        help: "Adaptive sync. Can cause flicker on some panels, which is why it's off by default.",
        kind: Kind::IntEnum { default: 0, choices: VRR },
    },
    Setting {
        key: "misc:mouse_move_enables_dpms",
        label: "Wake screens on mouse move",
        help: "Moving the mouse wakes displays that have been turned off.",
        kind: Kind::Bool { default: false },
    },
    Setting {
        key: "misc:key_press_enables_dpms",
        label: "Wake screens on keypress",
        help: "Pressing a key wakes displays that have been turned off.",
        kind: Kind::Bool { default: false },
    },
    Setting {
        key: "misc:render_unfocused_fps",
        label: "Background frame rate",
        help: "Frame rate cap for windows rendering while unfocused.",
        kind: Kind::Int { default: 15, min: Some(1), max: Some(240) },
    },
    // ---- misc: lock screen ----
    Setting {
        key: "misc:allow_session_lock_restore",
        label: "Allow restarting the lock screen",
        help: "If the lock screen crashes, another can take over instead of leaving you stuck.",
        kind: Kind::Bool { default: false },
    },
    Setting {
        key: "misc:session_lock_xray",
        label: "Render behind the lock screen",
        help: "Keep drawing your workspaces underneath the lock screen.",
        kind: Kind::Bool { default: false },
    },
    Setting {
        key: "misc:session_lock_blur",
        label: "Blur behind the lock screen",
        help: "Blur what's behind the lock screen. Needs the setting above.",
        kind: Kind::Bool { default: false },
    },
    Setting {
        key: "misc:lockdead_screen_delay",
        label: "Lock failure delay",
        help: "Milliseconds before the emergency screen appears if the lock screen dies.",
        kind: Kind::Int { default: 1000, min: Some(0), max: Some(60_000) },
    },
    // ---- misc: appearance of Hyprland's own bits ----
    Setting {
        key: "misc:disable_hyprland_logo",
        label: "Hide the Hyprland wallpaper",
        help: "Turn off the default background so a plain colour or your own wallpaper shows.",
        kind: Kind::Bool { default: false },
    },
    Setting {
        key: "misc:background_color",
        label: "Background colour",
        help: "The colour behind everything. Only visible with the Hyprland wallpaper off.",
        kind: Kind::ColorInt { default: "rgba(111111ff)" },
    },
    Setting {
        key: "misc:force_default_wallpaper",
        label: "Which default wallpaper",
        help: "Which of Hyprland's own backgrounds to use.",
        kind: Kind::IntEnum { default: -1, choices: DEFAULT_WALLPAPER },
    },
    Setting {
        key: "misc:disable_splash_rendering",
        label: "Hide the splash text",
        help: "Turn off the version text drawn over the background.",
        kind: Kind::Bool { default: false },
    },
    Setting {
        key: "misc:col:splash",
        label: "Splash text colour",
        help: "Colour of the version text over the background.",
        kind: Kind::ColorInt { default: "rgba(ffffff55)" },
    },
    Setting {
        key: "misc:font_family",
        label: "Interface font",
        help: "Font Hyprland uses for its own text — dialogs, debug overlays.",
        kind: Kind::Text { default: "Sans" },
    },
    Setting {
        key: "misc:splash_font_family",
        label: "Splash font",
        help: "Font for the splash text. Empty uses the interface font.",
        kind: Kind::Text { default: "" },
    },
    // ---- misc: warnings and dialogs ----
    Setting {
        key: "misc:enable_anr_dialog",
        label: "Warn about frozen apps",
        help: "Offer to close an application that has stopped responding.",
        kind: Kind::Bool { default: true },
    },
    Setting {
        key: "misc:anr_missed_pings",
        label: "How long before warning",
        help: "Missed responses before an app is called frozen.",
        kind: Kind::Int { default: 5, min: Some(1), max: Some(100) },
    },
    Setting {
        key: "misc:disable_scale_notification",
        label: "Hide scale warnings",
        help: "Don't warn when a display can't take the scale you asked for.",
        kind: Kind::Bool { default: false },
    },
    Setting {
        key: "misc:disable_xdg_env_checks",
        label: "Hide XDG environment warnings",
        help: "Don't warn when the desktop environment variables are managed elsewhere.",
        kind: Kind::Bool { default: false },
    },
    Setting {
        key: "misc:disable_hyprland_guiutils_check",
        label: "Hide the guiutils warning",
        help: "Don't warn when hyprland-guiutils isn't installed.",
        kind: Kind::Bool { default: false },
    },
    Setting {
        key: "misc:disable_watchdog_warning",
        label: "Hide the startup warning",
        help: "Don't warn about starting Hyprland without its wrapper script.",
        kind: Kind::Bool { default: false },
    },
    Setting {
        key: "misc:disable_autoreload",
        label: "Don't reload on save",
        help: "Stop Hyprland reloading the config when the file changes. Hyprforge reloads explicitly, so this is safe to turn on.",
        kind: Kind::Bool { default: false },
    },
    Setting {
        key: "misc:animate_manual_resizes",
        label: "Animate manual resizes",
        help: "Animate windows while you drag them to a new size.",
        kind: Kind::Bool { default: false },
    },
    Setting {
        key: "misc:animate_mouse_windowdragging",
        label: "Animate window dragging",
        help: "Animate windows while dragging them by mouse. Can feel laggy.",
        kind: Kind::Bool { default: false },
    },
    Setting {
        key: "misc:name_vk_after_proc",
        label: "Name virtual keyboards after their app",
        help: "Virtual keyboards take the name of whatever created them.",
        kind: Kind::Bool { default: true },
    },
    Setting {
        key: "misc:screencopy_force_8b",
        label: "8-bit screen capture",
        help: "Force screen capture to 8 bits per channel. Fixes some recorders.",
        kind: Kind::Bool { default: true },
    },
    // ---- binds ----
    Setting {
        key: "binds:workspace_back_and_forth",
        label: "Switch back with the same key",
        help: "Pressing the shortcut for the workspace you're already on returns to the previous one.",
        kind: Kind::Bool { default: false },
    },
    Setting {
        key: "binds:allow_workspace_cycles",
        label: "Remember the previous workspace",
        help: "Workspaces keep track of where you came from, so back-and-forth chains.",
        kind: Kind::Bool { default: false },
    },
    Setting {
        key: "binds:hide_special_on_workspace_change",
        label: "Hide special workspaces on switch",
        help: "Changing workspace closes any open special workspace.",
        kind: Kind::Bool { default: false },
    },
    Setting {
        key: "binds:workspace_center_on",
        label: "Centre the cursor on switch",
        help: "Whether switching workspace moves the pointer to the middle.",
        kind: Kind::IntEnum { default: 1, choices: CENTER_ON },
    },
    Setting {
        key: "binds:focus_preferred_method",
        label: "How directional focus picks",
        help: "Which window wins when several sit in the direction you moved.",
        kind: Kind::IntEnum { default: 0, choices: FOCUS_METHOD },
    },
    Setting {
        key: "binds:movefocus_cycles_fullscreen",
        label: "Move focus out of fullscreen",
        help: "Directional focus leaves a fullscreen window instead of stopping.",
        kind: Kind::Bool { default: false },
    },
    Setting {
        key: "binds:movefocus_cycles_groupfirst",
        label: "Move focus within a group first",
        help: "Directional focus moves through a group's tabs before leaving it.",
        kind: Kind::Bool { default: false },
    },
    Setting {
        key: "binds:window_direction_monitor_fallback",
        label: "Cross screens with focus",
        help: "Moving focus past a screen edge continues onto the next monitor.",
        kind: Kind::Bool { default: true },
    },
    Setting {
        key: "binds:ignore_group_lock",
        label: "Ignore group locks",
        help: "Dispatchers move windows into locked groups anyway.",
        kind: Kind::Bool { default: false },
    },
    Setting {
        key: "binds:allow_pin_fullscreen",
        label: "Allow pinning fullscreen windows",
        help: "A pinned window can be fullscreened and keep its pin.",
        kind: Kind::Bool { default: false },
    },
    Setting {
        key: "binds:pass_mouse_when_bound",
        label: "Pass mouse to apps when bound",
        help: "Mouse events still reach the app when the same button is bound to a shortcut.",
        kind: Kind::Bool { default: false },
    },
    Setting {
        key: "binds:scroll_event_delay",
        label: "Scroll shortcut delay",
        help: "Milliseconds after a scroll shortcut before scrolling reaches the app again.",
        kind: Kind::Int { default: 300, min: Some(0), max: Some(5000) },
    },
    Setting {
        key: "binds:drag_threshold",
        label: "Drag threshold",
        help: "Pixels the mouse must move before a click becomes a drag. 0 starts immediately.",
        kind: Kind::Int { default: 0, min: Some(0), max: Some(200) },
    },
    Setting {
        key: "binds:disable_keybind_grabbing",
        label: "Never let apps take shortcuts",
        help: "Ignore applications asking to grab your keybindings — remote desktop and VMs do this.",
        kind: Kind::Bool { default: false },
    },
    // ---- xwayland ----
    Setting {
        key: "xwayland:enabled",
        label: "Run X11 applications",
        help: "Turn off to refuse X11 apps entirely.",
        kind: Kind::Bool { default: true },
    },
    Setting {
        key: "xwayland:use_nearest_neighbor",
        label: "Sharp scaling for X11 apps",
        help: "Scale X11 windows without blurring, at the cost of jagged edges.",
        kind: Kind::Bool { default: true },
    },
    Setting {
        key: "xwayland:force_zero_scaling",
        label: "Don't scale X11 apps",
        help: "Leave X11 windows unscaled on a scaled display. Fixes blurry apps that handle scaling themselves.",
        kind: Kind::Bool { default: false },
    },
    Setting {
        key: "xwayland:create_abstract_socket",
        label: "Create the abstract socket",
        help: "An extra X11 socket some sandboxed apps look for.",
        kind: Kind::Bool { default: false },
    },
    // ---- render ----
    Setting {
        key: "render:direct_scanout",
        label: "Direct scanout",
        help: "Sends a fullscreen window straight to the display for lower latency. Can cause black screens or stutter on some drivers.",
        kind: Kind::IntEnum { default: 0, choices: DIRECT_SCANOUT },
    },
    Setting {
        key: "render:new_render_scheduling",
        label: "Triple buffering",
        help: "Buffers ahead when frames are slow, improving smoothness at the cost of latency.",
        kind: Kind::Bool { default: false },
    },
    Setting {
        key: "render:expand_undersized_textures",
        label: "Expand small textures",
        help: "Stretch a texture to its window instead of leaving a gap while an app resizes.",
        kind: Kind::Bool { default: true },
    },
    Setting {
        key: "render:xp_mode",
        label: "Minimal rendering",
        help: "Skips the back buffer and bottom layer. Saves power and breaks some effects.",
        kind: Kind::Bool { default: false },
    },
    Setting {
        key: "render:ctm_animation",
        label: "Fade colour changes",
        help: "Animate colour temperature changes from hyprsunset instead of snapping.",
        kind: Kind::IntEnum { default: 2, choices: CTM_ANIMATION },
    },
    Setting {
        key: "render:cm_enabled",
        label: "Colour management",
        help: "The colour pipeline, needed for HDR and ICC profiles. Turning it off can change how everything looks.",
        kind: Kind::Bool { default: true },
    },
    Setting {
        key: "render:cm_auto_hdr",
        label: "Automatic HDR",
        help: "Switch the display into HDR when fullscreen content wants it.",
        kind: Kind::IntEnum { default: 1, choices: AUTO_HDR },
    },
    Setting {
        key: "render:cm_sdr_eotf",
        label: "SDR transfer function",
        help: "How ordinary apps are mapped onto an HDR display.",
        kind: Kind::Text { default: "default" },
    },
    Setting {
        key: "render:non_shader_cm",
        label: "Colour management without shaders",
        help: "Uses the display hardware for colour management where possible.",
        kind: Kind::IntEnum { default: 3, choices: NON_SHADER_CM },
    },
    Setting {
        key: "render:non_shader_cm_interop",
        label: "Hardware colour with hyprsunset",
        help: "Whether hyprsunset's tint still applies while hardware colour management is in use.",
        kind: Kind::IntEnum { default: 2, choices: OFF_ON_AUTO },
    },
    Setting {
        key: "render:use_fp16",
        label: "16-bit colour buffers",
        help: "Higher precision internally. Costs memory and bandwidth.",
        kind: Kind::IntEnum { default: 2, choices: OFF_ON_AUTO },
    },
    Setting {
        key: "render:fp16_sdr_tf",
        label: "16-bit SDR transfer function",
        help: "Which transfer function the 16-bit buffer uses for ordinary content.",
        kind: Kind::Int { default: 0, min: Some(0), max: Some(10) },
    },
    Setting {
        key: "render:keep_unmodified_copy",
        label: "Keep a clean frame for sharing",
        help: "Keeps an un-tinted copy so screen sharing isn't affected by night light.",
        kind: Kind::IntEnum { default: 2, choices: OFF_ON_AUTO },
    },
    Setting {
        key: "render:send_content_type",
        label: "Tell displays the content type",
        help: "Lets a monitor switch its own picture mode. Some TVs react badly.",
        kind: Kind::Bool { default: true },
    },
    Setting {
        key: "render:icc_vcgt_enabled",
        label: "Apply ICC calibration curves",
        help: "Send the calibration ramps from an ICC profile to the display.",
        kind: Kind::Bool { default: true },
    },
    Setting {
        key: "render:commit_timing_enabled",
        label: "Commit timing protocol",
        help: "Lets apps schedule frames more precisely. Needs a restart to change.",
        kind: Kind::Bool { default: true },
    },
    Setting {
        key: "render:use_shader_blur_blend",
        label: "Experimental blur blending",
        help: "A different blur blend. Known to glitch on rotated screens.",
        kind: Kind::Bool { default: false },
    },
    // ---- opengl ----
    Setting {
        key: "opengl:nvidia_anti_flicker",
        label: "Nvidia anti-flicker",
        help: "Reduces flicker on Nvidia at the cost of occasional dropped frames.",
        kind: Kind::Bool { default: true },
    },
    // ---- ecosystem ----
    Setting {
        key: "ecosystem:no_update_news",
        label: "Hide update news",
        help: "Stop the popup that appears after Hyprland updates.",
        kind: Kind::Bool { default: false },
    },
    Setting {
        key: "ecosystem:no_donation_nag",
        label: "Hide donation notices",
        help: "Stop the twice-yearly donation popup.",
        kind: Kind::Bool { default: false },
    },
    Setting {
        key: "ecosystem:enforce_permissions",
        label: "Enforce permission rules",
        help: "Turn on Hyprland's permission system. Without this, the rules on the Session screen are ignored.",
        kind: Kind::Bool { default: false },
    },
    // ---- quirks ----
    Setting {
        key: "quirks:prefer_hdr",
        label: "Report HDR as preferred",
        help: "Tell applications HDR is the preferred mode.",
        kind: Kind::IntEnum { default: 0, choices: PREFER_HDR },
    },
    Setting {
        key: "quirks:skip_non_kms_dmabuf_formats",
        label: "Skip unusable buffer formats",
        help: "Hide buffer formats the display hardware can't take. A workaround for specific drivers.",
        kind: Kind::Bool { default: false },
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
                CATALOG.category(s.category()).is_some(),
                "{} is in undeclared category {}",
                s.key,
                s.category()
            );
        }
    }

    #[test]
    fn every_category_has_settings() {
        for c in CATEGORIES {
            assert!(
                CATALOG.in_category(c.key).next().is_some(),
                "category {} has no settings",
                c.key
            );
        }
    }

    #[test]
    fn defaults_sit_inside_their_declared_range() {
        for s in SETTINGS {
            match s.kind {
                Kind::Int { default, min, max } | Kind::Gaps { default, min, max } => {
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
    fn every_default_passes_validation() {
        use hyprforge_core::hlconfig::{Settings, Value};
        for s in SETTINGS {
            let stored = Settings::from_one(s.key, Value::default_for(&s.kind));
            assert_eq!(stored.validate(&CATALOG), vec![], "{} rejected its own default", s.key);
        }
    }

    /// Four options the wiki documents that Hyprland 0.56.1 doesn't have.
    /// Catalogued, they would be settings that silently do nothing.
    #[test]
    fn options_absent_from_this_hyprland_are_declared_unsupported() {
        for key in [
            "misc:bell_sound",
            "misc:float_force_onscreen",
            "misc:new_float_force_onscreen",
            "render:not_shown_fifo_lock",
        ] {
            assert!(get(key).is_none(), "{key} should not be catalogued");
            // The exact key, which is what a user's file would hold —
            // not a made-up child of it.
            assert!(
                CATALOG.unknown_key_reason(key).contains("0.56"),
                "{key} should be explained rather than called a typo"
            );
        }
    }

    /// The permission rules on the Session screen do nothing unless this
    /// is on, so it has to be reachable.
    #[test]
    fn the_permission_switch_is_catalogued() {
        assert!(get("ecosystem:enforce_permissions").is_some());
    }
}
