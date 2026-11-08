//! What the preview pane shows for the one selected entry.
//!
//! Built by the host, never here: every part of a [`Preview`] is read
//! from disk or from another program — the first lines of a text file,
//! a PDF's first page, a video's frame, a folder's names — and the
//! browser does no I/O (see its module doc). It asks with
//! `Outcome::LoadPreview` and draws whatever comes back.
//!
//! One shape for every kind of file, rather than an enum of kinds,
//! because the kinds overlap: a song has a cover *and* an artist, a video
//! a frame *and* a duration. Each part is simply there or not, and the
//! pane draws what is there in one fixed order — picture, excerpt,
//! listing, details — so two previews of different things still read as
//! the same pane.

use std::path::Path;

use iced::{Element, Length};

/// Everything known about one entry beyond what its listing row says.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Preview {
    /// A picture of it: the image itself, a document's first page, a
    /// video's frame, a song's cover.
    pub picture: Option<Picture>,
    /// The start of a text file, as it is — drawn in the monospace face.
    pub text: Option<Excerpt>,
    /// What is inside a folder or an archive.
    pub listing: Option<Listing>,
    /// Rows beyond Kind, Size and Modified, which the pane always has:
    /// "Pages", "Duration", "Dimensions", "Artist".
    pub details: Vec<(String, String)>,
}

impl Preview {
    /// Whether there is anything here the listing does not already say.
    pub fn is_empty(&self) -> bool {
        self.picture.is_none() && self.text.is_none() && self.listing.is_none() && self.details.is_empty()
    }
}

/// The first part of a text file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Excerpt {
    pub text: String,
    /// Whether the file goes on past what is shown, so the pane can say
    /// so rather than implying the file ends there.
    pub truncated: bool,
}

/// The first few names inside a folder or archive, and how many there
/// are in all.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Listing {
    /// `(name, is_folder)`, folders first, then by name.
    pub names: Vec<(String, bool)>,
    pub total: usize,
}

/// Where a thumbnail of a file comes from — what it costs, and which
/// `[thumbnails]` switch governs it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Source {
    /// A picture this build decodes itself (`hyprforge-image`).
    Picture,
    /// An SVG, which iced draws from the file at any size.
    Svg,
    /// A PDF's first page, by poppler's `pdftoppm`.
    Pdf,
    /// A video's first real frame, by `ffmpeg`.
    Video,
    /// An STL, 3MF or OBJ model, loaded and drawn by `hyprforge-mesh`.
    Model,
    /// Anything else a `*.thumbnailer` file installed on this machine
    /// claims — somebody else's program.
    System,
}

impl Source {
    /// Only in the grid. Everything but a picture runs a program or
    /// loads a whole model per file — a video's frame is about half a
    /// second of `ffmpeg` — which is worth it for a cell big enough to
    /// show a page or a scene, and not for a list row's twenty-pixel icon.
    pub fn grid_only(self) -> bool {
        !matches!(self, Source::Picture | Source::Svg)
    }

    /// The built-in reader for a MIME type, if there is one. Pictures and
    /// models are not here: the decoder and the mesh loader each decide
    /// by their own format lists, which is what [`ThumbnailTypes`] asks.
    pub fn for_mime(mime: &str) -> Option<Source> {
        match mime {
            "image/svg+xml" | "image/svg+xml-compressed" => Some(Source::Svg),
            "application/pdf" => Some(Source::Pdf),
            _ if mime.starts_with("video/") => Some(Source::Video),
            _ => None,
        }
    }
}

/// What a file's thumbnail would come from, by its name — the host's
/// answer, from the MIME database and the thumbnailers installed.
///
/// The browser does no I/O and loads no MIME database (see its module
/// doc), so it cannot answer this itself; and it is a fact about the
/// machine rather than about any one tab, so the host installs it once
/// with [`install_thumbnail_types`] and every browser — every tab, every
/// pane, the open/save dialog — asks the same one. Asked of every row on
/// every listing, so it must decide by name: a glob lookup, never a read.
#[derive(Clone)]
pub struct ThumbnailTypes(std::sync::Arc<SourceOf>);

