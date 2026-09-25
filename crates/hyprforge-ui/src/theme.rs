use iced::Color;

/// Spacing scale shared by every Hyprforge app, so a sidebar, a settings
/// row, and a dialog all breathe the same amount no matter which app they're
/// in (vision pillar #2: one shared visual grammar).
pub mod spacing {
    /// Tightest gap: between a label and the control right beside it.
    pub const XS: f32 = 4.0;
    /// Between closely related elements within one row or group.
    pub const SM: f32 = 8.0;
    /// The default gap between rows and between a card's edge and its
    /// content — reach for this one unless a specific case argues
    /// otherwise.
    pub const MD: f32 = 16.0;
    /// Between distinct sections of a screen.
    pub const LG: f32 = 24.0;
    /// Between top-level regions — sidebar and content, page margins.
    pub const XL: f32 = 32.0;
}

/// Base text size in logical pixels before `FontScale` is applied.
pub const BASE_TEXT_SIZE: f32 = 14.0;

/// A single scale factor threaded through every text/spacing measurement in
/// the shared widget layer, so accessibility (larger text) is a property of
/// the theme rather than something bolted onto individual screens later.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct FontScale(pub f32);

impl Default for FontScale {
    fn default() -> Self {
        FontScale(1.0)
    }
}

impl FontScale {
    /// Scales a logical-pixel size by this factor. `base` is almost
    /// always [`BASE_TEXT_SIZE`] or a spacing constant, never a value a
    /// caller already scaled — applying this twice compounds instead of
    /// replacing.
    pub fn apply(&self, base: f32) -> f32 {
        base * self.0
    }
}

/// The theme every widget in this process draws from.
///
/// Process-global and read-mostly, set once at startup — the same
/// lifetime the font scale already had. The alternative, threading a
/// `&Theme` beside `FontScale` through some four hundred widget call
/// sites, buys nothing while there is one window per process.
///
/// When that stops being true — a second app wanting a per-window theme
/// — this becomes the fallback rather than the answer, and the widgets
/// take a `&Theme` argument. Nothing here has to change for that; the
/// call sites do.
static ACTIVE: std::sync::OnceLock<hyprforge_look::Theme> = std::sync::OnceLock::new();

/// Installs the resolved theme. Call once, before the first frame.
///
/// Later calls are ignored rather than rejected: a second call is a bug
/// in the caller, not a reason to bring down a window the user is
/// looking at.
pub fn init(theme: hyprforge_look::Theme) {
    let _ = ACTIVE.set(theme);
}

/// The active theme, or the default if nobody installed one.
///
/// Deliberately `get_or_init` rather than `expect`: an app that forgets
/// to call [`init`] renders exactly what it would have rendered before
/// any of this was configurable, instead of panicking on its first
/// frame.
pub fn active() -> &'static hyprforge_look::Theme {
    ACTIVE.get_or_init(hyprforge_look::Theme::default)
}

/// Explicit surface colors, used directly by the shared widget layer
/// instead of leaning on `Theme::extended_palette()`'s auto-derived
/// shades. Auto-derivation (`deviate(base, 0.1)` etc.) only shifts
/// lightness a few percent, which reads as "everything is the same flat
/// gray" at normal monitor brightness — real elevation (root vs sidebar
/// vs card) needs a deliberately chosen jump, the way macOS/Windows
/// layer system chrome vs content vs cards.
pub mod surface {
    use crate::color::to_iced;
    use iced::Color;

    /// Window root — behind everything.
    pub fn root() -> Color {
        to_iced(super::active().surfaces.root)
    }
    /// Sidebar — recessed a step darker than root.
    pub fn sidebar() -> Color {
        to_iced(super::active().surfaces.sidebar)
    }
    /// Card/section background — raised a step lighter than root.
    pub fn card() -> Color {
        to_iced(super::active().surfaces.card)
    }
    /// Card border.
    pub fn card_border() -> Color {
        to_iced(super::active().surfaces.card_border)
    }
    /// A list row's background on hover/emphasis, between root and card.
    pub fn row() -> Color {
        to_iced(super::active().surfaces.row)
    }
}

/// Primary text color (near-white, not pure white — easier on the eyes on
/// a dark surface).
pub fn text() -> Color {
    crate::color::to_iced(active().surfaces.text)
}

/// Secondary/meta text — timestamps, hints, IDs.
pub fn text_dim() -> Color {
    crate::color::to_iced(active().surfaces.text_dim)
}

/// Something the user should look at before saving — a chord already bound,
/// a combination the compositor will refuse. Not an error: nothing has gone
/// wrong yet.
pub fn warning() -> Color {
    crate::color::to_iced(active().warning)
}

/// Somewhere that is not this machine — a remote host, a mounted share.
///
/// Reach for this only for that meaning. It is a state colour, and the
/// point of a state colour is that seeing it tells you something; using
/// it as a second accent because it looks nice would make it stop
/// meaning anything, which is the failure the whole reserved-colour
/// scheme exists to prevent.
pub fn info() -> Color {
    crate::color::to_iced(active().info)
}

/// Something went well — a change applied, a device connected, a battery
/// charging.
///
/// A state colour, like [`info`]: it means something only as long as it
/// is never used for anything else.
pub fn success() -> Color {
    crate::color::to_iced(active().success)
}

/// Danger or failure — a conflicting bind, a destructive button, a
/// value the compositor refused.
pub fn error() -> Color {
    crate::color::to_iced(active().error)
}

