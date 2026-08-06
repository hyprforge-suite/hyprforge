use iced::{color, Color};

/// Spacing scale shared by every Hyprforge app, so a sidebar, a settings
/// row, and a dialog all breathe the same amount no matter which app they're
/// in (vision pillar #2: one shared visual grammar).
pub mod spacing {
    pub const XS: f32 = 4.0;
    pub const SM: f32 = 8.0;
    pub const MD: f32 = 16.0;
    pub const LG: f32 = 24.0;
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
    pub fn apply(&self, base: f32) -> f32 {
        base * self.0
    }
}

/// Explicit surface colors, used directly by the shared widget layer
/// instead of leaning on `Theme::extended_palette()`'s auto-derived
/// shades. Auto-derivation (`deviate(base, 0.1)` etc.) only shifts
/// lightness a few percent, which reads as "everything is the same flat
/// gray" at normal monitor brightness — real elevation (root vs sidebar
/// vs card) needs a deliberately chosen jump, the way macOS/Windows
/// layer system chrome vs content vs cards.
pub mod surface {
    use iced::Color;

    /// Window root — behind everything.
    pub const ROOT: Color = Color::from_rgb(0.098, 0.102, 0.129); // #191a21
    /// Sidebar — recessed a step darker than root.
    pub const SIDEBAR: Color = Color::from_rgb(0.071, 0.075, 0.098); // #121319
    /// Card/section background — raised a step lighter than root.
    pub const CARD: Color = Color::from_rgb(0.145, 0.153, 0.192); // #252731
    /// Card border.
    pub const CARD_BORDER: Color = Color::from_rgb(0.216, 0.227, 0.278); // #373a47
    /// A list row's background on hover/emphasis, between root and card.
    pub const ROW: Color = Color::from_rgb(0.122, 0.129, 0.161); // #1f2129
}

/// Primary text color (near-white, not pure white — easier on the eyes on
/// a dark surface).
pub const TEXT: Color = Color::from_rgb(0.914, 0.918, 0.937); // #e9eaef
/// Secondary/meta text — timestamps, hints, IDs.
pub const TEXT_DIM: Color = Color::from_rgb(0.573, 0.588, 0.643); // #9296a4

fn palette() -> iced::theme::Palette {
    iced::theme::Palette {
        background: surface::ROOT,
        text: TEXT,
        // Violet accent — matches this desktop's own window-border/waybar
        // accent color rather than iced's default Discord-blurple, so the
        // app doesn't look like it wandered in from a different system.
        primary: color!(0x9b8cf5),
        success: color!(0x3ecf8e),
        warning: color!(0xf5b942),
        danger: color!(0xe5555f),
    }
}

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