/// The question [`ThumbnailTypes`] answers.
type SourceOf = dyn Fn(&Path) -> Option<Source> + Send + Sync;

impl ThumbnailTypes {
    pub fn new(of: impl Fn(&Path) -> Option<Source> + Send + Sync + 'static) -> ThumbnailTypes {
        ThumbnailTypes(std::sync::Arc::new(of))
    }

    pub fn of(&self, path: &Path) -> Option<Source> {
        (self.0)(path)
    }
}

static TYPES: std::sync::OnceLock<ThumbnailTypes> = std::sync::OnceLock::new();

/// Makes `types` the answer for this process. The first call wins; a
/// host calls it once, before its first window.
pub fn install_thumbnail_types(types: ThumbnailTypes) {
    let _ = TYPES.set(types);
}

/// Where `path`'s thumbnail would come from: the installed answer, or —
/// before a host installed one, and in this crate's own tests — by
/// extension, which is what this always did.
pub fn source_of(path: &Path) -> Option<Source> {
    match TYPES.get() {
        Some(types) => types.of(path),
        None => by_extension(path),
    }
}

/// The answer without a MIME database: pictures by the decoder's own
/// list, and the common SVG, PDF, video and model extensions.
pub fn by_extension(path: &Path) -> Option<Source> {
    if hyprforge_image::format::looks_decodable(path) {
        return Some(Source::Picture);
    }
    let ext = path.extension().and_then(|e| e.to_str()).map(str::to_ascii_lowercase)?;
    match ext.as_str() {
        "svg" | "svgz" => Some(Source::Svg),
        "pdf" => Some(Source::Pdf),
        "mp4" | "mkv" | "webm" | "mov" | "avi" | "m4v" | "wmv" | "mpg" | "mpeg" | "ogv" => Some(Source::Video),
        "stl" | "3mf" | "obj" => Some(Source::Model),
        _ => None,
    }
}

/// Whether an entry is worth asking the host for a thumbnail of: it has
/// a source, that source is switched on in `[thumbnails]`, and — for one
/// that runs a program — the grid is showing. See [`Source::grid_only`].
///
/// By name, like the picture check, so it can be wrong about a misnamed
/// file; the host then answers nothing and the icon stays. The size cap
/// is the host's to apply: it is the one that reads the file's size.
pub fn wants_thumbnail(path: &Path, grid: bool, allowed: &crate::config::Thumbnails) -> bool {
    wants(source_of(path), grid, allowed)
}

/// [`wants_thumbnail`] for a source already found.
pub fn wants(source: Option<Source>, grid: bool, allowed: &crate::config::Thumbnails) -> bool {
    source.is_some_and(|s| allowed.allows(s) && (grid || !s.grid_only()))
}

/// The thumbnail sizes there are, in physical pixels, smallest first —
/// the freedesktop cache's `normal`, `large` and `x-large`.
/// `hyprforge-thumbnails` has the same list; the host's tests hold the
/// two to each other.
pub const BUCKETS: [u32; 3] = [128, 256, 512];

/// The smallest bucket that covers `edge` physical pixels, or the
/// largest when none does — so a thumbnail is never drawn bigger than it
/// was made unless nothing bigger can be made.
pub fn bucket_for(edge: f32) -> u32 {
    BUCKETS.into_iter().find(|&b| b as f32 >= edge).unwrap_or(BUCKETS[BUCKETS.len() - 1])
}

/// Something to draw in a box: decoded pixels, or an SVG iced renders at
/// whatever size it is given.
///
/// Also what a theme icon and a grid thumbnail are — one type, so every
/// picture in the listing is drawn by the same code.
#[derive(Debug, Clone, PartialEq)]
pub enum Picture {
    Svg(iced::widget::svg::Handle),
    Raster(iced::widget::image::Handle),
}

