//! The freedesktop thumbnail cache: `~/.cache/thumbnails`.
//!
//! One cache for the whole desktop, by specification. A thumbnail is a
//! PNG named by the MD5 of its source's URI, carrying that URI and the
//! source's modification time in two tEXt chunks, `Thumb::URI` and
//! `Thumb::MTime`. A lookup is current only while both still match — so
//! a file is thumbnailed once, and again only after it changes.
//!
//! # Why the shared cache rather than one of our own
//!
//! It is the smallest cache there can be. A picture some other file
//! manager or image viewer has already thumbnailed costs this one
//! nothing, and one this one makes costs them nothing — this machine had
//! 85 of them in `large/` before Files wrote a single one. A private
//! cache would store every one of those a second time.
//!
//! # As small as it can be
//!
//! - The smallest of the specification's sizes that covers what is
//!   drawn: `normal`, 128 pixels, for the file manager's listing at its
//!   usual size; `large`, 256, for the photo viewer's grid and a zoomed
//!   listing; `x-large`, 512, for the listing's Extra large icons on a
//!   scaled screen, where a 256-pixel picture would be drawn at more than
//!   twice its size. [`Size::for_edge`] picks it. Nothing bigger:
//!   `xx-large` is for sizes no window here draws.
//! - Encoded with the PNG encoder's strongest compression and adaptive
//!   filtering, and without an alpha channel when every pixel is opaque —
//!   which a photograph, a video frame and a PDF page all are, and which
//!   takes a quarter off before compression starts.
//! - A file that cannot be thumbnailed is recorded under `fail/` as the
//!   specification describes — a 1×1 image carrying the same two chunks —
//!   so a broken video is not handed to ffmpeg on every visit.
//! - [`Cache::prune`] removes thumbnails whose file is gone.
//!
//! # Other programs' thumbnailers
//!
//! [`thumbnailers`] reads the `*.thumbnailer` files other packages
//! install — glycin's for AVIF, HEIF and JPEG XL on this machine — and
//! runs one for a type nothing built in can read: bounded by
//! `hyprforge_process::TIMEOUT`, into a private temporary file, read
//! back through the same size cap as a thumbnail from the cache.
//!
//! What *not* to cache is the caller's decision, because it knows what a
//! thumbnail costs to make: a picture already small enough to decode
//! directly gains nothing from a second copy of itself.
//!
//! # The URI has to be GLib's, byte for byte
//!
//! The file name is a hash of the URI, so a URI escaped differently
//! finds nothing — and worse, writes a second thumbnail beside the one
//! another program made. [`file_uri`] escapes exactly the characters
//! `g_filename_to_uri` does, which was measured with `gio info` rather
//! than read from a specification: `;` is escaped, `!$&'()*+,=:@` are not.

use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use md5::{Digest, Md5};

/// The `normal` size's edge, by specification.
pub const NORMAL: u32 = 128;
/// The `large` size's edge, by specification.
pub const LARGE: u32 = 256;
/// The `x-large` size's edge, by specification.
pub const X_LARGE: u32 = 512;

pub mod thumbnailers;

/// Which of the specification's sizes a thumbnail is.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Size {
    Normal,
    Large,
    XLarge,
}

impl Size {
    /// Every size, smallest first.
    pub const ALL: [Size; 3] = [Size::Normal, Size::Large, Size::XLarge];

    /// The edge a thumbnail of this size fits inside.
    pub fn edge(self) -> u32 {
        match self {
            Size::Normal => NORMAL,
            Size::Large => LARGE,
            Size::XLarge => X_LARGE,
        }
    }

    /// The smallest size whose edge covers `edge` physical pixels — the
    /// largest there is when nothing does.
    ///
    /// Covering, not nearest: a 200-pixel cell drawn from a 128-pixel
    /// thumbnail is a blur, and one drawn from 256 is a picture scaled
    /// down, which is what every other thumbnail here already is.
    pub fn for_edge(edge: u32) -> Size {
        Size::ALL.into_iter().find(|s| s.edge() >= edge).unwrap_or(Size::XLarge)
    }

