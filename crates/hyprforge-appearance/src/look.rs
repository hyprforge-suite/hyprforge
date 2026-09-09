//! Turning the settings the user already controls into one shared theme.
//!
//! Nothing here invents a colour. The accent is the window border
//! colour, the fonts are the desktop's fonts, the corner radius is
//! Hyprland's — all things the user sets once, in the place they already
//! expect to set them. That is deliberate: the alternative is a
//! Hyprforge-only theme file, which means a second place to configure
//! colours that already exist and that the user has to discover.
//!
//! It also follows what the app already did for text scaling, which has
//! always followed the desktop's accessibility setting rather than
//! offering a control of its own.
//!
//! This module produces a [`hyprforge_look::Theme`] and hands it back.
//! It does not install it anywhere: that would mean this crate knowing
//! about the GUI, and the whole point is that a lock screen with no
//! toolkit at all reads the same struct.

use crate::desktop;
use crate::storage;
use hyprforge_core::hlconfig::Value;
use hyprforge_look::{Color, Theme};

/// The Hyprland setting that is, in practice, the desktop's accent: the
/// colour of the focused window's border.
const ACCENT_KEY: &str = "general:col:active_border";

/// The corner radius, so panels match window rounding.
const ROUNDING_KEY: &str = "decoration:rounding";

/// Builds the shared theme from what the user has already set.
///
/// Never fails. Every input is optional and every fallback is the
/// default theme's own value, because the caller is an app that is about
/// to draw a window and there is nothing useful for it to do with an
/// error.
pub fn resolve() -> Theme {
    let mut theme = Theme::default();

    // A missing file is first-run, not a problem. A file that exists and
    // cannot be read *is* a problem, and falling back to defaults without
    // saying so is the exact mistake `storage`'s own documentation warns
    // about — it is how a user's settings quietly stop taking effect with
    // nothing on screen to explain it. Resolving still can't fail (the
    // caller is about to draw a window), so it is loud instead.
    let stored = match storage::load(&hyprforge_paths::appearance_toml_path()) {
        Ok(stored) => stored,
        Err(e) => {
            tracing::warn!(
                error = %e,
                "couldn't read the appearance settings; falling back to the default look"
            );
            crate::Appearance::default()
        }
    };

    if let Some(accent) = stored_color(&stored, ACCENT_KEY) {
        theme.accent = accent;
    }
    if let Some(Value::Int(rounding)) = stored.settings.get(ROUNDING_KEY) {
        if let Ok(rounding) = u32::try_from(*rounding) {
            theme.rounding = rounding;
        }
    }
    if let Ok(Some(font)) = desktop::read("font-name") {
        let (family, size) = desktop::split_font(&font);
        if !family.is_empty() {
            theme.font = family;
        }
        if let Some(size) = size {
            theme.font_size = size as f32;
        }
    }
    theme.font_scale = desktop::text_scaling_factor();
    theme
}

/// A stored colour setting, if it is set and parses.
///
/// Deliberately reads the *stored* value rather than the catalogue
/// default. The catalogue says `general:col:active_border` defaults to
/// `rgba(ffffffff)` — that is a checked claim about what Hyprland does,
/// not a statement about what looks right, and resolving to it would
/// make every app white.
fn stored_color(stored: &crate::Appearance, key: &str) -> Option<Color> {
    match stored.settings.get(key) {
        Some(Value::Text(raw)) => Color::parse(raw).ok(),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn with_settings(pairs: &[(&str, Value)]) -> crate::Appearance {
        let mut appearance = crate::Appearance::default();
        for (key, value) in pairs {
            appearance.settings.set(key, value.clone());
        }
        appearance
    }

    /// The accent is the whole point of resolving anything: it is the
    /// one value that used to be a hand-picked constant in the Settings
    /// app, eyeballed against the window border it was meant to match.
    #[test]
    fn a_stored_border_colour_becomes_the_accent() {
        let stored = with_settings(&[(ACCENT_KEY, Value::Text("rgba(bd93f9ff)".into()))]);
        assert_eq!(
            stored_color(&stored, ACCENT_KEY),
            Some(Color::rgba(0xbd, 0x93, 0xf9, 0xff))
        );
    }

    /// The catalogue default for the border is white. It is a checked
    /// claim about what Hyprland does, not a statement about what looks
    /// right, and resolving to it would make every app in the suite
    /// white. Unset must mean "keep the theme's own accent".
    #[test]
    fn an_unset_border_does_not_resolve_to_the_catalogue_default() {
        let stored = crate::Appearance::default();
        assert_eq!(stored_color(&stored, ACCENT_KEY), None);

        let catalogue_default = crate::CATALOG
            .settings
            .iter()
            .find(|s| s.key == ACCENT_KEY)
            .expect("the border colour is catalogued");
        assert!(
            format!("{:?}", catalogue_default.kind).contains("ffffffff"),
            "if this default stops being white the comment above needs revisiting"
        );
    }

    /// A half-typed colour is not a reason to draw a white window.
    #[test]
    fn an_unparseable_border_colour_is_ignored_rather_than_used() {
        for bad in ["", "0xffbd93f9", "rgba(nope)", "rgb(bd93f9ff)"] {
            let stored = with_settings(&[(ACCENT_KEY, Value::Text(bad.into()))]);
            assert_eq!(stored_color(&stored, ACCENT_KEY), None, "{bad}");
        }
    }

    /// Resolving must never fail or panic: the caller is an app about to
    /// draw a window, and there is nothing useful for it to do with an
    /// error. On a machine with no gsettings and no stored settings it
    /// still has to produce a legible theme.
    #[test]
    fn resolving_always_produces_a_usable_theme() {
        let theme = resolve();
        assert_eq!(theme.accent.a, 0xff, "a transparent accent shows nothing");
        assert!(theme.font_size > 0.0);
        assert!(theme.font_scale > 0.0 && theme.font_scale.is_finite());
        assert!(!theme.font.is_empty());
    }
}
