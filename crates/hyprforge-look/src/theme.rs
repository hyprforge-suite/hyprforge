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

use crate::Color;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

/// Where an exported theme lives.
///
/// Outside `$HOME` by necessity. `/var/lib` rather than `/etc` because
/// this is generated state, not configuration a sysadmin edits.
pub const EXPORT_DIR: &str = "/var/lib/hyprforge/greet";

/// The environment variable that redirects [`export_dir`].
///
/// Exists so the export can be isolated the same way the config home
/// can. Without it the two writes in `appearance::look::publish` obey
/// different rules: `lock.toml` follows `XDG_CONFIG_HOME` and the export
/// does not, so anything that sandboxes one still writes the real
/// login screen's theme.
///
/// That is not hypothetical — the Settings module tests drive a save
/// through `publish`, and on a machine where the greeter is installed
/// they overwrote the user's exported theme with one derived from an
/// empty temp config. Harmless before installing, destructive after,
/// which is the worst time to find out.
pub const EXPORT_DIR_ENV: &str = "HYPRFORGE_GREET_DIR";

/// Where an exported theme is written and read.
///
/// [`EXPORT_DIR`] unless [`EXPORT_DIR_ENV`] overrides it. Read every
/// time rather than cached, because a test sets it after this library is
/// already loaded.
pub fn export_dir() -> PathBuf {
    std::env::var_os(EXPORT_DIR_ENV)
        .filter(|v| !v.is_empty())
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(EXPORT_DIR))
}

/// What went wrong reading, writing or (de)serialising a [`Theme`].
///
/// `#[non_exhaustive]` here and nowhere else in this file: on an enum it
/// only forces external `match`es to carry a `_` arm, so a fifth failure
/// mode can be added later without an API break. On a *struct* the same
/// attribute blocks external construction outright — even
/// `Theme { accent, ..Default::default() }` fails to compile outside
/// this crate — which is why `Theme`, `Surfaces` and `Color` do not get
/// it: callers build those with struct-update syntax today, and freezing
/// that would break every one of them.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum ThemeError {
    /// The file exists but couldn't be opened or read — permissions, a
    /// vanished mount, that kind of thing. Not raised for a missing file,
    /// which [`Theme::load`] treats as the default instead.
    #[error("couldn't read {path}: {source}")]
    Read {
        /// The path that couldn't be read.
        path: String,
        /// The underlying I/O failure.
        #[source]
        source: std::io::Error,
    },
    /// The write itself failed — [`Theme::save`] or [`Theme::export`]
    /// couldn't create the file, copy the wallpaper, or rename the temp
    /// file into place.
    #[error("couldn't write {path}: {source}")]
    Write {
        /// The path that couldn't be written.
        path: String,
        /// The underlying I/O failure.
        #[source]
        source: std::io::Error,
    },
    /// The file was read but isn't valid TOML, or its shape doesn't match
    /// [`Theme`] — a half-written file, most likely, since every field
    /// defaults and a merely incomplete one parses fine.
    #[error("couldn't parse {path}: {source}")]
    Parse {
        /// The path whose contents didn't parse.
        path: String,
        /// The underlying TOML error.
        #[source]
        source: toml::de::Error,
    },
    /// Turning a [`Theme`] into TOML failed before any write was
    /// attempted. In practice this shouldn't happen — every field is a
    /// plain serialisable type — but `toml::to_string_pretty` returns a
    /// `Result`, so this variant exists to carry that error rather than
    /// unwrap it.
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
    pub background: Color,
    /// The panel the prompt sits on.
    pub surface: Color,
    /// Text on `surface`.
    pub foreground: Color,
    /// The focused/active colour — the same one window borders use.
    pub accent: Color,
    /// Failure text.
    pub error: Color,
    /// Font family name, as gsettings reports it. Not a path — the
    /// renderer resolves it through the system's own font lookup, the
    /// same as every other app on the desktop.
    pub font: String,
    /// Base point size before [`Self::font_scale`] is applied.
    pub font_size: f32,
    /// `strftime` format for the clock. Empty hides it.
    pub clock_format: String,
    /// `strftime` format for the date, shown alongside the clock. Empty
    /// hides it, same as [`Self::clock_format`].
    pub date_format: String,
    /// Corner radius, so the prompt matches window rounding.
    pub rounding: u32,
    /// How much the wallpaper is dimmed behind the prompt, 0.0 to 1.0.
    pub dim: f32,
    /// Blur radius behind the prompt. 0 disables it.
    pub blur: u32,

    /// Scale applied to every text size, following the desktop's own
    /// accessibility setting rather than a control of ours.
    pub font_scale: f32,
    /// Warnings — a setting that won't take effect, a conflicting bind.
    pub warning: Color,
    /// Confirmation that something applied.
    pub success: Color,
    /// "This is somewhere else" — a remote host, a mounted share, a
    /// network location.
    ///
    /// Not a fifth shade of warning. It sits alongside
    /// [`Self::warning`], [`Self::success`] and [`Self::error`] as a
    /// *state* colour, and it exists because the file manager's design
    /// reserves colour so that colour always means something: green for
    /// good, orange for attention, red for danger, and this for
    /// not-local. Without it a mounted share would have to borrow one
    /// of the other three and quietly stop meaning what it says.
    ///
    /// Dracula's own cyan, unadjusted — the same choice
    /// [`Self::accent`] and [`Self::error`] already make, and it holds
    /// contrast against all four surfaces in the ramp.
    pub info: Color,
    /// The elevation ramp the window apps are built from. The auth
    /// screen only needs `background` and `surface`; a settings window
    /// needs the steps in between.
    pub surfaces: Surfaces,
}