    fn dir(self) -> &'static str {
        match self {
            Size::Normal => "normal",
            Size::Large => "large",
            Size::XLarge => "x-large",
        }
    }
}

/// The directory `fail/` entries go under — the specification's
/// `<program>-<version>`, so a newer build retries what an older one
/// could not do.
const FAIL_DIR: &str = concat!("hyprforge-", env!("CARGO_PKG_VERSION"));

/// A thumbnail file larger than this is not one: a 128-pixel PNG is tens
/// of kilobytes at worst, and a 512-pixel photograph a few hundred —
/// 512 × 512 × 3 bytes is 768KB *before* compression. Refused before
/// decoding, so a planted file cannot make a lookup allocate.
const MAX_FILE: u64 = 4 * 1024 * 1024;

/// The widest a decoded thumbnail may be: twice `x-large`, the room a
/// thumbnailer is given to overshoot before it is scaled down. 4MB of
/// pixels at most.
pub const MAX_EDGE: u32 = 2 * X_LARGE;

/// Pixels, eight bits per channel, four channels.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Rgba {
    pub width: u32,
    pub height: u32,
    pub pixels: Vec<u8>,
}

/// What a source file is, for deciding whether a thumbnail is current.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Stamp {
    /// Whole seconds since the epoch — the resolution the specification
    /// records, so a sub-second change is not a change.
    pub mtime: u64,
    pub size: u64,
}

impl Stamp {
    pub fn of(path: &Path) -> io::Result<Stamp> {
        let meta = fs::metadata(path)?;
        let mtime = meta.modified()?.duration_since(UNIX_EPOCH).map_or(0, |d| d.as_secs());
        Ok(Stamp { mtime, size: meta.len() })
    }
}

/// What the cache knows about a file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Lookup {
    /// A thumbnail made from the file as it is now.
    Current(Rgba),
    /// This file, as it is now, could not be thumbnailed before.
    Failed,
    /// Nothing current — never made, or made before the file changed.
    Missing,
}

/// The cache directory.
#[derive(Debug, Clone)]
pub struct Cache {
    root: PathBuf,
}

