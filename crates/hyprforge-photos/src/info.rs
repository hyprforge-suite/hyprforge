//! What the info panel says, as data.
//!
//! A `Vec` of label-and-value rather than widgets, so "what does this
//! say about a sideways JPEG" is a unit test rather than a screenshot.
//! The view turns these into rows and adds nothing of its own.

use crate::folder::{Item, Media};
use crate::rotation::Turns;
use hyprforge_image::Measured;

/// One row of the panel.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Row {
    pub label: &'static str,
    pub value: String,
}

fn row(label: &'static str, value: impl Into<String>) -> Row {
    Row { label, value: value.into() }
}

/// The rows for an item, with whatever is known about it.
///
/// `measured` is `None` before the decode lands, or for a clip — which
/// is not a failure and not a blank panel: the name and what kind of
/// thing it is are known immediately, and those rows appear at once.
pub fn rows(
    item: &Item,
    measured: Option<&Measured>,
    turns: Turns,
    position: Option<(usize, usize)>,
    size_on_disk: Option<u64>,
) -> Vec<Row> {
    let mut rows = vec![row("Name", item.name.clone())];

    if let Some((at, total)) = position {
        rows.push(row("In folder", format!("{at} of {total}")));
    }

    match item.media {
        Media::Still => {
            if let Some(m) = measured {
                let (w, h) = m.display_size();
                rows.push(row("Dimensions", format!("{w} × {h}")));
                rows.push(row("Format", format_name(m.format)));
                // Only worth a row when there is something to say: a
                // picture that is upright and untouched does not need to
                // be told it is upright.
                if crate::rotation::is_turned(m.orientation, turns) {
                    rows.push(row("Rotation", describe_rotation(m, turns)));
                }
            }
        }
        Media::Clip => {
            // Said plainly rather than left blank. A viewer that shows
            // nothing for a clip looks broken; one that says what it is
            // and what Enter will do is simply honest about its scope.
            rows.push(row("Kind", "Video — press Enter to play it"));
        }
    }

    if let Some(bytes) = size_on_disk {
        rows.push(row("Size", human_size(bytes)));
    }

    rows
}

fn format_name(format: image::ImageFormat) -> String {
    format!("{format:?}").to_uppercase()
}

fn describe_rotation(measured: &Measured, turns: Turns) -> String {
    let from_file = measured.orientation != hyprforge_image::Orientation::Upright;
    match (from_file, turns.quarters()) {
        (true, 0) => "Upright, as the camera recorded".to_string(),
        (true, q) => format!("Turned {}° by you, on the camera's orientation", q as u32 * 90),
        (false, q) => format!("Turned {}° by you", q as u32 * 90),
    }
}

/// Bytes in the units people read them in.
///
/// Powers of 1024 with the short names, matching what the file manager's
/// own `format::human_readable_size` shows — two windows disagreeing
/// about how big one file is would be a small, silly thing to ship.
fn human_size(bytes: u64) -> String {
    const UNITS: [&str; 5] = ["B", "KB", "MB", "GB", "TB"];
    let mut value = bytes as f64;
    let mut unit = 0;
    while value >= 1024.0 && unit < UNITS.len() - 1 {
        value /= 1024.0;
        unit += 1;
    }
    if unit == 0 {
        format!("{bytes} B")
    } else {
        format!("{value:.1} {}", UNITS[unit])
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use hyprforge_image::measure::SourcePixels;
    use hyprforge_image::Orientation;
    use std::path::PathBuf;

    fn still(name: &str) -> Item {
        Item { path: PathBuf::from("/photos").join(name), name: name.into(), media: Media::Still }
    }

    fn clip(name: &str) -> Item {
        Item { path: PathBuf::from("/photos").join(name), name: name.into(), media: Media::Clip }
    }

    fn measured(width: u32, height: u32, orientation: Orientation) -> Measured {
        Measured {
            source: SourcePixels { width, height },
            orientation,
            format: image::ImageFormat::Jpeg,
        }
    }

    fn value_of<'a>(rows: &'a [Row], label: &str) -> Option<&'a str> {
        rows.iter().find(|r| r.label == label).map(|r| r.value.as_str())
    }

    #[test]
    fn the_panel_names_the_picture_and_where_it_is_in_the_folder() {
        let rows = rows(&still("a.jpg"), None, Turns::none(), Some((3, 47)), None);
        assert_eq!(value_of(&rows, "Name"), Some("a.jpg"));
        assert_eq!(value_of(&rows, "In folder"), Some("3 of 47"));
    }

    /// The dimensions shown are the ones the picture *presents*, so the
    /// panel and the window agree about a sideways photograph.
    #[test]
    fn a_sideways_jpeg_reports_the_size_you_can_see() {
        let m = measured(4032, 3024, Orientation::Rotate90);
        let rows = rows(&still("phone.jpg"), Some(&m), Turns::none(), None, None);
        assert_eq!(value_of(&rows, "Dimensions"), Some("3024 × 4032"));
    }

    #[test]
    fn an_upright_untouched_picture_says_nothing_about_rotation() {
        let m = measured(800, 600, Orientation::Upright);
        let rows = rows(&still("a.jpg"), Some(&m), Turns::none(), None, None);
        assert_eq!(value_of(&rows, "Rotation"), None);
    }

    #[test]
    fn a_turn_the_user_made_is_described_as_theirs() {
        let m = measured(800, 600, Orientation::Upright);
        let rows = rows(&still("a.jpg"), Some(&m), Turns::none().right(), None, None);
        assert_eq!(value_of(&rows, "Rotation"), Some("Turned 90° by you"));
    }

    /// A clip is not a blank panel. It says what it is and what will
    /// happen if you press Enter.
    #[test]
    fn a_clip_says_what_it_is_rather_than_showing_nothing() {
        let rows = rows(&clip("holiday.mp4"), None, Turns::none(), Some((2, 5)), None);
        assert_eq!(value_of(&rows, "Name"), Some("holiday.mp4"));
        assert!(value_of(&rows, "Kind").unwrap().contains("Enter"));
        assert_eq!(value_of(&rows, "Dimensions"), None);
    }

    /// Before the decode lands there is still something to show, rather
    /// than an empty panel that flashes.
    #[test]
    fn a_picture_that_has_not_decoded_yet_still_has_rows() {
        let rows = rows(&still("a.jpg"), None, Turns::none(), Some((1, 1)), Some(2048));
        assert_eq!(value_of(&rows, "Name"), Some("a.jpg"));
        assert_eq!(value_of(&rows, "Size"), Some("2.0 KB"));
    }

    #[test]
    fn sizes_read_the_way_people_read_them() {
        assert_eq!(human_size(0), "0 B");
        assert_eq!(human_size(512), "512 B");
        assert_eq!(human_size(1024), "1.0 KB");
        assert_eq!(human_size(1024 * 1024 * 3 / 2), "1.5 MB");
    }
}