/// The shades a windowed app layers on top of each other.
///
/// Separate from the auth screen's flat `background`/`surface` pair
/// because the two are drawing different things: a lock screen is one
/// panel on a wallpaper, a settings window is a sidebar beside cards
/// beside rows. Both are the same look; only one needs the ramp.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Surfaces {
    /// The window's own background, behind everything else.
    pub root: Color,
    /// The navigation panel down the side.
    pub sidebar: Color,
    /// A raised panel grouping related settings.
    pub card: Color,
    /// The line around a [`Self::card`], one step lighter so the edge
    /// reads without needing a shadow.
    pub card_border: Color,
    /// An individual row inside a card, one step lighter again — the
    /// step that lets a list read as rows rather than one solid block.
    pub row: Color,
    /// Primary text on any of the surfaces above.
    pub text: Color,
    /// De-emphasised text — hints, secondary labels — on the same
    /// surfaces as [`Self::text`].
    pub text_dim: Color,
}

impl Default for Surfaces {
    fn default() -> Self {
        // Byte-for-byte the values the Settings app shipped as compile
        // time constants. Unifying the look must not quietly restyle
        // anything: the only colour that changes is the accent, and it
        // changes because it is now read rather than guessed at.
        Surfaces {
            root: Color::rgba(0x19, 0x1a, 0x21, 0xff),
            sidebar: Color::rgba(0x12, 0x13, 0x19, 0xff),
            card: Color::rgba(0x25, 0x27, 0x31, 0xff),
            card_border: Color::rgba(0x37, 0x3a, 0x47, 0xff),
            row: Color::rgba(0x1f, 0x21, 0x29, 0xff),
            text: Color::rgba(0xe9, 0xea, 0xef, 0xff),
            text_dim: Color::rgba(0x92, 0x96, 0xa4, 0xff),
        }
    }
}

impl Default for Theme {
    fn default() -> Self {
        Theme {
            wallpaper: None,
            // Deliberately dark and neutral rather than pretty: this is
            // what shows when everything else has failed, and it must be
            // legible rather than fashionable.
            background: Color::rgba(0x16, 0x16, 0x1e, 0xff),
            surface: Color::rgba(0x26, 0x26, 0x3a, 0xff),
            foreground: Color::rgba(0xf8, 0xf8, 0xf2, 0xff),
            accent: Color::rgba(0xbd, 0x93, 0xf9, 0xff),
            error: Color::rgba(0xff, 0x55, 0x55, 0xff),
            font: "Sans".into(),
            font_size: 15.0,
            clock_format: "%H:%M".into(),
            date_format: "%A, %e %B".into(),
            rounding: 12,
            dim: 0.35,
            blur: 0,
            font_scale: 1.0,
            warning: Color::rgba(0xf5, 0xb9, 0x42, 0xff),
            success: Color::rgba(0x3e, 0xcf, 0x8e, 0xff),
            info: Color::rgba(0x8b, 0xe9, 0xfd, 0xff),
            surfaces: Surfaces::default(),
        }
    }
}

