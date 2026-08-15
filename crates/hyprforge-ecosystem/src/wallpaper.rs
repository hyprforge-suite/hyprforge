//! Wallpapers, via hyprpaper.
//!
//! A `wallpaper` block per monitor, plus the splash options. An entry
//! whose `monitor` is empty is hyprpaper's fallback — it applies to every
//! output that has no block of its own, which is what makes a setup
//! survive hotplugging. Per-monitor blocks keyed to connector names go
//! stale the moment the names change (`DP-4` becomes `DP-10` on a
//! different dock), and this machine's own config carries a comment
//! saying exactly that happened.
//!
//! `preload` is deliberately not generated. Older examples pair every
//! `wallpaper` block with a `preload` line, and hyprpaper 0.8.4 no longer
//! lists `preload` among its IPC requests at all; the current wiki example
//! is blocks alone.
//!
//! A `path` may be a **directory**, which hyprpaper cycles through — that
//! is what `timeout`, `order` and `recursive` are for, and they mean
//! nothing for a single image.

use hyprforge_core::hyprlang;
use serde::{Deserialize, Serialize};

/// How an image is fitted to the output.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum FitMode {
    /// Fills the screen, cropping the overflow. hyprpaper's default.
    #[default]
    Cover,
    /// Fits the whole image, leaving bars.
    Contain,
    /// Repeats the image.
    Tile,
    /// Stretches to fill, ignoring aspect ratio.
    Fill,
}

impl FitMode {
    pub const ALL: [FitMode; 4] = [FitMode::Cover, FitMode::Contain, FitMode::Tile, FitMode::Fill];

    pub fn as_str(self) -> &'static str {
        match self {
            FitMode::Cover => "cover",
            FitMode::Contain => "contain",
            FitMode::Tile => "tile",
            FitMode::Fill => "fill",
        }
    }

    pub fn parse(s: &str) -> Option<FitMode> {
        FitMode::ALL.into_iter().find(|m| m.as_str() == s.trim())
    }
}

impl std::fmt::Display for FitMode {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

/// One `wallpaper { … }` block.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Entry {
    /// Connector name, or empty for the fallback that covers every
    /// monitor without a block of its own.
    #[serde(default)]
    pub monitor: String,
    /// An image file, or a directory to cycle through.
    pub path: String,
    #[serde(default)]
    pub fit_mode: FitMode,
    /// Seconds between images when `path` is a directory. `None` leaves
    /// hyprpaper's own default (30s).
    #[serde(default)]
    pub timeout: Option<u32>,
    /// Shuffle a directory instead of taking it in order.
    #[serde(default)]
    pub random_order: bool,
    /// Descend into subdirectories when `path` is a directory.
    #[serde(default)]
    pub recursive: bool,
}

impl Entry {
    /// Whether this entry's directory-only options mean anything.
    ///
    /// Not a guess from the path string — a path that doesn't exist yet
    /// (a directory about to be created, a typo) should still be
    /// editable, so this asks the filesystem and treats "don't know" as
    /// "not a directory".
    pub fn is_directory(&self) -> bool {
        std::path::Path::new(&expand_tilde(&self.path)).is_dir()
    }

    /// Whether this is hyprpaper's fallback rather than a per-monitor
    /// block.
    pub fn is_fallback(&self) -> bool {
        self.monitor.trim().is_empty()
    }
}

/// Everything the wallpaper screen owns.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Settings {
    #[serde(default, rename = "wallpaper")]
    pub entries: Vec<Entry>,
    /// Hyprland's splash text over the wallpaper. `None` leaves
    /// hyprpaper's default (on).
    #[serde(default)]
    pub splash: Option<bool>,
    #[serde(default)]
    pub splash_offset: Option<f64>,
    #[serde(default)]
    pub splash_opacity: Option<f64>,
}

