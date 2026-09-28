//! What the inspector says, as data — mockup `2c`.
//!
//! Sections of label-and-value rather than widgets, so "what does this
//! say about a sideways JPEG" is a unit test rather than a screenshot.
//! The view turns these into rows and adds nothing of its own.
//!
//! The name and the date sit above the sections, as the mockup's heading
//! does — the date is [`when`]'s business — and the status bar's one
//! line is [`status_line`]'s.

use crate::folder::{Item, Media};
use crate::rotation::Turns;
use chrono::TimeZone;
use hyprforge_image::{Camera, Measured};

/// One row of a section.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Row {
    pub label: &'static str,
    pub value: String,
}

fn row(label: &'static str, value: impl Into<String>) -> Row {
    Row { label, value: value.into() }
}

/// What a loaded 3D model is, for the inspector and the status bar —
/// taken from the mesh once it has loaded, the way `Measured` is taken
/// from a picture's header.
#[derive(Debug, Clone, PartialEq)]
pub struct ModelFacts {
    pub format: hyprforge_mesh::Format,
    pub triangles: usize,
    pub vertices: usize,
    /// The bounding box's extent on each axis, in the file's units.
    pub size: [f32; 3],
    pub has_colors: bool,
}

impl ModelFacts {
    pub fn of(format: hyprforge_mesh::Format, mesh: &hyprforge_mesh::Mesh) -> ModelFacts {
        ModelFacts {
            format,
            triangles: mesh.tri_count(),
            vertices: mesh.verts.len(),
            size: mesh.bounds.size().to_array(),
            has_colors: mesh.has_colors,
        }
    }
}

/// A titled group of rows — `FILE`, `CAMERA`, `LOCATION`, `MODEL`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Section {
    pub title: &'static str,
    pub rows: Vec<Row>,
}

/// The inspector's sections for an item, with whatever is known about
/// it.
///
/// `measured` is `None` before the decode lands, or for a clip — which
/// is not a failure and not a blank panel: what the listing knows is
/// shown at once. A section with nothing in it is left out rather than
/// drawn empty: a screenshot has no camera, and a `CAMERA` heading over
/// nothing reads as data that failed to load.
///
/// No Location map and no Tags, both of which the mockup draws: a map
/// needs tiles from a network service this suite does not talk to, and
/// tags need somewhere to keep them that is not the photograph. The
/// coordinates are shown as text, which is what the map would be of.
pub fn sections(
    item: &Item,
    measured: Option<&Measured>,
    camera: Option<&Camera>,
    model: Option<&ModelFacts>,
    turns: Turns,
    size_on_disk: Option<u64>,
) -> Vec<Section> {
    let mut file = Vec::new();
    if let Some(bytes) = size_on_disk.or(item.bytes) {
        file.push(row("Size", human_size(bytes)));
    }
    match item.media {
        Media::Still => {
            if let Some(m) = measured {
                let (w, h) = m.display_size();
                file.push(row("Pixels", format!("{w} × {h}")));
                file.push(row("Format", format_name(m.format)));
                // Only worth a row when there is something to say: a
                // picture that is upright and untouched does not need to
                // be told it is upright.
                if crate::rotation::is_turned(m.orientation, turns) {
                    file.push(row("Rotation", describe_rotation(m, turns)));
                }
            }
        }
        Media::Clip => {
            // Said plainly rather than left blank. A viewer that shows
            // nothing for a clip looks broken; one that says what it is
            // and what Enter will do is simply honest about its scope.
            file.push(row("Kind", "Video"));
        }
        Media::Model => {
            if let Some(format) = hyprforge_mesh::detect(&item.path) {
                file.push(row("Format", format!("{} model", format.name())));
            }
        }
    }

    let mut out = vec![Section { title: "File", rows: file }];

    if let Some(m) = model.filter(|_| item.media == Media::Model) {
        out.push(Section { title: "Model", rows: model_rows(m) });
    }

    if let Some(camera) = camera {
        let mut rows = Vec::new();
        if let Some(body) = camera.body() {
            rows.push(row("Body", body));
        }
        if let Some(lens) = &camera.lens {
            rows.push(row("Lens", lens.clone()));
        }
        if let Some(exposure) = camera.exposure_line() {
            rows.push(row("Exposure", exposure));
        }
        if let Some(mm) = camera.focal_length_mm {
            rows.push(row("Focal length", format!("{} mm", trim(mm))));
        }
        out.push(Section { title: "Camera", rows });
        if let Some(location) = camera.location {
            out.push(Section { title: "Location", rows: vec![row("Where", location.describe())] });
        }
    }

    out.retain(|s| !s.rows.is_empty());
    out
}