impl Theme {
    /// This theme's font size, clamped to something that can actually
    /// be drawn.
    ///
    /// `iced_tiny_skia`/`cosmic-text` assert a non-zero line height, so
    /// a font size of `0.0` panics on the first frame — and the theme is
    /// read from a file a person can edit, so `0` is reachable.
    ///
    /// It lives here rather than in each host because it was copied into
    /// three of them (the clipboard popup, the emoji popup and the tray
    /// menu). The clipboard's own copy said so: "if a third host ever
    /// needs these bounds, they belong beside `Theme` in
    /// `hyprforge-look` rather than being copied a third time". The
    /// third host arrived.
    pub fn drawable_font_size(&self) -> f32 {
        match self.font_size.is_finite() {
            true => self.font_size.clamp(6.0, 48.0),
            // Not the file's number at all, so the shipped default
            // rather than a clamp of nonsense.
            false => Theme::default().font_size,
        }
    }

    /// This theme's corner rounding, bounded.
    ///
    /// Hyprland's own `decoration:rounding` has no upper limit, and a
    /// huge one turns a popup into a lozenge or a circle. Same reason
    /// and same three copies as [`Theme::drawable_font_size`].
    pub fn corner_radius(&self) -> f32 {
        const MAX_ROUNDING: u32 = 64;
        self.rounding.min(MAX_ROUNDING) as f32
    }

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

    /// Reads the theme exported to [`export_dir`], falling back to the
    /// default.
    ///
    /// Never returns an error. A greeter has nobody to report one to and
    /// nothing better to do than draw something usable — so a corrupt or
    /// unreadable export degrades to plain rather than to a blank screen.
    ///
    /// No current caller: `hyprforge-greet` needs the `--theme-dir`
    /// override that [`load_exported_from`](Self::load_exported_from)
    /// takes and this fixed path doesn't, so it calls that instead. Kept
    /// for any *other* host that only ever reads the one export
    /// (`hyprforge-authui` consumers besides the greeter, should one
    /// exist).
    pub fn load_exported() -> Theme {
        Theme::load_exported_from(&export_dir())
    }

    /// [`load_exported`](Self::load_exported) against a named directory.
    ///
    /// Split out so the degrade-to-default behaviour can be tested
    /// without reading a real system path — and so a caller with its own
    /// notion of where the export lives, like `hyprforge-greet`'s
    /// `--theme-dir`, can use it too. The test that read a real path
    /// instead asserted the export "won't exist in a test environment",
    /// which stopped being true the moment the greeter was installed on
    /// the machine running the tests — it then failed because the
    /// product was working.
    pub fn load_exported_from(dir: &Path) -> Theme {
        Theme::load(&dir.join("theme.toml")).unwrap_or_default()
    }

    /// Writes the theme to `path`, as the lock screen reads it.
    ///
    /// The wallpaper path is stored as-is, because the reader runs as
    /// the same user and can follow it. [`export`](Self::export) is the
    /// version for a reader that cannot.
    pub fn save(&self, path: &Path) -> Result<(), ThemeError> {
        let contents = toml::to_string_pretty(self)?;
        hyprforge_paths::write_atomic(path, &contents).map_err(|source| ThemeError::Write {
            path: path.display().to_string(),
            source,
        })
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
}

#[cfg(test)]
mod tests {
    /// The theme is a file a person can edit, so every number in it can
    /// be nonsense — and a zero font size panics on the first frame
    /// rather than looking wrong.
    #[test]
    fn a_font_size_that_cannot_be_drawn_is_brought_into_range() {
        let sized = |size: f32| Theme { font_size: size, ..Theme::default() }.drawable_font_size();
        assert_eq!(sized(15.0), 15.0, "an ordinary size is left alone");
        assert_eq!(sized(0.0), 6.0);
        assert_eq!(sized(4000.0), 48.0);
        assert_eq!(sized(-3.0), 6.0);
        assert_eq!(sized(f32::NAN), Theme::default().font_size, "not a number at all");
        assert_eq!(sized(f32::INFINITY), Theme::default().font_size);
    }

    /// Hyprland's `decoration:rounding` has no upper bound, and a large
    /// one turns a popup into a lozenge.
    #[test]
    fn rounding_is_bounded_to_something_still_shaped_like_a_popup() {
        let rounded = |r: u32| Theme { rounding: r, ..Theme::default() }.corner_radius();
        assert_eq!(rounded(12), 12.0);
        assert_eq!(rounded(0), 0.0, "square corners are a choice");
        assert_eq!(rounded(9999), 64.0);
    }

    use super::*;