/// The most of an SVG read to draw it as a thumbnail or a preview. A
/// drawing bigger than this is not a drawing anyone wants rasterised on
/// the thread that paints the window, which is where iced parses one.
pub const SVG_BYTES: u64 = 8 * 1024 * 1024;

/// The only `data:` pictures an SVG drawn here may embed. Not
/// `image/svg+xml`, nor anything else: usvg parses an embedded SVG as a
/// drawing of its own, and that one's `href`s are hidden from
/// [`svg_is_self_contained`] inside base64.
const EMBEDDABLE: [&str; 5] = ["data:image/png", "data:image/jpeg", "data:image/jpg", "data:image/gif", "data:image/webp"];

/// Whether an SVG can be drawn without reading any file but itself.
///
/// usvg, which iced draws SVGs with, resolves an `<image href>` that is
/// a path by reading that path — `/dev/zero` freezes the window and
/// runs it out of memory, a FIFO hangs it, and any picture this user can
/// read could be drawn into a file someone sent them. iced gives no way
/// to pass usvg a resolver that refuses, and rasterising here instead
/// would need usvg as a dependency of this crate, so this refuses the
/// whole drawing whenever it names anything outside itself. Crude on
/// purpose — a text scan, erring towards refusing — and the cost of a
/// refusal is an icon where a thumbnail would have been.
///
/// Refused, besides any `href` that is neither `#fragment` nor one of
/// `EMBEDDABLE`:
/// - gzip (`.svgz`): the text cannot be scanned without unpacking it;
/// - any `<!ENTITY`: an entity's text is expanded where it is used, and
///   character references inside it can spell out an `href` this scan
///   would never see. Some editors declare a few harmless ones; those
///   drawings keep their icon.
pub fn svg_is_self_contained(svg: &[u8]) -> bool {
    if svg.starts_with(&[0x1f, 0x8b]) {
        return false;
    }
    let text = String::from_utf8_lossy(svg).to_ascii_lowercase();
    if text.contains("<!entity") {
        return false;
    }
    let mut rest = text.as_str();
    while let Some(at) = rest.find("href") {
        rest = &rest[at + "href".len()..];
        // Not an attribute: the word in a text node, or in a longer name.
        let Some(value) = rest.trim_start().strip_prefix('=') else { continue };
        let value = value.trim_start();
        let Some(value) = value.strip_prefix('"').or_else(|| value.strip_prefix('\'')) else { continue };
        let value = value.trim_start();
        if !(value.starts_with('#') || EMBEDDABLE.iter().any(|kind| value.starts_with(kind))) {
            return false;
        }
    }
    true
}

impl Picture {
    /// An SVG from a file nobody vouches for — anything in a listing, a
    /// preview, an icon a user picked — or `None` to keep the icon.
    ///
    /// Read here, once, and handed to iced as bytes, never as a path:
    /// iced reads a path itself when it first draws it, on the thread
    /// that paints, with no size cap and whatever the file has become by
    /// then. Only a regular file is opened (a FIFO named `x.svg` would
    /// block the read for good), only up to `max_bytes`, and only one
    /// [`svg_is_self_contained`] accepts is drawn.
    pub fn untrusted_svg(path: &Path, max_bytes: u64) -> Option<Picture> {
        use std::io::Read;
        let meta = std::fs::metadata(path).ok()?;
        if !meta.is_file() || meta.len() > max_bytes {
            return None;
        }
        let mut bytes = Vec::new();
        std::fs::File::open(path).ok()?.take(max_bytes.saturating_add(1)).read_to_end(&mut bytes).ok()?;
        if bytes.len() as u64 > max_bytes || !svg_is_self_contained(&bytes) {
            return None;
        }
        Some(Picture::Svg(iced::widget::svg::Handle::from_memory(bytes)))
    }

    /// By extension: for a theme icon, which `hyprforge-icons` only ever
    /// answers with as a `.png` or an `.svg`. iced reads the file itself
    /// the first time it draws it, so this is for files the system
    /// installed — anything a user could have been sent goes through
    /// [`Self::untrusted_svg`].
    pub fn from_path(path: &Path) -> Picture {
        if path.extension().is_some_and(|e| e.eq_ignore_ascii_case("svg")) {
            Picture::Svg(iced::widget::svg::Handle::from_path(path))
        } else {
            Picture::Raster(iced::widget::image::Handle::from_path(path))
        }
    }