impl Cache {
    /// `$XDG_CACHE_HOME/thumbnails`, or `~/.cache/thumbnails`. `None` with
    /// neither set, which leaves nowhere to put one.
    pub fn standard() -> Option<Cache> {
        let base = std::env::var_os("XDG_CACHE_HOME")
            .filter(|v| !v.is_empty())
            .map(PathBuf::from)
            .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".cache")))?;
        Some(Cache::at(base.join("thumbnails")))
    }

    /// A cache at `root` — the seam the tests use.
    pub fn at(root: PathBuf) -> Cache {
        Cache { root }
    }

    /// The current thumbnail of `source`, if there is one.
    ///
    /// `source` must be absolute, as the URI it is named by is.
    pub fn get(&self, source: &Path, stamp: Stamp) -> Lookup {
        self.get_sized(Size::Normal, source, stamp)
    }

    /// [`Cache::get`] at a given size. A failure is recorded once for
    /// every size — the file could not be read, whatever size was asked.
    pub fn get_sized(&self, size: Size, source: &Path, stamp: Stamp) -> Lookup {
        let name = thumbnail_name(source);
        if let Some(rgba) = read_if_current(&self.root.join(size.dir()).join(&name), source, stamp) {
            return Lookup::Current(rgba);
        }
        if read_if_current(&self.root.join("fail").join(FAIL_DIR).join(&name), source, stamp).is_some() {
            return Lookup::Failed;
        }
        Lookup::Missing
    }

    /// Stores `thumbnail` as `source`'s, replacing any older one.
    ///
    /// Written beside its final name and renamed over it, so another
    /// program reading the cache never sees half a file; owner-only, as
    /// the specification asks, since a thumbnail shows what the file
    /// holds.
    pub fn put(&self, source: &Path, stamp: Stamp, thumbnail: &Rgba) -> io::Result<()> {
        self.put_sized(Size::Normal, source, stamp, thumbnail)
    }

    /// [`Cache::put`] at a given size.
    pub fn put_sized(&self, size: Size, source: &Path, stamp: Stamp, thumbnail: &Rgba) -> io::Result<()> {
        self.write(&self.root.join(size.dir()), source, stamp, thumbnail)
    }

    /// Records that `source`, as it is now, could not be thumbnailed.
    pub fn put_failed(&self, source: &Path, stamp: Stamp) -> io::Result<()> {
        let one = Rgba { width: 1, height: 1, pixels: vec![0, 0, 0, 0] };
        self.write(&self.root.join("fail").join(FAIL_DIR), source, stamp, &one)
    }

    fn write(&self, dir: &Path, source: &Path, stamp: Stamp, thumbnail: &Rgba) -> io::Result<()> {
        create_private_dir(dir)?;
        let bytes = encode(thumbnail, &file_uri(source), stamp)?;
        let target = dir.join(thumbnail_name(source));
        let temp = dir.join(format!(".{}.{}.tmp", thumbnail_name(source), std::process::id()));
        write_private(&temp, &bytes)?;
        fs::rename(&temp, &target).inspect_err(|_| {
            let _ = fs::remove_file(&temp);
        })
    }

    /// Removes thumbnails whose file no longer exists, at most once per
    /// `every` — every program's, in every size, as the specification's
    /// section on deleting thumbnails recommends — and returns how many.
    ///
    /// "No longer exists" is decided cautiously: the file must be gone
    /// *and its folder still there*. A drive that is simply unplugged
    /// takes its folders with it, and its thumbnails are kept for when it
    /// comes back.
    pub fn prune(&self, every: Duration) -> usize {
        let marker = self.root.join(".hyprforge-pruned");
        let due = fs::metadata(&marker)
            .and_then(|m| m.modified())
            .ok()
            .and_then(|at| SystemTime::now().duration_since(at).ok())
            .is_none_or(|age| age >= every);
        if !due {
            return 0;
        }
        let mut removed = 0;
        let dirs = ["normal", "large", "x-large", "xx-large"]
            .iter()
            .map(|d| self.root.join(d))
            .chain(std::iter::once(self.root.join("fail").join(FAIL_DIR)));
        for dir in dirs {
            let Ok(entries) = fs::read_dir(&dir) else { continue };
            for entry in entries.flatten() {
                let path = entry.path();
                let Some(source) = source_of(&path) else { continue };
                let orphaned = !source.exists() && source.parent().is_some_and(Path::is_dir);
                if orphaned && fs::remove_file(&path).is_ok() {
                    removed += 1;
                }
            }
        }
        if fs::create_dir_all(&self.root).is_ok() {
            let _ = fs::write(&marker, b"");
        }
        removed
    }
}

/// `file://` and the path, escaped as GLib escapes it — see the module
/// doc for why that exactness matters.
pub fn file_uri(path: &Path) -> String {
    use std::os::unix::ffi::OsStrExt;
    let mut uri = String::from("file://");
    for &byte in path.as_os_str().as_bytes() {
        let keep = byte.is_ascii_alphanumeric() || b"-._~!$&'()*+,=:@/".contains(&byte);
        if keep {
            uri.push(byte as char);
        } else {
            uri.push_str(&format!("%{byte:02X}"));
        }
    }
    uri
}

/// The file name a thumbnail of `source` has: the MD5 of its URI, in
/// lowercase hex, then `.png`.
pub fn thumbnail_name(source: &Path) -> String {
    let digest = Md5::digest(file_uri(source).as_bytes());
    let hex: String = digest.iter().map(|b| format!("{b:02x}")).collect();
    format!("{hex}.png")
}

/// A `file://` URI back to its path. `None` for another scheme.
fn path_of_uri(uri: &str) -> Option<PathBuf> {
    use std::os::unix::ffi::OsStringExt;
    let rest = uri.strip_prefix("file://")?.as_bytes();
    let mut bytes = Vec::with_capacity(rest.len());
    let mut i = 0;
    while i < rest.len() {
        if rest[i] == b'%' && i + 2 < rest.len() {
            let hex = std::str::from_utf8(&rest[i + 1..i + 3]).ok()?;
            bytes.push(u8::from_str_radix(hex, 16).ok()?);
            i += 3;
        } else {
            bytes.push(rest[i]);
            i += 1;
        }
    }
    Some(PathBuf::from(std::ffi::OsString::from_vec(bytes)))
}

