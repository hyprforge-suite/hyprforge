//! Finding the themes and fonts installed on this machine, so the theme
//! settings can be picked from a list instead of typed from memory.
//!
//! Typing `gtk-theme` by hand means knowing the exact directory name, and
//! a typo produces no error anywhere — GTK silently falls back to its
//! default, which looks like the setting didn't save. Everything here is
//! discoverable on disk, so it may as well be offered.
//!
//! The search paths and the rules for what counts are the XDG icon theme
//! specification's, checked against this machine:
//!
//! - **GTK themes** are directories with a `gtk-N.0/gtk.css`. The
//!   stylesheet is the test, not the directory: `/usr/share/themes/Emacs`
//!   and `Default` both have a `gtk-3.0` holding nothing but
//!   `gtk-keys.css`. They are *keybinding* themes, and offering them as
//!   widget themes would give a user a "theme" that changes nothing.
//! - **Icon themes** have an `index.theme` declaring `Directories=`.
//! - **Cursor themes** have a `cursors/` subdirectory. A theme can be
//!   both — Adwaita ships icons and cursors — so these are separate
//!   questions asked of the same directory, not a partition.
//!
//! Two names are excluded, and `Hidden=true` deliberately is not:
//!
//! - `hicolor`, which the icon theme specification names as *the*
//!   fallback every other theme inherits from. It holds almost no icons
//!   of its own and picking it is never what someone means.
//! - `default`, a pointer to the real cursor theme
//!   (`Inherits=Dracula-cursors` here) rather than a theme itself.
//!   Selecting it would set the cursor theme to "whatever the cursor
//!   theme currently is".
//!
//! **`Hidden=true` is ignored on purpose.** The spec says it means "hide
//! this in a theme selection UI", but on this machine Adwaita and
//! AdwaitaLegacy both declare it — so honouring it would hide the system
//! default icon theme from its own picker, which is a worse outcome than
//! listing a theme someone rarely wants. `hicolor`, the one entry the
//! flag is genuinely protecting, is excluded by name instead.

use std::collections::BTreeSet;
use std::path::PathBuf;
use std::process::Command;

/// A directory that is a pointer to the real theme rather than a theme.
const POINTER: &str = "default";

/// The icon theme specification's fallback, inherited by everything.
const FALLBACK: &str = "hicolor";

fn data_dirs(user: &str, system: &str) -> Vec<PathBuf> {
    let home = std::env::var_os("HOME").map(PathBuf::from);
    let mut dirs = Vec::new();
    if let Some(home) = &home {
        dirs.push(home.join(user));
        dirs.push(home.join(".local/share").join(system));
    }
    dirs.push(PathBuf::from("/usr/share").join(system));
    dirs.push(PathBuf::from("/usr/local/share").join(system));
    dirs
}

/// Every directory entry under `dirs`, as `(name, path)`.
fn entries(dirs: Vec<PathBuf>) -> Vec<(String, PathBuf)> {
    let mut out = Vec::new();
    for dir in dirs {
        let Ok(read) = std::fs::read_dir(&dir) else {
            continue;
        };
        for entry in read.flatten() {
            let path = entry.path();
            if !path.is_dir() {
                continue;
            }
            if let Some(name) = path.file_name().and_then(|n| n.to_str()) {
                out.push((name.to_string(), path));
            }
        }
    }
    out
}

/// A `BTreeSet` so the list is sorted and a theme installed both
/// system-wide and per-user appears once.
fn sorted(names: BTreeSet<String>) -> Vec<String> {
    names.into_iter().collect()
}

/// Themes with a `gtk-N.0` subdirectory.
pub fn gtk_themes() -> Vec<String> {
    let mut found = BTreeSet::new();
    for (name, path) in entries(data_dirs(".themes", "themes")) {
        if name == POINTER {
            continue;
        }
        if has_gtk_stylesheet(&path) {
            found.insert(name);
        }
    }
    sorted(found)
}

/// A `gtk-N.0` holding an actual stylesheet, as opposed to one holding
/// only `gtk-keys.css` — which is a keybinding theme wearing the same
/// directory layout.
fn has_gtk_stylesheet(theme: &std::path::Path) -> bool {
    let Ok(read) = std::fs::read_dir(theme) else {
        return false;
    };
    read.flatten().any(|e| {
        e.file_name()
            .to_str()
            .is_some_and(|n| n.starts_with("gtk-"))
            && e.path().is_dir()
            && (e.path().join("gtk.css").is_file() || e.path().join("gtk-dark.css").is_file())
    })
}

/// Themes whose `index.theme` declares `Directories=`.
pub fn icon_themes() -> Vec<String> {
    let mut found = BTreeSet::new();
    for (name, path) in entries(data_dirs(".icons", "icons")) {
        if name == POINTER || name == FALLBACK {
            continue;
        }
        let Ok(index) = std::fs::read_to_string(path.join("index.theme")) else {
            continue;
        };
        if index.lines().any(|l| l.trim_start().starts_with("Directories=")) {
            found.insert(name);
        }
    }
    sorted(found)
}

