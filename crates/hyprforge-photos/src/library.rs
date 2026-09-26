//! Library mode, and the sidebar's list of neighbouring folders: a
//! folder's subfolders, each summarised as a card.
//!
//! Mockup `1h` — "folders as the library" — which is also what `2a`'s
//! sidebar lists under the parent folder's name. Both ask the same
//! question of the same directory, so both read one [`Summary`].
//!
//! # A library that is only folders
//!
//! There is no catalogue here, no database and no import step. The
//! library *is* the directory tree the user already keeps, which is the
//! vision doc's "folder-first, library optional", and it means nothing
//! this app shows can disagree with what the file manager shows. What a
//! card knows is what one listing of that folder says: how many pictures
//! it holds, the span of their dates, and which one to show on the cover.

use crate::folder::{Folder, Item, Media};
use chrono::{Datelike, NaiveDate, TimeZone};
use hyprforge_listing::order::Order;
use hyprforge_listing::types::Entry;
use std::path::PathBuf;
use std::time::SystemTime;

/// How many subfolders a library card grid will summarise. Each costs a
/// directory read; a folder with thousands of subfolders is somebody's
/// cache, not a photo library, and should not stall the window.
pub const MOST_FOLDERS: usize = 400;

/// One folder, summarised for a card or a sidebar row.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Summary {
    pub name: String,
    pub path: PathBuf,
    /// Pictures and clips directly inside — not in subfolders, which
    /// would need the whole tree walked to count.
    pub photos: usize,
    /// 3D models directly inside, counted apart so a folder of prints is
    /// not described as photos.
    pub models: usize,
    /// The first still in the file manager's order: the picture people
    /// will recognise the folder by, because it is the one they see first
    /// when they open it.
    pub cover: Option<PathBuf>,
    pub first: Option<SystemTime>,
    pub last: Option<SystemTime>,
}

/// The directories in a listing, in the file manager's order — the
/// folders a library card grid shows.
pub fn subfolders(entries: Vec<Entry>, order: &Order) -> Vec<(String, PathBuf)> {
    let mut dirs = hyprforge_listing::filter::filter_hidden(entries, order.show_hidden);
    dirs.retain(|e| e.is_dir);
    hyprforge_listing::sort::sort(&mut dirs, order.sort_column, order.sort_direction, true);
    dirs.into_iter().take(MOST_FOLDERS).map(|e| (e.name, e.path)).collect()
}

/// Summarises one folder from its own listing.
pub fn summarise(name: String, path: PathBuf, entries: Vec<Entry>, order: &Order) -> Summary {
    let folder = Folder::build(entries, order);
    let items: &[Item] = folder.items();
    let times = items.iter().filter_map(|i| i.modified);
    Summary {
        name,
        path,
        photos: items.iter().filter(|i| i.media != Media::Model).count(),
        models: items.iter().filter(|i| i.media == Media::Model).count(),
        cover: items.iter().find(|i| i.media == Media::Still).map(|i| i.path.clone()),
        first: times.clone().min(),
        last: times.max(),
    }
}

/// The span of a folder's dates, as short as it can honestly be:
/// "19 Apr", "14–18 Apr", "2 Jun – 9 Jul", "Dec 2025 – Jan 2026".
///
/// The year is left off when both ends are this year — a card in the
/// current year's folder does not need to say so on every line.
pub fn span<Tz: TimeZone>(first: Option<SystemTime>, last: Option<SystemTime>, tz: &Tz, today: NaiveDate) -> Option<String> {
    let day = |t: SystemTime| chrono::DateTime::<chrono::Utc>::from(t).with_timezone(tz).date_naive();
    let (a, b) = (day(first?), day(last?));
    let year = |d: NaiveDate| if d.year() == today.year() { String::new() } else { format!(" {}", d.year()) };
    Some(if a == b {
        format!("{}{}", a.format("%-d %b"), year(a))
    } else if a.year() == b.year() && a.month() == b.month() {
        format!("{}–{}{}", a.day(), b.format("%-d %b"), year(b))
    } else if a.year() == b.year() {
        format!("{} – {}{}", a.format("%-d %b"), b.format("%-d %b"), year(b))
    } else {
        format!("{} – {}", a.format("%b %Y"), b.format("%b %Y"))
    })
}

/// The line under a card's name: "48 photos · 14–18 Apr".
pub fn card_line<Tz: TimeZone>(summary: &Summary, tz: &Tz, today: NaiveDate) -> String {
    if summary.photos + summary.models == 0 {
        return "No photos".to_string();
    }
    let count = describe_counts(summary.photos, summary.models);
    match span(summary.first, summary.last, tz, today) {
        Some(span) => format!("{count} · {span}"),
        None => count,
    }
}

