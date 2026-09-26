//! Finding an icon's file from its name, through the configured icon
//! theme.
//!
//! The freedesktop icon theme specification, and only the parts of it a
//! lookup needs: a theme's `index.theme` names its directories and what
//! size each holds, and names the themes it `Inherits=` from; a lookup
//! searches the configured theme, then what it inherits depth first,
//! then `hicolor`, then `/usr/share/pixmaps`.
//!
//! # Why the inheritance walk is the whole point
//!
//! It is the part that is tempting to skip and the part this machine
//! depends on. The configured theme here, Dracula, ships no `mimetypes/`
//! directory at all and names eight parents of which six are not
//! installed — so every file-type icon resolves through `breeze-dark`,
//! two levels down, or not at all. A lookup that only searched the named
//! theme and hicolor would find almost nothing on this machine and look
//! finished on a tidier one.
//!
//! # Theme first, then name
//!
//! Given several names — `text-x-rust`, then `text-x-script`, then
//! `text-x-generic` — this searches every name in one theme before
//! moving to the next theme, which is the order the specification's
//! `FindBestIcon` gives. It means a generic icon from the user's own
//! theme beats a specific one from a theme it inherits. That is the
//! right way round: a listing whose icons come from three different
//! themes looks broken even when every one of them resolved, which the
//! tray learned the hard way (see CLAUDE.md on two icon names that both
//! resolve and still look wrong together).
//!
//! # What this does not do
//!
//! No `.xpm`: nothing in this suite can draw one, and a path to a file
//! the caller cannot draw is worse than no answer, since it stops the
//! caller falling back to something it can.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

/// The extensions looked for, in the specification's order of
/// preference.
const EXTENSIONS: [&str; 2] = ["png", "svg"];

/// A loaded theme chain, ready to answer lookups.
///
/// Loading reads a handful of `index.theme` files; looking up stats
/// files. Every answer is remembered, so a listing of ten thousand PNGs
/// asks the disk about `image-png` once.
#[derive(Debug)]
pub struct Icons {
    /// The configured theme, then everything it inherits that is
    /// installed, then `hicolor` — see [`Icons::chain`].
    themes: Vec<Theme>,
    /// `/usr/share/pixmaps` and friends: the specification's last resort,
    /// flat directories with no sizes.
    pixmaps: Vec<PathBuf>,
    cache: Mutex<HashMap<LookupKey, Option<PathBuf>>>,
}

/// The names asked for, the size, and the scale.
type LookupKey = (Vec<String>, u32, u32);

/// One installed theme.
#[derive(Debug, Clone)]
struct Theme {
    name: String,
    /// Every base directory holding a copy of this theme — a user's
    /// `~/.local/share/icons/hicolor` adds to the system's rather than
    /// replacing it.
    roots: Vec<PathBuf>,
    dirs: Vec<Subdir>,
}

/// One entry of `index.theme`'s `Directories=`.
#[derive(Debug, Clone, PartialEq)]
struct Subdir {
    path: String,
    size: u32,
    scale: u32,
    kind: Kind,
    min: u32,
    max: u32,
    threshold: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Kind {
    Fixed,
    Scalable,
    Threshold,
}

impl Icons {
    /// The chain for the configured theme, from the usual directories.
    pub fn load() -> Icons {
        let theme = configured_theme().unwrap_or_else(|| "hicolor".to_string());
        Icons::load_from(&theme, &base_dirs(), &pixmap_dirs())
    }

    /// [`Icons::load`] for a named theme and given directories — the seam
    /// the tests use.
    ///
    /// A theme that is not installed is not an error: it is skipped, as
    /// the specification says, and `hicolor` still answers. The same
    /// goes for a parent named in `Inherits=` and absent, which is the
    /// ordinary case on this machine rather than a broken one.
    pub fn load_from(theme: &str, base_dirs: &[PathBuf], pixmaps: &[PathBuf]) -> Icons {
        let mut themes = Vec::new();
        let mut visited = Vec::new();
        visit(theme, base_dirs, &mut themes, &mut visited);
        // hicolor ends every chain by specification, whether or not
        // anything named it.
        visit("hicolor", base_dirs, &mut themes, &mut visited);
        Icons { themes, pixmaps: pixmaps.to_vec(), cache: Mutex::new(HashMap::new()) }
    }