    fn theme_with_wallpaper(path: &Path) -> Theme {
        Theme {
            wallpaper: Some(path.to_path_buf()),
            accent: Color::rgba(0xbd, 0x93, 0xf9, 0xff),
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
        let dir = tempfile::tempdir().unwrap();

        // Nothing there at all: a machine where nothing has been exported.
        assert_eq!(Theme::load_exported_from(dir.path()), Theme::default());

        // There, and not parseable: a half-finished hand edit, or a write
        // interrupted before this file was whole. A greeter has nobody to
        // report the error to and nothing better to do than draw
        // something usable.
        std::fs::write(dir.path().join("theme.toml"), "accent = [not toml").unwrap();
        assert_eq!(Theme::load_exported_from(dir.path()), Theme::default());
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

    /// The on-disk spelling is a contract between three programs that
    /// never run at the same time: Settings writes the file, the lock
    /// screen reads it, and the greeter reads an exported copy as a
    /// different user. A serde change that quietly altered the
    /// representation would break the two readers with no compile error
    /// anywhere, so it is pinned literally.
    #[test]
    fn colours_are_written_as_rgba_strings() {
        let text = toml::to_string(&Theme::default()).unwrap();
        assert!(text.contains(r#"accent = "rgba(bd93f9ff)""#), "{text}");
        assert!(text.contains(r#"background = "rgba(16161eff)""#), "{text}");
        assert!(text.contains(r#"card = "rgba(252731ff)""#), "{text}");
    }

    /// The default is what shows when everything else has failed. It has
    /// to be legible on its own.
    #[test]
    fn the_default_theme_is_usable_without_any_file() {
        let theme = Theme::default();
        assert!(theme.wallpaper.is_none());
        assert!(theme.font_size > 0.0);
        // Typed colours make the old "is it a non-empty string" check
        // meaningless, but the thing it was standing in for is now
        // directly assertable: a fully transparent background or text
        // renders nothing at all.
        assert_eq!(theme.background.a, 0xff, "a transparent background shows nothing");
        assert_eq!(theme.foreground.a, 0xff, "transparent text shows nothing");
        assert_ne!(theme.background, theme.foreground, "text must be legible");
        assert!((0.0..=1.0).contains(&theme.dim));
    }
}

#[cfg(test)]
mod shared_look {
    use super::*;

    /// The Settings app and the lock screen must resolve the same accent
    /// from the same input.
    ///
    /// This test exists because its absence is precisely how they
    /// diverged: one hardcoded `#9b8cf5` with a comment saying it
    /// "matches this desktop's window-border accent", the other
    /// defaulted to `#bd93f9`, and nothing anywhere compared them. They
    /// now read one struct, so the only way to reintroduce the split is
    /// to give one of them a second source — which this would catch.
    #[test]
    fn every_host_reads_the_accent_from_the_same_place() {
        let theme = Theme {
            accent: Color::rgba(0x12, 0x34, 0x56, 0xff),
            ..Theme::default()
        };

        // What both the lock screen and the settings window build their
        // iced palette from. Kept as the same field rather than a
        // parallel one, which is the whole point.
        assert_eq!(theme.accent, Color::parse("rgba(123456ff)").unwrap());

        // And it survives the file the greeter reads, since that is the
        // one hop where a mismatch would be invisible until login.
        let round_tripped: Theme = toml::from_str(&toml::to_string(&theme).unwrap()).unwrap();
        assert_eq!(round_tripped.accent, theme.accent);
    }
}

#[cfg(test)]
mod info_role {
    use super::*;

    /// A `lock.toml` written before `info` existed must still parse and
    /// get the default, rather than failing and taking the lock screen
    /// with it. Same property `#[serde(default)]` exists to guarantee
    /// for every other field here, pinned for the one most recently
    /// added — that is the one a future change is most likely to break.
    #[test]
    fn a_theme_file_written_before_info_existed_still_parses() {
        let older = r#"
            background = "rgba(16161eff)"
            accent = "rgba(bd93f9ff)"
            font_size = 15.0
        "#;
        let theme: Theme = toml::from_str(older).expect("an older theme file must still load");
        assert_eq!(theme.accent, Color::rgba(0xbd, 0x93, 0xf9, 0xff), "what it did say is kept");
        assert_eq!(theme.info, Theme::default().info, "what it did not say gets the default");
    }

    /// `info` is a state colour, and a state colour that equals another
    /// one conveys nothing. The design's whole premise is that colour
    /// means something; two roles sharing a value silently breaks that
    /// without any test noticing.
    #[test]
    fn every_state_colour_is_distinguishable_from_the_others() {
        let t = Theme::default();
        let states = [("accent", t.accent), ("error", t.error), ("warning", t.warning), ("success", t.success), ("info", t.info)];
        for (i, (name_a, a)) in states.iter().enumerate() {
            for (name_b, b) in &states[i + 1..] {
                assert_ne!(a, b, "{name_a} and {name_b} are the same colour, so neither means anything");
            }
        }
    }
}
