//! Names a person types: renaming, and naming a new folder.
//!
//! Pure, so the rules are tested without a disk. The host still checks
//! the disk at the moment it acts — a sibling can appear between the
//! check and the rename — but refusing an obviously bad name here means
//! the person hears about it while the field is still open to fix.

/// What renaming to a typed name would do.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RenameCheck {
    /// Nothing: the name is unchanged, or empty. Closing the field is
    /// the whole answer — an empty field is how people back out.
    Unchanged,
    /// Rename to this (trimmed) name.
    To(String),
    /// Refused, with the sentence to show. The field stays open.
    Refused(String),
}

/// Checks renaming `old` to what was typed, against the names already in
/// the folder.
///
/// Leading and trailing spaces are dropped: a trailing space in a file
/// name is invisible in every listing and nearly always a slip. Anything
/// a Linux filesystem cannot hold in one name (a `/`, a NUL), and the two
/// names that already mean something (`.` and `..`), are refused.
/// Changing only the case of a name is allowed even though the siblings
/// compare equal case-insensitively on some filesystems — on the ones
/// this app runs on, `Notes` and `notes` are different names.
pub fn check_rename<'a>(
    old: &str,
    typed: &str,
    siblings: impl IntoIterator<Item = &'a str>,
) -> RenameCheck {
    let name = typed.trim();
    if name.is_empty() || name == old {
        return RenameCheck::Unchanged;
    }
    if name.contains('/') {
        return RenameCheck::Refused("A name can't contain \"/\".".to_string());
    }
    if name.contains('\0') {
        return RenameCheck::Refused("A name can't contain a null character.".to_string());
    }
    if name == "." || name == ".." {
        return RenameCheck::Refused(format!("\"{name}\" is reserved and can't be used as a name."));
    }
    if siblings.into_iter().any(|s| s == name) {
        return RenameCheck::Refused(format!("Something called \"{name}\" is already here."));
    }
    RenameCheck::To(name.to_string())
}

/// How many characters of `name` a rename field should select at first:
/// the name without its extension, so typing replaces "report" and keeps
/// ".pdf". A folder, and a dotfile with nothing after the dot, select
/// the whole name.
///
/// Counted in characters. iced's text field counts graphemes, which is
/// the same count for every name that does not combine characters —
/// and for one that does, the selection falls a little short, which is
/// the harmless direction.
pub fn stem_len(name: &str, is_dir: bool) -> usize {
    if is_dir {
        return name.chars().count();
    }
    match name.rsplit_once('.') {
        Some((stem, _)) if !stem.is_empty() => stem.chars().count(),
        _ => name.chars().count(),
    }
}

/// The name a new folder gets: "New folder", or "New folder 2", "3" and
/// so on past whatever is taken.
///
/// Not `hyprforge_fileops`' `stem.N.ext` scheme. That is right for the
/// trash's stored names, which nobody reads, and wrong for a name a
/// person is about to see and probably keep — "New folder.2" reads like
/// a folder with an extension.
pub fn new_folder_name<'a>(taken: impl IntoIterator<Item = &'a str>) -> String {
    let taken: std::collections::HashSet<&str> = taken.into_iter().collect();
    if !taken.contains(NEW_FOLDER) {
        return NEW_FOLDER.to_string();
    }
    (2..)
        .map(|n| format!("{NEW_FOLDER} {n}"))
        .find(|name| !taken.contains(name.as_str()))
        .expect("an unbounded range always finds a free number")
}

const NEW_FOLDER: &str = "New folder";

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_empty_or_unchanged_name_closes_the_field_quietly() {
        assert_eq!(check_rename("a.txt", "", []), RenameCheck::Unchanged);
        assert_eq!(check_rename("a.txt", "   ", []), RenameCheck::Unchanged);
        assert_eq!(check_rename("a.txt", "a.txt", []), RenameCheck::Unchanged);
        assert_eq!(check_rename("a.txt", " a.txt ", []), RenameCheck::Unchanged);
    }

    #[test]
    fn a_new_name_is_trimmed() {
        assert_eq!(check_rename("a.txt", "  b.txt ", []), RenameCheck::To("b.txt".into()));
    }

    #[test]
    fn a_name_a_filesystem_cannot_hold_is_refused() {
        for bad in ["a/b", "x\0y", ".", ".."] {
            assert!(matches!(check_rename("a.txt", bad, []), RenameCheck::Refused(_)), "{bad:?}");
        }
    }

    /// The field stays open with a reason, rather than the rename
    /// failing — or worse, overwriting — once it reaches the disk.
    #[test]
    fn a_name_already_in_the_folder_is_refused() {
        let check = check_rename("a.txt", "b.txt", ["a.txt", "b.txt"]);
        assert!(matches!(check, RenameCheck::Refused(ref why) if why.contains("b.txt")), "{check:?}");
    }

    #[test]
    fn changing_only_the_case_is_a_rename() {
        assert_eq!(check_rename("notes", "Notes", ["notes"]), RenameCheck::To("Notes".into()));
    }

    #[test]
    fn the_selection_covers_the_name_but_not_the_extension() {
        assert_eq!(stem_len("report.pdf", false), 6);
        assert_eq!(stem_len("archive.tar.gz", false), 11, "only the last extension");
        assert_eq!(stem_len("Makefile", false), 8);
        assert_eq!(stem_len(".bashrc", false), 7, "a dotfile is all name");
        assert_eq!(stem_len("photos.2024", true), 11, "a folder is all name");
        assert_eq!(stem_len("résumé.pdf", false), 6, "counted in characters, not bytes");
    }

    #[test]
    fn a_new_folder_takes_the_first_free_number() {
        assert_eq!(new_folder_name(["a"]), "New folder");
        assert_eq!(new_folder_name(["New folder"]), "New folder 2");
        assert_eq!(new_folder_name(["New folder", "New folder 2", "New folder 4"]), "New folder 3");
    }
}