    /// Drawn in a `side`-pixel square, logical, already scaled, keeping
    /// its own proportions inside it.
    pub fn view<'a, Message: 'a>(&self, side: f32) -> Element<'a, Message> {
        self.view_in(side, side)
    }

    /// Drawn `width` wide, as tall as its own proportions make it — the
    /// preview pane's way, so a landscape photograph does not sit in a
    /// square with a band of nothing under it.
    pub fn view_width<'a, Message: 'a>(&self, width: f32) -> Element<'a, Message> {
        match self {
            Picture::Svg(handle) => iced::widget::svg(handle.clone()).width(Length::Fixed(width)).into(),
            Picture::Raster(handle) => iced::widget::image(handle.clone()).width(Length::Fixed(width)).into(),
        }
    }

    /// Drawn as large as the space it is given, keeping its proportions —
    /// and never larger than itself: `ScaleDown`, not `Contain`, so a
    /// 64-pixel icon in Quick Look is a 64-pixel icon rather than a
    /// smear of invented pixels filling the window. An SVG has no pixels
    /// to invent and is drawn to fit, whatever size it declares.
    pub fn view_fill<'a, Message: 'a>(&self) -> Element<'a, Message> {
        match self {
            Picture::Svg(handle) => iced::widget::svg(handle.clone())
                .width(Length::Fill)
                .height(Length::Fill)
                .content_fit(iced::ContentFit::Contain)
                .into(),
            Picture::Raster(handle) => iced::widget::image(handle.clone())
                .width(Length::Fill)
                .height(Length::Fill)
                .content_fit(iced::ContentFit::ScaleDown)
                .into(),
        }
    }

    /// Drawn within `width` by `height`, keeping its proportions.
    pub fn view_in<'a, Message: 'a>(&self, width: f32, height: f32) -> Element<'a, Message> {
        match self {
            Picture::Svg(handle) => iced::widget::svg(handle.clone())
                .width(Length::Fixed(width))
                .height(Length::Fixed(height))
                .content_fit(iced::ContentFit::Contain)
                .into(),
            Picture::Raster(handle) => iced::widget::image(handle.clone())
                .width(Length::Fixed(width))
                .height(Length::Fixed(height))
                .content_fit(iced::ContentFit::Contain)
                .into(),
        }
    }
}

#[cfg(test)]
mod tests {
    // --- SVGs from files nobody vouches for (D2) ------------------------

    #[test]
    fn an_svg_that_draws_only_itself_is_accepted() {
        let svg = br##"<svg xmlns="http://www.w3.org/2000/svg" xmlns:xlink="http://www.w3.org/1999/xlink">
            <defs><linearGradient id="g"/></defs>
            <use xlink:href="#g"/><image href = 'data:image/png;base64,AAAA'/>
            <text>an href in a sentence</text></svg>"##;
        assert!(super::svg_is_self_contained(svg));
    }

