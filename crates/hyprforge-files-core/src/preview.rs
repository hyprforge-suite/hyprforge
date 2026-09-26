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

/// Whether an entry is worth asking the host for a thumbnail of, by its
/// name alone — the browser does no I/O, and this is asked of every row.
///
/// Pictures and SVGs in either view; PDFs and videos in the grid only.
/// Those two need another program run per file — a video's frame costs
/// about half a second of `ffmpeg` — which is worth it for a grid cell
/// big enough to show a page or a scene, and not for a list row's
/// twenty-pixel icon. Like the picture check it can be wrong about a
/// misnamed file; the host then answers nothing and the icon stays.
pub fn wants_thumbnail(path: &Path, grid: bool) -> bool {
    if hyprforge_image::format::looks_decodable(path) {
        return true;
    }
    let Some(ext) = path.extension().and_then(|e| e.to_str()).map(str::to_ascii_lowercase) else {
        return false;
    };
    match ext.as_str() {
        "svg" => true,
        "pdf" | "mp4" | "mkv" | "webm" | "mov" | "avi" | "m4v" | "wmv" | "mpg" | "mpeg" | "ogv" => grid,
        _ => false,
    }
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

impl Picture {
    /// By extension: for a theme icon, which `hyprforge-icons` only ever
    /// answers with as a `.png` or an `.svg`, and for an SVG preview.
    /// iced reads the file itself the first time it draws it.
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
    use super::*;

    /// The grid shows pages and scenes; a list row shows only what is
    /// cheap to draw at twenty pixels.
    #[test]
    fn pdfs_and_videos_get_thumbnails_in_the_grid_alone() {
        for name in ["a.png", "b.JPG", "c.svg"] {
            assert!(wants_thumbnail(Path::new(name), false), "{name} in a list");
            assert!(wants_thumbnail(Path::new(name), true), "{name} in a grid");
        }
        for name in ["doc.pdf", "clip.mp4", "film.MKV"] {
            assert!(!wants_thumbnail(Path::new(name), false), "{name} in a list");
            assert!(wants_thumbnail(Path::new(name), true), "{name} in a grid");
        }
        assert!(!wants_thumbnail(Path::new("notes.txt"), true));
        assert!(!wants_thumbnail(Path::new("Makefile"), true));
    }
}
