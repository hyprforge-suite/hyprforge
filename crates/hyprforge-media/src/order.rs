//! Reading the order the file manager was showing.
//!
//! One direction only: this app reads `files.toml` and never writes it.
//! That file already has two processes racing over it — the file manager
//! window and the portal's dialog, as `hyprforge_files_core::prefs`
//! documents — and a third *reader* costs nothing where a third writer
//! would not.
//!
//! Three states, kept distinct, and the first one is the interesting
//! one: **missing** means the file manager has never run, or is not
//! installed at all. That is not an error and not a degraded mode. It is
//! the defaults, which are name-ascending, which is exactly what a
//! viewer would have chosen on its own.
//!
//! That is the "every component runs alone" test passed by
//! construction rather than by a fallback path somebody has to remember
//! to write.

use hyprforge_listing::order::Order;
use serde::Deserialize;
use std::path::{Path, PathBuf};

/// Just the ordering fields out of `files.toml`, ignoring everything
/// else in it.
///
/// `#[serde(flatten)]` so the four keys are read at the top level where
/// the file manager writes them, and `#[serde(default)]` so a file
/// missing any of them still loads.
#[derive(Debug, Default, Deserialize)]
struct FilesPrefs {
    #[serde(flatten)]
    order: Order,
}

/// What was read, and whether anything needs saying about it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Loaded {
    pub order: Order,
    /// A sentence for the user, when the file was there and unreadable.
    /// `None` covers both "read fine" and "not there", because not
    /// being there is not a problem.
    pub problem: Option<String>,
}

pub fn path() -> PathBuf {
    hyprforge_paths::files_toml_path()
}

pub fn load() -> Loaded {
    load_from(&path())
}

pub fn load_from(path: &Path) -> Loaded {
    let text = match std::fs::read_to_string(path) {
        Ok(text) => text,
        // First run, or the file manager is not installed. Both are
        // ordinary, and both get the order a viewer would have picked.
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            return Loaded { order: Order::default(), problem: None }
        }
        Err(e) => {
            return Loaded {
                order: Order::default(),
                problem: Some(format!(
                    "{} couldn't be read ({e}), so pictures are in name order",
                    path.display()
                )),
            }
        }
    };

    match toml::from_str::<FilesPrefs>(&text) {
        Ok(prefs) => Loaded { order: prefs.order, problem: None },
        // Present and unparseable is worth saying — but it is the file
        // manager's file, and a viewer that refused to open a photograph
        // over it would be reporting somebody else's problem as its own.
        Err(e) => Loaded {
            order: Order::default(),
            problem: Some(format!(
                "{} couldn't be read ({e}), so pictures are in name order",
                path.display()
            )),
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use hyprforge_listing::sort::{SortColumn, SortDirection};

    /// The "runs alone" case: no file manager, no problem.
    #[test]
    fn with_no_files_toml_at_all_the_order_is_by_name_and_nothing_is_reported() {
        let dir = tempfile::tempdir().unwrap();
        let loaded = load_from(&dir.path().join("files.toml"));
        assert_eq!(loaded.order, Order::default());
        assert_eq!(loaded.order.sort_column, SortColumn::Name);
        assert_eq!(loaded.problem, None);
    }

    #[test]
    fn the_file_managers_order_is_read_out_of_its_own_file() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("files.toml");
        std::fs::write(
            &path,
            "sort_column = \"modified\"\nsort_direction = \"descending\"\nshow_hidden = true\n\
             directories_first = false\nwindow_width = 900\n",
        )
        .unwrap();

        let loaded = load_from(&path);
        assert_eq!(loaded.problem, None);
        assert_eq!(loaded.order.sort_column, SortColumn::Modified);
        assert_eq!(loaded.order.sort_direction, SortDirection::Descending);
        assert!(loaded.order.show_hidden);
        assert!(!loaded.order.directories_first);
    }

    /// Everything else in the file manager's preferences is none of this
    /// app's business, and must not stop the four fields being read.
    #[test]
    fn the_rest_of_the_file_managers_settings_are_ignored_not_fatal() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("files.toml");
        std::fs::write(
            &path,
            "sort_column = \"size\"\npinned = [\"/home/a\"]\n\n[columns]\nsize = true\n",
        )
        .unwrap();
        let loaded = load_from(&path);
        assert_eq!(loaded.problem, None);
        assert_eq!(loaded.order.sort_column, SortColumn::Size);
    }

    #[test]
    fn a_files_toml_that_will_not_parse_falls_back_to_name_order_and_says_so() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("files.toml");
        std::fs::write(&path, "sort_column = = =\n").unwrap();

        let loaded = load_from(&path);
        assert_eq!(loaded.order, Order::default());
        let problem = loaded.problem.expect("a broken file is worth saying");
        assert!(problem.contains("name order"), "{problem}");
    }

    /// And it is never written back, whatever happened — this app has no
    /// business in the file manager's settings.
    #[test]
    fn a_broken_files_toml_is_left_exactly_as_it_was() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("files.toml");
        let broken = "sort_column = = =\n";
        std::fs::write(&path, broken).unwrap();
        let _ = load_from(&path);
        assert_eq!(std::fs::read_to_string(&path).unwrap(), broken);
    }
}