/// Which file a cached thumbnail is of, by its `Thumb::URI`.
fn source_of(thumbnail: &Path) -> Option<PathBuf> {
    let (_, text) = decode(thumbnail, false)?;
    path_of_uri(&chunk(&text, "Thumb::URI")?)
}

fn chunk(text: &[(String, String)], key: &str) -> Option<String> {
    text.iter().find(|(k, _)| k == key).map(|(_, v)| v.clone())
}

/// The pixels at `path`, if it is a thumbnail of `source` as it is now.
fn read_if_current(path: &Path, source: &Path, stamp: Stamp) -> Option<Rgba> {
    let (rgba, text) = decode(path, true)?;
    let current = chunk(&text, "Thumb::URI")? == file_uri(source)
        && chunk(&text, "Thumb::MTime")?.parse::<u64>().ok()? == stamp.mtime
        // Optional by specification; checked when present, because two
        // saves within one second leave the mtime alone and change this.
        && chunk(&text, "Thumb::Size").is_none_or(|s| s.parse::<u64>().ok() == Some(stamp.size));
    current.then_some(rgba?)
}

/// A PNG's tEXt chunks, keyword and text.
type TextChunks = Vec<(String, String)>;

/// Reads a PNG's text chunks and, when `pixels`, its pixels as RGBA.
fn decode(path: &Path, pixels: bool) -> Option<(Option<Rgba>, TextChunks)> {
    let file = fs::File::open(path).ok()?;
    if file.metadata().ok()?.len() > MAX_FILE {
        return None;
    }
    let mut decoder = png::Decoder::new(io::BufReader::new(file));
    decoder.set_transformations(png::Transformations::normalize_to_color8());
    let reader = decoder.read_info().ok()?;
    let text = reader
        .info()
        .uncompressed_latin1_text
        .iter()
        .map(|c| (c.keyword.clone(), c.text.clone()))
        .collect();
    if !pixels {
        return Some((None, text));
    }
    Some((Some(pixels_of(reader)?), text))
}

/// A PNG's pixels as RGBA, refusing anything bigger than a thumbnail
/// could honestly be — before the buffer for it is allocated.
fn pixels_of<R: io::BufRead + io::Seek>(mut reader: png::Reader<R>) -> Option<Rgba> {
    let (width, height) = (reader.info().width, reader.info().height);
    if width > MAX_EDGE || height > MAX_EDGE {
        return None;
    }
    let mut buffer = vec![0; reader.output_buffer_size()?];
    let frame = reader.next_frame(&mut buffer).ok()?;
    let data = &buffer[..frame.buffer_size()];
    // `normalize_to_color8` has already expanded palettes and cut 16-bit
    // channels to 8, so these four are every shape left.
    let pixels = match frame.color_type {
        png::ColorType::Rgba => data.to_vec(),
        png::ColorType::Rgb => data.chunks_exact(3).flat_map(|p| [p[0], p[1], p[2], 255]).collect(),
        png::ColorType::GrayscaleAlpha => data.chunks_exact(2).flat_map(|p| [p[0], p[0], p[0], p[1]]).collect(),
        png::ColorType::Grayscale => data.iter().flat_map(|&g| [g, g, g, 255]).collect(),
        png::ColorType::Indexed => return None,
    };
    Some(Rgba { width, height, pixels })
}

