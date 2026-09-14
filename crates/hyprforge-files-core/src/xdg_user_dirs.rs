//! The XDG user directories — Documents, Downloads, Pictures, Music,
//! Videos, Desktop — read from `~/.config/user-dirs.dirs` rather than
//! hardcoded, so a sidebar built on a machine set up in another language
//! shows that language's own folder names instead of a translated set of
//! English guesses sitting next to directories that don't exist.
//!
//! `xdg-user-dirs-update` writes lines like:
//! ```text
//! XDG_DOCUMENTS_DIR="$HOME/Dokumente"
//! ```
//! — one `KEY="value"` pair per line, `$HOME` a literal prefix to be
//! substituted, `#`-comments and blank lines allowed anywhere. This
//! parses exactly that shape and nothing more: no shell expansion beyond
//! the one `$HOME` substitution the format actually uses, because a
//! general shell parser is a dependency this crate does not need for a
//! file with eight fixed keys.

use std::path::{Path, PathBuf};

/// The subset of XDG user directories this browser's sidebar cares
/// about. `None` for a directory the file doesn't mention — a partially
/// customised file is common (someone deletes the `XDG_TEMPLATES_DIR`
/// line and nothing else), and a missing key is not this module's
/// business to guess at; [`build_sidebar`]/[`crate::sidebar::build`]
/// simply won't offer a shortcut for it.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct UserDirs {
    pub desktop: Option<PathBuf>,
    pub download: Option<PathBuf>,
    pub documents: Option<PathBuf>,
    pub music: Option<PathBuf>,
    pub pictures: Option<PathBuf>,
    pub videos: Option<PathBuf>,
}

/// Reads and parses `user-dirs.dirs` from the config directory, or falls
/// back to the conventional English names under `home` if the file
/// isn't there.
///
/// The fallback is deliberately only for a **missing** file, not a file
/// that exists and fails to parse in some unexpected way — a
/// syntactically odd but present file still goes through [`parse`],
/// which is forgiving line-by-line (an unrecognised line is skipped, not
/// a hard failure) rather than needing its own could-not-read/could-not-parse
/// split the way `Prefs` does. There is no realistic way for this format
/// to be "present but unparseable" as a whole, unlike a TOML file: every
/// line either matches the one shape this parser looks for, or it
/// doesn't and is ignored. Performs blocking file I/O — call this off
/// the UI thread, the same as [`crate::backend::FsBackend`].
pub fn load(config_dir: &Path, home: &Path) -> UserDirs {
    let path = config_dir.join("user-dirs.dirs");
    match std::fs::read_to_string(&path) {
        Ok(text) => parse(&text, home),
        Err(_) => fallback(home),
    }
}

/// The conventional English names, used only when there is no
/// `user-dirs.dirs` to read at all (see [`load`]). [`crate::sidebar::build`]
/// still checks each of these against the real filesystem before
/// offering it, so a guess that doesn't exist on this machine is dropped
/// rather than shown as a dead shortcut.
pub fn fallback(home: &Path) -> UserDirs {
    UserDirs {
        desktop: Some(home.join("Desktop")),
        download: Some(home.join("Downloads")),
        documents: Some(home.join("Documents")),
        music: Some(home.join("Music")),
        pictures: Some(home.join("Pictures")),
        videos: Some(home.join("Videos")),
    }
}

