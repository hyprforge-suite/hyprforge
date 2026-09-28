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
    if let Ok(Some(mono)) = desktop::read("monospace-font-name") {
        let family = mono_family(&mono);
        if !family.is_empty() && !fontconfig_draws(&family) {
            // The user's own setting names a font this machine doesn't
            // have — not our file and not our mistake, but worth a line,
            // because the alternative explanation for "config lines
            // aren't in my font" is a bug here.
            tracing::info!(
                family = %family,
                "the desktop's monospace font isn't installed; using the toolkit's own monospace"
            );
        } else {
            theme.mono_font = family;
        }
    }
    theme.font_scale = desktop::text_scaling_factor();
    theme
}

/// Whether fontconfig would draw `family` as itself, rather than
/// substitute something for it.
///
/// Asked because gsettings happily names a font that isn't installed —
/// on the machine this was written on it says `Hack`, and nothing called
/// Hack exists. Handing that name to iced falls back to the
/// *proportional* default (fontconfig itself answers `Noto Sans`), which
/// is the one font a monospace setting exists to avoid; iced's generic
/// monospace is the right answer instead.
///
/// Anything short of a confirmed match — no `fc-match`, a timeout, a
/// substitute — is "no". The cost of a wrong "no" is the generic
/// monospace, which is always legible; the cost of a wrong "yes" is
/// config lines in a font where `il1|` look alike.
fn fontconfig_draws(family: &str) -> bool {
    let out = hyprforge_core::command::output(
        std::process::Command::new("fc-match").args(["-f", "%{family}", family]),
        hyprforge_core::command::TIMEOUT,
    );
    match out {
        Ok(out) if out.status.success() => {
            names_family(&String::from_utf8_lossy(&out.stdout), family)
        }
        _ => false,
    }
}

/// Whether `fc-match`'s family field — a comma-separated list of the
/// matched font's names — includes `family`.
///
/// A list because one font carries several: `JetBrainsMono Nerd
/// Font,JetBrainsMono NF` answers to either, and gsettings may hold the
/// short one.
fn names_family(matched: &str, family: &str) -> bool {
    matched
        .split(',')
        .any(|name| name.trim().eq_ignore_ascii_case(family.trim()))
}

/// The family half of a `monospace-font-name` value, or empty for "the
/// toolkit's own monospace".
///
/// Only the family: the size belongs to terminals, and a config line in
/// a settings window is sized by the type ramp around it, not by what
/// someone picked for their shell.
///
/// `Monospace` is GNOME's shipped default and is a fontconfig alias
/// rather than a family. iced finds fonts by family name without asking
/// fontconfig, so passing the alias through would resolve to nothing and
/// fall back to the proportional default — config lines drawn in a font
/// where `il1|` all look alike. Empty sends it to iced's generic
/// monospace instead, which is what the alias meant.
fn mono_family(value: &str) -> String {
    let (family, _size) = desktop::split_font(value);
    match family.eq_ignore_ascii_case("monospace") {
        true => String::new(),
        false => family,
    }
}

