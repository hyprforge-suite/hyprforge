//! Grid mode: the folder as tiles, grouped by the day they were taken.
//!
//! Mockup `2b`. Pure over a slice of [`Item`]s, so which day a picture
//! lands under and where Down goes from the last row of a group are
//! tests rather than things to squint at.
//!
//! # Days come from the listing, not from EXIF
//!
//! The day a tile is filed under is the file's modification time in the
//! local zone. `DateTimeOriginal` would be the better answer for a
//! camera card and the worse one for everything else — screenshots and
//! downloads have none — and it would cost opening every file in the
//! folder before the grid could draw. The listing already has the mtime,
//! for free, for every file; a camera import that preserves times (as
//! `cp -p` and every card importer does) gets the right day anyway.
//!
//! # Newest first, and the folder's own order inside a day
//!
//! Groups run newest day first, the way the mockup has Saturday above
//! Friday. Inside a day the tiles keep the file manager's order, which is
//! the order Right steps through in Photo mode — so the grid and the
//! filmstrip never disagree about which picture follows which within
//! a day.

use crate::folder::Item;
use chrono::{Datelike, NaiveDate, TimeZone};

/// One day's tiles.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Group {
    /// `None` for files the filesystem would not give a time for — kept,
    /// under a heading of their own, and last.
    pub day: Option<NaiveDate>,
    /// Indices into the folder's items, in the folder's order.
    pub indices: Vec<usize>,
}

/// The folder's items, grouped by local day, newest day first.
pub fn groups<Tz: TimeZone>(items: &[Item], tz: &Tz) -> Vec<Group> {
    let mut out: Vec<Group> = Vec::new();
    for (index, item) in items.iter().enumerate() {
        let day = item.modified.map(|t| chrono::DateTime::<chrono::Utc>::from(t).with_timezone(tz).date_naive());
        match out.iter_mut().find(|g| g.day == day) {
            Some(group) => group.indices.push(index),
            None => out.push(Group { day, indices: vec![index] }),
        }
    }
    // `None` sorts below every `Some` in reverse order, which is where
    // "date unknown" belongs.
    out.sort_by_key(|g| std::cmp::Reverse(g.day));
    out
}

/// A group's heading: "Sat 12 Sep", with the year only when it is not
/// this one — the mockup's heading, and the form a person scanning back
/// through a year of photographs can read without doing arithmetic.
pub fn heading(day: Option<NaiveDate>, today: NaiveDate) -> String {
    match day {
        Some(day) if day.year() == today.year() => day.format("%a %-d %b").to_string(),
        Some(day) => day.format("%a %-d %b %Y").to_string(),
        None => "Date unknown".to_string(),
    }
}

/// "14 photos", "1 photo".
pub fn count(n: usize) -> String {
    if n == 1 {
        "1 photo".to_string()
    } else {
        format!("{n} photos")
    }
}

/// Every index in the order the grid draws them.
pub fn display_order(groups: &[Group]) -> Vec<usize> {
    groups.iter().flat_map(|g| g.indices.iter().copied()).collect()
}

/// A move of the selection in the grid.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Move {
    Next,
    Previous,
    /// One row down, keeping the column where the next row has one.
    Down,
    Up,
    First,
    Last,
}

