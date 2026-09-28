//! Thumbnails of videos and models, through the shared freedesktop cache.
//!
//! A still is decoded straight to its tile size (`hyprforge_image`), as it
//! always was — decoding a picture at 300 pixels costs about what reading
//! a cached PNG does. A video's frame costs half a second of ffmpeg and a
//! model's drawing a load and a raster, so those go through the cache the
//! file manager uses: made once, read back until the file changes, and
//! shared with every other program that reads `~/.cache/thumbnails`.
//!
//! The `large` size, 256 pixels: a grid tile is about 180 logical pixels,
//! which is nearly 300 physical on a 1.6-scale screen, and the `normal`
//! 128 would be drawn at more than twice its size.
//!
//! Blocking; call it off the UI thread.

use crate::folder::Media;
use hyprforge_thumbnails::{Cache, Lookup, Rgba, Size, Stamp};
use std::path::Path;

/// The thumbnail of `path`, a clip or a model: from the cache while the
/// file is unchanged, made and stored otherwise. `None` when there is no
/// thumbnail to show — the tile keeps its badge.
///
/// A tool that is missing or timed out is not a verdict on the file, so
/// it is never recorded as one: the same video may thumbnail fine once
/// ffmpeg is installed.
pub fn of(cache: Option<&Cache>, path: &Path, media: Media) -> Option<Rgba> {
    let stamp = Stamp::of(path).ok();
    if let (Some(cache), Some(stamp)) = (cache, stamp) {
        match cache.get_sized(Size::Large, path, stamp) {
            Lookup::Current(rgba) => return Some(rgba),
            Lookup::Failed => return None,
            Lookup::Missing => {}
        }
    }
    let made = match media {
        Media::Clip => match hyprforge_video::frame::first_real_frame(path, Size::Large.edge()) {
            Ok(Some(png)) => Made::Thumbnail(hyprforge_thumbnails::decode_png(&png)),
            Ok(None) => Made::Failed,
            Err(_) => Made::NoVerdict,
        },
        Media::Model => match hyprforge_mesh::load(path, true) {
            Ok((mesh, _)) => {
                let img = hyprforge_mesh::thumbnail::render(&mesh, Size::Large.edge());
                Made::Thumbnail(Some(Rgba { width: img.width, height: img.height, pixels: img.pixels }))
            }
            Err(_) => Made::Failed,
        },
        // Stills are decoded directly; see the module doc.
        Media::Still => Made::NoVerdict,
    };
    match made {
        Made::Thumbnail(Some(rgba)) => {
            if let (Some(cache), Some(stamp)) = (cache, stamp) {
                if let Err(e) = cache.put_sized(Size::Large, path, stamp, &rgba) {
                    tracing::debug!(error = %e, "thumbnail not stored");
                }
            }
            Some(rgba)
        }
        Made::Thumbnail(None) | Made::Failed => {
            if let (Some(cache), Some(stamp)) = (cache, stamp) {
                let _ = cache.put_failed(path, stamp);
            }
            None
        }
        Made::NoVerdict => None,
    }
}

enum Made {
    Thumbnail(Option<Rgba>),
    /// The file itself could not be thumbnailed.
    Failed,
    /// Nothing is known about the file: the tool is missing or was busy.
    NoVerdict,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cube_stl(dir: &Path) -> std::path::PathBuf {
        // Twelve triangles of a 10mm cube, as ASCII STL.
        let c = |i: usize| [if i & 1 != 0 { 10 } else { 0 }, if i & 2 != 0 { 10 } else { 0 }, if i & 4 != 0 { 10 } else { 0 }];
        let faces = [[0, 2, 3, 1], [4, 5, 7, 6], [0, 1, 5, 4], [2, 6, 7, 3], [0, 4, 6, 2], [1, 3, 7, 5]];
        let mut text = String::from("solid cube\n");
        for f in faces {
            for t in [[f[0], f[1], f[2]], [f[0], f[2], f[3]]] {
                text.push_str("facet normal 0 0 0\nouter loop\n");
                for i in t {
                    let [x, y, z] = c(i);
                    text.push_str(&format!("vertex {x} {y} {z}\n"));
                }
                text.push_str("endloop\nendfacet\n");
            }
        }
        text.push_str("endsolid cube\n");
        let path = dir.join("cube.stl");
        std::fs::write(&path, text).unwrap();
        path
    }

    /// A model is drawn once, stored at the large size, and read back from
    /// the cache the second time rather than drawn again.
    #[test]
    fn a_model_is_drawn_once_and_then_read_from_the_cache() {
        let dir = tempfile::tempdir().unwrap();
        let cache = Cache::at(dir.path().join("thumbnails"));
        let model = cube_stl(dir.path());
        let first = of(Some(&cache), &model, Media::Model).expect("a thumbnail");
        assert_eq!((first.width, first.height), (256, 256));
        assert!(dir.path().join("thumbnails/large").join(hyprforge_thumbnails::thumbnail_name(&model)).exists());
        assert!(matches!(cache.get_sized(Size::Large, &model, Stamp::of(&model).unwrap()), Lookup::Current(_)));
        assert!(of(Some(&cache), &model, Media::Model).is_some());
    }

    /// A model that will not load is remembered as a failure, so the grid
    /// does not try again on every visit.
    #[test]
    fn a_broken_model_is_remembered_as_a_failure() {
        let dir = tempfile::tempdir().unwrap();
        let cache = Cache::at(dir.path().join("thumbnails"));
        let broken = dir.path().join("broken.3mf");
        std::fs::write(&broken, b"not a zip").unwrap();
        assert!(of(Some(&cache), &broken, Media::Model).is_none());
        assert_eq!(cache.get_sized(Size::Large, &broken, Stamp::of(&broken).unwrap()), Lookup::Failed);
    }

    /// With nowhere to cache, a thumbnail is still made — just every time.
    #[test]
    fn without_a_cache_a_thumbnail_is_still_made() {
        let dir = tempfile::tempdir().unwrap();
        assert!(of(None, &cube_stl(dir.path()), Media::Model).is_some());
    }
}