/// [`crate::grid::describe`] for counts rather than items: "48 photos",
/// "3 models", "4 items".
pub fn describe_counts(photos: usize, models: usize) -> String {
    crate::grid::describe(
        std::iter::repeat_n(Media::Still, photos).chain(std::iter::repeat_n(Media::Model, models)),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::FixedOffset;
    use hyprforge_listing::types::{EntryKind, EntrySize, ItemCount};
    use std::time::Duration;

    const SAT_12_SEP_2026: u64 = 1_789_171_200;
    const DAY: u64 = 86_400;

    fn at(secs: u64) -> Option<SystemTime> {
        Some(SystemTime::UNIX_EPOCH + Duration::from_secs(secs))
    }

    fn entry(name: &str, secs: u64) -> Entry {
        let is_dir = !name.contains('.');
        Entry {
            name: name.to_string(),
            path: PathBuf::from("/lib").join(name),
            is_dir,
            size: if is_dir { EntrySize::Items(ItemCount::Known(0)) } else { EntrySize::Bytes(10) },
            modified: at(secs),
            is_symlink: false,
            link_broken: false,
            hidden: name.starts_with('.'),
            kind: EntryKind::classify(is_dir, name),
            mode: 0o644,
            packed: None,
            uid: 1000,
            owner: None,
            origin: None,
        }
    }

    fn utc() -> FixedOffset {
        FixedOffset::east_opt(0).unwrap()
    }

    fn today() -> NaiveDate {
        NaiveDate::from_ymd_opt(2026, 9, 25).unwrap()
    }

    #[test]
    fn only_folders_are_library_cards_and_hidden_ones_stay_hidden() {
        let entries = vec![entry("Porto", 0), entry("a.jpg", 0), entry(".cache", 0), entry("Lisbon", 0)];
        let names: Vec<String> = subfolders(entries, &Order::default()).into_iter().map(|(n, _)| n).collect();
        assert_eq!(names, ["Lisbon", "Porto"]);
    }

    #[test]
    fn a_summary_counts_pictures_and_takes_the_first_still_as_its_cover() {
        let entries = vec![
            entry("b.jpg", SAT_12_SEP_2026),
            entry("a.mp4", SAT_12_SEP_2026 - 3 * DAY),
            entry("notes.txt", 0),
            entry("c.png", SAT_12_SEP_2026 - DAY),
        ];
        let s = summarise("Lisbon".into(), "/lib/Lisbon".into(), entries, &Order::default());
        assert_eq!(s.photos, 3);
        assert_eq!(s.models, 0);
        // `a.mp4` sorts first but is a clip; the cover is a picture.
        assert_eq!(s.cover, Some(PathBuf::from("/lib/b.jpg")));
        assert_eq!(s.first, at(SAT_12_SEP_2026 - 3 * DAY));
        assert_eq!(s.last, at(SAT_12_SEP_2026));
    }

    #[test]
    fn a_span_is_as_short_as_it_can_honestly_be() {
        let t = |d: u64| at(SAT_12_SEP_2026 + d * DAY);
        assert_eq!(span(t(0), t(0), &utc(), today()).as_deref(), Some("12 Sep"));
        assert_eq!(span(t(0), t(4), &utc(), today()).as_deref(), Some("12–16 Sep"));
        assert_eq!(span(t(0), t(30), &utc(), today()).as_deref(), Some("12 Sep – 12 Oct"));
        assert_eq!(span(t(0), t(120), &utc(), today()).as_deref(), Some("Sep 2026 – Jan 2027"));
    }

    #[test]
    fn a_span_in_another_year_says_which() {
        let last_year = SAT_12_SEP_2026 - 365 * DAY;
        assert_eq!(span(at(last_year), at(last_year), &utc(), today()).as_deref(), Some("12 Sep 2025"));
    }

    /// A folder of prints says models, and its cover stays a picture
    /// when it has one — a model has no thumbnail to be a cover.
    #[test]
    fn a_folder_of_models_is_not_described_as_photos() {
        let entries = vec![entry("a.stl", SAT_12_SEP_2026), entry("b.3mf", SAT_12_SEP_2026)];
        let s = summarise("Prints".into(), "/lib/Prints".into(), entries, &Order::default());
        assert_eq!((s.photos, s.models), (0, 2));
        assert_eq!(s.cover, None);
        assert_eq!(card_line(&s, &utc(), today()), "2 models · 12 Sep");
    }

    #[test]
    fn an_empty_folder_says_so_rather_than_zero_photos() {
        let s = summarise("Empty".into(), "/lib/Empty".into(), Vec::new(), &Order::default());
        assert_eq!(card_line(&s, &utc(), today()), "No photos");
        assert_eq!(s.cover, None);
    }

    #[test]
    fn a_card_line_reads_like_the_mockup() {
        let s = Summary {
            name: "Lisbon".into(),
            path: "/lib/Lisbon".into(),
            photos: 48,
            models: 0,
            cover: None,
            first: at(SAT_12_SEP_2026),
            last: at(SAT_12_SEP_2026 + 4 * DAY),
        };
        assert_eq!(card_line(&s, &utc(), today()), "48 photos · 12–16 Sep");
    }
}