/// PNG bytes for `thumbnail`, with its two chunks and `Thumb::Size`.
fn encode(thumbnail: &Rgba, uri: &str, stamp: Stamp) -> io::Result<Vec<u8>> {
    let opaque = thumbnail.pixels.chunks_exact(4).all(|p| p[3] == 255);
    let mut out = Vec::new();
    {
        let mut encoder = png::Encoder::new(&mut out, thumbnail.width, thumbnail.height);
        encoder.set_depth(png::BitDepth::Eight);
        encoder.set_color(if opaque { png::ColorType::Rgb } else { png::ColorType::Rgba });
        encoder.set_compression(png::Compression::High);
        encoder.set_filter(png::Filter::Adaptive);
        let text = |e: png::EncodingError| io::Error::other(e.to_string());
        encoder.add_text_chunk("Thumb::URI".into(), uri.into()).map_err(text)?;
        encoder.add_text_chunk("Thumb::MTime".into(), stamp.mtime.to_string()).map_err(text)?;
        encoder.add_text_chunk("Thumb::Size".into(), stamp.size.to_string()).map_err(text)?;
        let mut writer = encoder.write_header().map_err(text)?;
        if opaque {
            let rgb: Vec<u8> = thumbnail.pixels.chunks_exact(4).flat_map(|p| [p[0], p[1], p[2]]).collect();
            writer.write_image_data(&rgb).map_err(text)?;
        } else {
            writer.write_image_data(&thumbnail.pixels).map_err(text)?;
        }
    }
    Ok(out)
}

/// Decodes PNG bytes another program printed — a PDF page from
/// `pdftoppm`, a frame from `ffmpeg` — into pixels to cache.
pub fn decode_png(bytes: &[u8]) -> Option<Rgba> {
    let mut decoder = png::Decoder::new(io::Cursor::new(bytes));
    decoder.set_transformations(png::Transformations::normalize_to_color8());
    pixels_of(decoder.read_info().ok()?)
}

/// `rgba` scaled down to fit inside an `edge`-pixel square, each new
/// pixel the average of the old ones it covers; unchanged when it
/// already fits. For a picture another program made bigger than it was
/// asked to — a thumbnail stored under `normal/` has to be one.
pub fn fit(rgba: Rgba, edge: u32) -> Rgba {
    let edge = edge.max(1);
    if rgba.width <= edge && rgba.height <= edge {
        return rgba;
    }
    let ratio = edge as f64 / rgba.width.max(rgba.height) as f64;
    let width = ((rgba.width as f64 * ratio).round() as u32).clamp(1, edge);
    let height = ((rgba.height as f64 * ratio).round() as u32).clamp(1, edge);
    let mut pixels = Vec::with_capacity(width as usize * height as usize * 4);
    for y in 0..height {
        let (y0, y1) = span(y, height, rgba.height);
        for x in 0..width {
            let (x0, x1) = span(x, width, rgba.width);
            let mut sum = [0u64; 4];
            for sy in y0..y1 {
                let row = sy as usize * rgba.width as usize;
                for sx in x0..x1 {
                    let at = (row + sx as usize) * 4;
                    for (c, total) in sum.iter_mut().enumerate() {
                        *total += rgba.pixels[at + c] as u64;
                    }
                }
            }
            let count = ((y1 - y0) * (x1 - x0)).max(1) as u64;
            pixels.extend(sum.iter().map(|total| (total / count) as u8));
        }
    }
    Rgba { width, height, pixels }
}

/// The source rows (or columns) that destination row `i` of `to` covers
/// out of `from` — never empty.
fn span(i: u32, to: u32, from: u32) -> (u32, u32) {
    let start = (i as u64 * from as u64 / to as u64) as u32;
    let end = (((i as u64 + 1) * from as u64).div_ceil(to as u64) as u32).clamp(start + 1, from);
    (start, end)
}

fn create_private_dir(dir: &Path) -> io::Result<()> {
    use std::os::unix::fs::DirBuilderExt;
    fs::DirBuilder::new().recursive(true).mode(0o700).create(dir)
}

