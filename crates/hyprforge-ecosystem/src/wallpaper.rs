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
use hyprforge_core::supersede;
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

/// Where Hyprforge keeps the wallpaper settings.
///
/// One definition, because two programs write them — the Desktop screen
/// in Settings and the image viewer's Set as Wallpaper — and a second
/// spelling of the path is how one of them ends up editing a file the
/// other never reads.
pub fn settings_path() -> std::path::PathBuf {
    hyprforge_core::paths::hyprforge_config_dir().join("wallpaper.toml")
}

/// The generated hyprpaper config the settings are rendered into.
pub fn generated_path() -> std::path::PathBuf {
    hyprforge_core::paths::hypr_hyprforge_dir().join("wallpaper.conf")
}

/// hyprpaper's own config, which gains one `source =` line pointing at
/// [`generated_path`] and is otherwise the user's.
pub fn hyprpaper_conf_path() -> std::path::PathBuf {
    hyprforge_core::paths::hypr_config_dir().join("hyprpaper.conf")
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
    /// Shows `image` on every monitor — what "set as wallpaper" means from
    /// the image viewer.
    ///
    /// Every existing entry keeps its monitor and fit mode and takes the
    /// new picture, and a fallback is added if there is none, so a
    /// monitor without a block of its own shows it too. The directory-only
    /// options are cleared: they describe cycling through a folder, and an
    /// entry that now names one picture has nothing to cycle. Nothing is
    /// removed — a per-monitor block the user wrote is kept, pointing at
    /// the new picture, rather than deleted to make "everywhere" simpler.
    pub fn set_everywhere(&mut self, image: &str) {
        for entry in &mut self.entries {
            entry.path = image.to_string();
            entry.timeout = None;
            entry.random_order = false;
            entry.recursive = false;
        }
        if !self.entries.iter().any(Entry::is_fallback) {
            self.entries.push(Entry { path: image.to_string(), ..Entry::default() });
        }
    }

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
        // every earlier one silently does nothing.
        //
        // The earlier one is what gets flagged, and that is load-bearing
        // rather than cosmetic — `generate` skips whatever lands here, so
        // flagging the block that actually applies dropped it and wrote the
        // superseded one instead. Setting a new wallpaper and leaving the
        // old block above it left the old image on screen.
        for i in supersede::superseded(&self.entries, |e| Some(e.monitor.trim().to_string())) {
            let monitor = self.entries[i].monitor.trim();
            out.push((
                i,
                if monitor.is_empty() {
                    "a second fallback below replaces this one".to_string()
                } else {
                    format!("a second block for {monitor} below replaces this one")
                },
            ));
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

/// The one image to show behind the lock screen and the greeter.
///
/// Those screens have a single background, while this list has one entry
/// per monitor, so something has to be chosen. The fallback entry — the
/// one with no monitor name, which hyprpaper applies to every output
/// that has no block of its own — is the closest thing to "the
/// desktop's wallpaper", so it wins. Otherwise the first entry, which is
/// the one the user added first.
///
/// A directory is skipped rather than resolved. A directory means the
/// wallpaper is cycling, and picking one image out of a rotation would
/// show something that is probably not what is on screen — a flat
/// background is the honest answer.
///
/// `None` means draw the theme's flat background, which is not a
/// failure: it is the normal case on a fresh install.
pub fn for_auth_screen(settings: &Settings) -> Option<std::path::PathBuf> {
    let usable = |entry: &&Entry| {
        let path = std::path::PathBuf::from(expand_tilde(&entry.path));
        path.is_file().then_some(path)
    };
    // The *last* fallback, not the first: hyprpaper takes the last block
    // for a given monitor, so reading the first would put a different
    // image behind the lock screen than the one on the desktop — two of
    // this suite's own apps disagreeing about the same setting, which is
    // the failure it exists to prevent.
    settings
        .entries
        .iter()
        .rev()
        .find(|e| e.is_fallback())
        .and_then(|e| usable(&e))
        .or_else(|| settings.entries.iter().find_map(|e| usable(&e)))
}

#[cfg(test)]
mod auth_screen_fallback {
    use super::*;

    /// hyprpaper takes the last block for a monitor. If the auth screens
    /// read the first, the lock screen and the desktop show different
    /// images from the same file.
    #[test]
    fn the_auth_screen_takes_the_fallback_hyprpaper_actually_uses() {
        let dir = tempfile::tempdir().unwrap();
        let old = dir.path().join("old.png");
        let new = dir.path().join("new.png");
        std::fs::write(&old, b"x").unwrap();
        std::fs::write(&new, b"x").unwrap();

        let settings = Settings {
            entries: vec![
                Entry { monitor: String::new(), path: old.display().to_string(), ..Entry::default() },
                Entry { monitor: String::new(), path: new.display().to_string(), ..Entry::default() },
            ],
            ..Settings::default()
        };
        assert_eq!(for_auth_screen(&settings), Some(new));
    }
}

/// Images and folders worth offering as wallpapers.
///
/// **Loose image files only come from directories that are *about*
/// wallpapers.** Scanning `~/Pictures` for images found 105 entries on
/// this machine, 100 of them hyprshot screenshots — a picker nobody could
/// use. Screenshots live in the same folder as wallpapers and are not
/// wallpapers, and no filename rule separates them, so the directory a
/// file sits in is the only honest signal available.
///
/// `~/Pictures` still contributes its **subfolders**, since a folder is a
/// legitimate choice — hyprpaper cycles it — and that is how someone with
/// `~/Pictures/Nature` reaches it without every loose file coming too.
///
/// Deliberately shallow: one level per root. A recursive scan of a large
/// picture library would stall the screen opening, and hyprpaper's own
/// `recursive` option already covers deep trees once a folder is chosen.
pub fn discover_images() -> Vec<String> {
    let Some(home) = std::env::var_os("HOME").map(std::path::PathBuf::from) else {
        return Vec::new();
    };
    let mut found = std::collections::BTreeSet::new();

    // Directories that exist to hold wallpapers: take their images.
    for root in [
        home.join("Pictures/Wallpapers"),
        home.join("Wallpapers"),
        home.join(".local/share/wallpapers"),
        home.join(".local/share/backgrounds"),
    ] {
        collect(&root, true, &mut found);
    }
    // General picture directories: folders only. `~/Downloads` is
    // deliberately not among them — its subfolders are unpacked archives,
    // not wallpaper collections, and they crowded out the real entries.
    collect(&home.join("Pictures"), false, &mut found);
    found.into_iter().collect()
}

/// Whether a path looks like an image this crate would offer.
pub fn is_image(path: &std::path::Path) -> bool {
    const EXTENSIONS: &[&str] = &["png", "jpg", "jpeg", "webp", "jxl", "bmp"];
    path.extension()
        .and_then(|e| e.to_str())
        .is_some_and(|e| EXTENSIONS.contains(&e.to_lowercase().as_str()))
}

fn collect(
    root: &std::path::Path,
    include_files: bool,
    found: &mut std::collections::BTreeSet<String>,
) {
    let Ok(read) = std::fs::read_dir(root) else {
        return;
    };
    let mut has_images = false;
    for entry in read.flatten() {
        let path = entry.path();
        if path.is_dir() {
            found.insert(path.to_string_lossy().to_string());
        } else if is_image(&path) {
            has_images = true;
            if include_files {
                found.insert(path.to_string_lossy().to_string());
            }
        }
    }
    // The root itself, when there is something in it to cycle.
    if has_images {
        found.insert(root.to_string_lossy().to_string());
    }
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
    ///
    /// This asserted index 1 — the block that applies — which contradicted
    /// the sentence above it and, because `generate` skips flagged rows,
    /// meant the winning block was the one thrown away.
    #[test]
    fn the_duplicate_block_that_loses_is_the_one_reported() {
        let mut s = Settings::default();
        s.entries.push(entry("eDP-2", "/a.png"));
        s.entries.push(entry("eDP-2", "/b.png"));
        let problems = s.invalid();
        assert_eq!(problems.len(), 1);
        assert_eq!(problems[0].0, 0, "the dead block is the first one");
        assert!(problems[0].1.contains("eDP-2"), "{}", problems[0].1);
    }

    /// The consequence, stated directly: picking a new wallpaper without
    /// deleting the old block left the old image on screen.
    #[test]
    fn a_duplicated_monitor_still_writes_the_block_that_wins() {
        let mut s = Settings::default();
        s.entries.push(entry("eDP-2", "/old.png"));
        s.entries.push(entry("eDP-2", "/new.png"));
        let out = generate(&s);
        assert!(out.contains("/new.png"), "the block that applies must be written: {out}");
        assert!(!out.contains("/old.png"), "the superseded block must not be: {out}");
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

    /// A dedicated wallpaper directory contributes its images and
    /// itself; a general picture directory contributes only folders.
    /// Scanning `~/Pictures` for images found 105 entries on this
    /// machine, 100 of them screenshots.
    #[test]
    fn only_wallpaper_directories_contribute_loose_files() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("Pictures");
        std::fs::create_dir_all(root.join("Screenshots")).unwrap();
        std::fs::write(root.join("shot.png"), b"").unwrap();
        std::fs::write(root.join("notes.txt"), b"").unwrap();

        let mut folders_only = std::collections::BTreeSet::new();
        collect(&root, false, &mut folders_only);
        assert!(folders_only.contains(&root.join("Screenshots").to_string_lossy().to_string()));
        assert!(
            !folders_only.contains(&root.join("shot.png").to_string_lossy().to_string()),
            "a loose screenshot is not a wallpaper"
        );
        assert!(
            folders_only.contains(&root.to_string_lossy().to_string()),
            "the folder itself is still cyclable"
        );

        let mut with_files = std::collections::BTreeSet::new();
        collect(&root, true, &mut with_files);
        assert!(with_files.contains(&root.join("shot.png").to_string_lossy().to_string()));
        assert!(
            !with_files.contains(&root.join("notes.txt").to_string_lossy().to_string()),
            "non-images never appear"
        );
    }

    #[test]
    fn image_extensions_are_matched_case_insensitively() {
        assert!(is_image(std::path::Path::new("/a/B.PNG")));
        assert!(is_image(std::path::Path::new("/a/b.jxl")));
        assert!(!is_image(std::path::Path::new("/a/b.txt")));
        assert!(!is_image(std::path::Path::new("/a/b")));
    }

    /// An empty folder has nothing to cycle, so offering it would be a
    /// choice that produces no wallpaper.
    #[test]
    fn a_folder_with_no_images_is_not_offered_as_itself() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("Empty");
        std::fs::create_dir_all(&root).unwrap();
        let mut found = std::collections::BTreeSet::new();
        collect(&root, true, &mut found);
        assert!(!found.contains(&root.to_string_lossy().to_string()));
    }

    #[test]
    fn discovery_never_panics_on_a_real_machine() {
        let _ = discover_images();
    }

    #[test]
    fn output_is_stable_across_runs() {
        let mut s = Settings::default();
        s.entries.push(entry("eDP-2", "/a.png"));
        s.entries.push(entry("", "/b.png"));
        assert_eq!(generate(&s), generate(&s));
    }
}


#[cfg(test)]
mod auth_screen_wallpaper {
    use super::*;

    fn entry(monitor: &str, path: &std::path::Path) -> Entry {
        Entry {
            monitor: monitor.into(),
            path: path.display().to_string(),
            ..Entry::default()
        }
    }

    /// The fallback entry is the one hyprpaper applies to every output
    /// without a block of its own, so it is the closest thing this list
    /// has to "the desktop's wallpaper".
    #[test]
    fn the_every_monitor_entry_wins_over_a_specific_one() {
        let dir = tempfile::tempdir().unwrap();
        let (specific, every) = (dir.path().join("a.png"), dir.path().join("b.png"));
        std::fs::write(&specific, b"x").unwrap();
        std::fs::write(&every, b"x").unwrap();

        let settings = Settings {
            entries: vec![entry("eDP-2", &specific), entry("", &every)],
            ..Settings::default()
        };
        assert_eq!(for_auth_screen(&settings), Some(every));
    }

    /// With only per-monitor entries there is no right answer, so it
    /// takes the first — the one the user added first.
    #[test]
    fn without_a_fallback_the_first_entry_is_used() {
        let dir = tempfile::tempdir().unwrap();
        let first = dir.path().join("a.png");
        std::fs::write(&first, b"x").unwrap();
        let settings = Settings {
            entries: vec![entry("eDP-2", &first), entry("HDMI-A-1", &dir.path().join("b.png"))],
            ..Settings::default()
        };
        assert_eq!(for_auth_screen(&settings), Some(first));
    }

    /// A directory means the wallpaper is cycling. Picking one image out
    /// of a rotation would show something that is probably not what is
    /// on screen, and a flat background is the honest answer.
    #[test]
    fn a_cycling_directory_is_skipped_rather_than_resolved() {
        let dir = tempfile::tempdir().unwrap();
        let settings = Settings {
            entries: vec![entry("", dir.path())],
            ..Settings::default()
        };
        assert_eq!(for_auth_screen(&settings), None);
    }

    /// A path that no longer exists must not reach the greeter, which
    /// would then show nothing and invite someone to debug a permissions
    /// problem that isn't there.
    #[test]
    fn a_missing_file_falls_through_to_the_next_usable_entry() {
        let dir = tempfile::tempdir().unwrap();
        let real = dir.path().join("real.png");
        std::fs::write(&real, b"x").unwrap();
        let settings = Settings {
            entries: vec![entry("", &dir.path().join("gone.png")), entry("eDP-2", &real)],
            ..Settings::default()
        };
        assert_eq!(for_auth_screen(&settings), Some(real));
    }

    /// No wallpaper configured is the normal case on a fresh install.
    #[test]
    fn nothing_configured_means_nothing_to_show() {
        assert_eq!(for_auth_screen(&Settings::default()), None);
    }

    #[test]
    fn setting_a_wallpaper_with_nothing_configured_adds_the_fallback() {
        let mut settings = Settings::default();
        settings.set_everywhere("/pics/a.jpg");
        assert_eq!(settings.entries.len(), 1);
        assert!(settings.entries[0].is_fallback());
        assert_eq!(settings.entries[0].path, "/pics/a.jpg");
    }

    /// A block the user wrote for one monitor keeps its monitor and fit,
    /// and shows the new picture — "everywhere" is not permission to
    /// delete it.
    #[test]
    fn setting_a_wallpaper_keeps_every_monitor_block_and_points_it_at_the_picture() {
        let mut settings = Settings {
            entries: vec![Entry {
                monitor: "DP-1".to_string(),
                path: "/walls".to_string(),
                fit_mode: FitMode::Contain,
                timeout: Some(60),
                random_order: true,
                recursive: true,
            }],
            ..Settings::default()
        };
        settings.set_everywhere("/pics/a.jpg");

        let dp1 = &settings.entries[0];
        assert_eq!((dp1.monitor.as_str(), dp1.path.as_str()), ("DP-1", "/pics/a.jpg"));
        assert_eq!(dp1.fit_mode, FitMode::Contain);
        assert_eq!((dp1.timeout, dp1.random_order, dp1.recursive), (None, false, false));
        assert!(
            settings.entries.iter().any(|e| e.is_fallback() && e.path == "/pics/a.jpg"),
            "a monitor with no block of its own must get the picture too"
        );
        assert!(settings.invalid().is_empty(), "{:?}", settings.invalid());
    }
}
