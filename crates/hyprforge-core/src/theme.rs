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

pub fn app_theme() -> iced::Theme {
    iced::Theme::Dark
}