/// Where the selection goes from `current` under `movement`, in a grid
/// `columns` wide. `None` when it cannot move — at an edge, or with
/// nothing selected in an empty grid.
///
/// Rows restart at every group, because each day starts a new row of
/// tiles under its heading. So Down from a group's last row lands in the
/// *first* row of the next day, at the same column or the last tile it
/// has — not wherever a flat index plus `columns` would happen to fall,
/// which would skip tiles or land mid-way through the next day.
pub fn step(groups: &[Group], columns: usize, current: usize, movement: Move) -> Option<usize> {
    let columns = columns.max(1);
    let order = display_order(groups);
    let at = order.iter().position(|&i| i == current);
    match movement {
        Move::First => order.first().copied(),
        Move::Last => order.last().copied(),
        Move::Next => match at {
            Some(p) => order.get(p + 1).copied(),
            None => order.first().copied(),
        },
        Move::Previous => match at {
            Some(p) => p.checked_sub(1).and_then(|p| order.get(p)).copied(),
            None => order.first().copied(),
        },
        Move::Down | Move::Up => {
            let Some((g, within)) = locate(groups, current) else {
                return order.first().copied();
            };
            let group = &groups[g].indices;
            let (row, column) = (within / columns, within % columns);
            let rows = group.len().div_ceil(columns);
            if movement == Move::Down {
                if row + 1 < rows {
                    let target = ((row + 1) * columns + column).min(group.len() - 1);
                    return Some(group[target]);
                }
                let next = groups.get(g + 1)?;
                Some(next.indices[column.min(next.indices.len() - 1)])
            } else {
                if row > 0 {
                    return Some(group[(row - 1) * columns + column]);
                }
                let previous = &groups.get(g.checked_sub(1)?)?.indices;
                let last_row = (previous.len() - 1) / columns;
                let target = (last_row * columns + column).min(previous.len() - 1);
                Some(previous[target])
            }
        }
    }
}

/// The fixed heights the grid is drawn at, logical pixels. The view
/// builds every header and row at exactly these, which is what lets
/// [`bands`] know where everything is without asking iced.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Metrics {
    pub columns: usize,
    pub header: f32,
    pub row: f32,
    /// Between rows inside a day.
    pub row_gap: f32,
    /// After a day's last row, before the next heading.
    pub group_gap: f32,
}

/// One horizontal band of the grid, top to bottom.
#[derive(Debug, Clone, PartialEq)]
pub enum Band {
    Header { top: f32, group: usize },
    Row { top: f32, indices: Vec<usize> },
}

impl Band {
    pub fn top(&self) -> f32 {
        match self {
            Band::Header { top, .. } | Band::Row { top, .. } => *top,
        }
    }
}

/// Where every heading and row sits.
pub fn bands(groups: &[Group], m: Metrics) -> Vec<Band> {
    let columns = m.columns.max(1);
    let mut out = Vec::new();
    let mut y = 0.0;
    for (g, group) in groups.iter().enumerate() {
        if g > 0 {
            y += m.group_gap;
        }
        out.push(Band::Header { top: y, group: g });
        y += m.header;
        for (r, chunk) in group.indices.chunks(columns).enumerate() {
            if r > 0 {
                y += m.row_gap;
            }
            out.push(Band::Row { top: y, indices: chunk.to_vec() });
            y += m.row;
        }
    }
    out
}

/// The items in rows that overlap the scrolled viewport, widened by
/// `margin` pixels either way — the tiles whose thumbnails are worth
/// decoding now.
///
/// This is the whole of the grid's memory bound. A folder of four
/// thousand photographs must not decode four thousand thumbnails because
/// someone opened the grid: only what can be seen, plus a screen's worth
/// of warning either side so scrolling does not show a wall of blanks.
pub fn visible(bands: &[Band], m: Metrics, top: f32, height: f32, margin: f32) -> Vec<usize> {
    let (from, to) = (top - margin, top + height + margin);
    bands
        .iter()
        .filter_map(|band| match band {
            Band::Row { top, indices } if *top + m.row >= from && *top <= to => Some(indices.iter().copied()),
            _ => None,
        })
        .flatten()
        .collect()
}

/// The scroll offset that brings `index`'s row into a viewport of
/// `height` scrolled to `top`, or `None` when it is already in view.
///
/// Keyboard movement must never walk the selection off the screen, and
/// scrolling when nothing needs it makes the grid jump under the pointer.
pub fn scroll_to_show(bands: &[Band], m: Metrics, index: usize, top: f32, height: f32) -> Option<f32> {
    let at = bands.iter().position(|band| matches!(band, Band::Row { indices, .. } if indices.contains(&index)))?;
    let row_top = bands[at].top();
    if row_top < top {
        // Bring its heading along when it is the first row of a day, so
        // scrolling up never hides the date the tile is filed under.
        let heading = at.checked_sub(1).and_then(|i| bands.get(i)).filter(|b| matches!(b, Band::Header { .. }));
        Some(heading.map_or(row_top, Band::top))
    } else if row_top + m.row > top + height {
        Some((row_top + m.row - height).max(0.0))
    } else {
        None
    }
}

