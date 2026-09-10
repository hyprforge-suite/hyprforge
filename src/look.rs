//! Publishing the shared look, from the one place that can see all of it.
//!
//! The colours and fonts come from `hyprforge-appearance` and the
//! wallpaper from `hyprforge-ecosystem`, and neither crate should depend
//! on the other to fetch half a theme. The Settings app already depends
//! on both, so the assembly happens here.
//!
//! Called from **both** screens that can change what the auth screens
//! look like: Appearance owns the colours, Desktop owns the wallpaper.
//! Wiring only one of them is how the lock screen ends up showing last
//! week's wallpaper.

use std::path::PathBuf;

fn wallpaper_toml() -> PathBuf {
    hyprforge_paths::hyprforge_config_dir().join("wallpaper.toml")
}

/// Resolves the look and writes it where the lock screen and the greeter
/// read it.
///
/// Never fails loudly: the settings the user was actually saving are
/// already on disk by the time this runs, and failing their save because
/// a lock screen theme could not be written would be reporting the wrong
/// problem.
pub fn republish() {
    let mut theme = hyprforge_appearance::look::resolve();
    theme.wallpaper = wallpaper();

    if let Err(e) = hyprforge_appearance::look::publish(&theme) {
        tracing::warn!(error = %e, "couldn't publish the look for the lock screen");
    }
}

/// The wallpaper the auth screens should show, if there is one.
fn wallpaper() -> Option<PathBuf> {
    match hyprforge_ecosystem::storage::load::<hyprforge_ecosystem::wallpaper::Settings>(
        &wallpaper_toml(),
    ) {
        Ok(settings) => hyprforge_ecosystem::wallpaper::for_auth_screen(&settings),
        Err(e) => {
            // A missing file is first-run and loads as empty. Anything
            // else means the wallpaper settings exist and could not be
            // read, which is worth saying rather than silently showing a
            // flat background.
            tracing::warn!(error = %e, "couldn't read the wallpaper settings");
            None
        }
    }
}