    /// The themes a lookup searches, in order. Only installed ones.
    pub fn chain(&self) -> Vec<&str> {
        self.themes.iter().map(|t| t.name.as_str()).collect()
    }

    /// The best file for the first of `names` any theme in the chain
    /// has, at `size` logical pixels on a display of integer `scale`.
    ///
    /// `None` when nothing anywhere has any of them — the caller's own
    /// fallback, not a failure.
    pub fn lookup(&self, names: &[&str], size: u32, scale: u32) -> Option<PathBuf> {
        let key = (names.iter().map(|n| n.to_string()).collect::<Vec<_>>(), size, scale);
        if let Some(found) = self.cache.lock().ok().and_then(|c| c.get(&key).cloned()) {
            return found;
        }
        let found = self
            .themes
            .iter()
            .find_map(|theme| names.iter().find_map(|name| theme.lookup(name, size, scale)))
            .or_else(|| names.iter().find_map(|name| self.pixmap(name)));
        if let Ok(mut cache) = self.cache.lock() {
            cache.insert(key, found.clone());
        }
        found
    }

    fn pixmap(&self, name: &str) -> Option<PathBuf> {
        self.pixmaps.iter().find_map(|dir| {
            EXTENSIONS.iter().map(|ext| dir.join(format!("{name}.{ext}"))).find(|p| p.is_file())
        })
    }
}

/// Adds `name` and, depth first, everything it inherits.
fn visit(name: &str, base_dirs: &[PathBuf], themes: &mut Vec<Theme>, visited: &mut Vec<String>) {
    if visited.iter().any(|v| v == name) {
        return;
    }
    visited.push(name.to_string());
    let roots: Vec<PathBuf> = base_dirs.iter().map(|b| b.join(name)).filter(|p| p.is_dir()).collect();
    // The first copy with an index is the one that describes the theme;
    // a copy without one only contributes files.
    let Some(index) = roots.iter().find_map(|r| std::fs::read_to_string(r.join("index.theme")).ok()) else {
        return;
    };
    let (dirs, parents) = parse_index(&index);
    themes.push(Theme { name: name.to_string(), roots, dirs });
    for parent in parents {
        visit(&parent, base_dirs, themes, visited);
    }
}

impl Theme {
    /// The specification's `LookupIcon`: an exact size match in any
    /// directory first, and only then the closest.
    fn lookup(&self, name: &str, size: u32, scale: u32) -> Option<PathBuf> {
        for dir in self.dirs.iter().filter(|d| d.matches(size, scale)) {
            if let Some(found) = self.find_in(dir, name) {
                return Some(found);
            }
        }
        let mut best: Option<(u32, PathBuf)> = None;
        for dir in &self.dirs {
            let distance = dir.distance(size, scale);
            if best.as_ref().is_some_and(|(d, _)| *d <= distance) {
                continue;
            }
            if let Some(found) = self.find_in(dir, name) {
                best = Some((distance, found));
            }
        }
        best.map(|(_, path)| path)
    }

    /// `name` in one directory of this theme, in any copy of the theme.
    fn find_in(&self, dir: &Subdir, name: &str) -> Option<PathBuf> {
        self.roots.iter().find_map(|root| {
            EXTENSIONS.iter().map(|ext| root.join(&dir.path).join(format!("{name}.{ext}"))).find(|p| p.is_file())
        })
    }
}

impl Subdir {
    /// The specification's `DirectoryMatchesSize`.
    fn matches(&self, size: u32, scale: u32) -> bool {
        if self.scale != scale {
            return false;
        }
        match self.kind {
            Kind::Fixed => self.size == size,
            Kind::Scalable => (self.min..=self.max).contains(&size),
            Kind::Threshold => {
                (self.size.saturating_sub(self.threshold)..=self.size + self.threshold).contains(&size)
            }
        }
    }