impl Settings {
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
            && self.splash.is_none()
            && self.splash_offset.is_none()
            && self.splash_opacity.is_none()
    }

    /// Entries that can't be written, with the reason.
    ///
    /// A blank path is the one that matters: hyprpaper takes
    /// `path = ` as a wallpaper with no image and the monitor ends up
    /// black, which reads as the app having broken something.
    pub fn invalid(&self) -> Vec<(usize, String)> {
        let mut out = Vec::new();
        for (i, entry) in self.entries.iter().enumerate() {
            if entry.path.trim().is_empty() {
                out.push((i, "needs an image or a folder".to_string()));
            } else if entry.timeout.is_some_and(|t| t == 0) {
                out.push((i, "a cycle time of 0 seconds would never advance".to_string()));
            }
        }
        // Two blocks for the same monitor: hyprpaper takes the last, so
        // the earlier one silently does nothing.
        let mut seen: Vec<&str> = Vec::new();
        for (i, entry) in self.entries.iter().enumerate() {
            let monitor = entry.monitor.trim();
            if seen.contains(&monitor) {
                out.push((
                    i,
                    if monitor.is_empty() {
                        "a second fallback — only the last one applies".to_string()
                    } else {
                        format!("a second block for {monitor} — only the last one applies")
                    },
                ));
            }
            seen.push(monitor);
        }
        out
    }
}

/// `~` is expanded here rather than left to hyprlang.
///
/// hyprlang does expand tildes, but the app also has to *check* whether a
/// path is a directory to know if the cycling options apply, and
/// `Path::is_dir` does not expand anything. Doing it in one place keeps
/// the check and the generated file agreeing.
pub fn expand_tilde(path: &str) -> String {
    let path = path.trim();
    let Some(rest) = path.strip_prefix('~') else {
        return path.to_string();
    };
    let Some(home) = std::env::var_os("HOME") else {
        return path.to_string();
    };
    format!("{}{}", home.to_string_lossy(), rest)
}

/// Renders the generated `wallpaper.conf`.
pub fn generate(settings: &Settings) -> String {
    let bad: Vec<usize> = settings.invalid().into_iter().map(|(i, _)| i).collect();
    let mut out = hyprlang::header("wallpapers");

    for (name, value) in [
        ("splash", settings.splash.map(|v| v.to_string())),
        ("splash_offset", settings.splash_offset.map(|v| format!("{v}"))),
        ("splash_opacity", settings.splash_opacity.map(|v| format!("{v}"))),
    ] {
        if let Some(value) = value {
            out.push_str(&hyprlang::keyword(name, value));
        }
    }

    for (i, entry) in settings.entries.iter().enumerate() {
        if bad.contains(&i) {
            continue;
        }
        out.push('\n');
        out.push_str(&render_one(entry));
    }
    out
}

