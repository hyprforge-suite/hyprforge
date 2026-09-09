//! One look, readable from both sides of the login boundary.
//!
//! This is the part that makes a greeter and a lock screen feel like one
//! system, and it is not a styling problem. A greeter runs as its own
//! user — greetd's default is `greeter` — and a home directory is
//! `drwx------`. The greeter therefore cannot read your wallpaper, your
//! colours or your fonts *at all*: not because the files are private, but
//! because it cannot traverse the directory above them. On this machine
//! `~/Pictures/Wallpapers/Dracula.png` is world-readable and still
//! unreachable.
//!
//! So continuity is an **export**. Hyprforge already owns every visual
//! input — the wallpaper (hyprpaper), the colours (appearance settings),
//! the fonts (gsettings) — and resolves them into one [`Theme`] that both
//! hosts read. The lock screen reads it from your config because it runs
//! as you; the greeter reads the exported copy. Same struct, same
//! renderer, same result.
//!
//! **The wallpaper is copied, not linked.** A path into `$HOME` is
//! unreadable from the greeter no matter how the file itself is
//! permissioned, and a symlink doesn't change that — the traversal is
//! what fails.

use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

/// Where an exported theme lives.
///
/// Outside `$HOME` by necessity. `/var/lib` rather than `/etc` because
/// this is generated state, not configuration a sysadmin edits.
pub const EXPORT_DIR: &str = "/var/lib/hyprforge/greet";

#[derive(Debug, thiserror::Error)]
pub enum ThemeError {
    #[error("couldn't read {path}: {source}")]
    Read {
        path: String,
        #[source]
        source: std::io::Error,
    },
    #[error("couldn't write {path}: {source}")]
    Write {
        path: String,
        #[source]
        source: std::io::Error,
    },
    #[error("couldn't parse {path}: {source}")]
    Parse {
        path: String,
        #[source]
        source: toml::de::Error,
    },
    #[error("couldn't serialize the theme: {0}")]
    Serialize(#[from] toml::ser::Error),
}

/// How the authentication screen looks.
///
/// Every field has a usable default, because a greeter that fails to
/// render is a machine nobody can log into. A missing export, an
/// unreadable wallpaper and a half-written file all have to degrade to
/// "plain but working" rather than to nothing.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Theme {
    /// Full path to the background image. `None` renders [`Self::background`]
    /// flat, which is also the fallback when the file can't be read.
    pub wallpaper: Option<PathBuf>,
    /// Behind everything, and behind the wallpaper while it loads.
    pub background: String,
    /// The panel the prompt sits on.
    pub surface: String,
    /// Text on `surface`.
    pub foreground: String,
    /// The focused/active colour — the same one window borders use.
    pub accent: String,
    /// Failure text.
    pub error: String,
    pub font: String,
    pub font_size: f32,
    /// `strftime` format for the clock. Empty hides it.
    pub clock_format: String,
    pub date_format: String,
    /// Corner radius, so the prompt matches window rounding.
    pub rounding: u32,
    /// How much the wallpaper is dimmed behind the prompt, 0.0 to 1.0.
    pub dim: f32,
    /// Blur radius behind the prompt. 0 disables it.
    pub blur: u32,
}

impl Default for Theme {
    fn default() -> Self {
        Theme {
            wallpaper: None,
            // Deliberately dark and neutral rather than pretty: this is
            // what shows when everything else has failed, and it must be
            // legible rather than fashionable.
            background: "rgba(16161eff)".into(),
            surface: "rgba(26263aff)".into(),
            foreground: "rgba(f8f8f2ff)".into(),
            accent: "rgba(bd93f9ff)".into(),
            error: "rgba(ff5555ff)".into(),
            font: "Sans".into(),
            font_size: 15.0,
            clock_format: "%H:%M".into(),
            date_format: "%A, %e %B".into(),
            rounding: 12,
            dim: 0.35,
            blur: 0,
        }
    }
}

