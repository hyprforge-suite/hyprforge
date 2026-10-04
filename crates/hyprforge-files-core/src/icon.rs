//! What an entry's row draws for its icon.
//!
//! The installed icon theme's icon for the file's type, when the host has
//! found one, and our own drawn badge otherwise.
//!
//! # Who finds the icon
//!
//! Not this crate. Which file is `image-png` in the configured theme is
//! a walk through that theme's `Inherits=` chain and a stat per
//! candidate directory, and the browser does no I/O — see its module doc.
//! So an entry's icon is named by an [`icon_key`], the browser asks its
//! host for the keys it has not seen (`Outcome::LoadIcons`), and the host
//! answers with a [`Picture`] per key. `hyprforge-mime` says which icon
//! *names* a type has, `hyprforge-icons` which *file* each name is.
//!
//! The key is the part of the name a type is decided by — the extension
//! — rather than the path, so a folder of ten thousand PNGs asks once.
//!
//! # The badge
//!
//! A small coloured shape: a folder or a page, in a colour read from
//! [`hyprforge_look::Theme`] — never a hardcoded colour, which CLAUDE.md
//! forbids for any app in this suite. It is what shows before the host
//! answers, on a machine with no icon theme at all, and for a type no
//! theme has anything for; eight kinds share the five colours the theme
//! exposes for this, so the shape carries the distinction and the colour
//! only adds a second cue.

use crate::types::{Entry, EntryKind};
use hyprforge_look::Color;
use hyprforge_ui::theme::{self, FontScale};
use iced::widget::container;
use iced::{Background, Border, Element, Length, Theme as IcedTheme};
use crate::preview::Picture;
use std::path::Path;

/// Which theme icon an entry takes, named by what decides its type:
/// `".png"`, `".tar.gz"`, a whole name with no extension (`"Makefile"`),
/// or [`FOLDER_KEY`].
///
/// A slice of the entry's own name rather than a new string, because it
/// is looked up for every visible row on every redraw — the same reason
/// the row borrows its name rather than cloning it.
///
/// From the first dot that is not the leading one: `.tar.gz` has to stay
/// whole, since the type is decided by both parts, and a dotfile's dot is
/// part of its name rather than an extension. (`.bashrc` and a file
/// ending in `.bashrc` share a key; a by-name type lookup gives both the
/// same answer, so that costs nothing.)
pub fn icon_key(entry: &Entry) -> &str {
    if entry.is_dir {
        return FOLDER_KEY;
    }
    let name = entry.name.as_str();
    match name.get(1..).and_then(|rest| rest.find('.')) {
        Some(dot) => &name[dot + 1..],
        None => name,
    }
}

/// A folder's key. A slash cannot appear in a file name, so it cannot
/// collide with one.
pub const FOLDER_KEY: &str = "/";

/// The key for a folder drawn with the icon theme's `name` — a place's
/// `folder-download`, or a name `[sidebar.icons]` chose.
///
/// Starts with a slash, which is what keeps it apart from every file's
/// key: those are an extension (starting with a dot) or a whole file
/// name, and a file name cannot contain a slash. Without it, a file
/// literally called `icon:x` would have been read as a theme name. The
/// host reads a key back with [`IconSource::of`].
pub fn themed_key(name: &str) -> String {
    format!("/icon:{name}")
}

/// The key for what `[sidebar.icons]` chose.
pub fn choice_key(choice: &crate::config::IconChoice) -> String {
    match choice {
        crate::config::IconChoice::Themed(name) => themed_key(name),
        crate::config::IconChoice::File(path) => format!("/file:{}", path.display()),
    }
}