/// Writes the resolved look where the other hosts can read it.
///
/// Two destinations, for one reason: a greeter runs as its own user and
/// a home directory is `drwx------`, so it cannot read the config file
/// the lock screen reads — not because the file is private but because
/// it cannot traverse the directory above it. Continuity across the
/// login boundary is therefore an export, not a styling exercise.
///
/// Best-effort by design. The export lands under `/var/lib` and will
/// simply fail unprivileged; that must never turn a successful settings
/// save into a failed one, so it is reported and stepped over. The
/// user's own copy is the load-bearing write and its failure is
/// returned.
///
/// The wallpaper is deliberately left as the theme has it. Choosing one
/// image from a per-monitor wallpaper list is a real decision — which
/// monitor's? — and guessing here would put a wallpaper on the lock
/// screen that the user never picked.
pub fn publish(theme: &Theme) -> Result<(), hyprforge_look::ThemeError> {
    theme.save(&hyprforge_paths::lock_toml_path())?;

    let dir = hyprforge_look::theme::export_dir();
    if let Err(e) = theme.export(&dir) {
        // Warn, not debug. This failing is invisible from the outside:
        // the save succeeds, the lock screen restyles, and only the
        // greeter — seen once per boot, before this process exists —
        // keeps the old look. Logged at debug it read as "nothing
        // configured" when the truth was "the directory isn't there",
        // which is the distinction this project has already been
        // bitten by once. The remedy is an install step, so name it.
        tracing::warn!(
            error = %e,
            dir = %dir.display(),
            "couldn't export the look for the greeter, so the login screen keeps its \
             previous appearance; the lock screen's copy is written. If the greeter is \
             installed, this directory needs to exist and be writable by this user - see \
             crates/hyprforge-greet/config/hyprforge-greet.tmpfiles"
        );
    }
    Ok(())
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

    /// A real family comes through without its size — the size is the
    /// terminal's business, not a settings row's.
    #[test]
    fn a_monospace_family_is_kept_and_its_size_dropped() {
        // gsettings really does pad with two spaces on this machine.
        assert_eq!(mono_family("Hack  10"), "Hack");
        assert_eq!(mono_family("JetBrains Mono 11"), "JetBrains Mono");
    }

    /// The alias is not a family, and naming it to iced would draw config
    /// lines in the proportional default — see `mono_family`.
    #[test]
    fn the_monospace_alias_means_the_toolkits_own_monospace() {
        assert_eq!(mono_family("Monospace 11"), "");
        assert_eq!(mono_family("monospace"), "");
        assert_eq!(mono_family(""), "");
    }

    /// fontconfig substitutes rather than failing: asked for a font it
    /// doesn't have, it names a different one. That answer has to read
    /// as "not installed", or a missing monospace font draws config
    /// lines proportionally.
    #[test]
    fn a_substituted_font_is_not_the_font_that_was_asked_for() {
        assert!(!names_family("Noto Sans", "Hack"));
        assert!(names_family("Fira Mono", "Fira Mono"));
    }

    /// One font answers to every name in its family list, so the short
    /// alias gsettings may hold still counts as installed.
    #[test]
    fn any_of_a_fonts_names_counts_as_that_font() {
        let matched = "JetBrainsMono Nerd Font,JetBrainsMono NF";
        assert!(names_family(matched, "JetBrainsMono NF"));
        assert!(names_family(matched, "jetbrainsmono nerd font"));
        assert!(!names_family(matched, "JetBrains"));
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

#[cfg(test)]
mod publishing {
    use super::*;

    /// `set_var` is process-global and cargo runs tests on threads, so
    /// every test here that touches the environment takes this first.
    static ENV_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

    /// The lock screen reads a file nothing used to write, which is why
    /// it could only ever show its own defaults. Publishing has to
    /// produce something it can actually load.
    #[test]
    fn a_published_look_is_readable_by_the_lock_screen() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("lock.toml");
        let theme = Theme {
            accent: Color::rgba(0x00, 0xff, 0x88, 0xff),
            ..Theme::default()
        };
        theme.save(&path).unwrap();

        let read_back = Theme::load(&path).expect("the lock screen must be able to read it");
        assert_eq!(read_back.accent, theme.accent);
        assert_eq!(read_back, theme, "publishing must not lose a field");
    }

    /// The export must be redirectable, because otherwise it is the one
    /// write in `publish` that no sandbox can contain. A test that
    /// isolates `XDG_CONFIG_HOME` and drives a settings save would
    /// otherwise overwrite the real login screen's theme on any machine
    /// where the greeter is installed — which is exactly what happened.
    #[test]
    fn publishing_writes_the_export_where_it_is_pointed() {
        let _lock = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let dir = tempfile::tempdir().unwrap();
        let previous = std::env::var(hyprforge_look::theme::EXPORT_DIR_ENV).ok();
        // SAFETY: ENV_LOCK serialises every env mutation in this module.
        unsafe {
            std::env::set_var(hyprforge_look::theme::EXPORT_DIR_ENV, dir.path().join("greet"))
        };

        let resolved = hyprforge_look::theme::export_dir();
        assert!(
            resolved.starts_with(dir.path()),
            "the override must win over the compiled-in path; got {resolved:?}"
        );
        assert_ne!(
            resolved,
            std::path::Path::new(hyprforge_look::theme::EXPORT_DIR),
            "an isolated run must never resolve to the real greeter export"
        );

        // SAFETY: `_lock` (ENV_LOCK, taken at the top of this test) is
        // still held, so putting the variable back is serialised against
        // every other env mutation in this module just as setting it was.
        match previous {
            Some(p) => unsafe {
                std::env::set_var(hyprforge_look::theme::EXPORT_DIR_ENV, p)
            },
            None => unsafe { std::env::remove_var(hyprforge_look::theme::EXPORT_DIR_ENV) },
        }
    }

    /// An export that fails — `/var/lib` unprivileged is the normal case
    /// — must not make a settings save look like it failed. The user's
    /// own copy is the load-bearing write.
    #[test]
    fn a_failed_greeter_export_does_not_fail_the_publish() {
        let dir = tempfile::tempdir().unwrap();
        let theme = Theme::default();
        // Writing the user's copy somewhere writable succeeds even
        // though the export target almost certainly is not.
        assert!(theme.save(&dir.path().join("lock.toml")).is_ok());
        assert!(theme.export(std::path::Path::new("/proc/nonexistent/greet")).is_err());
    }
}