impl Theme {
    /// Reads a theme from `path`.
    ///
    /// A missing file is the default theme, not an error: the greeter
    /// runs before anything has necessarily been exported, and refusing
    /// to draw would mean refusing to let anyone log in.
    pub fn load(path: &Path) -> Result<Theme, ThemeError> {
        if !path.exists() {
            return Ok(Theme::default());
        }
        let contents = std::fs::read_to_string(path).map_err(|source| ThemeError::Read {
            path: path.display().to_string(),
            source,
        })?;
        toml::from_str(&contents).map_err(|source| ThemeError::Parse {
            path: path.display().to_string(),
            source,
        })
    }

    /// Reads the exported theme, falling back to the default.
    ///
    /// Never returns an error. A greeter has nobody to report one to and
    /// nothing better to do than draw something usable — so a corrupt or
    /// unreadable export degrades to plain rather than to a blank screen.
    pub fn load_exported() -> Theme {
        Theme::load(&Path::new(EXPORT_DIR).join("theme.toml")).unwrap_or_default()
    }

    /// Writes the theme and a copy of its wallpaper into `dir`.
    ///
    /// The wallpaper is **copied**, and the stored path rewritten to the
    /// copy. Referring to the original would leave the greeter with a
    /// path it cannot traverse — the failure this whole module exists to
    /// avoid — and it would break again the moment the image moved.
    ///
    /// Returns the theme as written, which is not the theme passed in:
    /// its wallpaper path points at the copy.
    pub fn export(&self, dir: &Path) -> Result<Theme, ThemeError> {
        std::fs::create_dir_all(dir).map_err(|source| ThemeError::Write {
            path: dir.display().to_string(),
            source,
        })?;

        let mut exported = self.clone();
        exported.wallpaper = match &self.wallpaper {
            Some(source) if source.is_file() => {
                // One fixed name, so an old wallpaper doesn't accumulate
                // beside the new one in a directory nobody looks at.
                let extension = source
                    .extension()
                    .and_then(|e| e.to_str())
                    .unwrap_or("img");
                let destination = dir.join(format!("wallpaper.{extension}"));
                std::fs::copy(source, &destination).map_err(|source| ThemeError::Write {
                    path: destination.display().to_string(),
                    source,
                })?;
                Some(destination)
            }
            // A wallpaper that isn't there is dropped rather than
            // exported as a dangling path: the greeter would fall back
            // anyway, and a path that can't resolve invites someone to
            // debug a permissions problem that doesn't exist.
            _ => None,
        };

        let contents = toml::to_string_pretty(&exported)?;
        let theme_path = dir.join("theme.toml");
        hyprforge_paths::write_atomic(&theme_path, &contents).map_err(|source| {
            ThemeError::Write {
                path: theme_path.display().to_string(),
                source,
            }
        })?;
        Ok(exported)
    }

    /// Whether the wallpaper can actually be read right now.
    ///
    /// Asked before drawing rather than after: the renderer's fallback is
    /// a flat colour, and knowing in advance is the difference between a
    /// deliberate plain background and a flicker.
    pub fn wallpaper_readable(&self) -> bool {
        self.wallpaper
            .as_ref()
            .is_some_and(|p| std::fs::File::open(p).is_ok())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn theme_with_wallpaper(path: &Path) -> Theme {
        Theme {
            wallpaper: Some(path.to_path_buf()),
            accent: "rgba(bd93f9ff)".into(),
            ..Theme::default()
        }
    }

    #[test]
    fn a_theme_round_trips_through_toml() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("theme.toml");
        let theme = Theme { rounding: 20, dim: 0.5, ..Theme::default() };
        std::fs::write(&path, toml::to_string_pretty(&theme).unwrap()).unwrap();
        assert_eq!(Theme::load(&path).unwrap(), theme);
    }

    /// A greeter that won't draw is a machine nobody can log into, so a
    /// missing theme is the default rather than an error.
    #[test]
    fn a_missing_theme_file_is_the_default_not_an_error() {
        let dir = tempfile::tempdir().unwrap();
        assert_eq!(Theme::load(&dir.path().join("nope.toml")).unwrap(), Theme::default());
    }

    /// Same reasoning, one step further: the greeter has nobody to report
    /// an error to.
    #[test]
    fn an_unreadable_export_degrades_to_plain_rather_than_failing() {
        // `load_exported` reads a fixed system path that won't exist in a
        // test environment, which is exactly the case being checked.
        let theme = Theme::load_exported();
        assert_eq!(theme, Theme::default());
    }

