//! What the camera wrote down: body, lens, exposure, when and where.
//!
//! Read with `kamadak-exif`, which iced already brings into the tree for
//! its own orientation handling — so the inspector's Camera and Location
//! sections cost this crate a dependency line and no new download.
//!
//! # Never an error
//!
//! Like [`measure`](crate::measure::measure)'s orientation, a missing or damaged EXIF block
//! is [`Camera::default`], not a failure. Screenshots, exported PNGs and
//! pictures that went through a messenger have none, and a panel that
//! says nothing about a camera is right for all of them. What *is*
//! written survives damage elsewhere: the reader is told to carry on past
//! a bad IFD and keep whatever parsed.
//!
//! # Split in two
//!
//! [`read`] is the only part that touches a file. Everything a person
//! sees — "Fujifilm X-T5", "1/500 · f/2 · ISO 160", "38.7139° N" — is a
//! pure function over [`Camera`], because the formatting is where the
//! mistakes are (a shutter of 0.3s is not "1/3", a make in capitals is
//! not a brand) and a test should be able to reach it without a JPEG.

use std::path::Path;

/// The EXIF a person cares about, as plain values.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Camera {
    /// `Make` as written — often in capitals (`FUJIFILM`).
    pub make: Option<String>,
    /// `Model` as written — sometimes repeats the make (`Canon EOS R5`),
    /// sometimes does not (`X-T5`). [`Camera::body`] reconciles the two.
    pub model: Option<String>,
    pub lens: Option<String>,
    /// Exposure time as the camera's own fraction, so 1/500 stays 1/500
    /// rather than going through a float and coming back as 1/499.
    pub exposure: Option<(u32, u32)>,
    pub f_number: Option<f64>,
    pub iso: Option<u32>,
    pub focal_length_mm: Option<f64>,
    /// When the shutter was pressed, in the camera's own clock — EXIF
    /// usually has no time zone, and inventing one would be a claim this
    /// crate has no basis for.
    pub taken: Option<Taken>,
    pub location: Option<Location>,
}

/// `DateTimeOriginal`, unzoned.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Taken {
    pub year: u16,
    pub month: u8,
    pub day: u8,
    pub hour: u8,
    pub minute: u8,
}

/// Where, in signed decimal degrees: north and east positive.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Location {
    pub latitude: f64,
    pub longitude: f64,
}

/// Reads the EXIF of `path`. Blocking file I/O; call it off the UI
/// thread. Never fails — see the module doc.
pub fn read(path: &Path) -> Camera {
    let Ok(file) = std::fs::File::open(path) else {
        return Camera::default();
    };
    let mut reader = std::io::BufReader::new(file);
    let exif = match exif::Reader::new().continue_on_error(true).read_from_container(&mut reader) {
        Ok(exif) => exif,
        // Some of it parsed: keep that. A bad maker note must not cost
        // the shutter speed.
        Err(exif::Error::PartialResult(partial)) => partial.into_inner().0,
        Err(_) => return Camera::default(),
    };
    from_exif(&exif)
}

fn from_exif(exif: &exif::Exif) -> Camera {
    let field = |tag| exif.get_field(tag, exif::In::PRIMARY).map(|f| &f.value);
    let ascii = |tag| field(tag).and_then(ascii_value);
    let rational = |tag| match field(tag) {
        Some(exif::Value::Rational(v)) => v.first().copied(),
        _ => None,
    };

    Camera {
        make: ascii(exif::Tag::Make),
        model: ascii(exif::Tag::Model),
        lens: ascii(exif::Tag::LensModel),
        exposure: rational(exif::Tag::ExposureTime)
            .filter(|r| r.num > 0 && r.denom > 0)
            .map(|r| (r.num, r.denom)),
        f_number: rational(exif::Tag::FNumber).filter(|r| r.denom > 0).map(|r| r.to_f64()),
        iso: field(exif::Tag::PhotographicSensitivity).and_then(|v| v.get_uint(0)),
        focal_length_mm: rational(exif::Tag::FocalLength).filter(|r| r.denom > 0).map(|r| r.to_f64()),
        taken: match field(exif::Tag::DateTimeOriginal) {
            Some(exif::Value::Ascii(values)) => values
                .first()
                .and_then(|raw| exif::DateTime::from_ascii(raw).ok())
                .filter(|t| (1..=12).contains(&t.month) && (1..=31).contains(&t.day))
                .map(|t| Taken { year: t.year, month: t.month, day: t.day, hour: t.hour, minute: t.minute }),
            _ => None,
        },
        location: location(exif),
    }
}