/// One `wallpaper` block, exactly as [`generate`] writes it. Public so an
/// editor can show the user the block their form produces — the same
/// function, so a preview can't drift into a plausible-looking lie.
pub fn render_one(entry: &Entry) -> String {
    // `monitor` is always emitted, empty included: an omitted `monitor`
    // is not the same as an empty one, and the empty form is precisely
    // how a fallback is declared.
    let mut fields = vec![
        ("monitor", entry.monitor.trim().to_string()),
        ("path", expand_tilde(&entry.path)),
        ("fit_mode", entry.fit_mode.to_string()),
    ];
    // Only meaningful for a directory, and hyprpaper ignores them
    // otherwise — but writing them anyway would suggest, to anyone
    // reading the file, that a single image cycles.
    if entry.is_directory() {
        if let Some(timeout) = entry.timeout {
            fields.push(("timeout", timeout.to_string()));
        }
        if entry.random_order {
            fields.push(("order", "random".to_string()));
        }
        if entry.recursive {
            fields.push(("recursive", "true".to_string()));
        }
    }
    hyprlang::block("wallpaper", &fields)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(monitor: &str, path: &str) -> Entry {
        Entry {
            monitor: monitor.to_string(),
            path: path.to_string(),
            ..Entry::default()
        }
    }

    #[test]
    fn a_block_carries_the_monitor_path_and_fit() {
        let out = render_one(&entry("eDP-2", "/w/a.png"));
        assert_eq!(
            out,
            "wallpaper {\n    monitor = eDP-2\n    path = /w/a.png\n    fit_mode = cover\n}\n"
        );
    }

    /// An omitted `monitor` is not the same as an empty one — the empty
    /// form is precisely how hyprpaper's fallback is declared, and it is
    /// what makes a setup survive connector names changing.
    #[test]
    fn a_fallback_emits_an_empty_monitor_rather_than_omitting_it() {
        let out = render_one(&entry("", "/w/a.png"));
        assert!(out.contains("monitor = \n"), "{out}");
        assert!(entry("", "/w/a.png").is_fallback());
        assert!(!entry("eDP-2", "/w/a.png").is_fallback());
    }

    /// Cycling options mean nothing for a single image, and writing them
    /// would suggest to anyone reading the file that it cycles.
    #[test]
    fn cycling_options_are_only_written_for_a_directory() {
        let mut e = entry("", "/w/a.png");
        e.timeout = Some(60);
        e.random_order = true;
        e.recursive = true;
        let out = render_one(&e);
        assert!(!out.contains("timeout"), "{out}");
        assert!(!out.contains("order"), "{out}");
        assert!(!out.contains("recursive"), "{out}");
    }

    #[test]
    fn cycling_options_are_written_for_a_real_directory() {
        let dir = tempfile::tempdir().unwrap();
        let mut e = entry("", dir.path().to_str().unwrap());
        e.timeout = Some(60);
        e.random_order = true;
        e.recursive = true;
        assert!(e.is_directory());
        let out = render_one(&e);
        assert!(out.contains("timeout = 60"), "{out}");
        assert!(out.contains("order = random"), "{out}");
        assert!(out.contains("recursive = true"), "{out}");
    }

    /// hyprlang expands tildes, but `Path::is_dir` doesn't — so the check
    /// and the generated file have to agree on one expanded form.
    #[test]
    fn a_tilde_is_expanded_once_so_the_check_and_the_file_agree() {
        let home = std::env::var("HOME").unwrap();
        assert_eq!(expand_tilde("~/Pictures/a.png"), format!("{home}/Pictures/a.png"));
        assert_eq!(expand_tilde("/absolute/a.png"), "/absolute/a.png");
        assert_eq!(expand_tilde("  ~/a.png  "), format!("{home}/a.png"));
    }

    /// hyprpaper takes `path = ` as a wallpaper with no image and the
    /// monitor goes black, which reads as the app breaking something.
    #[test]
    fn an_entry_without_a_path_is_reported_and_skipped() {
        let mut s = Settings::default();
        s.entries.push(entry("", ""));
        s.entries.push(entry("eDP-2", "/w/a.png"));
        assert_eq!(s.invalid().len(), 1);
        let out = generate(&s);
        assert_eq!(out.matches("wallpaper {").count(), 1, "{out}");
        assert!(out.contains("/w/a.png"), "{out}");
    }

    /// hyprpaper takes the last block for a monitor, so an earlier
    /// duplicate silently does nothing.
    #[test]
    fn a_duplicate_monitor_is_reported() {
        let mut s = Settings::default();
        s.entries.push(entry("eDP-2", "/a.png"));
        s.entries.push(entry("eDP-2", "/b.png"));
        let problems = s.invalid();
        assert_eq!(problems.len(), 1);
        assert_eq!(problems[0].0, 1);
        assert!(problems[0].1.contains("eDP-2"), "{}", problems[0].1);
    }

    #[test]
    fn a_second_fallback_is_reported_as_such() {
        let mut s = Settings::default();
        s.entries.push(entry("", "/a.png"));
        s.entries.push(entry("", "/b.png"));
        assert!(s.invalid()[0].1.contains("fallback"), "{:?}", s.invalid());
    }

    #[test]
    fn a_zero_cycle_time_is_refused() {
        let mut s = Settings::default();
        let mut e = entry("", "/a.png");
        e.timeout = Some(0);
        s.entries.push(e);
        assert_eq!(s.invalid().len(), 1);
    }

    #[test]
    fn splash_options_are_only_written_when_set() {
        let mut s = Settings::default();
        assert!(!generate(&s).contains("splash"));
        s.splash = Some(false);
        s.splash_opacity = Some(0.5);
        let out = generate(&s);
        assert!(out.contains("splash = false"), "{out}");
        assert!(out.contains("splash_opacity = 0.5"), "{out}");
        assert!(!out.contains("splash_offset"), "unset stays unwritten: {out}");
    }

    #[test]
    fn an_empty_set_is_still_a_loadable_file() {
        let out = generate(&Settings::default());
        assert!(out.starts_with("# Generated by Hyprforge"));
        assert!(!out.contains("wallpaper {"));
    }

    #[test]
    fn fit_modes_round_trip_through_their_names() {
        for mode in FitMode::ALL {
            assert_eq!(FitMode::parse(mode.as_str()), Some(mode));
        }
        assert_eq!(FitMode::parse("stretch"), None);
    }

    #[test]
    fn output_is_stable_across_runs() {
        let mut s = Settings::default();
        s.entries.push(entry("eDP-2", "/a.png"));
        s.entries.push(entry("", "/b.png"));
        assert_eq!(generate(&s), generate(&s));
    }
}