/// What a key asks the host to find.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum IconSource<'a> {
    /// An icon theme name, for a folder with an icon of its own — or
    /// several, comma-separated and best first (no icon name has a comma
    /// in it); see [`IconSource::names`].
    Themed(&'a str),
    /// An image of the user's own.
    File(&'a Path),
    /// A file's type, by a stand-in name — [`sample_name`]'s.
    Type(String),
    /// An ordinary folder.
    Folder,
}

impl<'a> IconSource<'a> {
    /// The names a [`IconSource::Themed`] key lists, best first.
    pub fn names(themed: &'a str) -> impl Iterator<Item = &'a str> {
        themed.split(',').map(str::trim).filter(|n| !n.is_empty())
    }

    pub fn of(key: &'a str) -> IconSource<'a> {
        if key == FOLDER_KEY {
            IconSource::Folder
        } else if let Some(name) = key.strip_prefix("/icon:") {
            IconSource::Themed(name)
        } else if let Some(path) = key.strip_prefix("/file:") {
            IconSource::File(Path::new(path))
        } else {
            IconSource::Type(sample_name(key))
        }
    }
}

/// A file name a by-name MIME lookup can be asked about, for `key`.
pub fn sample_name(key: &str) -> String {
    if key.starts_with('.') {
        format!("x{key}")
    } else {
        key.to_string()
    }
}


/// The badge colour for `kind`, read from the active [`hyprforge_look::Theme`]
/// rather than hardcoded — see this module's doc.
pub fn badge_color(kind: EntryKind) -> Color {
    let t = theme::active();
    match kind {
        EntryKind::Folder => t.accent,
        // `info` is the "somewhere else" role, and a document is not
        // somewhere else — these take the roles whose *meaning* is
        // nearest, and where nothing fits, the dim text colour rather
        // than borrowing a state colour that would then mean two things.
        EntryKind::Image => t.error,
        EntryKind::Document => t.info,
        EntryKind::Archive => t.warning,
        EntryKind::Code => t.warning,
        EntryKind::Audio => t.success,
        EntryKind::Video => t.error,
        EntryKind::Other => t.surfaces.text_dim,
    }
}

/// An entry's icon: the theme's, when the host found one, and the badge
/// for `kind` otherwise. `size` is the side length in logical pixels
/// before `scale` is applied.
pub fn entry_icon<'a, Message: 'a>(
    kind: EntryKind,
    themed: Option<&Picture>,
    size: f32,
    scale: FontScale,
) -> Element<'a, Message> {
    if let Some(icon) = themed {
        return icon.view(scale.apply(size));
    }
    let color = hyprforge_ui::color::to_iced(badge_color(kind));
    if kind == EntryKind::Folder {
        folder_mark(color, size, scale)
    } else {
        file_mark(color, size, scale)
    }
}

/// A folder: a filled rectangle, wider than it is tall.
///
/// The proportion is the whole point and is why this is not the same
/// shape as [`file_mark`]. A folder in the world is a wide pocket and a
/// file is a tall page, and at 15px those two silhouettes are
/// distinguishable before any colour is — which matters most in exactly
/// the case where colour helps least, a directory of thirty folders and
/// three files.
pub fn folder_mark<'a, Message: 'a>(
    color: iced::Color,
    size: f32,
    scale: FontScale,
) -> Element<'a, Message> {
    let w = scale.apply(size);
    let h = w * FOLDER_ASPECT;
    container(iced::widget::Space::new())
        .width(Length::Fixed(w))
        .height(Length::Fixed(h))
        .style(move |_theme: &IcedTheme| container::Style {
            background: Some(Background::Color(color)),
            // Radius scaled with the mark rather than fixed, so it stays
            // a rounded rectangle at 200% instead of a rectangle with a
            // decorative nick in each corner.
            border: Border { radius: (h * CORNER_FRACTION).into(), width: 0.0, color },
            ..container::Style::default()
        })
        .into()
}

/// A file: an outlined rectangle, taller than it is wide — a page.
pub fn file_mark<'a, Message: 'a>(
    color: iced::Color,
    size: f32,
    scale: FontScale,
) -> Element<'a, Message> {
    let w = scale.apply(size) * FILE_WIDTH_FRACTION;
    let h = scale.apply(size);
    container(iced::widget::Space::new())
        .width(Length::Fixed(w))
        .height(Length::Fixed(h))
        .style(move |_theme: &IcedTheme| container::Style {
            // A whisper of fill inside the outline, so the shape reads as
            // an object rather than as a hole in the row.
            background: Some(Background::Color(iced::Color { a: 0.12, ..color })),
            border: Border { radius: (w * CORNER_FRACTION).into(), width: OUTLINE_WIDTH, color },
            ..container::Style::default()
        })
        .into()
}

/// A folder is this much taller than wide — 12/15, from the design.
const FOLDER_ASPECT: f32 = 0.8;
/// A file is this much narrower than tall — 12/14, from the design.
const FILE_WIDTH_FRACTION: f32 = 0.857;
/// Corner radius as a fraction of the mark's shorter side, so it scales.
const CORNER_FRACTION: f32 = 0.17;

/// The outline on a file's badge. Thin enough to read as a drawn edge
/// rather than as a second filled square.
const OUTLINE_WIDTH: f32 = 1.5;



#[cfg(test)]
mod tests {
    use super::*;

    /// Only the name and whether it is a folder matter to a key.
    fn entry(name: &str, is_dir: bool) -> Entry {
        Entry {
            name: name.to_string(),
            path: std::path::PathBuf::from("/dir").join(name),
            is_dir,
            size: crate::types::EntrySize::Bytes(0),
            modified: None,
            is_symlink: false,
            link_broken: false,
            hidden: name.starts_with('.'),
            kind: EntryKind::classify(is_dir, name),
            mode: 0o644,
            uid: 1000,
            owner: None,
            origin: None,
            packed: None,
        }
    }

    /// A folder of ten thousand photographs asks for one icon, not ten
    /// thousand: the key is what decides the type, not the path.
    #[test]
    fn files_that_share_an_extension_share_an_icon() {
        assert_eq!(icon_key(&entry("a.png", false)), icon_key(&entry("b.png", false)));
        assert_ne!(icon_key(&entry("a.png", false)), icon_key(&entry("a.jpg", false)));
        assert_eq!(icon_key(&entry("photos.2024", true)), FOLDER_KEY, "a folder is a folder whatever its name");
    }

