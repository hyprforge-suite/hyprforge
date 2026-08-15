//! Every appearance option Hyprforge can set, with its type, default and
//! the values it accepts.
//!
//! Like the input and dispatcher catalogs, this is the single place that
//! claims anything about this slice of Hyprland's surface, and
//! `tests/live_appearance.rs` checks every claim against a running
//! compositor. That is not a formality: the wiki documents a whole
//! `decoration:wobble` subcategory that **does not exist** in Hyprland
//! 0.56.1 (`hyprctl getoption decoration:wobble:enabled` → "no such
//! option"), so it is absent here. A catalog built from the wiki alone
//! would have shipped eight settings that silently do nothing.
//!
//! Two options are deliberately omitted rather than listed as
//! unsupported, because they aren't appearance:
//!
//! - `general:locale` — an i18n setting that happens to live in
//!   `general`; it belongs with users and time.
//! - `decoration:shadow:offset` — a `vec2`, the one type this editor has
//!   no control for. It is in the unsupported list so a config using it
//!   is explained rather than called a typo.

pub use hyprforge_core::hlconfig::{Catalog, Category, Kind, Setting};

pub const CATEGORIES: &[Category] = &[
    Category {
        key: "general",
        label: "Windows & borders",
        help: "Gaps, borders and how windows are laid out.",
    },
    Category {
        // Hyprland nests these under `general.col`, so the key has to
        // match even though "colours" reads oddly as a category.
        key: "general:col",
        label: "Border colours",
        help: "Which colour each window's border is drawn in.",
    },
    Category {
        key: "general:snap",
        label: "Snapping",
        help: "How floating windows snap to each other and to screen edges.",
    },
    Category {
        key: "decoration",
        label: "Window appearance",
        help: "Corners, opacity and dimming.",
    },
    Category {
        key: "decoration:blur",
        label: "Blur",
        help: "Background blur behind translucent windows. The most expensive effect here.",
    },
    Category {
        key: "decoration:shadow",
        label: "Shadow",
        help: "Drop shadows behind windows.",
    },
    Category {
        key: "decoration:glow",
        label: "Glow",
        help: "An inner glow on window edges.",
    },
    Category {
        key: "decoration:motion_blur",
        label: "Motion blur",
        help: "Blur while windows are moving or resizing.",
    },
    Category {
        key: "dwindle",
        label: "Dwindle layout",
        help: "How the dwindle layout splits. Only applies while it's the active layout.",
    },
    Category {
        key: "master",
        label: "Master layout",
        help: "How the master layout arranges windows. Only applies while it's the active layout.",
    },
    Category {
        key: "group",
        label: "Window groups",
        help: "Tabbed groups of windows, and how they merge.",
    },
    Category {
        key: "group:col",
        label: "Group border colours",
        help: "Border colours for grouped windows.",
    },
    Category {
        key: "group:groupbar",
        label: "Group bar",
        help: "The tab strip drawn above a group.",
    },
    Category {
        key: "group:groupbar:col",
        label: "Group bar colours",
        help: "Tab backgrounds. Only drawn with gradient backgrounds turned on.",
    },
    Category {
        key: "cursor",
        label: "Cursor",
        help: "How the pointer is drawn and when it hides.",
    },
];

/// Everything this module claims about Hyprland's appearance surface.
pub const CATALOG: Catalog = Catalog {
    settings: SETTINGS,
    categories: CATEGORIES,
    unsupported: &[
        (
            "decoration:wobble",
            "Wobbly windows aren't in Hyprland 0.56 — the wiki documents them ahead of the release.",
        ),
        (
            "group:groupbar:font_weight",
            "Group bar font weight isn't in Hyprland 0.56 — the wiki documents it ahead of the release.",
        ),
        (
            "decoration:shadow:offset",
            "A shadow offset is a pair of numbers, which isn't editable here yet.",
        ),
    ],
};

pub fn get(key: &str) -> Option<&'static Setting> {
    CATALOG.get(key)
}

const LAYOUTS: &[&str] = &["dwindle", "master", "scrolling", "monocle"];

const RESIZE_CORNER: &[(i64, &str)] = &[
    (0, "Whichever corner is nearest"),
    (1, "Top left"),
    (2, "Top right"),
    (3, "Bottom right"),
    (4, "Bottom left"),
];

const FORCE_SPLIT: &[(i64, &str)] = &[
    (0, "Wherever the mouse is"),
    (1, "Always left or top"),
    (2, "Always right or bottom"),
];

const SPLIT_BIAS: &[(i64, &str)] = &[
    (0, "The top or left window"),
    (1, "The window being split"),
];

const DRAG_INTO_GROUP: &[(i64, &str)] = &[
    (0, "Never"),
    (1, "Anywhere on the group"),
    (2, "Only on the tab strip"),
];

const ORIENTATIONS: &[&str] = &["left", "right", "top", "bottom", "center"];
const FALLBACK_SIDES: &[&str] = &["left", "right", "top", "bottom"];
const NEW_STATUS: &[&str] = &["master", "slave", "inherit"];
const NEW_ON_ACTIVE: &[&str] = &["none", "before", "after"];

const HW_CURSORS: &[(i64, &str)] = &[
    (0, "Use hardware cursors"),
    (1, "Never use hardware cursors"),
    (2, "Automatic"),
];