fn locate(groups: &[Group], index: usize) -> Option<(usize, usize)> {
    groups.iter().enumerate().find_map(|(g, group)| {
        group.indices.iter().position(|&i| i == index).map(|within| (g, within))
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::folder::Media;
    use chrono::FixedOffset;
    use std::path::PathBuf;
    use std::time::{Duration, SystemTime};

    /// 2026-09-12 00:00 UTC.
    const SAT_12_SEP_2026: u64 = 1_789_171_200;
    const DAY: u64 = 86_400;

    fn item(name: &str, secs: Option<u64>) -> Item {
        Item {
            path: PathBuf::from("/p").join(name),
            name: name.into(),
            media: Media::Still,
            modified: secs.map(|s| SystemTime::UNIX_EPOCH + Duration::from_secs(s)),
            bytes: Some(1),
        }
    }

    fn utc() -> FixedOffset {
        FixedOffset::east_opt(0).unwrap()
    }

    #[test]
    fn the_newest_day_comes_first_and_each_day_keeps_the_folders_order() {
        let items = [
            item("a.jpg", Some(SAT_12_SEP_2026 - DAY + 10)),
            item("b.jpg", Some(SAT_12_SEP_2026 + 10)),
            item("c.jpg", Some(SAT_12_SEP_2026 - DAY + 20)),
            item("d.jpg", Some(SAT_12_SEP_2026 + 5)),
        ];
        let g = groups(&items, &utc());
        assert_eq!(g.len(), 2);
        assert_eq!(g[0].day, NaiveDate::from_ymd_opt(2026, 9, 12));
        assert_eq!(g[0].indices, [1, 3]);
        assert_eq!(g[1].indices, [0, 2]);
    }

    /// The day is the *local* one. A photograph taken at 23:30 in UTC+2
    /// is filed under that evening, not under the next UTC morning.
    #[test]
    fn a_picture_is_filed_under_the_local_day() {
        let items = [item("late.jpg", Some(SAT_12_SEP_2026 - 30 * 60))];
        let plus_two = FixedOffset::east_opt(2 * 3600).unwrap();
        assert_eq!(groups(&items, &plus_two)[0].day, NaiveDate::from_ymd_opt(2026, 9, 12));
        assert_eq!(groups(&items, &utc())[0].day, NaiveDate::from_ymd_opt(2026, 9, 11));
    }

    #[test]
    fn a_file_with_no_time_is_kept_and_filed_last() {
        let items = [item("a.jpg", None), item("b.jpg", Some(SAT_12_SEP_2026))];
        let g = groups(&items, &utc());
        assert_eq!(g.last().unwrap().day, None);
        assert_eq!(g.last().unwrap().indices, [0]);
        assert_eq!(heading(None, NaiveDate::from_ymd_opt(2026, 9, 25).unwrap()), "Date unknown");
    }

    #[test]
    fn a_heading_names_the_year_only_when_it_is_not_this_one() {
        let today = NaiveDate::from_ymd_opt(2026, 9, 25).unwrap();
        assert_eq!(heading(NaiveDate::from_ymd_opt(2026, 9, 12), today), "Sat 12 Sep");
        assert_eq!(heading(NaiveDate::from_ymd_opt(2025, 9, 12), today), "Fri 12 Sep 2025");
    }

    #[test]
    fn one_photo_is_not_one_photos() {
        assert_eq!(count(1), "1 photo");
        assert_eq!(count(14), "14 photos");
    }

    fn two_days() -> Vec<Group> {
        // A day of seven tiles over a day of three, four columns wide:
        //   0 1 2 3
        //   4 5 6
        //   ---
        //   7 8 9
        vec![
            Group { day: NaiveDate::from_ymd_opt(2026, 9, 12), indices: (0..7).collect() },
            Group { day: NaiveDate::from_ymd_opt(2026, 9, 11), indices: vec![7, 8, 9] },
        ]
    }

    #[test]
    fn down_keeps_the_column_inside_a_day() {
        assert_eq!(step(&two_days(), 4, 1, Move::Down), Some(5));
        // Row two is short: from column three it lands on the last tile.
        assert_eq!(step(&two_days(), 4, 3, Move::Down), Some(6));
    }

    /// Down from a day's last row goes to the first row of the next day,
    /// not to wherever index-plus-four would fall.
    #[test]
    fn down_from_the_last_row_of_a_day_starts_the_next_one() {
        assert_eq!(step(&two_days(), 4, 5, Move::Down), Some(8));
        assert_eq!(step(&two_days(), 4, 6, Move::Down), Some(9));
    }

    #[test]
    fn up_from_the_first_row_of_a_day_lands_in_the_last_row_of_the_one_before() {
        assert_eq!(step(&two_days(), 4, 8, Move::Up), Some(5));
        assert_eq!(step(&two_days(), 4, 9, Move::Up), Some(6));
        assert_eq!(step(&two_days(), 4, 1, Move::Up), None);
    }

    #[test]
    fn next_and_previous_run_through_the_days_in_display_order() {
        assert_eq!(step(&two_days(), 4, 6, Move::Next), Some(7));
        assert_eq!(step(&two_days(), 4, 7, Move::Previous), Some(6));
        assert_eq!(step(&two_days(), 4, 9, Move::Next), None);
        assert_eq!(step(&two_days(), 4, 0, Move::Last), Some(9));
    }

    fn metrics() -> Metrics {
        Metrics { columns: 4, header: 30.0, row: 100.0, row_gap: 10.0, group_gap: 20.0 }
    }

    #[test]
    fn bands_stack_headings_rows_and_gaps_in_order() {
        let b = bands(&two_days(), metrics());
        let tops: Vec<f32> = b.iter().map(Band::top).collect();
        // Header 0; rows at 30 and 140; gap; header at 260; row at 290.
        assert_eq!(tops, [0.0, 30.0, 140.0, 260.0, 290.0]);
    }

    /// The memory bound: a viewport over the first row decodes that row
    /// (and its margin), never the whole folder.
    #[test]
    fn only_rows_in_or_near_the_viewport_want_thumbnails() {
        let b = bands(&two_days(), metrics());
        assert_eq!(visible(&b, metrics(), 0.0, 100.0, 0.0), [0, 1, 2, 3]);
        assert_eq!(visible(&b, metrics(), 0.0, 100.0, 50.0), [0, 1, 2, 3, 4, 5, 6]);
        assert_eq!(visible(&b, metrics(), 280.0, 100.0, 0.0), [7, 8, 9]);
    }

    #[test]
    fn keyboard_movement_scrolls_only_when_the_tile_is_out_of_view() {
        let b = bands(&two_days(), metrics());
        assert_eq!(scroll_to_show(&b, metrics(), 5, 0.0, 250.0), None);
        // Below the fold: scrolled just far enough to show the row.
        assert_eq!(scroll_to_show(&b, metrics(), 8, 0.0, 250.0), Some(140.0));
        // Above it, on a day's first row: its heading comes too.
        assert_eq!(scroll_to_show(&b, metrics(), 7, 300.0, 250.0), Some(260.0));
        assert_eq!(scroll_to_show(&b, metrics(), 4, 200.0, 250.0), Some(140.0));
    }

    /// Every tile is reachable from every other with the arrows, however
    /// the days fall and however wide the grid is.
    #[test]
    fn no_tile_is_unreachable_by_down() {
        for columns in 1..6 {
            let g = two_days();
            let mut seen = std::collections::HashSet::new();
            let mut at = 0;
            seen.insert(at);
            while let Some(next) = step(&g, columns, at, Move::Down) {
                at = next;
                seen.insert(at);
            }
            // Down alone visits one tile per row; with Next, every tile.
            let mut at = 0;
            while let Some(next) = step(&g, columns, at, Move::Next) {
                at = next;
                seen.insert(at);
            }
            assert_eq!(seen.len(), 10, "columns = {columns}");
        }
    }
}