/// A video's section, once the player knows its length and size: the
/// two things a file listing cannot say about it.
pub fn video_section(duration: Option<f64>, size: Option<(u32, u32)>) -> Option<Section> {
    let mut rows = Vec::new();
    if let Some(d) = duration {
        rows.push(row("Length", clock(d)));
    }
    if let Some((w, h)) = size.filter(|(w, h)| *w > 0 && *h > 0) {
        rows.push(row("Pixels", format!("{w} × {h}")));
    }
    (!rows.is_empty()).then_some(Section { title: "Video", rows })
}

/// Seconds as a player's clock: `0:07`, `4:56`, `1:02:03`.
pub fn clock(seconds: f64) -> String {
    let total = seconds.max(0.0).floor() as u64;
    let (h, m, s) = (total / 3600, (total % 3600) / 60, total % 60);
    if h > 0 {
        format!("{h}:{m:02}:{s:02}")
    } else {
        format!("{m}:{s:02}")
    }
}

/// The status bar's line for a video: "clip.mp4 · 1:23 / 4:56 · 38.2 MB".
pub fn video_status_line(item: &Item, position: Option<f64>, duration: Option<f64>, size_on_disk: Option<u64>) -> String {
    let mut parts = vec![item.name.clone()];
    match (position, duration) {
        (Some(p), Some(d)) => parts.push(format!("{} / {}", clock(p), clock(d))),
        (None, Some(d)) => parts.push(clock(d)),
        _ => {}
    }
    if let Some(bytes) = size_on_disk.or(item.bytes) {
        parts.push(human_size(bytes));
    }
    parts.join(" · ")
}

/// A model's rows: how much geometry, how big, and whether it brought
/// its own colours — the three things worth knowing before printing it.
///
/// Size is in millimetres for a 3MF, whose loader converts from the
/// file's declared unit, and in bare file units otherwise: STL and OBJ
/// declare no unit at all, and most STLs are millimetres by convention
/// only, which is not a claim to print.
fn model_rows(m: &ModelFacts) -> Vec<Row> {
    let [x, y, z] = m.size;
    let unit = if m.format == hyprforge_mesh::Format::ThreeMf { " mm" } else { "" };
    vec![
        row("Triangles", thousands(m.triangles)),
        row("Vertices", thousands(m.vertices)),
        row("Size", format!("{} × {} × {}{unit}", trim(x as f64), trim(y as f64), trim(z as f64))),
        row("Colours", if m.has_colors { "From the file" } else { "None" }),
    ]
}

/// The status bar's line for a model: "bracket.stl · 12,480 triangles ·
/// 609.4 KB".
pub fn model_status_line(item: &Item, model: Option<&ModelFacts>, size_on_disk: Option<u64>) -> String {
    let mut parts = vec![item.name.clone()];
    if let Some(m) = model {
        parts.push(format!("{} triangles", thousands(m.triangles)));
    }
    if let Some(bytes) = size_on_disk.or(item.bytes) {
        parts.push(human_size(bytes));
    }
    parts.join(" · ")
}

/// `12480` as `12,480`: triangle counts run to millions, and seven
/// unbroken digits is a number nobody reads at a glance.
fn thousands(n: usize) -> String {
    let digits = n.to_string();
    let mut out = String::with_capacity(digits.len() + digits.len() / 3);
    for (i, c) in digits.chars().enumerate() {
        if i > 0 && (digits.len() - i).is_multiple_of(3) {
            out.push(',');
        }
        out.push(c);
    }
    out
}

/// The line under the name: when the picture was taken, "Sat 12 Sep
/// 2026 · 18:42".
///
/// The camera's own clock when it wrote one, else the file's time in the
/// local zone. Not both, and not labelled: the two usually agree, and a
/// line that said "modified" beneath a photograph would be answering a
/// question nobody looking at one asks.
pub fn when<Tz: TimeZone>(
    camera: Option<&Camera>,
    modified: Option<std::time::SystemTime>,
    tz: &Tz,
) -> Option<String>
where
    Tz::Offset: std::fmt::Display,
{
    if let Some(t) = camera.and_then(|c| c.taken) {
        let date = chrono::NaiveDate::from_ymd_opt(t.year as i32, t.month as u32, t.day as u32)?;
        return Some(format!("{} · {:02}:{:02}", date.format("%a %-d %b %Y"), t.hour, t.minute));
    }
    let local = chrono::DateTime::<chrono::Utc>::from(modified?).with_timezone(tz);
    Some(local.format("%a %-d %b %Y · %H:%M").to_string())
}

/// The status bar's left-hand line: "IMG_2052.jpg · 6000×4000 · 8.4 MB",
/// with whatever of the last two is known.
pub fn status_line(item: &Item, measured: Option<&Measured>, size_on_disk: Option<u64>) -> String {
    let mut parts = vec![item.name.clone()];
    if let Some(m) = measured {
        let (w, h) = m.display_size();
        parts.push(format!("{w}×{h}"));
    }
    if let Some(bytes) = size_on_disk.or(item.bytes) {
        parts.push(human_size(bytes));
    }
    parts.join(" · ")
}

