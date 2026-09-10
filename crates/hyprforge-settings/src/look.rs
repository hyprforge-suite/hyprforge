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

use std::path::{Path, PathBuf};

/// The longest edge the auth screens' wallpaper is allowed to have.
///
/// Not about looks — about what the lock screen has to allocate. A
/// 8001x4501 photograph is 36 megapixels, which decodes to 144MB and
/// peaks near 300MB once the renderer has its own premultiplied copy.
/// That is a lot to ask of the one process on the machine that must not
/// fail, and the failure is not graceful: an allocation failure during
/// decode is exactly the case that poisons `iced_tiny_skia`'s image
/// cache and panics on the next frame, which on a lock screen means a
/// machine you need another TTY to get into.
///
/// 4K's long edge, so anything up to a 4K display is still shown at
/// full resolution and nothing larger is ever decoded.
const MAX_WALLPAPER_EDGE: u32 = 3840;

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

/// Where the downscaled copy lives. Derived, and regenerated whenever
/// the original is newer.
fn scaled_wallpaper_path() -> PathBuf {
    hyprforge_paths::hyprforge_config_dir().join("lock-wallpaper.png")
}

/// A copy of `original` no larger than [`MAX_WALLPAPER_EDGE`], or the
/// original when it is already small enough.
///
/// Done here, once, when settings are saved — rather than on every lock,
/// where it would cost the same allocation every time and where failing
/// is unrecoverable. Failure here is free: the caller keeps the original,
/// which is what would have been used anyway.
fn scaled_wallpaper(original: &Path) -> Option<PathBuf> {
    scale_into(original, &scaled_wallpaper_path())
}

/// The scaling itself, with the destination passed in so it can be
/// tested without writing into the real config directory.
fn scale_into(original: &Path, scaled: &Path) -> Option<PathBuf> {
    let reader = image::ImageReader::open(original)
        .ok()?
        .with_guessed_format()
        .ok()?;
    let (width, height) = reader.into_dimensions().ok()?;
    if width.max(height) <= MAX_WALLPAPER_EDGE {
        return None;
    }

    // Skip the work when the copy is already current. Comparing mtimes
    // rather than hashing: this runs on a settings save, and reading 36
    // megapixels to decide whether to read 36 megapixels is silly.
    let up_to_date = || -> Option<bool> {
        let source = std::fs::metadata(original).ok()?.modified().ok()?;
        let derived = std::fs::metadata(scaled).ok()?.modified().ok()?;
        Some(derived >= source)
    };
    if up_to_date().unwrap_or(false) {
        return Some(scaled.to_path_buf());
    }

    let image = image::ImageReader::open(original).ok()?.with_guessed_format().ok()?;
    let image = image.decode().ok()?;
    // Lanczos3 because this happens once and the result is looked at
    // every time the machine is locked.
    let resized = image.resize(
        MAX_WALLPAPER_EDGE,
        MAX_WALLPAPER_EDGE,
        image::imageops::FilterType::Lanczos3,
    );
    if let Some(parent) = scaled.parent() {
        std::fs::create_dir_all(parent).ok()?;
    }
    resized.save(scaled).ok()?;
    tracing::debug!(
        from = %format!("{width}x{height}"),
        to = %format!("{}x{}", resized.width(), resized.height()),
        "scaled the wallpaper for the auth screens"
    );
    Some(scaled.to_path_buf())
}

/// The wallpaper the auth screens should show, if there is one.
fn wallpaper() -> Option<PathBuf> {
    match hyprforge_ecosystem::storage::load::<hyprforge_ecosystem::wallpaper::Settings>(
        &wallpaper_toml(),
    ) {
        Ok(settings) => {
            let chosen = hyprforge_ecosystem::wallpaper::for_auth_screen(&settings)?;
            // Falls back to the original if scaling fails, since that is
            // what would have been used anyway.
            Some(scaled_wallpaper(&chosen).unwrap_or(chosen))
        }
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

#[cfg(test)]
mod tests {
    use super::*;

    /// A photograph far larger than any screen must not reach the lock
    /// screen at full size. 36 megapixels peaks near 300MB in the one
    /// process on the machine that must not fail, and an allocation
    /// failure during decode is the case that panics the renderer.
    #[test]
    fn an_oversized_wallpaper_is_scaled_down() {
        let dir = tempfile::tempdir().unwrap();
        let original = dir.path().join("huge.png");
        // Wider than 4K in one edge, so it has to be reduced.
        image::RgbaImage::from_pixel(5000, 1200, image::Rgba([10, 20, 30, 255]))
            .save(&original)
            .unwrap();

        // Point the derived copy inside the temp dir rather than the
        // real config directory.
        let scaled = dir.path().join("lock-wallpaper.png");
        let out = scale_into(&original, &scaled).expect("an oversized image is scaled");
        assert_eq!(out, scaled);

        let (w, h) = image::ImageReader::open(&scaled)
            .unwrap()
            .with_guessed_format()
            .unwrap()
            .into_dimensions()
            .unwrap();
        assert!(w.max(h) <= MAX_WALLPAPER_EDGE, "still {w}x{h}");
        // The aspect ratio has to survive, or the lock screen shows a
        // stretched photograph.
        assert!((w as f32 / h as f32 - 5000.0 / 1200.0).abs() < 0.01, "{w}x{h}");
    }

    /// Anything a screen could actually display is left alone — copying
    /// and re-encoding it would cost quality for nothing.
    #[test]
    fn a_reasonable_wallpaper_is_left_as_it_is() {
        let dir = tempfile::tempdir().unwrap();
        let original = dir.path().join("fine.png");
        image::RgbaImage::from_pixel(2560, 1600, image::Rgba([1, 2, 3, 255]))
            .save(&original)
            .unwrap();
        assert_eq!(scale_into(&original, &dir.path().join("out.png")), None);
    }

    /// Not an image, and nothing to scale. The caller keeps the original,
    /// which the renderer guard will then reject on its own terms.
    #[test]
    fn something_that_is_not_an_image_scales_to_nothing() {
        let dir = tempfile::tempdir().unwrap();
        let text = dir.path().join("notes.png");
        std::fs::write(&text, b"not a png").unwrap();
        assert_eq!(scale_into(&text, &dir.path().join("out.png")), None);
        assert_eq!(scale_into(&dir.path().join("missing.png"), &dir.path().join("out.png")), None);
    }

    /// Re-scaling 36 megapixels on every settings save would be a
    /// noticeable stall for no gain, so a current copy is reused.
    #[test]
    fn a_current_copy_is_not_regenerated() {
        let dir = tempfile::tempdir().unwrap();
        let original = dir.path().join("huge.png");
        image::RgbaImage::from_pixel(5000, 1200, image::Rgba([9, 9, 9, 255]))
            .save(&original)
            .unwrap();
        let scaled = dir.path().join("lock-wallpaper.png");

        scale_into(&original, &scaled).unwrap();
        let first = std::fs::metadata(&scaled).unwrap().modified().unwrap();
        std::thread::sleep(std::time::Duration::from_millis(20));
        scale_into(&original, &scaled).unwrap();
        assert_eq!(
            std::fs::metadata(&scaled).unwrap().modified().unwrap(),
            first,
            "the copy was rewritten even though the original had not changed"
        );
    }
}