const FS_VRR: &[(i64, &str)] = &[(0, "Off"), (1, "On"), (2, "Automatic (on for games)")];

const WARP: &[(i64, &str)] = &[
    (0, "Don't move the cursor"),
    (1, "Move it to the focused window"),
    (2, "Move it even if warping is off"),
];

const CPU_BUFFER: &[(i64, &str)] = &[(0, "Off"), (1, "On"), (2, "Automatic (Nvidia only)")];

pub const SETTINGS: &[Setting] = &[
    // ---- general ----
    Setting {
        key: "general:gaps_in",
        label: "Gaps between windows",
        help: "Space between neighbouring windows, in pixels.",
        kind: Kind::Gaps { default: 5, min: Some(0), max: Some(200) },
    },
    Setting {
        key: "general:gaps_out",
        label: "Gaps at screen edges",
        help: "Space between windows and the edge of the screen, in pixels.",
        kind: Kind::Gaps { default: 20, min: Some(0), max: Some(400) },
    },
    Setting {
        key: "general:float_gaps",
        label: "Gaps for floating windows",
        help: "Edge spacing for floating windows specifically. 0 uses the same as tiled.",
        kind: Kind::Gaps { default: 0, min: Some(0), max: Some(400) },
    },
    Setting {
        key: "general:gaps_workspaces",
        label: "Gaps between workspaces",
        help: "Extra space between workspaces while sliding between them.",
        kind: Kind::Int { default: 0, min: Some(0), max: Some(400) },
    },
    Setting {
        key: "general:border_size",
        label: "Border width",
        help: "Thickness of the border drawn around each window, in pixels.",
        kind: Kind::Int { default: 1, min: Some(0), max: Some(50) },
    },
    Setting {
        key: "general:col:active_border",
        label: "Active border colour",
        help: "Border colour of the focused window.",
        kind: Kind::Color { default: "rgba(ffffffff)" },
    },
    Setting {
        key: "general:col:inactive_border",
        label: "Inactive border colour",
        help: "Border colour of every unfocused window.",
        kind: Kind::Color { default: "rgba(444444ff)" },
    },
    Setting {
        key: "general:col:nogroup_border",
        label: "Ungroupable border colour",
        help: "Border colour of an unfocused window that refuses to join groups.",
        kind: Kind::Color { default: "rgba(ffaaffff)" },
    },
    Setting {
        key: "general:col:nogroup_border_active",
        label: "Ungroupable border colour (active)",
        help: "Border colour of a focused window that refuses to join groups.",
        kind: Kind::Color { default: "rgba(ff00ffff)" },
    },
    Setting {
        key: "general:layout",
        label: "Tiling layout",
        help: "How windows are arranged: dwindle splits, master keeps one main window, scrolling pans sideways, monocle shows one at a time.",
        kind: Kind::TextEnum { default: "dwindle", choices: LAYOUTS },
    },
    Setting {
        key: "general:resize_on_border",
        label: "Resize by dragging borders",
        help: "Click and drag a window's border or gap to resize it.",
        kind: Kind::Bool { default: false },
    },
    Setting {
        key: "general:extend_border_grab_area",
        label: "Border grab area",
        help: "How far outside a border counts as grabbing it, in pixels. Only with border resizing on.",
        kind: Kind::Int { default: 15, min: Some(0), max: Some(100) },
    },
    Setting {
        key: "general:hover_icon_on_border",
        label: "Resize cursor on borders",
        help: "Show a resize pointer when hovering a border. Only with border resizing on.",
        kind: Kind::Bool { default: true },
    },
    Setting {
        key: "general:resize_corner",
        label: "Floating resize corner",
        help: "Force floating windows to resize from one corner.",
        kind: Kind::IntEnum { default: 0, choices: RESIZE_CORNER },
    },
    Setting {
        key: "general:no_focus_fallback",
        label: "Don't fall back when focusing",
        help: "Moving focus in a direction with no window there does nothing, instead of jumping to the next window.",
        kind: Kind::Bool { default: false },
    },
    Setting {
        key: "general:modal_parent_blocking",
        label: "Modals block their parent",
        help: "A modal dialog's parent window stays uninteractive while it's open.",
        kind: Kind::Bool { default: true },
    },
    Setting {
        key: "general:allow_tearing",
        label: "Allow tearing",
        help: "Lets games bypass vsync for lower latency, at the cost of visible tearing. Also needs a window rule.",
        kind: Kind::Bool { default: false },
    },
    // ---- general:snap ----
    Setting {
        key: "general:snap:enabled",
        label: "Snap floating windows",
        help: "Floating windows stick to each other and to screen edges when dragged close.",
        kind: Kind::Bool { default: false },
    },
    Setting {
        key: "general:snap:window_gap",
        label: "Snap distance to windows",
        help: "How close another window must be before snapping, in pixels.",
        kind: Kind::Int { default: 10, min: Some(0), max: Some(200) },
    },
    Setting {
        key: "general:snap:monitor_gap",
        label: "Snap distance to edges",
        help: "How close a screen edge must be before snapping, in pixels.",
        kind: Kind::Int { default: 10, min: Some(0), max: Some(200) },
    },
    Setting {
        key: "general:snap:border_overlap",
        label: "Overlap borders when snapped",
        help: "Snapped windows share one border's width instead of showing both.",
        kind: Kind::Bool { default: false },
    },
    Setting {
        key: "general:snap:respect_gaps",
        label: "Keep gaps when snapped",
        help: "Snapping leaves the usual window gap rather than butting windows together.",
        kind: Kind::Bool { default: false },
    },
    // ---- decoration ----
    Setting {
        key: "decoration:rounding",
        label: "Corner radius",
        help: "How rounded window corners are, in pixels. 0 for square.",
        kind: Kind::Int { default: 0, min: Some(0), max: Some(50) },
    },
    Setting {
        key: "decoration:rounding_power",
        label: "Corner shape",
        help: "2.0 is a circular corner, higher is squarer — a squircle.",
        kind: Kind::Float { default: 2.0, min: Some(2.0), max: Some(10.0) },
    },
    Setting {
        key: "decoration:active_opacity",
        label: "Active window opacity",
        help: "Opacity of the focused window, 0.0 to 1.0.",
        kind: Kind::Float { default: 1.0, min: Some(0.0), max: Some(1.0) },
    },
    Setting {
        key: "decoration:inactive_opacity",
        label: "Inactive window opacity",
        help: "Opacity of unfocused windows, 0.0 to 1.0.",
        kind: Kind::Float { default: 1.0, min: Some(0.0), max: Some(1.0) },
    },
    Setting {
        key: "decoration:fullscreen_opacity",
        label: "Fullscreen opacity",
        help: "Opacity of fullscreen windows, 0.0 to 1.0.",
        kind: Kind::Float { default: 1.0, min: Some(0.0), max: Some(1.0) },
    },
    Setting {
        key: "decoration:dim_inactive",
        label: "Dim inactive windows",
        help: "Darken every window except the focused one.",
        kind: Kind::Bool { default: false },
    },
    Setting {
        key: "decoration:dim_strength",
        label: "Dim amount",
        help: "How much inactive windows are darkened, 0.0 to 1.0.",
        kind: Kind::Float { default: 0.5, min: Some(0.0), max: Some(1.0) },
    },
    Setting {
        key: "decoration:dim_special",
        label: "Special workspace dim",
        help: "How much the rest of the screen darkens while a special workspace is open.",
        kind: Kind::Float { default: 0.2, min: Some(0.0), max: Some(1.0) },
    },
    Setting {
        key: "decoration:dim_around",
        label: "Dim-around amount",
        help: "How much the `dim_around` window rule darkens by.",
        kind: Kind::Float { default: 0.4, min: Some(0.0), max: Some(1.0) },
    },
    Setting {
        key: "decoration:dim_modal",
        label: "Dim behind modals",
        help: "Darken a window while one of its modal dialogs is open.",
        kind: Kind::Bool { default: true },
    },
    Setting {
        key: "decoration:border_part_of_window",
        label: "Border counts as window",
        help: "Whether the border is inside the window's own area for layout purposes.",
        kind: Kind::Bool { default: true },
    },
    Setting {
        key: "decoration:screen_shader",
        label: "Screen shader",
        help: "Path to a GLSL fragment shader applied to the whole screen. Empty for none.",
        kind: Kind::Text { default: "" },
    },
    // ---- decoration:blur ----
    Setting {
        key: "decoration:blur:enabled",
        label: "Blur enabled",
        help: "Blur whatever is behind translucent windows.",
        kind: Kind::Bool { default: true },
    },
    Setting {
        key: "decoration:blur:size",
        label: "Blur size",
        help: "How far the blur reaches. Larger needs more passes to look right.",
        kind: Kind::Int { default: 8, min: Some(1), max: Some(50) },
    },
    Setting {
        key: "decoration:blur:passes",
        label: "Blur passes",
        help: "How many times to blur. More is smoother and markedly more GPU work.",
        kind: Kind::Int { default: 1, min: Some(1), max: Some(10) },
    },
    Setting {
        key: "decoration:blur:noise",
        label: "Blur noise",
        help: "Grain added to the blur to hide banding, 0.0 to 1.0.",
        kind: Kind::Float { default: 0.0117, min: Some(0.0), max: Some(1.0) },
    },
    Setting {
        key: "decoration:blur:contrast",
        label: "Blur contrast",
        help: "Contrast of the blurred image, 0.0 to 2.0.",
        kind: Kind::Float { default: 0.8916, min: Some(0.0), max: Some(2.0) },
    },
    Setting {
        key: "decoration:blur:brightness",
        label: "Blur brightness",
        help: "Brightness of the blurred image, 0.0 to 2.0.",
        kind: Kind::Float { default: 1.0, min: Some(0.0), max: Some(2.0) },
    },
    Setting {
        key: "decoration:blur:vibrancy",
        label: "Blur vibrancy",
        help: "Saturation boost for blurred colours, 0.0 to 1.0.",
        kind: Kind::Float { default: 0.1696, min: Some(0.0), max: Some(1.0) },
    },
    Setting {
        key: "decoration:blur:vibrancy_darkness",
        label: "Vibrancy in dark areas",
        help: "How strongly vibrancy applies to dark parts of the image.",
        kind: Kind::Float { default: 0.0, min: Some(0.0), max: Some(1.0) },
    },
    Setting {
        key: "decoration:blur:ignore_opacity",
        label: "Ignore window opacity",
        help: "Blur at full strength regardless of how transparent the window is.",
        kind: Kind::Bool { default: true },
    },
    Setting {
        key: "decoration:blur:new_optimizations",
        label: "Blur optimisations",
        help: "Substantially faster blur. Recommended on.",
        kind: Kind::Bool { default: true },
    },
    Setting {
        key: "decoration:blur:xray",
        label: "Floating windows see through",
        help: "Floating windows blur the desktop rather than the tiled windows beneath them. Needs optimisations on.",
        kind: Kind::Bool { default: false },
    },
    Setting {
        key: "decoration:blur:special",
        label: "Blur special workspaces",
        help: "Blur behind the special workspace. Expensive.",
        kind: Kind::Bool { default: false },
    },
    Setting {
        key: "decoration:blur:popups",
        label: "Blur popups",
        help: "Blur behind menus and other popups.",
        kind: Kind::Bool { default: false },
    },
    Setting {
        key: "decoration:blur:popups_ignorealpha",
        label: "Popup blur threshold",
        help: "Pixels more transparent than this aren't blurred, 0.0 to 1.0.",
        kind: Kind::Float { default: 0.2, min: Some(0.0), max: Some(1.0) },
    },
    Setting {
        key: "decoration:blur:input_methods",
        label: "Blur input methods",
        help: "Blur behind on-screen input method popups.",
        kind: Kind::Bool { default: false },
    },
    Setting {
        key: "decoration:blur:input_methods_ignorealpha",
        label: "Input method blur threshold",
        help: "Pixels more transparent than this aren't blurred, 0.0 to 1.0.",
        kind: Kind::Float { default: 0.2, min: Some(0.0), max: Some(1.0) },
    },
    // ---- decoration:shadow ----
    Setting {
        key: "decoration:shadow:enabled",
        label: "Shadow enabled",
        help: "Draw a drop shadow behind windows.",
        kind: Kind::Bool { default: true },
    },
    Setting {
        key: "decoration:shadow:range",
        label: "Shadow size",
        help: "How far the shadow extends, in pixels.",
        kind: Kind::Int { default: 4, min: Some(0), max: Some(100) },
    },
    Setting {
        key: "decoration:shadow:render_power",
        label: "Shadow falloff",
        help: "How sharply the shadow fades out. Higher fades faster.",
        kind: Kind::Int { default: 3, min: Some(1), max: Some(4) },
    },
    Setting {
        key: "decoration:shadow:sharp",
        label: "Sharp shadow",
        help: "No fade at all — a hard-edged shadow.",
        kind: Kind::Bool { default: false },
    },
    Setting {
        key: "decoration:shadow:scale",
        label: "Shadow scale",
        help: "Size of the shadow relative to the window, 0.0 to 1.0.",
        kind: Kind::Float { default: 1.0, min: Some(0.0), max: Some(1.0) },
    },
    Setting {
        key: "decoration:shadow:color",
        label: "Shadow colour",
        help: "Shadow colour; its alpha sets how dark the shadow is.",
        kind: Kind::Color { default: "rgba(1a1a1aee)" },
    },
    Setting {
        key: "decoration:shadow:color_inactive",
        label: "Inactive shadow colour",
        help: "Shadow colour for unfocused windows. Falls back to the shadow colour when unset.",
        kind: Kind::Color { default: "rgba(1a1a1aee)" },
    },
    // ---- decoration:glow ----
    Setting {
        key: "decoration:glow:enabled",
        label: "Glow enabled",
        help: "Draw an inner glow along window edges.",
        kind: Kind::Bool { default: false },
    },
    Setting {
        key: "decoration:glow:range",
        label: "Glow size",
        help: "How far the glow reaches inward, in pixels.",
        kind: Kind::Int { default: 10, min: Some(0), max: Some(100) },
    },
    Setting {
        key: "decoration:glow:render_power",
        label: "Glow falloff",
        help: "How sharply the glow fades. Higher fades faster.",
        kind: Kind::Int { default: 3, min: Some(1), max: Some(4) },
    },
    Setting {
        key: "decoration:glow:color",
        label: "Glow colour",
        help: "Glow colour; its alpha sets the intensity.",
        kind: Kind::Color { default: "rgba(33ccffee)" },
    },
    Setting {
        key: "decoration:glow:color_inactive",
        label: "Inactive glow colour",
        help: "Glow colour for unfocused windows. Falls back to the glow colour when unset.",
        kind: Kind::Color { default: "rgba(33ccffee)" },
    },
    // ---- decoration:motion_blur ----
    Setting {
        key: "decoration:motion_blur:enabled",
        label: "Motion blur enabled",
        help: "Blur windows while they move or resize.",
        kind: Kind::Bool { default: false },
    },
    Setting {
        key: "decoration:motion_blur:samples",
        label: "Motion blur samples",
        help: "How many samples to render. More is smoother and more GPU work.",
        kind: Kind::Int { default: 7, min: Some(1), max: Some(32) },
    },
    // ---- dwindle ----
    Setting {
        key: "dwindle:force_split",
        label: "Where new windows go",
        help: "Which side a new window takes when a window is split.",
        kind: Kind::IntEnum { default: 0, choices: FORCE_SPLIT },
    },
    Setting {
        key: "dwindle:preserve_split",
        label: "Keep split direction",
        help: "A split stays horizontal or vertical whatever happens to the windows around it.",
        kind: Kind::Bool { default: false },
    },
    Setting {
        key: "dwindle:smart_split",
        label: "Split by cursor corner",
        help: "The quarter of the window your cursor is in decides the split direction. Turns on \"keep split direction\" as well.",
        kind: Kind::Bool { default: false },
    },
    Setting {
        key: "dwindle:smart_resizing",
        label: "Resize toward the nearest corner",
        help: "Resizing follows the corner your mouse is closest to, rather than the window's place in the tree.",
        kind: Kind::Bool { default: true },
    },
    Setting {
        key: "dwindle:use_active_for_splits",
        label: "Split from the active window",
        help: "Prefer the focused window over the mouse position when deciding where to split.",
        kind: Kind::Bool { default: true },
    },
    Setting {
        key: "dwindle:default_split_ratio",
        label: "Default split ratio",
        help: "How the space is divided on a new split. 1.0 is an even 50/50.",
        kind: Kind::Float { default: 1.0, min: Some(0.1), max: Some(1.9) },
    },
    Setting {
        key: "dwindle:split_width_multiplier",
        label: "Split width multiplier",
        help: "Biases splits wider. Useful on ultrawide screens where windows stay wider than tall.",
        kind: Kind::Float { default: 1.0, min: Some(0.1), max: Some(10.0) },
    },
    Setting {
        key: "dwindle:split_bias",
        label: "Which window keeps the ratio",
        help: "Which side of a new split gets the larger share.",
        kind: Kind::IntEnum { default: 0, choices: SPLIT_BIAS },
    },
    Setting {
        key: "dwindle:special_scale_factor",
        label: "Special workspace scale",
        help: "How much windows shrink on the special workspace.",
        kind: Kind::Float { default: 1.0, min: Some(0.0), max: Some(1.0) },
    },
    Setting {
        key: "dwindle:permanent_direction_override",
        label: "Keep the preselected direction",
        help: "A preselected split direction stays until it's changed, instead of resetting after one window.",
        kind: Kind::Bool { default: false },
    },
    Setting {
        key: "dwindle:precise_mouse_move",
        label: "Precise drag placement",
        help: "Dragging a window drops it exactly where the mouse is.",
        kind: Kind::Bool { default: false },
    },
    // ---- master ----
    Setting {
        key: "master:mfact",
        label: "Master size",
        help: "How much of the screen the master window takes, as a fraction. 0.55 is just over half.",
        kind: Kind::Float { default: 0.55, min: Some(0.0), max: Some(1.0) },
    },
    Setting {
        key: "master:orientation",
        label: "Master position",
        help: "Which side of the screen the master window occupies.",
        kind: Kind::TextEnum { default: "left", choices: ORIENTATIONS },
    },
    Setting {
        key: "master:new_status",
        label: "New windows become",
        help: "Whether a new window takes the master slot or joins the stack.",
        kind: Kind::TextEnum { default: "slave", choices: NEW_STATUS },
    },
    Setting {
        key: "master:new_on_top",
        label: "Add to the top of the stack",
        help: "A new window goes to the top of the stack rather than the bottom.",
        kind: Kind::Bool { default: false },
    },
    Setting {
        key: "master:new_on_active",
        label: "Place relative to the focused window",
        help: "Put a new window before or after the focused one, instead of by the setting above.",
        kind: Kind::TextEnum { default: "none", choices: NEW_ON_ACTIVE },
    },
    Setting {
        key: "master:slave_count_for_center_master",
        label: "Centre master after",
        help: "With centre orientation, how many stacked windows before the master centres. 0 always centres it.",
        kind: Kind::Int { default: 2, min: Some(0), max: Some(20) },
    },
    Setting {
        key: "master:center_master_fallback",
        label: "Centre master falls back to",
        help: "Where the master sits while there are too few stacked windows to centre it.",
        kind: Kind::TextEnum { default: "left", choices: FALLBACK_SIDES },
    },
    Setting {
        key: "master:allow_small_split",
        label: "Allow extra master windows",
        help: "More than one master window, split horizontally.",
        kind: Kind::Bool { default: false },
    },
    Setting {
        key: "master:smart_resizing",
        label: "Resize toward the nearest corner",
        help: "Resizing follows the corner your mouse is closest to.",
        kind: Kind::Bool { default: true },
    },
    Setting {
        key: "master:drop_at_cursor",
        label: "Drop where the cursor is",
        help: "A dragged window lands at the cursor rather than at the end of the stack.",
        kind: Kind::Bool { default: true },
    },
    Setting {
        key: "master:always_keep_position",
        label: "Keep master in place when alone",
        help: "The master window stays in its configured position even with nothing stacked beside it.",
        kind: Kind::Bool { default: false },
    },
    Setting {
        key: "master:focus_master_on_close",
        label: "Focus master when a window closes",
        help: "Closing a window moves focus to the master rather than to a neighbour.",
        kind: Kind::Bool { default: false },
    },
    Setting {
        key: "master:special_scale_factor",
        label: "Special workspace scale",
        help: "How much windows shrink on the special workspace.",
        kind: Kind::Float { default: 1.0, min: Some(0.0), max: Some(1.0) },
    },
    // ---- group ----
    Setting {
        key: "group:auto_group",
        label: "Group new windows automatically",
        help: "A new window joins the focused group instead of tiling beside it.",
        kind: Kind::Bool { default: true },
    },
    Setting {
        key: "group:insert_after_current",
        label: "Insert after the current tab",
        help: "A new window joins next to the current tab rather than at the end.",
        kind: Kind::Bool { default: true },
    },
    Setting {
        key: "group:focus_removed_window",
        label: "Follow a window out of its group",
        help: "Focus moves with a window that's just been pulled out of a group.",
        kind: Kind::Bool { default: true },
    },
    Setting {
        key: "group:drag_into_group",
        label: "Drag windows into groups",
        help: "Whether dropping a window on a group merges it in.",
        kind: Kind::IntEnum { default: 1, choices: DRAG_INTO_GROUP },
    },
    Setting {
        key: "group:merge_groups_on_drag",
        label: "Merge groups by dragging",
        help: "Dragging one group onto another combines them.",
        kind: Kind::Bool { default: true },
    },
    Setting {
        key: "group:merge_groups_on_groupbar",
        label: "Merge onto the group bar",
        help: "Dropping a group on another's tab strip merges them. Needs the two settings above.",
        kind: Kind::Bool { default: true },
    },
    Setting {
        key: "group:merge_floated_into_tiled_on_groupbar",
        label: "Merge floating onto the group bar",
        help: "Dropping a floating window on a tiled window's tab strip merges it in.",
        kind: Kind::Bool { default: false },
    },
    Setting {
        key: "group:group_on_movetoworkspace",
        label: "Group when moved to a workspace",
        help: "Moving a window to a workspace merges it into the group already there.",
        kind: Kind::Bool { default: false },
    },
    // ---- group:col ----
    Setting {
        key: "group:col:border_active",
        label: "Active group border",
        help: "Border colour of the focused group.",
        kind: Kind::Color { default: "rgba(ffff0066)" },
    },
    Setting {
        key: "group:col:border_inactive",
        label: "Inactive group border",
        help: "Border colour of unfocused groups.",
        kind: Kind::Color { default: "rgba(77770066)" },
    },
    Setting {
        key: "group:col:border_locked_active",
        label: "Active locked group border",
        help: "Border colour of a focused group that won't accept new windows.",
        kind: Kind::Color { default: "rgba(ff550066)" },
    },
    Setting {
        key: "group:col:border_locked_inactive",
        label: "Inactive locked group border",
        help: "Border colour of an unfocused locked group.",
        kind: Kind::Color { default: "rgba(77550066)" },
    },
    // ---- group:groupbar ----
    //
    // `font_weight_active` and `font_weight_inactive` are absent: the
    // wiki documents them with a `font_weight` type, and Hyprland 0.56.1
    // answers "no such option" for both.
    Setting {
        key: "group:groupbar:enabled",
        label: "Show the group bar",
        help: "Draw a tab strip above grouped windows.",
        kind: Kind::Bool { default: true },
    },
    Setting {
        key: "group:groupbar:disable_when_only",
        label: "Hide it for a single window",
        help: "No tab strip while a group holds only one window.",
        kind: Kind::Bool { default: false },
    },
    Setting {
        key: "group:groupbar:stacked",
        label: "Stack tabs vertically",
        help: "Draw the tabs stacked rather than in a row.",
        kind: Kind::Bool { default: false },
    },
    Setting {
        key: "group:groupbar:height",
        label: "Height",
        help: "Height of the tab strip, in pixels.",
        kind: Kind::Int { default: 14, min: Some(1), max: Some(200) },
    },
    Setting {
        key: "group:groupbar:indicator_height",
        label: "Indicator height",
        help: "Height of the bar marking the active tab.",
        kind: Kind::Int { default: 3, min: Some(0), max: Some(100) },
    },
    Setting {
        key: "group:groupbar:indicator_gap",
        label: "Indicator gap",
        help: "Space between the indicator and the title.",
        kind: Kind::Int { default: 0, min: Some(0), max: Some(100) },
    },
    Setting {
        key: "group:groupbar:render_titles",
        label: "Show window titles",
        help: "Write each window's title in its tab.",
        kind: Kind::Bool { default: true },
    },
    Setting {
        key: "group:groupbar:font_family",
        label: "Font",
        help: "Font for tab titles. Empty uses the general interface font.",
        kind: Kind::Text { default: "" },
    },
    Setting {
        key: "group:groupbar:font_size",
        label: "Font size",
        help: "Size of tab titles.",
        kind: Kind::Int { default: 8, min: Some(1), max: Some(100) },
    },
    Setting {
        key: "group:groupbar:text_offset",
        label: "Title vertical offset",
        help: "Nudge titles up or down within the tab.",
        kind: Kind::Int { default: 0, min: Some(-50), max: Some(50) },
    },
    Setting {
        key: "group:groupbar:text_padding",
        label: "Title horizontal padding",
        help: "Space either side of a title.",
        kind: Kind::Int { default: 0, min: Some(0), max: Some(100) },
    },
    Setting {
        key: "group:groupbar:text_color",
        label: "Title colour",
        help: "Colour of window titles in the tab strip.",
        kind: Kind::ColorInt { default: "rgba(ffffffff)" },
    },
    Setting {
        key: "group:groupbar:text_color_inactive",
        label: "Inactive title colour",
        help: "Title colour for unfocused tabs. Falls back to the title colour when unset.",
        kind: Kind::ColorInt { default: "rgba(ffffffff)" },
    },
    Setting {
        key: "group:groupbar:text_color_locked_active",
        label: "Locked active title colour",
        help: "Title colour for the focused tab of a locked group.",
        kind: Kind::ColorInt { default: "rgba(ffffffff)" },
    },
    Setting {
        key: "group:groupbar:text_color_locked_inactive",
        label: "Locked inactive title colour",
        help: "Title colour for unfocused tabs of a locked group.",
        kind: Kind::ColorInt { default: "rgba(ffffffff)" },
    },
    Setting {
        key: "group:groupbar:gradients",
        label: "Gradient backgrounds",
        help: "Fill tabs with a colour rather than drawing only the indicator.",
        kind: Kind::Bool { default: false },
    },
    Setting {
        key: "group:groupbar:col:active",
        label: "Active tab background",
        help: "Background of the focused tab. Needs gradient backgrounds on.",
        kind: Kind::Color { default: "rgba(ffff0066)" },
    },
    Setting {
        key: "group:groupbar:col:inactive",
        label: "Inactive tab background",
        help: "Background of unfocused tabs.",
        kind: Kind::Color { default: "rgba(77770066)" },
    },
    Setting {
        key: "group:groupbar:col:locked_active",
        label: "Locked active tab background",
        help: "Background of the focused tab in a locked group.",
        kind: Kind::Color { default: "rgba(ff550066)" },
    },
    Setting {
        key: "group:groupbar:col:locked_inactive",
        label: "Locked inactive tab background",
        help: "Background of unfocused tabs in a locked group.",
        kind: Kind::Color { default: "rgba(77550066)" },
    },
    Setting {
        key: "group:groupbar:rounding",
        label: "Indicator corner radius",
        help: "How rounded the active-tab indicator is.",
        kind: Kind::Int { default: 1, min: Some(0), max: Some(50) },
    },
    Setting {
        key: "group:groupbar:rounding_power",
        label: "Indicator corner shape",
        help: "2.0 is a circular corner, higher is squarer.",
        kind: Kind::Float { default: 2.0, min: Some(2.0), max: Some(10.0) },
    },
    Setting {
        key: "group:groupbar:gradient_rounding",
        label: "Background corner radius",
        help: "How rounded the tab backgrounds are.",
        kind: Kind::Int { default: 2, min: Some(0), max: Some(50) },
    },
    Setting {
        key: "group:groupbar:gradient_rounding_power",
        label: "Background corner shape",
        help: "2.0 is a circular corner, higher is squarer.",
        kind: Kind::Float { default: 2.0, min: Some(2.0), max: Some(10.0) },
    },
    Setting {
        key: "group:groupbar:round_only_edges",
        label: "Round only the outer edges",
        help: "Round the ends of the indicator strip rather than every tab.",
        kind: Kind::Bool { default: true },
    },
    Setting {
        key: "group:groupbar:gradient_round_only_edges",
        label: "Round only the outer backgrounds",
        help: "Round the ends of the background strip rather than every tab.",
        kind: Kind::Bool { default: true },
    },
    Setting {
        key: "group:groupbar:gaps_in",
        label: "Gap between tabs",
        help: "Space between neighbouring tab backgrounds.",
        kind: Kind::Int { default: 2, min: Some(0), max: Some(50) },
    },
    Setting {
        key: "group:groupbar:gaps_out",
        label: "Gap below the tabs",
        help: "Space between the tab strip and the window.",
        kind: Kind::Int { default: 2, min: Some(0), max: Some(50) },
    },
    Setting {
        key: "group:groupbar:keep_upper_gap",
        label: "Gap above the tabs",
        help: "Leave space above the tab strip as well as below.",
        kind: Kind::Bool { default: true },
    },
    Setting {
        key: "group:groupbar:blur",
        label: "Blur behind the tabs",
        help: "Apply the blur effect to tab backgrounds.",
        kind: Kind::Bool { default: false },
    },
    Setting {
        key: "group:groupbar:scrolling",
        label: "Scroll to change tab",
        help: "Scrolling over the tab strip switches which window is shown.",
        kind: Kind::Bool { default: true },
    },
    Setting {
        key: "group:groupbar:middle_click_close",
        label: "Middle-click to close",
        help: "Middle-clicking a tab closes that window.",
        kind: Kind::Bool { default: true },
    },
    Setting {
        key: "group:groupbar:priority",
        label: "Decoration priority",
        help: "Where the tab strip sits relative to other window decorations.",
        kind: Kind::Int { default: 3, min: Some(0), max: Some(10) },
    },
    // ---- cursor ----
    Setting {
        key: "cursor:inactive_timeout",
        label: "Hide after",
        help: "Seconds of stillness before the cursor hides. 0 never hides it.",
        kind: Kind::Float { default: 0.0, min: Some(0.0), max: Some(600.0) },
    },
    Setting {
        key: "cursor:hide_on_key_press",
        label: "Hide while typing",
        help: "Hide the cursor on any keypress until the mouse moves again.",
        kind: Kind::Bool { default: false },
    },
    Setting {
        key: "cursor:hide_on_touch",
        label: "Hide after touch",
        help: "Hide the cursor after touch input until a mouse is used.",
        kind: Kind::Bool { default: true },
    },
    Setting {
        key: "cursor:hide_on_tablet",
        label: "Hide after tablet",
        help: "Hide the cursor after tablet input until a mouse is used.",
        kind: Kind::Bool { default: false },
    },
    Setting {
        key: "cursor:invisible",
        label: "Never draw the cursor",
        help: "Don't render a cursor at all. Easy to lose track of the pointer.",
        kind: Kind::Bool { default: false },
    },
    Setting {
        key: "cursor:enable_hyprcursor",
        label: "Use hyprcursor themes",
        help: "Support hyprcursor themes, which scale better than xcursor.",
        kind: Kind::Bool { default: true },
    },
    Setting {
        key: "cursor:sync_gsettings_theme",
        label: "Share cursor theme with GTK apps",
        help: "Push the cursor theme and size to gsettings so GTK apps match.",
        kind: Kind::Bool { default: true },
    },
    Setting {
        key: "cursor:no_hardware_cursors",
        label: "Hardware cursors",
        help: "Hardware cursors are smoother; turning them off fixes some screen-capture and Nvidia issues.",
        kind: Kind::IntEnum { default: 2, choices: HW_CURSORS },
    },
    Setting {
        key: "cursor:use_cpu_buffer",
        label: "CPU buffer for cursors",
        help: "Required for hardware cursors on Nvidia.",
        kind: Kind::IntEnum { default: 2, choices: CPU_BUFFER },
    },
    Setting {
        key: "cursor:no_break_fs_vrr",
        label: "Steady frames for VRR fullscreen",
        help: "Stop cursor movement from spiking the framerate in VRR fullscreen apps.",
        kind: Kind::IntEnum { default: 2, choices: FS_VRR },
    },
    Setting {
        key: "cursor:min_refresh_rate",
        label: "Minimum refresh rate",
        help: "Lowest refresh rate to hold while the above is active, in Hz.",
        kind: Kind::Int { default: 24, min: Some(1), max: Some(500) },
    },
    Setting {
        key: "cursor:hotspot_padding",
        label: "Edge padding",
        help: "Keep the cursor this many pixels away from screen edges.",
        kind: Kind::Int { default: 0, min: Some(0), max: Some(100) },
    },
    Setting {
        key: "cursor:no_warps",
        label: "Never move the cursor",
        help: "Stop Hyprland repositioning the pointer when focus changes.",
        kind: Kind::Bool { default: false },
    },
    Setting {
        key: "cursor:persistent_warps",
        label: "Remember position per window",
        help: "Refocusing a window returns the cursor where it was in that window, not to its centre.",
        kind: Kind::Bool { default: false },
    },
    Setting {
        key: "cursor:warp_on_change_workspace",
        label: "Move on workspace change",
        help: "Move the cursor to the focused window after switching workspace.",
        kind: Kind::IntEnum { default: 0, choices: WARP },
    },
    Setting {
        key: "cursor:warp_on_toggle_special",
        label: "Move on special workspace",
        help: "Move the cursor to the focused window when toggling a special workspace.",
        kind: Kind::IntEnum { default: 0, choices: WARP },
    },
    Setting {
        key: "cursor:warp_back_after_non_mouse_input",
        label: "Return after keyboard focus",
        help: "Put the cursor back where it was once you use the mouse again.",
        kind: Kind::Bool { default: false },
    },
    Setting {
        key: "cursor:default_monitor",
        label: "Startup monitor",
        help: "Which monitor the cursor starts on, by name. Empty for automatic.",
        kind: Kind::Text { default: "" },
    },
    Setting {
        key: "cursor:zoom_factor",
        label: "Zoom",
        help: "Magnify the screen around the cursor. 1.0 is no zoom.",
        kind: Kind::Float { default: 1.0, min: Some(1.0), max: Some(20.0) },
    },
    Setting {
        key: "cursor:zoom_rigid",
        label: "Rigid zoom",
        help: "Keep the cursor centred while zoomed, instead of letting the view lag behind.",
        kind: Kind::Bool { default: false },
    },
    Setting {
        key: "cursor:zoom_detached_camera",
        label: "Detached zoom camera",
        help: "The zoomed view only follows the cursor when it reaches the edge.",
        kind: Kind::Bool { default: true },
    },
    Setting {
        key: "cursor:zoom_disable_aa",
        label: "Pixelated zoom",
        help: "Show sharp pixels when zoomed instead of a blurry image.",
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

    /// An empty category would render as a heading with nothing under it.
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

    /// A default outside its own declared range would make the editor open
    /// showing a value it then refuses to save.
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

    /// Every default must survive the validator, colours included — a
    /// malformed default would be refused the moment a row was touched.
    #[test]
    fn every_default_passes_validation() {
        use hyprforge_core::hlconfig::{Settings, Value};
        for s in SETTINGS {
            let stored = Settings::from_one(s.key, Value::default_for(&s.kind));
            assert_eq!(
                stored.validate(&CATALOG),
                vec![],
                "{} rejected its own default",
                s.key
            );
        }
    }

    /// The wiki documents `decoration:wobble`; Hyprland 0.56.1 has no such
    /// option. Catalogued, it would be eight settings that silently do
    /// nothing — so it's listed as unsupported instead, and this pins that
    /// it never quietly reappears in the settings list.
    #[test]
    fn wobble_is_declared_unsupported_rather_than_catalogued() {
        assert!(SETTINGS.iter().all(|s| !s.key.starts_with("decoration:wobble")));
        let reason = CATALOG.unknown_key_reason("decoration:wobble:enabled");
        assert!(reason.contains("0.56"), "{reason}");
    }
}