fn trim(value: f64) -> String {
    if value.fract() == 0.0 {
        format!("{}", value as i64)
    } else {
        format!("{value:.1}")
    }
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
pub fn human_size(bytes: u64) -> String {
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
    use chrono::FixedOffset;
    use hyprforge_image::camera::{Location, Taken};
    use hyprforge_image::measure::SourcePixels;
    use hyprforge_image::Orientation;
    use std::path::PathBuf;

    fn still(name: &str) -> Item {
        Item { path: PathBuf::from("/photos").join(name), name: name.into(), media: Media::Still, modified: None, bytes: None }
    }

    fn clip(name: &str) -> Item {
        Item { path: PathBuf::from("/photos").join(name), name: name.into(), media: Media::Clip, modified: None, bytes: None }
    }

    fn measured(width: u32, height: u32, orientation: Orientation) -> Measured {
        Measured {
            source: SourcePixels { width, height },
            orientation,
            format: image::ImageFormat::Jpeg,
        }
    }

    fn value_of<'a>(sections: &'a [Section], label: &str) -> Option<&'a str> {
        sections.iter().flat_map(|s| &s.rows).find(|r| r.label == label).map(|r| r.value.as_str())
    }

    fn titles(sections: &[Section]) -> Vec<&'static str> {
        sections.iter().map(|s| s.title).collect()
    }

    /// The dimensions shown are the ones the picture *presents*, so the
    /// panel, the status bar and the window agree about a sideways
    /// photograph.
    #[test]
    fn a_sideways_jpeg_reports_the_size_you_can_see() {
        let m = measured(4032, 3024, Orientation::Rotate90);
        let s = sections(&still("phone.jpg"), Some(&m), None, None, Turns::none(), None);
        assert_eq!(value_of(&s, "Pixels"), Some("3024 × 4032"));
        assert_eq!(status_line(&still("phone.jpg"), Some(&m), Some(2048)), "phone.jpg · 3024×4032 · 2.0 KB");
    }

    #[test]
    fn an_upright_untouched_picture_says_nothing_about_rotation() {
        let m = measured(800, 600, Orientation::Upright);
        let s = sections(&still("a.jpg"), Some(&m), None, None, Turns::none(), None);
        assert_eq!(value_of(&s, "Rotation"), None);
    }

    #[test]
    fn a_turn_the_user_made_is_described_as_theirs() {
        let m = measured(800, 600, Orientation::Upright);
        let s = sections(&still("a.jpg"), Some(&m), None, None, Turns::none().right(), None);
        assert_eq!(value_of(&s, "Rotation"), Some("Turned 90° by you"));
    }

    /// A clip is not a blank panel: it says what it is before the player
    /// has said anything about it.
    #[test]
    fn a_clip_says_what_it_is_rather_than_showing_nothing() {
        let s = sections(&clip("holiday.mp4"), None, None, None, Turns::none(), None);
        assert_eq!(value_of(&s, "Kind"), Some("Video"));
        assert_eq!(value_of(&s, "Pixels"), None);
    }

    #[test]
    fn a_clock_reads_like_a_players() {
        assert_eq!(clock(7.9), "0:07");
        assert_eq!(clock(296.0), "4:56");
        assert_eq!(clock(3723.0), "1:02:03");
        assert_eq!(clock(-1.0), "0:00");
    }

    #[test]
    fn a_video_section_appears_once_the_player_knows_something() {
        assert_eq!(video_section(None, None), None);
        let s = video_section(Some(83.0), Some((1920, 1080))).unwrap();
        assert_eq!(s.title, "Video");
        assert_eq!(value_of(std::slice::from_ref(&s), "Length"), Some("1:23"));
        assert_eq!(value_of(std::slice::from_ref(&s), "Pixels"), Some("1920 × 1080"));
        assert_eq!(
            video_status_line(&clip("a.mp4"), Some(12.0), Some(83.0), Some(2048)),
            "a.mp4 · 0:12 / 1:23 · 2.0 KB"
        );
    }

    /// Before the decode lands there is still something to show, rather
    /// than an empty panel that flashes.
    #[test]
    fn a_picture_that_has_not_decoded_yet_still_has_rows() {
        let s = sections(&still("a.jpg"), None, None, None, Turns::none(), Some(2048));
        assert_eq!(value_of(&s, "Size"), Some("2.0 KB"));
    }

    /// A screenshot has no camera, and a heading over nothing reads as
    /// something that failed to load.
    #[test]
    fn a_picture_with_no_camera_has_no_camera_section() {
        let m = measured(800, 600, Orientation::Upright);
        let s = sections(&still("shot.png"), Some(&m), Some(&Camera::default()), None, Turns::none(), Some(1));
        assert_eq!(titles(&s), ["File"]);
    }

    #[test]
    fn a_camera_picture_has_the_mockups_sections() {
        let camera = Camera {
            make: Some("FUJIFILM".into()),
            model: Some("X-T5".into()),
            lens: Some("XF23mmF2 R WR".into()),
            exposure: Some((1, 500)),
            f_number: Some(2.0),
            iso: Some(160),
            focal_length_mm: Some(23.0),
            taken: None,
            location: Some(Location { latitude: 38.7139, longitude: -9.1334 }),
        };
        let m = measured(6000, 4000, Orientation::Upright);
        let s = sections(&still("IMG_2052.jpg"), Some(&m), Some(&camera), None, Turns::none(), Some(8_808_038));
        assert_eq!(titles(&s), ["File", "Camera", "Location"]);
        assert_eq!(value_of(&s, "Body"), Some("Fujifilm X-T5"));
        assert_eq!(value_of(&s, "Exposure"), Some("1/500 · f/2 · ISO 160"));
        assert_eq!(value_of(&s, "Size"), Some("8.4 MB"));
    }

    /// The camera's clock wins over the file's, because copying a card
    /// moves the file's time and not the moment the shutter went.
    #[test]
    fn the_date_taken_prefers_the_camera_to_the_file() {
        let camera = Camera {
            taken: Some(Taken { year: 2026, month: 9, day: 12, hour: 18, minute: 42 }),
            ..Camera::default()
        };
        let utc = FixedOffset::east_opt(0).unwrap();
        let copied = std::time::SystemTime::UNIX_EPOCH;
        assert_eq!(when(Some(&camera), Some(copied), &utc).as_deref(), Some("Sat 12 Sep 2026 · 18:42"));
        assert_eq!(when(None, Some(copied), &utc).as_deref(), Some("Thu 1 Jan 1970 · 00:00"));
        assert_eq!(when(None, None, &utc), None);
    }

    fn model(format: hyprforge_mesh::Format) -> ModelFacts {
        ModelFacts { format, triangles: 1_234_567, vertices: 617_000, size: [80.0, 60.25, 57.0], has_colors: false }
    }

    fn part(name: &str) -> Item {
        Item { media: Media::Model, ..still(name) }
    }

    #[test]
    fn a_model_says_how_much_geometry_and_how_big() {
        let facts = model(hyprforge_mesh::Format::Stl);
        let s = sections(&part("bracket.stl"), None, None, Some(&facts), Turns::none(), Some(2048));
        assert_eq!(titles(&s), ["File", "Model"]);
        assert_eq!(value_of(&s, "Format"), Some("STL model"));
        assert_eq!(value_of(&s, "Triangles"), Some("1,234,567"));
        assert_eq!(value_of(&s, "Colours"), Some("None"));
        assert_eq!(model_status_line(&part("bracket.stl"), Some(&facts), Some(2048)), "bracket.stl · 1,234,567 triangles · 2.0 KB");
    }

    /// Only a 3MF declares its unit, so only a 3MF's size says
    /// millimetres; an STL's is a number in whatever the file meant.
    #[test]
    fn only_a_model_that_declares_its_unit_is_given_one() {
        let stl = sections(&part("a.stl"), None, None, Some(&model(hyprforge_mesh::Format::Stl)), Turns::none(), None);
        assert_eq!(value_of(&stl, "Size"), Some("80 × 60.2 × 57"));
        let threemf = sections(&part("a.3mf"), None, None, Some(&model(hyprforge_mesh::Format::ThreeMf)), Turns::none(), None);
        assert_eq!(value_of(&threemf, "Size"), Some("80 × 60.2 × 57 mm"));
    }

    /// Before the mesh has loaded there is no Model section — not an
    /// empty one — and the File section already says what it is.
    #[test]
    fn a_model_that_has_not_loaded_yet_says_what_it_is() {
        let s = sections(&part("case.3mf"), None, None, None, Turns::none(), Some(10));
        assert_eq!(titles(&s), ["File"]);
        assert_eq!(value_of(&s, "Format"), Some("3MF model"));
    }

    #[test]
    fn big_counts_are_grouped_in_thousands() {
        assert_eq!(thousands(0), "0");
        assert_eq!(thousands(999), "999");
        assert_eq!(thousands(1000), "1,000");
        assert_eq!(thousands(2_000_000), "2,000,000");
    }

    #[test]
    fn sizes_read_the_way_people_read_them() {
        assert_eq!(human_size(0), "0 B");
        assert_eq!(human_size(512), "512 B");
        assert_eq!(human_size(1024), "1.0 KB");
        assert_eq!(human_size(1024 * 1024 * 3 / 2), "1.5 MB");
    }
}