fn ascii_value(value: &exif::Value) -> Option<String> {
    match value {
        exif::Value::Ascii(values) => values
            .first()
            .map(|raw| String::from_utf8_lossy(raw).trim().to_string())
            .filter(|s| !s.is_empty()),
        _ => None,
    }
}

fn location(exif: &exif::Exif) -> Option<Location> {
    let degrees = |tag, reference| {
        let parts = match exif.get_field(tag, exif::In::PRIMARY).map(|f| &f.value) {
            Some(exif::Value::Rational(v)) if v.len() == 3 && v.iter().all(|r| r.denom > 0) => v,
            _ => return None,
        };
        let value = parts[0].to_f64() + parts[1].to_f64() / 60.0 + parts[2].to_f64() / 3600.0;
        let negative = exif
            .get_field(reference, exif::In::PRIMARY)
            .and_then(|f| ascii_value(&f.value))
            .is_some_and(|r| r.eq_ignore_ascii_case("S") || r.eq_ignore_ascii_case("W"));
        Some(if negative { -value } else { value })
    };
    let latitude = degrees(exif::Tag::GPSLatitude, exif::Tag::GPSLatitudeRef)?;
    let longitude = degrees(exif::Tag::GPSLongitude, exif::Tag::GPSLongitudeRef)?;
    // A receiver with no fix writes zeroes, and "0° N, 0° E" is a point
    // in the Atlantic nobody photographed.
    if latitude == 0.0 && longitude == 0.0 {
        return None;
    }
    ((-90.0..=90.0).contains(&latitude) && (-180.0..=180.0).contains(&longitude))
        .then_some(Location { latitude, longitude })
}

impl Camera {
    /// Whether there is anything at all to show under "Camera".
    pub fn is_empty(&self) -> bool {
        self.body().is_none() && self.lens.is_none() && self.exposure_line().is_none()
    }

    /// The camera, the way a person names it: "Fujifilm X-T5",
    /// "Canon EOS R5" — never "FUJIFILM X-T5" or "Canon Canon EOS R5".
    ///
    /// Makers disagree about whether the model repeats the brand, so the
    /// brand (the make's first word) is added only when the model does
    /// not already start with it. A make written in capitals is
    /// title-cased when it is a word rather than an initialism.
    pub fn body(&self) -> Option<String> {
        let model = self.model.as_deref().map(str::trim).filter(|m| !m.is_empty());
        let brand = self.make.as_deref().and_then(|m| m.split_whitespace().next()).map(brand_case);
        match (brand, model) {
            (Some(brand), Some(model)) => {
                if model.to_lowercase().starts_with(&brand.to_lowercase()) {
                    Some(model.to_string())
                } else {
                    Some(format!("{brand} {model}"))
                }
            }
            (None, Some(model)) => Some(model.to_string()),
            (Some(brand), None) => Some(brand),
            (None, None) => None,
        }
    }

    /// "1/500 · f/2 · ISO 160" — whichever of the three are known.
    pub fn exposure_line(&self) -> Option<String> {
        let parts: Vec<String> = [
            self.exposure.map(|(n, d)| shutter(n, d)),
            self.f_number.map(aperture),
            self.iso.map(|iso| format!("ISO {iso}")),
        ]
        .into_iter()
        .flatten()
        .collect();
        (!parts.is_empty()).then(|| parts.join(" · "))
    }