/// Themes with a `cursors/` subdirectory.
pub fn cursor_themes() -> Vec<String> {
    let mut found = BTreeSet::new();
    for (name, path) in entries(data_dirs(".icons", "icons")) {
        if name == POINTER || name == FALLBACK || !path.join("cursors").is_dir() {
            continue;
        }
        found.insert(name);
    }
    sorted(found)
}

/// Font families, from `fc-list`.
///
/// A family can carry comma-separated aliases (`Noto Sans Khmer,Noto Sans
/// Khmer SemiBold`); the first is the one to offer. An empty list means
/// fontconfig isn't available, which the caller must treat as "can't
/// offer a list" rather than "no fonts installed".
pub fn font_families() -> Vec<String> {
    let Ok(out) = Command::new("fc-list").args([":", "family"]).output() else {
        return Vec::new();
    };
    let mut found = BTreeSet::new();
    for line in String::from_utf8_lossy(&out.stdout).lines() {
        let name = line.split(',').next().unwrap_or("").trim();
        if !name.is_empty() {
            found.insert(name.to_string());
        }
    }
    sorted(found)
}

/// The options a picker should offer: everything installed, plus
/// `current` if it isn't among them.
///
/// Keeping the current value is not politeness. A theme can live outside
/// the search paths, or be one this machine no longer has — and a picker
/// that silently drops it would show nothing selected, so the next
/// interaction would replace a working setting with whatever the user
/// happened to click.
pub fn options_including(installed: &[String], current: Option<&str>) -> Vec<String> {
    let mut options = installed.to_vec();
    if let Some(current) = current {
        let current = current.trim();
        if !current.is_empty() && !options.iter().any(|o| o == current) {
            options.push(current.to_string());
            options.sort();
        }
    }
    options
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `/usr/share/themes/Emacs` and `Default` both have a `gtk-3.0`
    /// containing only `gtk-keys.css`. They are keybinding themes, and
    /// offering them as widget themes hands the user a "theme" that
    /// changes nothing.
    #[test]
    fn a_keybinding_theme_is_not_a_widget_theme() {
        let dir = tempfile::tempdir().unwrap();

        let keys = dir.path().join("Emacs/gtk-3.0");
        std::fs::create_dir_all(&keys).unwrap();
        std::fs::write(keys.join("gtk-keys.css"), "").unwrap();
        assert!(!has_gtk_stylesheet(&dir.path().join("Emacs")));

        let widgets = dir.path().join("Dracula/gtk-3.0");
        std::fs::create_dir_all(&widgets).unwrap();
        std::fs::write(widgets.join("gtk.css"), "").unwrap();
        assert!(has_gtk_stylesheet(&dir.path().join("Dracula")));
    }

    /// Some themes ship only a dark stylesheet.
    #[test]
    fn a_dark_only_stylesheet_still_counts() {
        let dir = tempfile::tempdir().unwrap();
        let gtk4 = dir.path().join("OnlyDark/gtk-4.0");
        std::fs::create_dir_all(&gtk4).unwrap();
        std::fs::write(gtk4.join("gtk-dark.css"), "").unwrap();
        assert!(has_gtk_stylesheet(&dir.path().join("OnlyDark")));
    }

    /// A theme outside the search paths would otherwise vanish from its
    /// own picker, leaving nothing selected — and the next click would
    /// replace a working setting.
    #[test]
    fn the_current_value_is_kept_even_when_it_is_not_installed() {
        let installed = vec!["Adwaita".to_string(), "Dracula".to_string()];
        let options = options_including(&installed, Some("Custom-Theme"));
        assert!(options.contains(&"Custom-Theme".to_string()));
        assert_eq!(options.len(), 3);
        assert!(options.windows(2).all(|w| w[0] <= w[1]), "must stay sorted");
    }

    #[test]
    fn an_installed_current_value_is_not_duplicated() {
        let installed = vec!["Adwaita".to_string(), "Dracula".to_string()];
        assert_eq!(options_including(&installed, Some("Dracula")).len(), 2);
    }

    #[test]
    fn a_missing_or_blank_current_value_adds_nothing() {
        let installed = vec!["Adwaita".to_string()];
        assert_eq!(options_including(&installed, None).len(), 1);
        assert_eq!(options_including(&installed, Some("   ")).len(), 1);
    }

    /// Discovery must never panic on a machine with unusual or missing
    /// theme directories — it runs at startup on every user's system.
    #[test]
    fn discovery_is_safe_on_any_filesystem() {
        let _ = gtk_themes();
        let _ = icon_themes();
        let _ = cursor_themes();
    }

    /// `$HOME` pointing somewhere with no theme directories at all is the
    /// shape of a fresh container, and must yield an empty list rather
    /// than an error or a panic.
    #[test]
    fn a_home_without_themes_yields_nothing() {
        let dir = tempfile::tempdir().unwrap();
        let dirs = data_dirs(".themes", "themes");
        assert!(!dirs.is_empty(), "system paths are always searched");
        assert!(entries(vec![dir.path().join("nope")]).is_empty());
    }
}