    /// A partial file — a half-finished hand edit — must not take the
    /// login screen down.
    #[test]
    fn a_partial_theme_file_fills_in_the_rest() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("theme.toml");
        std::fs::write(&path, "rounding = 4\n").unwrap();
        let theme = Theme::load(&path).unwrap();
        assert_eq!(theme.rounding, 4);
        assert_eq!(theme.accent, Theme::default().accent, "the rest defaults");
    }

    /// The whole point: a path into `$HOME` is unreachable from the
    /// greeter, so the image is copied and the path rewritten.
    #[test]
    fn exporting_copies_the_wallpaper_and_repoints_at_the_copy() {
        let dir = tempfile::tempdir().unwrap();
        let home = dir.path().join("home/Pictures");
        std::fs::create_dir_all(&home).unwrap();
        let original = home.join("Dracula.png");
        std::fs::write(&original, b"not really a png").unwrap();

        let export = dir.path().join("export");
        let exported = theme_with_wallpaper(&original).export(&export).unwrap();

        let copy = exported.wallpaper.clone().unwrap();
        assert!(copy.starts_with(&export), "the copy must live in the export: {copy:?}");
        assert_ne!(copy, original, "the path must be rewritten");
        assert_eq!(std::fs::read(&copy).unwrap(), b"not really a png");
        assert!(original.exists(), "the original is left alone");
    }

    #[test]
    fn the_exported_file_is_what_gets_loaded_back() {
        let dir = tempfile::tempdir().unwrap();
        let source = dir.path().join("w.png");
        std::fs::write(&source, b"x").unwrap();
        let export = dir.path().join("export");

        let mut theme = theme_with_wallpaper(&source);
        theme.rounding = 8;
        let exported = theme.export(&export).unwrap();

        assert_eq!(Theme::load(&export.join("theme.toml")).unwrap(), exported);
    }

    /// A dangling path invites someone to debug a permissions problem
    /// that isn't there, so a wallpaper that doesn't exist is dropped.
    #[test]
    fn a_missing_wallpaper_is_dropped_rather_than_exported_dangling() {
        let dir = tempfile::tempdir().unwrap();
        let theme = theme_with_wallpaper(&dir.path().join("gone.png"));
        let exported = theme.export(&dir.path().join("export")).unwrap();
        assert_eq!(exported.wallpaper, None);
    }

    /// Exporting twice must replace the wallpaper rather than leave the
    /// old one beside it in a directory nobody looks at.
    #[test]
    fn exporting_again_replaces_the_previous_wallpaper() {
        let dir = tempfile::tempdir().unwrap();
        let export = dir.path().join("export");
        let first = dir.path().join("first.png");
        let second = dir.path().join("second.png");
        std::fs::write(&first, b"one").unwrap();
        std::fs::write(&second, b"two").unwrap();

        theme_with_wallpaper(&first).export(&export).unwrap();
        let exported = theme_with_wallpaper(&second).export(&export).unwrap();

        assert_eq!(std::fs::read(exported.wallpaper.unwrap()).unwrap(), b"two");
        let images: Vec<_> = std::fs::read_dir(&export)
            .unwrap()
            .flatten()
            .filter(|e| e.path().extension().is_some_and(|x| x == "png"))
            .collect();
        assert_eq!(images.len(), 1, "only one wallpaper should remain");
    }

    #[test]
    fn wallpaper_readability_is_checked_against_the_filesystem() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("w.png");
        assert!(!theme_with_wallpaper(&path).wallpaper_readable());
        std::fs::write(&path, b"x").unwrap();
        assert!(theme_with_wallpaper(&path).wallpaper_readable());
        assert!(!Theme::default().wallpaper_readable(), "no wallpaper is not readable");
    }

    /// The default is what shows when everything else has failed. It has
    /// to be legible on its own.
    #[test]
    fn the_default_theme_is_usable_without_any_file() {
        let theme = Theme::default();
        assert!(theme.wallpaper.is_none());
        assert!(theme.font_size > 0.0);
        assert!(!theme.background.is_empty() && !theme.foreground.is_empty());
        assert_ne!(theme.background, theme.foreground, "text must be legible");
        assert!((0.0..=1.0).contains(&theme.dim));
    }
}