    /// One line for a status bar: "X-T5 · 23mm f/2 · 1/500 · ISO 160".
    ///
    /// The model alone, not [`Camera::body`]: the bar is short, and the
    /// brand is the part a person already knows.
    pub fn summary(&self) -> Option<String> {
        let mut parts = Vec::new();
        if let Some(model) = self.model.as_deref().filter(|m| !m.is_empty()) {
            parts.push(model.to_string());
        }
        match (self.focal_length_mm, self.f_number) {
            (Some(mm), Some(f)) => parts.push(format!("{}mm {}", trim_number(mm), aperture(f))),
            (Some(mm), None) => parts.push(format!("{}mm", trim_number(mm))),
            (None, Some(f)) => parts.push(aperture(f)),
            (None, None) => {}
        }
        if let Some((n, d)) = self.exposure {
            parts.push(shutter(n, d));
        }
        if let Some(iso) = self.iso {
            parts.push(format!("ISO {iso}"));
        }
        (!parts.is_empty()).then(|| parts.join(" · "))
    }
}

impl Location {
    /// "38.7139° N, 9.1334° W" — four places is about ten metres, which
    /// is as precise as a phone's fix is honest about.
    pub fn describe(&self) -> String {
        let ns = if self.latitude < 0.0 { 'S' } else { 'N' };
        let ew = if self.longitude < 0.0 { 'W' } else { 'E' };
        format!("{:.4}° {ns}, {:.4}° {ew}", self.latitude.abs(), self.longitude.abs())
    }
}

/// A shutter speed the way a camera's own screen shows it: a fraction
/// under a second (`1/500`), seconds from there up (`0.3s`, `2s`).
///
/// A camera often stores `10/5000` rather than `1/500`, so the fraction
/// is reduced — but only when that lands on a whole denominator. `3/10`
/// is not a stop anyone dials in as `1/3.33`.
pub fn shutter(num: u32, denom: u32) -> String {
    if num == 0 || denom == 0 {
        return "—".to_string();
    }
    if num >= denom {
        return format!("{}s", trim_number(num as f64 / denom as f64));
    }
    if denom.is_multiple_of(num) {
        return format!("1/{}", denom / num);
    }
    let seconds = num as f64 / denom as f64;
    if seconds >= 0.25 {
        format!("{}s", trim_number(seconds))
    } else {
        format!("1/{}", (1.0 / seconds).round() as u32)
    }
}

/// `f/2`, `f/1.4`, `f/5.6` — one decimal, and none when it is whole.
pub fn aperture(f: f64) -> String {
    format!("f/{}", trim_number(f))
}

/// At most one decimal place, and no `.0`.
fn trim_number(value: f64) -> String {
    let rounded = (value * 10.0).round() / 10.0;
    if rounded.fract() == 0.0 {
        format!("{}", rounded as i64)
    } else {
        format!("{rounded:.1}")
    }
}