    /// usvg reads whatever path an `<image href>` names: `/dev/zero`
    /// froze the window and ran it out of memory.
    #[test]
    fn an_svg_that_reaches_for_a_file_is_refused() {
        for href in ["/dev/zero", "file:///etc/passwd", "../secret.png", " /dev/zero", "&x;", "&#47;dev/zero"] {
            let svg = format!(r#"<svg xmlns="http://www.w3.org/2000/svg"><image href="{href}"/></svg>"#);
            assert!(!super::svg_is_self_contained(svg.as_bytes()), "{href}");
        }
        let xlink = br#"<svg><image xlink:href='/dev/zero'/></svg>"#;
        assert!(!super::svg_is_self_contained(xlink));
    }

    /// An embedded SVG is parsed as a drawing of its own, with its own
    /// `href`s hidden in base64; an entity can spell one out with
    /// character references; a gzipped one cannot be read at all.
    #[test]
    fn an_svg_that_could_hide_a_reach_is_refused() {
        let nested = br#"<svg><image href="data:image/svg+xml;base64,PHN2Zz4="/></svg>"#;
        assert!(!super::svg_is_self_contained(nested));
        let entity = br#"<!DOCTYPE svg [<!ENTITY i "<image hr&#101;f='/dev/zero'/>">]><svg>&i;</svg>"#;
        assert!(!super::svg_is_self_contained(entity));
        assert!(!super::svg_is_self_contained(&[0x1f, 0x8b, 8, 0]));
    }

    #[test]
    fn an_untrusted_svg_over_the_cap_or_not_a_file_is_not_read() {
        let dir = tempfile::tempdir().unwrap();
        let fine = dir.path().join("fine.svg");
        std::fs::write(&fine, br#"<svg xmlns="http://www.w3.org/2000/svg"/>"#).unwrap();
        assert!(matches!(super::Picture::untrusted_svg(&fine, 1024), Some(super::Picture::Svg(_))));
        assert!(super::Picture::untrusted_svg(&fine, 8).is_none(), "over the cap");
        assert!(super::Picture::untrusted_svg(dir.path(), 1024).is_none(), "a folder");
        assert!(super::Picture::untrusted_svg(Path::new("/dev/zero"), 1024).is_none(), "a device");
    }

    use super::*;

    use crate::config::Thumbnails;

    /// The grid shows pages and scenes; a list row shows only what is
    /// cheap to draw at twenty pixels.
    #[test]
    fn pdfs_and_videos_get_thumbnails_in_the_grid_alone() {
        let all = Thumbnails::default();
        for name in ["a.png", "b.JPG", "c.svg"] {
            assert!(wants_thumbnail(Path::new(name), false, &all), "{name} in a list");
            assert!(wants_thumbnail(Path::new(name), true, &all), "{name} in a grid");
        }
        for name in ["doc.pdf", "clip.mp4", "film.MKV", "part.stl"] {
            assert!(!wants_thumbnail(Path::new(name), false, &all), "{name} in a list");
            assert!(wants_thumbnail(Path::new(name), true, &all), "{name} in a grid");
        }
        assert!(!wants_thumbnail(Path::new("notes.txt"), true, &all));
        assert!(!wants_thumbnail(Path::new("Makefile"), true, &all));
    }

    /// Each `[thumbnails]` switch turns off its own source and no other.
    #[test]
    fn a_source_switched_off_is_not_asked_for() {
        let no_videos = Thumbnails { videos: false, ..Thumbnails::default() };
        assert!(!wants(Some(Source::Video), true, &no_videos));
        assert!(wants(Some(Source::Pdf), true, &no_videos));
        let no_system = Thumbnails { system: false, ..Thumbnails::default() };
        assert!(!wants(Some(Source::System), true, &no_system));
        assert!(wants(Some(Source::System), true, &Thumbnails::default()));
        assert!(!wants(Some(Source::System), false, &Thumbnails::default()), "a program per row is grid only");
    }

    /// Video by family, SVG and PDF by name; everything else is somebody
    /// else's to claim.
    #[test]
    fn the_built_in_readers_are_found_by_mime_type() {
        assert_eq!(Source::for_mime("video/x-matroska"), Some(Source::Video));
        assert_eq!(Source::for_mime("application/pdf"), Some(Source::Pdf));
        assert_eq!(Source::for_mime("image/svg+xml"), Some(Source::Svg));
        assert_eq!(Source::for_mime("image/jxl"), None);
    }

    /// The bucket covers the edge; past the largest, the largest.
    #[test]
    fn the_bucket_is_the_smallest_that_covers_the_edge() {
        assert_eq!(bucket_for(56.0), 128);
        assert_eq!(bucket_for(128.0), 128);
        assert_eq!(bucket_for(128.5), 256);
        assert_eq!(bucket_for(358.4), 512);
        assert_eq!(bucket_for(2000.0), 512);
    }
}