    /// A folder's own icon is never mistaken for a file's type, whatever
    /// the file is called — the prefix starts with a slash no file name
    /// can hold.
    /// A themed key may list fallbacks, best first, and each is asked for
    /// in that order — the one that exists in the configured theme wins
    /// over a parent theme's better match, which is why the list exists.
    #[test]
    fn a_themed_key_lists_its_names_best_first() {
        let key = themed_key(crate::starred::Collection::Starred.icon_name());
        let IconSource::Themed(names) = IconSource::of(&key) else { panic!("themed: {key}") };
        assert_eq!(IconSource::names(names).collect::<Vec<_>>(), ["folder-favorites", "starred", "emblem-favorite"]);
        assert_eq!(IconSource::names("folder").collect::<Vec<_>>(), ["folder"], "one name is a list of one");
    }

    #[test]
    fn a_file_named_like_an_icon_key_is_still_a_file() {
        let odd = entry("icon:folder-cloud", false);
        assert_eq!(IconSource::of(icon_key(&odd)), IconSource::Type("icon:folder-cloud".into()));
        assert_eq!(IconSource::of(&themed_key("folder-cloud")), IconSource::Themed("folder-cloud"));
        let chosen = crate::config::IconChoice::File("/home/a/p.svg".into());
        assert_eq!(IconSource::of(&choice_key(&chosen)), IconSource::File(Path::new("/home/a/p.svg")));
        assert_eq!(IconSource::of(FOLDER_KEY), IconSource::Folder);
    }

    /// A compound extension stays whole — `.tar.gz` is a tarball, `.gz`
    /// alone is not — and a dotfile's dot is its name, not an extension.
    #[test]
    fn a_compound_extension_stays_whole_and_a_dotfile_stands_for_itself() {
        assert_eq!(icon_key(&entry("backup.tar.gz", false)), ".tar.gz");
        assert_eq!(icon_key(&entry(".bashrc", false)), ".bashrc");
        assert_eq!(icon_key(&entry("Makefile", false)), "Makefile");
        assert_eq!(sample_name(".tar.gz"), "x.tar.gz");
        assert_eq!(sample_name("Makefile"), "Makefile");
    }

    #[test]
    fn every_kind_gets_a_colour_that_depends_on_the_kind() {
        let kinds = [
            EntryKind::Folder,
            EntryKind::Image,
            EntryKind::Document,
            EntryKind::Archive,
            EntryKind::Code,
            EntryKind::Audio,
            EntryKind::Video,
            EntryKind::Other,
        ];
        let mut colors: Vec<[u8; 4]> = kinds
            .iter()
            .map(|k| {
                let c = badge_color(*k);
                [c.r, c.g, c.b, c.a]
            })
            .collect();
        colors.sort_unstable();
        colors.dedup();
        // Eight kinds, five colours the theme exposes for this: some
        // sharing is unavoidable and fine. What must not happen is all
        // of them collapsing to one, which would mean `badge_color` had
        // stopped reading the kind at all.
        assert!(colors.len() >= 4, "the badge colour must actually vary with the kind, got {colors:?}");
    }

    /// The badges are drawn, not typed.
    ///
    /// They used to be emoji, and a colour emoji renders in the emoji
    /// font with *its* colours — so the badge's carefully theme-derived
    /// colour was computed and then thrown away. Every folder came out
    /// the font's yellow whatever the theme said. The property that keeps
    /// it fixed is structural: nothing in this module's public API
    /// returns a string to draw as an icon. What an entry draws is a
    /// [`Picture`] — a file the icon theme chose — or the badge.
    #[test]
    fn an_icon_carries_no_text_so_no_font_can_override_the_theme() {
        let _: fn(EntryKind) -> Color = badge_color;
        let _: fn(&Path) -> Picture = Picture::from_path;
    }

    #[test]
    fn badge_color_never_hardcodes_a_colour_it_reads_the_active_theme() {
        // Try to install a theme with a distinctive accent. `theme::init`
        // is backed by a `OnceLock` that accepts only the *first* call in
        // the whole test binary — CLAUDE.md's own note on this — and
        // this crate now has more than one test that can reach
        // `theme::active()` (the browser's selection-colour tests do,
        // through `entry_row_style`), so this call is not guaranteed to
        // win the race. Asserting against `theme::active().accent`
        // instead of a literal `0x010203` keeps the test proving the
        // property that actually matters — "badge_color reads whatever
        // the active theme says, not a constant" — regardless of which
        // test happened to set that theme first.
        let theme = hyprforge_look::Theme {
            accent: Color::rgba(0x01, 0x02, 0x03, 0xff),
            ..hyprforge_look::Theme::default()
        };
        theme::init(theme);
        assert_eq!(badge_color(EntryKind::Folder), theme::active().accent);
    }
}