    /// The specification's `DirectorySizeDistance`, in physical pixels
    /// so a 2x directory and a 1x one compare fairly.
    fn distance(&self, size: u32, scale: u32) -> u32 {
        let want = size * scale;
        let (low, high) = match self.kind {
            Kind::Fixed => (self.size * self.scale, self.size * self.scale),
            Kind::Scalable => (self.min * self.scale, self.max * self.scale),
            Kind::Threshold => (
                self.size.saturating_sub(self.threshold) * self.scale,
                (self.size + self.threshold) * self.scale,
            ),
        };
        if want < low {
            low - want
        } else {
            want.saturating_sub(high)
        }
    }
}

/// An `index.theme`'s directories and parents.
///
/// A directory listed in `Directories=` (or `ScaledDirectories=`) with no
/// section of its own, or a `Size` that does not parse, is dropped: the
/// specification makes `Size` mandatory, and guessing one would put an
/// icon at the wrong size rather than not at all.
fn parse_index(text: &str) -> (Vec<Subdir>, Vec<String>) {
    let mut sections: HashMap<String, HashMap<String, String>> = HashMap::new();
    let mut current = String::new();
    for line in text.lines().map(str::trim) {
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        if let Some(name) = line.strip_prefix('[').and_then(|l| l.strip_suffix(']')) {
            current = name.to_string();
            continue;
        }
        if let Some((key, value)) = line.split_once('=') {
            sections.entry(current.clone()).or_default().insert(key.trim().to_string(), value.trim().to_string());
        }
    }
    let list = |section: &HashMap<String, String>, key: &str| -> Vec<String> {
        section
            .get(key)
            .map(|v| v.split(',').map(str::trim).filter(|s| !s.is_empty()).map(String::from).collect())
            .unwrap_or_default()
    };
    let Some(head) = sections.get("Icon Theme") else {
        return (Vec::new(), Vec::new());
    };
    let parents = list(head, "Inherits");
    let mut names = list(head, "Directories");
    names.extend(list(head, "ScaledDirectories"));

    let mut dirs = Vec::new();
    for path in names {
        let Some(section) = sections.get(&path) else { continue };
        let number = |key: &str| section.get(key).and_then(|v| v.parse::<u32>().ok());
        let Some(size) = number("Size") else { continue };
        let kind = match section.get("Type").map(String::as_str) {
            Some("Fixed") => Kind::Fixed,
            Some("Scalable") => Kind::Scalable,
            _ => Kind::Threshold,
        };
        if dirs.iter().any(|d: &Subdir| d.path == path) {
            continue;
        }
        dirs.push(Subdir {
            path,
            size,
            scale: number("Scale").unwrap_or(1).max(1),
            kind,
            min: number("MinSize").unwrap_or(size),
            max: number("MaxSize").unwrap_or(size),
            threshold: number("Threshold").unwrap_or(2),
        });
    }
    (dirs, parents)
}

/// Where themes live, by specification: `~/.icons` (for compatibility),
/// then `$XDG_DATA_HOME/icons`, then each of `$XDG_DATA_DIRS`.
pub fn base_dirs() -> Vec<PathBuf> {
    let home = std::env::var_os("HOME").map(PathBuf::from);
    let mut dirs = Vec::new();
    if let Some(home) = &home {
        dirs.push(home.join(".icons"));
    }
    match std::env::var_os("XDG_DATA_HOME").filter(|v| !v.is_empty()) {
        Some(data) => dirs.push(PathBuf::from(data).join("icons")),
        None => dirs.extend(home.map(|h| h.join(".local/share/icons"))),
    }
    dirs.extend(data_dirs().into_iter().map(|d| d.join("icons")));
    dirs
}

/// The flat fallback directories — `pixmaps` under each data directory.
pub fn pixmap_dirs() -> Vec<PathBuf> {
    data_dirs().into_iter().map(|d| d.join("pixmaps")).collect()
}

fn data_dirs() -> Vec<PathBuf> {
    let value = std::env::var("XDG_DATA_DIRS").ok().filter(|v| !v.is_empty());
    value
        .as_deref()
        .unwrap_or("/usr/local/share:/usr/share")
        .split(':')
        .filter(|s| !s.is_empty())
        .map(PathBuf::from)
        .collect()
}

/// The icon theme the desktop is set to: gsettings first, since that is
/// what the Settings app writes, and GTK's own `settings.ini` for a
/// machine that has never run gsettings.
///
/// `None` when neither says — the caller falls back to `hicolor`, which
/// is what the specification says an unset theme means.
pub fn configured_theme() -> Option<String> {
    from_gsettings().or_else(|| {
        let config = std::env::var_os("XDG_CONFIG_HOME")
            .filter(|v| !v.is_empty())
            .map(PathBuf::from)
            .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".config")))?;
        from_settings_ini(&config.join("gtk-3.0/settings.ini"))
    })
}