/// `FUJIFILM` → `Fujifilm`; `LG`, `DJI` and `Canon` stay as they are.
fn brand_case(word: &str) -> String {
    let all_caps = word.chars().all(|c| !c.is_alphabetic() || c.is_uppercase());
    if all_caps && word.chars().filter(|c| c.is_alphabetic()).count() > 3 {
        let mut chars = word.chars();
        match chars.next() {
            Some(first) => first.to_uppercase().chain(chars.flat_map(char::to_lowercase)).collect(),
            None => String::new(),
        }
    } else {
        word.to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn x_t5() -> Camera {
        Camera {
            make: Some("FUJIFILM".into()),
            model: Some("X-T5".into()),
            lens: Some("XF23mmF2 R WR".into()),
            exposure: Some((1, 500)),
            f_number: Some(2.0),
            iso: Some(160),
            focal_length_mm: Some(23.0),
            taken: None,
            location: None,
        }
    }

    #[test]
    fn a_make_in_capitals_is_named_the_way_people_say_it() {
        assert_eq!(x_t5().body().as_deref(), Some("Fujifilm X-T5"));
    }

    #[test]
    fn a_model_that_already_names_its_brand_is_not_given_it_twice() {
        let canon = Camera { make: Some("Canon".into()), model: Some("Canon EOS R5".into()), ..Camera::default() };
        assert_eq!(canon.body().as_deref(), Some("Canon EOS R5"));
        let nikon = Camera { make: Some("NIKON CORPORATION".into()), model: Some("NIKON Z 6".into()), ..Camera::default() };
        assert_eq!(nikon.body().as_deref(), Some("NIKON Z 6"));
    }

    #[test]
    fn an_initialism_keeps_its_capitals() {
        assert_eq!(brand_case("DJI"), "DJI");
        assert_eq!(brand_case("LG"), "LG");
        assert_eq!(brand_case("SONY"), "Sony");
    }

    #[test]
    fn the_status_line_reads_like_the_mockup() {
        assert_eq!(x_t5().summary().as_deref(), Some("X-T5 · 23mm f/2 · 1/500 · ISO 160"));
        assert_eq!(x_t5().exposure_line().as_deref(), Some("1/500 · f/2 · ISO 160"));
    }

    #[test]
    fn shutter_speeds_read_the_way_a_camera_shows_them() {
        assert_eq!(shutter(1, 500), "1/500");
        // Stored unreduced by plenty of bodies.
        assert_eq!(shutter(10, 5000), "1/500");
        assert_eq!(shutter(3, 10), "0.3s");
        assert_eq!(shutter(2, 1), "2s");
        assert_eq!(shutter(5, 2), "2.5s");
        assert_eq!(shutter(0, 1), "—");
    }

    #[test]
    fn apertures_carry_a_decimal_only_when_they_have_one() {
        assert_eq!(aperture(2.0), "f/2");
        assert_eq!(aperture(1.4), "f/1.4");
        assert_eq!(aperture(5.6), "f/5.6");
    }

    #[test]
    fn a_picture_with_no_camera_data_has_nothing_to_show() {
        assert!(Camera::default().is_empty());
        assert_eq!(Camera::default().summary(), None);
        assert!(!x_t5().is_empty());
    }

    #[test]
    fn a_location_says_which_hemisphere() {
        let lisbon = Location { latitude: 38.71389, longitude: -9.13339 };
        assert_eq!(lisbon.describe(), "38.7139° N, 9.1334° W");
    }

    #[test]
    fn a_file_with_no_exif_reads_as_no_camera_rather_than_failing() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("plain.png");
        image::RgbImage::new(4, 4).save(&path).unwrap();
        assert_eq!(read(&path), Camera::default());
        assert_eq!(read(&dir.path().join("missing.jpg")), Camera::default());
    }

    /// The marshalling, against real EXIF bytes rather than a struct
    /// built by hand: tags in the Exif and GPS sub-IFDs, rationals,
    /// a short and ASCII, parsed by the library this module trusts.
    #[test]
    fn the_camera_section_is_read_from_real_exif() {
        let exif = exif::Reader::new().read_raw(tiff_fixture()).unwrap();
        let camera = from_exif(&exif);
        assert_eq!(camera.body().as_deref(), Some("Fujifilm X-T5"));
        assert_eq!(camera.lens.as_deref(), Some("XF23mmF2 R WR"));
        assert_eq!(camera.exposure_line().as_deref(), Some("1/500 · f/2 · ISO 160"));
        assert_eq!(camera.focal_length_mm, Some(23.0));
        assert_eq!(
            camera.taken,
            Some(Taken { year: 2026, month: 9, day: 12, hour: 18, minute: 42 })
        );
        let at = camera.location.expect("a GPS block was written");
        assert_eq!(at.describe(), "38.7139° N, 9.1334° W");
    }

    #[test]
    fn a_gps_block_of_zeroes_is_no_location() {
        let exif = exif::Reader::new().read_raw(tiff_with_zero_gps()).unwrap();
        assert_eq!(from_exif(&exif).location, None);
    }

    // --- a tiny big-endian TIFF writer, enough for the tests above ---

    enum V {
        Ascii(&'static str),
        Short(u16),
        Long(u32),
        Rationals(Vec<(u32, u32)>),
    }

    /// Lays out IFDs one after another, each entry's out-of-line data
    /// straight after its IFD. `pointers` patch a LONG entry (the Exif
    /// and GPS IFD pointers) with the offset of another IFD by index.
    fn tiff(ifds: Vec<Vec<(u16, V)>>, pointers: &[(usize, u16, usize)]) -> Vec<u8> {
        let mut out = vec![0x4D, 0x4D, 0x00, 0x2A, 0, 0, 0, 8];
        let mut starts = Vec::new();
        // First pass: sizes, so pointers can be resolved.
        let mut at = 8usize;
        for ifd in &ifds {
            starts.push(at);
            let extra: usize = ifd.iter().map(|(_, v)| out_of_line(v).len()).sum();
            at += 2 + ifd.len() * 12 + 4 + extra;
        }
        for (i, ifd) in ifds.iter().enumerate() {
            let data_start = starts[i] + 2 + ifd.len() * 12 + 4;
            let mut data = Vec::new();
            out.extend_from_slice(&(ifd.len() as u16).to_be_bytes());
            for (tag, value) in ifd {
                let (kind, count) = match value {
                    V::Ascii(s) => (2u16, s.len() as u32 + 1),
                    V::Short(_) => (3, 1),
                    V::Long(_) => (4, 1),
                    V::Rationals(r) => (5, r.len() as u32),
                };
                out.extend_from_slice(&tag.to_be_bytes());
                out.extend_from_slice(&kind.to_be_bytes());
                out.extend_from_slice(&count.to_be_bytes());
                let extra = out_of_line(value);
                if extra.is_empty() {
                    let mut inline = match value {
                        V::Ascii(s) => {
                            let mut b = s.as_bytes().to_vec();
                            b.push(0);
                            b
                        }
                        V::Short(n) => n.to_be_bytes().to_vec(),
                        V::Long(n) => {
                            let patched = pointers
                                .iter()
                                .find(|(from, t, _)| *from == i && t == tag)
                                .map(|(_, _, to)| starts[*to] as u32)
                                .unwrap_or(*n);
                            patched.to_be_bytes().to_vec()
                        }
                        V::Rationals(_) => unreachable!(),
                    };
                    inline.resize(4, 0);
                    out.extend_from_slice(&inline);
                } else {
                    out.extend_from_slice(&((data_start + data.len()) as u32).to_be_bytes());
                    data.extend_from_slice(&extra);
                }
            }
            out.extend_from_slice(&[0, 0, 0, 0]);
            out.extend_from_slice(&data);
        }
        out
    }

    fn out_of_line(value: &V) -> Vec<u8> {
        match value {
            V::Ascii(s) if s.len() + 1 > 4 => {
                let mut b = s.as_bytes().to_vec();
                b.push(0);
                // Keep every offset even, as TIFF asks.
                if b.len() % 2 == 1 {
                    b.push(0);
                }
                b
            }
            V::Rationals(r) => r.iter().flat_map(|(n, d)| [n.to_be_bytes(), d.to_be_bytes()]).flatten().collect(),
            _ => Vec::new(),
        }
    }

    fn tiff_fixture() -> Vec<u8> {
        tiff(
            vec![
                vec![
                    (0x010F, V::Ascii("FUJIFILM")),
                    (0x0110, V::Ascii("X-T5")),
                    (0x8769, V::Long(0)),
                    (0x8825, V::Long(0)),
                ],
                vec![
                    (0x829A, V::Rationals(vec![(1, 500)])),
                    (0x829D, V::Rationals(vec![(20, 10)])),
                    (0x8827, V::Short(160)),
                    (0x9003, V::Ascii("2026:09:12 18:42:07")),
                    (0x920A, V::Rationals(vec![(230, 10)])),
                    (0xA434, V::Ascii("XF23mmF2 R WR")),
                ],
                vec![
                    (0x0001, V::Ascii("N")),
                    (0x0002, V::Rationals(vec![(38, 1), (42, 1), (50, 1)])),
                    (0x0003, V::Ascii("W")),
                    (0x0004, V::Rationals(vec![(9, 1), (8, 1), (241, 1000)])),
                ],
            ],
            &[(0, 0x8769, 1), (0, 0x8825, 2)],
        )
    }

    fn tiff_with_zero_gps() -> Vec<u8> {
        tiff(
            vec![
                vec![(0x8825, V::Long(0))],
                vec![
                    (0x0001, V::Ascii("N")),
                    (0x0002, V::Rationals(vec![(0, 1), (0, 1), (0, 1)])),
                    (0x0003, V::Ascii("E")),
                    (0x0004, V::Rationals(vec![(0, 1), (0, 1), (0, 1)])),
                ],
            ],
            &[(0, 0x8825, 1)],
        )
    }
}