fn write_private(path: &Path, bytes: &[u8]) -> io::Result<()> {
    use std::io::Write;
    use std::os::unix::fs::OpenOptionsExt;
    let mut file = fs::OpenOptions::new().write(true).create(true).truncate(true).mode(0o600).open(path)?;
    file.write_all(bytes)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn picture(opaque: bool) -> Rgba {
        let pixels = (0..16 * 16).flat_map(|i| [i as u8, 0, 255 - i as u8, if opaque { 255 } else { 128 }]).collect();
        Rgba { width: 16, height: 16, pixels }
    }

    fn source(dir: &Path, name: &str) -> (PathBuf, Stamp) {
        let path = dir.join(name);
        fs::write(&path, b"contents").unwrap();
        let stamp = Stamp::of(&path).unwrap();
        (path, stamp)
    }

    /// The two sizes live side by side under the specification's own
    /// directories: a `large` thumbnail is not a `normal` one, and a
    /// program reading `normal/` must not find the viewer's 256-pixel
    /// tile there.
    #[test]
    fn each_size_is_stored_in_its_own_directory() {
        let dir = tempfile::tempdir().unwrap();
        let cache = Cache::at(dir.path().join("thumbnails"));
        let (path, stamp) = source(dir.path(), "clip.mp4");
        cache.put_sized(Size::Large, &path, stamp, &picture(true)).unwrap();
        assert!(dir.path().join("thumbnails/large").join(thumbnail_name(&path)).exists());
        assert!(matches!(cache.get_sized(Size::Large, &path, stamp), Lookup::Current(_)));
        assert_eq!(cache.get(&path, stamp), Lookup::Missing, "a large thumbnail answered for normal");
        assert_eq!((Size::Normal.edge(), Size::Large.edge(), Size::XLarge.edge()), (128, 256, 512));
        cache.put_sized(Size::XLarge, &path, stamp, &picture(true)).unwrap();
        assert!(dir.path().join("thumbnails/x-large").join(thumbnail_name(&path)).exists());
    }

    /// The bucket covers what is drawn, and is never bigger than it has
    /// to be: a 200-pixel cell takes `large`, not `x-large`.
    #[test]
    fn the_size_for_an_edge_is_the_smallest_that_covers_it() {
        assert_eq!(Size::for_edge(1), Size::Normal);
        assert_eq!(Size::for_edge(128), Size::Normal);
        assert_eq!(Size::for_edge(129), Size::Large);
        assert_eq!(Size::for_edge(256), Size::Large);
        assert_eq!(Size::for_edge(300), Size::XLarge);
        assert_eq!(Size::for_edge(4000), Size::XLarge, "the largest there is");
    }

    /// A 512-pixel photograph — noise, the worst case for PNG — is stored
    /// and read back: the file cap has to leave room for the size it
    /// stores.
    #[test]
    fn an_x_large_thumbnail_of_noise_still_fits_the_file_cap() {
        let dir = tempfile::tempdir().unwrap();
        let cache = Cache::at(dir.path().join("thumbnails"));
        let (path, stamp) = source(dir.path(), "noise.jpg");
        let mut seed = 0x2545_f491_u32;
        let pixels = (0..512 * 512)
            .flat_map(|_| {
                seed ^= seed << 13;
                seed ^= seed >> 17;
                seed ^= seed << 5;
                let b = seed.to_le_bytes();
                [b[0], b[1], b[2], 255]
            })
            .collect();
        let noise = Rgba { width: 512, height: 512, pixels };
        cache.put_sized(Size::XLarge, &path, stamp, &noise).unwrap();
        let stored = dir.path().join("thumbnails/x-large").join(thumbnail_name(&path));
        let bytes = fs::metadata(&stored).unwrap().len();
        assert!(bytes <= MAX_FILE, "{bytes} bytes");
        assert_eq!(cache.get_sized(Size::XLarge, &path, stamp), Lookup::Current(noise));
    }

    /// Scaled down to fit, proportions kept, averaging rather than
    /// picking; something already small enough is left alone.
    #[test]
    fn fitting_scales_down_to_the_edge_and_keeps_the_shape() {
        let wide = Rgba { width: 600, height: 300, pixels: [200, 100, 50, 255].repeat(600 * 300) };
        let fitted = fit(wide, 256);
        assert_eq!((fitted.width, fitted.height), (256, 128));
        assert_eq!(fitted.pixels.len(), 256 * 128 * 4);
        assert_eq!(&fitted.pixels[..4], &[200, 100, 50, 255], "a flat colour stays that colour");
        assert_eq!(fit(picture(true), 256), picture(true));
    }

    /// A file that could not be read fails at every size.
    #[test]
    fn a_failure_holds_for_every_size() {
        let dir = tempfile::tempdir().unwrap();
        let cache = Cache::at(dir.path().join("thumbnails"));
        let (path, stamp) = source(dir.path(), "broken.mp4");
        cache.put_failed(&path, stamp).unwrap();
        assert_eq!(cache.get_sized(Size::Large, &path, stamp), Lookup::Failed);
        assert_eq!(cache.get(&path, stamp), Lookup::Failed);
    }

    /// Each of these is what `gio info` printed for the same name on this
    /// machine. A URI escaped any other way hashes to a different file
    /// and never finds a thumbnail another program made.
    #[test]
    fn a_uri_is_escaped_exactly_as_glib_escapes_it() {
        let cases = [
            ("/d/a b.png", "file:///d/a%20b.png"),
            ("/d/x(1)!$&'*+,;=:@.png", "file:///d/x(1)!$&'*+,%3B=:@.png"),
            ("/d/q?#%[]{}.png", "file:///d/q%3F%23%25%5B%5D%7B%7D.png"),
            ("/d/ü é.png", "file:///d/%C3%BC%20%C3%A9.png"),
            ("/d/tab\tx.png", "file:///d/tab%09x.png"),
            ("/d/\"quote\"<>|^`.png", "file:///d/%22quote%22%3C%3E%7C%5E%60.png"),
            ("/d/back\\\\slash~_-.png", "file:///d/back%5C%5Cslash~_-.png"),
        ];
        for (path, uri) in cases {
            assert_eq!(file_uri(Path::new(path)), uri);
            assert_eq!(path_of_uri(uri).as_deref(), Some(Path::new(path)), "and back");
        }
    }

    /// A known name from the specification's own example.
    #[test]
    fn a_thumbnail_is_named_by_the_md5_of_its_uri() {
        // md5("file:///home/jens/photos/me.png"), from the thumbnail
        // specification's worked example.
        assert_eq!(thumbnail_name(Path::new("/home/jens/photos/me.png")), "c6ee772d9e49320e97ec29a7eb5b1697.png");
    }

    #[test]
    fn a_stored_thumbnail_is_found_while_the_file_is_unchanged() {
        let dir = tempfile::tempdir().unwrap();
        let cache = Cache::at(dir.path().join("thumbnails"));
        let (path, stamp) = source(dir.path(), "a.png");
        assert_eq!(cache.get(&path, stamp), Lookup::Missing);
        cache.put(&path, stamp, &picture(false)).unwrap();
        assert_eq!(cache.get(&path, stamp), Lookup::Current(picture(false)));
    }

    /// The whole point: a changed file is a new thumbnail.
    #[test]
    fn a_changed_file_is_not_given_its_old_thumbnail() {
        let dir = tempfile::tempdir().unwrap();
        let cache = Cache::at(dir.path().join("thumbnails"));
        let (path, stamp) = source(dir.path(), "a.png");
        cache.put(&path, stamp, &picture(true)).unwrap();
        assert_eq!(cache.get(&path, Stamp { mtime: stamp.mtime + 1, ..stamp }), Lookup::Missing, "newer");
        assert_eq!(cache.get(&path, Stamp { size: stamp.size + 1, ..stamp }), Lookup::Missing, "same second, new size");
    }

    /// Opaque pictures are stored without an alpha channel — a quarter of
    /// the pixels gone before compression — and come back the same.
    #[test]
    fn an_opaque_thumbnail_is_stored_without_alpha_and_reads_back_the_same() {
        let dir = tempfile::tempdir().unwrap();
        let cache = Cache::at(dir.path().join("thumbnails"));
        let (path, stamp) = source(dir.path(), "a.jpg");
        cache.put(&path, stamp, &picture(true)).unwrap();
        let stored = dir.path().join("thumbnails/normal").join(thumbnail_name(&path));
        let decoder = png::Decoder::new(io::BufReader::new(fs::File::open(&stored).unwrap()));
        assert_eq!(decoder.read_info().unwrap().info().color_type, png::ColorType::Rgb);
        assert_eq!(cache.get(&path, stamp), Lookup::Current(picture(true)));
    }

    /// Owner-only, as the specification asks — a thumbnail shows what the
    /// file holds.
    #[test]
    fn thumbnails_are_readable_only_by_their_owner() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        let cache = Cache::at(dir.path().join("thumbnails"));
        let (path, stamp) = source(dir.path(), "a.png");
        cache.put(&path, stamp, &picture(true)).unwrap();
        let stored = dir.path().join("thumbnails/normal").join(thumbnail_name(&path));
        assert_eq!(fs::metadata(&stored).unwrap().permissions().mode() & 0o777, 0o600);
        let normal = dir.path().join("thumbnails/normal");
        assert_eq!(fs::metadata(normal).unwrap().permissions().mode() & 0o777, 0o700);
    }

    /// A failure is remembered for the file as it is, and forgotten when
    /// it changes — a re-encoded video gets another try.
    #[test]
    fn a_failure_is_remembered_until_the_file_changes() {
        let dir = tempfile::tempdir().unwrap();
        let cache = Cache::at(dir.path().join("thumbnails"));
        let (path, stamp) = source(dir.path(), "broken.mp4");
        cache.put_failed(&path, stamp).unwrap();
        assert_eq!(cache.get(&path, stamp), Lookup::Failed);
        assert_eq!(cache.get(&path, Stamp { mtime: stamp.mtime + 5, ..stamp }), Lookup::Missing);
    }

    /// A file with the right name but somebody else's URI — a collision,
    /// or a planted file — is not this file's thumbnail.
    #[test]
    fn a_thumbnail_of_another_file_is_never_returned() {
        let dir = tempfile::tempdir().unwrap();
        let cache = Cache::at(dir.path().join("thumbnails"));
        let (a, stamp) = source(dir.path(), "a.png");
        let (b, _) = source(dir.path(), "b.png");
        cache.put(&b, stamp, &picture(true)).unwrap();
        let normal = dir.path().join("thumbnails/normal");
        fs::rename(normal.join(thumbnail_name(&b)), normal.join(thumbnail_name(&a))).unwrap();
        assert_eq!(cache.get(&a, stamp), Lookup::Missing);
    }

    /// Pruning removes what belongs to a deleted file, keeps what belongs
    /// to a file on a folder that is not there (an unplugged drive), and
    /// runs at most once per interval.
    #[test]
    fn pruning_removes_deleted_files_thumbnails_and_keeps_unplugged_ones() {
        let dir = tempfile::tempdir().unwrap();
        let cache = Cache::at(dir.path().join("thumbnails"));
        let (kept, stamp) = source(dir.path(), "kept.png");
        let (deleted, _) = source(dir.path(), "deleted.png");
        let unplugged = dir.path().join("drive-not-mounted/photo.png");
        for path in [&kept, &deleted, &unplugged] {
            cache.put(path, stamp, &picture(true)).unwrap();
        }
        fs::remove_file(&deleted).unwrap();

        assert_eq!(cache.prune(Duration::from_secs(3600)), 1);
        assert_eq!(cache.get(&kept, stamp), Lookup::Current(picture(true)));
        assert_eq!(cache.get(&unplugged, stamp), Lookup::Current(picture(true)), "its folder is gone, not the file");

        fs::remove_file(&kept).unwrap();
        assert_eq!(cache.prune(Duration::from_secs(3600)), 0, "not due again yet");
    }

    /// A PNG from ffmpeg or pdftoppm decodes; one claiming to be vast is
    /// refused before its buffer is allocated.
    #[test]
    fn a_tools_png_decodes_and_an_oversized_one_is_refused() {
        let small = encode(&picture(true), "file:///x", Stamp { mtime: 0, size: 0 }).unwrap();
        assert_eq!(decode_png(&small), Some(picture(true)));
        let big = Rgba { width: MAX_EDGE + 1, height: 1, pixels: vec![255; (MAX_EDGE as usize + 1) * 4] };
        let big = encode(&big, "file:///x", Stamp { mtime: 0, size: 0 }).unwrap();
        assert_eq!(decode_png(&big), None);
    }
}