fn from_gsettings() -> Option<String> {
    let output = hyprforge_process::output(
        std::process::Command::new("gsettings").args(["get", "org.gnome.desktop.interface", "icon-theme"]),
        hyprforge_process::TIMEOUT,
    )
    .ok()
    .filter(|o| o.status.success())?;
    unquote(&String::from_utf8_lossy(&output.stdout))
}

/// `'Dracula'` to `Dracula`: gsettings prints a GVariant string.
fn unquote(value: &str) -> Option<String> {
    let value = value.trim().trim_matches('\'').trim();
    (!value.is_empty()).then(|| value.to_string())
}

fn from_settings_ini(path: &Path) -> Option<String> {
    let text = std::fs::read_to_string(path).ok()?;
    text.lines()
        .filter_map(|l| l.split_once('='))
        .find(|(k, _)| k.trim() == "gtk-icon-theme-name")
        .and_then(|(_, v)| unquote(v.trim().trim_matches('"')))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    /// A throwaway icon root: `theme(name, index, files)`.
    struct Fixture {
        dir: tempfile::TempDir,
    }

    impl Fixture {
        fn new() -> Fixture {
            Fixture { dir: tempfile::tempdir().unwrap() }
        }
        fn base(&self) -> PathBuf {
            self.dir.path().join("icons")
        }
        fn theme(&self, name: &str, inherits: &str, dirs: &[(&str, &str)], files: &[&str]) {
            let root = self.base().join(name);
            let mut index = format!(
                "[Icon Theme]\nName={name}\nInherits={inherits}\nDirectories={}\n",
                dirs.iter().map(|(p, _)| *p).collect::<Vec<_>>().join(",")
            );
            for (path, section) in dirs {
                index.push_str(&format!("\n[{path}]\n{section}\n"));
            }
            fs::create_dir_all(&root).unwrap();
            fs::write(root.join("index.theme"), index).unwrap();
            for file in files {
                let path = root.join(file);
                fs::create_dir_all(path.parent().unwrap()).unwrap();
                fs::write(path, b"x").unwrap();
            }
        }
        fn icons(&self, theme: &str) -> Icons {
            Icons::load_from(theme, &[self.base()], &[self.dir.path().join("pixmaps")])
        }
    }

    const M48: (&str, &str) = ("mimetypes/48", "Size=48\nType=Fixed");

    /// This machine's shape: the configured theme has no file-type icons
    /// at all, names a parent that is not installed, and one that is.
    #[test]
    fn a_name_the_theme_lacks_is_found_through_what_it_inherits() {
        let f = Fixture::new();
        f.theme("Dracula", "Papirus-Dark,breeze-dark", &[("apps/48", "Size=48\nType=Fixed")], &[]);
        f.theme("breeze-dark", "hicolor", &[M48], &["mimetypes/48/image-png.svg"]);
        f.theme("hicolor", "", &[M48], &[]);

        let icons = f.icons("Dracula");
        assert_eq!(icons.chain(), ["Dracula", "breeze-dark", "hicolor"], "the absent parent is skipped");
        let found = icons.lookup(&["image-png"], 48, 1).expect("resolved through breeze-dark");
        assert!(found.ends_with("breeze-dark/mimetypes/48/image-png.svg"));
    }

    /// hicolor ends every chain even when nothing names it.
    #[test]
    fn hicolor_is_searched_even_when_nothing_inherits_it() {
        let f = Fixture::new();
        f.theme("Mine", "", &[M48], &[]);
        f.theme("hicolor", "", &[M48], &["mimetypes/48/text-x-generic.png"]);
        assert!(f.icons("Mine").lookup(&["text-x-generic"], 48, 1).is_some());
    }

    /// Theme first, then name — see the module doc. The user's theme's
    /// generic icon beats a parent's specific one, so a listing is drawn
    /// by one theme rather than a patchwork.
    #[test]
    fn the_themes_own_generic_icon_beats_a_specific_one_it_inherits() {
        let f = Fixture::new();
        f.theme("Mine", "Parent", &[M48], &["mimetypes/48/text-x-generic.svg"]);
        f.theme("Parent", "", &[M48], &["mimetypes/48/text-x-rust.svg"]);
        let found = f.icons("Mine").lookup(&["text-x-rust", "text-x-generic"], 48, 1).unwrap();
        assert!(found.ends_with("Mine/mimetypes/48/text-x-generic.svg"), "{found:?}");
    }

    /// With no exact size, the nearest one — not the first directory
    /// listed, which would draw a 256-pixel icon squeezed into 16.
    #[test]
    fn the_nearest_size_wins_when_no_directory_matches_exactly() {
        let f = Fixture::new();
        f.theme(
            "hicolor",
            "",
            &[("256", "Size=256\nType=Fixed"), ("22", "Size=22\nType=Fixed")],
            &["256/folder.png", "22/folder.png"],
        );
        let found = f.icons("hicolor").lookup(&["folder"], 16, 1).unwrap();
        assert!(found.ends_with("22/folder.png"), "{found:?}");
    }

    /// A scalable directory covers its whole range.
    #[test]
    fn a_scalable_directory_answers_any_size_in_its_range() {
        let f = Fixture::new();
        f.theme("hicolor", "", &[("scalable", "Size=64\nType=Scalable\nMinSize=8\nMaxSize=512")], &["scalable/folder.svg"]);
        assert!(f.icons("hicolor").lookup(&["folder"], 20, 1).is_some());
    }

    /// Nothing anywhere is an answer, not a panic — the caller draws its
    /// own badge. And a theme that is not installed at all still leaves
    /// hicolor and the pixmaps to answer.
    #[test]
    fn a_missing_theme_still_falls_back_to_hicolor_and_pixmaps() {
        let f = Fixture::new();
        f.theme("hicolor", "", &[M48], &[]);
        let icons = f.icons("NotInstalled");
        assert_eq!(icons.chain(), ["hicolor"]);
        assert_eq!(icons.lookup(&["nothing-has-this"], 48, 1), None);

        fs::create_dir_all(f.dir.path().join("pixmaps")).unwrap();
        fs::write(f.dir.path().join("pixmaps/legacy.png"), b"x").unwrap();
        assert!(icons.lookup(&["legacy"], 48, 1).is_some());
    }

    /// A theme that inherits itself, directly or round a loop, must end.
    #[test]
    fn an_inheritance_loop_ends() {
        let f = Fixture::new();
        f.theme("A", "B", &[M48], &[]);
        f.theme("B", "A", &[M48], &[]);
        assert_eq!(f.icons("A").chain(), ["A", "B"]);
    }

    #[test]
    fn gsettings_quoting_is_removed() {
        assert_eq!(unquote("'Dracula'\n").as_deref(), Some("Dracula"));
        assert_eq!(unquote("''"), None);
    }
}