/// Parses the contents of a `user-dirs.dirs` file. `home` is substituted
/// for a leading `$HOME` in each value, matching how `xdg-user-dirs`
/// itself writes the file.
pub fn parse(contents: &str, home: &Path) -> UserDirs {
    let mut dirs = UserDirs::default();
    for line in contents.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let Some((key, value)) = line.split_once('=') else {
            continue;
        };
        let key = key.trim();
        let Some(value) = unquote(value.trim()) else {
            continue;
        };
        let path = resolve(&value, home);
        match key {
            "XDG_DESKTOP_DIR" => dirs.desktop = Some(path),
            "XDG_DOWNLOAD_DIR" => dirs.download = Some(path),
            "XDG_DOCUMENTS_DIR" => dirs.documents = Some(path),
            "XDG_MUSIC_DIR" => dirs.music = Some(path),
            "XDG_PICTURES_DIR" => dirs.pictures = Some(path),
            "XDG_VIDEOS_DIR" => dirs.videos = Some(path),
            // XDG_PUBLICSHARE_DIR and XDG_TEMPLATES_DIR exist in the
            // real format too, but nothing in this browser's sidebar
            // brief asks for either — see the module doc's own scoping.
            _ => {}
        }
    }
    dirs
}

/// Strips one layer of double quotes, the only quoting `user-dirs.dirs`
/// actually uses. A value with no closing quote is malformed and
/// skipped rather than guessed at.
fn unquote(value: &str) -> Option<String> {
    let value = value.strip_prefix('"')?;
    let value = value.strip_suffix('"')?;
    Some(value.to_string())
}

/// Replaces a leading `$HOME` with the real home directory; anything
/// else is treated as already-absolute (or, rarely, relative — left as
/// written, since the spec doesn't otherwise define what a bare relative
/// path here would mean).
fn resolve(value: &str, home: &Path) -> PathBuf {
    match value.strip_prefix("$HOME") {
        Some(rest) => {
            let rest = rest.trim_start_matches('/');
            if rest.is_empty() {
                home.to_path_buf()
            } else {
                home.join(rest)
            }
        }
        None => PathBuf::from(value),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_typical_file_resolves_home_relative_paths() {
        let contents = r#"
            # This file is written by xdg-user-dirs-update
            XDG_DESKTOP_DIR="$HOME/Desktop"
            XDG_DOCUMENTS_DIR="$HOME/Dokumente"
            XDG_DOWNLOAD_DIR="$HOME/Downloads"
        "#;
        let home = Path::new("/home/alex");
        let dirs = parse(contents, home);
        assert_eq!(dirs.desktop, Some(PathBuf::from("/home/alex/Desktop")));
        assert_eq!(dirs.documents, Some(PathBuf::from("/home/alex/Dokumente")));
        assert_eq!(dirs.download, Some(PathBuf::from("/home/alex/Downloads")));
    }

    #[test]
    fn a_key_the_file_never_mentions_stays_none_rather_than_guessed() {
        let contents = r#"XDG_DOCUMENTS_DIR="$HOME/Documents""#;
        let dirs = parse(contents, Path::new("/home/alex"));
        assert_eq!(dirs.documents, Some(PathBuf::from("/home/alex/Documents")));
        assert_eq!(dirs.music, None, "a directory the file doesn't mention is not a guess");
    }

    #[test]
    fn an_unrecognised_line_is_skipped_not_fatal() {
        let contents = "this is not a valid line at all\nXDG_MUSIC_DIR=\"$HOME/Music\"";
        let dirs = parse(contents, Path::new("/home/alex"));
        assert_eq!(dirs.music, Some(PathBuf::from("/home/alex/Music")));
    }

    #[test]
    fn a_missing_file_falls_back_to_english_names_under_home() {
        let dir = tempfile::tempdir().unwrap();
        let home = Path::new("/home/alex");
        let dirs = load(dir.path(), home);
        assert_eq!(dirs, fallback(home));
        assert_eq!(dirs.documents, Some(PathBuf::from("/home/alex/Documents")));
    }

    #[test]
    fn a_present_file_is_used_instead_of_the_fallback() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(
            dir.path().join("user-dirs.dirs"),
            r#"XDG_DOCUMENTS_DIR="$HOME/OnlyThis""#,
        )
        .unwrap();
        let home = Path::new("/home/alex");
        let dirs = load(dir.path(), home);
        assert_eq!(dirs.documents, Some(PathBuf::from("/home/alex/OnlyThis")));
        assert_eq!(dirs.download, None, "present file wins outright, no fallback merge");
    }
}