/// How opaque the alternating row fill is, laid over whatever the rows
/// sit on.
///
/// The mockup's 22%. A tint rather than a fifth opaque grey on purpose:
/// the same stripe then reads correctly on a card and on the window
/// root, where an opaque shade chosen for one would be a visible step on
/// the other.
const ROW_TINT_ALPHA: f32 = 0.22;

/// The fill every other setting row gets, so a long page scans as rows
/// without a divider between each one.
///
/// Derived from [`surface::card_border`] rather than chosen: it is the
/// lightest step in the ramp, so a stripe of it stays inside the look
/// whatever the user's theme does to the greys.
pub fn row_tint() -> Color {
    Color {
        a: ROW_TINT_ALPHA,
        ..surface::card_border()
    }
}

/// The monospace font for anything shown as it is written — a config
/// line, a key, a value.
///
/// The theme's family when it names one, and iced's generic monospace
/// otherwise; `hyprforge_appearance::look` turns the fontconfig alias
/// `Monospace` into "otherwise" before it gets here.
///
/// Interned once for the process, because an iced `Font` holds a
/// `&'static str` and the theme is fixed for the process's lifetime
/// anyway (see [`init`]).
pub fn mono_font() -> iced::Font {
    static MONO: std::sync::OnceLock<iced::Font> = std::sync::OnceLock::new();
    *MONO.get_or_init(|| mono_font_for(&active().mono_font))
}

/// [`mono_font`] for a given family name, without the process-wide
/// cache — separate so a test can reach it without installing a theme.
fn mono_font_for(family: &str) -> iced::Font {
    match family.trim() {
        "" => iced::Font::MONOSPACE,
        name => iced::Font::with_name(Box::leak(name.to_owned().into_boxed_str())),
    }
}

fn palette() -> iced::theme::Palette {
    let theme = active();
    iced::theme::Palette {
        background: crate::color::to_iced(theme.surfaces.root),
        text: crate::color::to_iced(theme.surfaces.text),
        // The accent is read, not chosen. It used to be a hand-picked
        // violet with a comment saying it "matches this desktop's own
        // window-border accent" — it was eyeballed, and it was wrong by
        // a few points. Now it *is* that colour.
        primary: crate::color::to_iced(theme.accent),
        success: crate::color::to_iced(theme.success),
        warning: crate::color::to_iced(theme.warning),
        danger: crate::color::to_iced(theme.error),
    }
}

/// Builds the `iced::Theme` every window in this process is created
/// with, from the resolved [`active`] palette rather than an iced
/// built-in — so a window matches the compositor's own accent instead of
/// iced's stock dark theme.
pub fn app_theme() -> iced::Theme {
    iced::Theme::custom("Hyprforge".to_string(), palette())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_font_scale_is_identity() {
        assert_eq!(FontScale::default().apply(BASE_TEXT_SIZE), BASE_TEXT_SIZE);
    }

    #[test]
    fn font_scale_multiplies_the_base_size() {
        assert_eq!(FontScale(1.5).apply(16.0), 24.0);
        assert_eq!(FontScale(0.5).apply(16.0), 8.0);
    }
}

#[cfg(test)]
mod palette_tests {
    use super::*;

    /// The values the Settings app shipped as compile-time constants,
    /// before the palette became runtime.
    ///
    /// Making the look configurable must not quietly restyle anything.
    /// If these ever need updating, that is a design decision and should
    /// arrive as its own commit with a reason — not as a side effect of
    /// plumbing.
    #[test]
    fn the_default_theme_reproduces_the_constants_it_replaced() {
        let c = |r, g, b| iced::Color::from_rgb8(r, g, b);
        assert_eq!(surface::root(), c(0x19, 0x1a, 0x21));
        assert_eq!(surface::sidebar(), c(0x12, 0x13, 0x19));
        assert_eq!(surface::card(), c(0x25, 0x27, 0x31));
        assert_eq!(surface::card_border(), c(0x37, 0x3a, 0x47));
        assert_eq!(surface::row(), c(0x1f, 0x21, 0x29));
        assert_eq!(text(), c(0xe9, 0xea, 0xef));
        assert_eq!(text_dim(), c(0x92, 0x96, 0xa4));
        assert_eq!(warning(), c(0xf5, 0xb9, 0x42));
    }

    /// The stripe is the border colour made translucent — not a new
    /// grey. If it ever becomes opaque it stops working on both of the
    /// surfaces rows sit on at once.
    #[test]
    fn the_row_tint_is_the_border_colour_made_translucent() {
        let tint = row_tint();
        let border = surface::card_border();
        assert_eq!((tint.r, tint.g, tint.b), (border.r, border.g, border.b));
        assert!(tint.a > 0.0 && tint.a < 0.5, "a stripe, not a fill: {}", tint.a);
    }

    /// No family named means iced's own monospace, never the
    /// proportional default a missing name would fall back to.
    #[test]
    fn an_unnamed_monospace_family_is_the_toolkits_monospace() {
        assert_eq!(mono_font_for(""), iced::Font::MONOSPACE);
        assert_eq!(mono_font_for("   "), iced::Font::MONOSPACE);
        assert_eq!(mono_font_for("Hack"), iced::Font::with_name("Hack"));
    }

    /// An app that never calls `init` must render, not panic. The lock
    /// screen's whole design rests on the same idea: the path where
    /// everything else failed still has to put something legible on
    /// screen.
    #[test]
    fn a_process_that_never_installs_a_theme_still_has_one() {
        assert_eq!(active().accent, hyprforge_look::Theme::default().accent);
        let _ = app_theme();
    }
}
