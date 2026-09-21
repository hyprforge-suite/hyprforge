//! What order a folder is shown in, as one value.
//!
//! Four settings that always travel together: which column sorts, which
//! way, whether directories come first, and whether hidden entries are
//! shown at all. They are one value because a second app needs all four
//! or none — asking "what is the next picture in this folder" means
//! applying every one of them, and applying three out of four gives an
//! answer that is wrong in exactly the cases nobody tests.
//!
//! The file manager owns these: they are its preferences, written into
//! `files.toml` when the user changes them. Another app reads that file
//! and never writes it. Two writers to one file is a problem
//! `hyprforge_files_core::prefs` already documents; a second *reader*
//! costs nothing.
//!
//! # What this is not
//!
//! Not per-directory. `files.toml` carries one global order, deliberately
//! (see that module's own note on what a per-directory map would need),
//! so "the order the file manager was showing" means its one order. Which
//! is, of course, the order the file manager was showing.

use crate::sort::{SortColumn, SortDirection};
use serde::{Deserialize, Serialize};

/// The order a folder's entries are shown in.
///
/// `#[serde(default)]` per field rather than on the struct alone: a
/// `files.toml` written before any one of these existed must still load,
/// and gain the default for the field it has never heard of.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Order {
    pub sort_column: SortColumn,
    pub sort_direction: SortDirection,
    pub directories_first: bool,
    pub show_hidden: bool,
}

/// Name, ascending, directories first, hidden entries hidden.
///
/// The same values `hyprforge_files_core::prefs::Prefs` has always
/// defaulted to — and the reason a viewer running on a machine where the
/// file manager has never been installed still behaves sensibly: its
/// first-run order is the one a viewer would have chosen anyway.
impl Default for Order {
    fn default() -> Self {
        Order {
            sort_column: SortColumn::Name,
            sort_direction: SortDirection::Ascending,
            directories_first: true,
            show_hidden: false,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The spelling on disk is part of the file format: these four keys
    /// are what `files.toml` has always carried, and renaming one would
    /// silently reset somebody's sort order to the default.
    #[test]
    fn the_keys_on_disk_are_the_ones_files_toml_already_uses() {
        let toml = toml_of(&Order {
            sort_column: SortColumn::Modified,
            sort_direction: SortDirection::Descending,
            directories_first: false,
            show_hidden: true,
        });
        assert!(toml.contains("sort_column = \"modified\""), "{toml}");
        assert!(toml.contains("sort_direction = \"descending\""), "{toml}");
        assert!(toml.contains("directories_first = false"), "{toml}");
        assert!(toml.contains("show_hidden = true"), "{toml}");
    }

    #[test]
    fn a_file_that_mentions_nothing_is_the_default_order() {
        let order: Order = toml_from("");
        assert_eq!(order, Order::default());
    }

    /// One key present and the rest absent keeps the rest at their
    /// defaults, rather than failing the whole file — the same rule
    /// every config in this suite follows.
    #[test]
    fn one_key_on_its_own_changes_only_that_one() {
        let order: Order = toml_from("show_hidden = true\n");
        assert!(order.show_hidden);
        assert_eq!(order.sort_column, SortColumn::Name);
        assert!(order.directories_first);
    }

    /// `toml` is a dev-dependency only. The crate itself stays free of
    /// it — a listing has no opinion about file formats — but a test
    /// about *what this looks like on disk* is worth nothing if it does
    /// not go through the real serialiser.
    fn toml_of(order: &Order) -> String {
        toml::to_string(order).unwrap()
    }

    fn toml_from(text: &str) -> Order {
        toml::from_str(text).unwrap()
    }
}
