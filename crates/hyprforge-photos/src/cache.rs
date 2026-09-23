//! How many decoded pictures are held at once, and what that costs.
//!
//! Its own module because the interesting assertions here are about
//! *megabytes*, not about hit rates — CLAUDE.md's rule to test the
//! resource and not just the result. A viewer that keeps every
//! photograph it has shown is a viewer that is killed by the OOM killer
//! partway through a holiday folder, and no functional test would ever
//! catch it.
//!
//! # Bounded by bytes, not by count
//!
//! Counting entries would mean a cache of five thumbnails and a cache of
//! five 56MB photographs were the same size, which is the one thing it
//! must not mean. So the bound is bytes, and a decode that would exceed
//! it evicts until it fits.
//!
//! # Why anything is kept at all
//!
//! Paging back and forth across two or three pictures is what people do,
//! and re-decoding on every press is exactly where a viewer feels slow.
//! The default holds a handful of screen-sized decodes — enough for the
//! neighbours on either side, which is what prefetching wants.

use std::collections::VecDeque;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use hyprforge_image::Decoded;

/// How many bytes of decoded picture may be held at once.
///
/// 192MB: on a 2560x1600 display a fit-to-window decode with the zoom
/// headroom is about 56MB, so this is three of them — the picture being
/// shown and one neighbour either side, which is what paging back and
/// forth actually touches. Past that the memory is speculative.
const DEFAULT_BUDGET_BYTES: u64 = 192 * 1024 * 1024;

/// A decoded picture and what it cost.
#[derive(Debug, Clone)]
struct Held {
    path: PathBuf,
    /// Behind an `Arc` so handing one to the view is a pointer copy
    /// rather than another 56MB.
    decoded: Arc<Decoded>,
    bytes: u64,
}

/// The decoded pictures currently in memory.
#[derive(Debug)]
pub struct Cache {
    held: VecDeque<Held>,
    bytes: u64,
    budget: u64,
}

impl Default for Cache {
    fn default() -> Self {
        Cache::with_budget(DEFAULT_BUDGET_BYTES)
    }
}

impl Cache {
    pub fn with_budget(budget: u64) -> Cache {
        Cache { held: VecDeque::new(), bytes: 0, budget }
    }

    /// The picture for `path`, if it is held. Touching it marks it as
    /// most recently used.
    pub fn get(&mut self, path: &Path) -> Option<Arc<Decoded>> {
        let index = self.held.iter().position(|h| h.path == path)?;
        let held = self.held.remove(index)?;
        let decoded = Arc::clone(&held.decoded);
        self.held.push_back(held);
        Some(decoded)
    }

    /// Holds a decoded picture, evicting the least recently used until
    /// it fits.
    ///
    /// A single picture larger than the whole budget is still held — it
    /// is the one being shown, and refusing it would mean showing
    /// nothing. The budget bounds *accumulation*, which is the failure
    /// worth preventing.
    pub fn put(&mut self, path: &Path, decoded: Arc<Decoded>) {
        self.remove(path);

        let bytes = decoded.pixels.len() as u64;
        self.held.push_back(Held { path: path.to_path_buf(), decoded, bytes });
        self.bytes += bytes;

        while self.bytes > self.budget && self.held.len() > 1 {
            if let Some(evicted) = self.held.pop_front() {
                self.bytes -= evicted.bytes;
            }
        }
    }

    /// Drops one picture — what trashing a file, or failing to decode
    /// it, should do to what is held of it.
    pub fn remove(&mut self, path: &Path) {
        if let Some(index) = self.held.iter().position(|h| h.path == path) {
            if let Some(held) = self.held.remove(index) {
                self.bytes -= held.bytes;
            }
        }
    }

    /// Bytes currently held. What a test asserts on.
    pub fn bytes(&self) -> u64 {
        self.bytes
    }

    pub fn len(&self) -> usize {
        self.held.len()
    }

    pub fn is_empty(&self) -> bool {
        self.held.is_empty()
    }

    /// Everything held, newest last — so a prefetcher can ask what it
    /// already has before asking for more.
    pub fn holds(&self, path: &Path) -> bool {
        self.held.iter().any(|h| h.path == path)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use hyprforge_image::budget::DecodePixels;
    use hyprforge_image::measure::{Measured, SourcePixels};
    use hyprforge_image::Orientation;

    /// A stand-in decode of a given size — the tests here are about
    /// bytes, so the pixels themselves may as well be zeroes.
    fn decoded(width: u32, height: u32) -> Arc<Decoded> {
        Arc::new(Decoded {
            pixels: vec![0; (width as usize) * (height as usize) * 4],
            size: DecodePixels { width, height },
            measured: Measured {
                source: SourcePixels { width, height },
                orientation: Orientation::Upright,
                format: image::ImageFormat::Png,
            },
        })
    }

    fn mb(n: u64) -> u64 {
        n * 1024 * 1024
    }

    #[test]
    fn a_picture_that_was_held_comes_back_without_decoding_again() {
        let mut cache = Cache::default();
        cache.put(Path::new("/a.png"), decoded(100, 100));
        assert!(cache.get(Path::new("/a.png")).is_some());
        assert!(cache.get(Path::new("/b.png")).is_none());
    }

    /// The headline: paging through a folder does not accumulate.
    #[test]
    fn paging_through_a_folder_never_grows_past_the_budget() {
        let mut cache = Cache::with_budget(mb(64));
        // Each of these is 16MB — 2048x2048x4.
        for i in 0..20 {
            cache.put(&PathBuf::from(format!("/photo-{i}.jpg")), decoded(2048, 2048));
            assert!(cache.bytes() <= mb(64), "after {i}: {} bytes", cache.bytes());
        }
        assert!(cache.len() < 20, "nothing was ever evicted");
    }

    /// Bytes, not entries: a cache bounded by count would treat these
    /// two cases as the same when they differ by two orders of
    /// magnitude.
    #[test]
    fn the_bound_is_bytes_and_not_a_number_of_pictures() {
        let mut small = Cache::with_budget(mb(64));
        for i in 0..8 {
            small.put(&PathBuf::from(format!("/thumb-{i}.png")), decoded(256, 256));
        }
        assert_eq!(small.len(), 8, "eight thumbnails are only 2MB and all fit");

        let mut large = Cache::with_budget(mb(64));
        for i in 0..8 {
            large.put(&PathBuf::from(format!("/photo-{i}.jpg")), decoded(4096, 4096));
        }
        assert!(large.len() < 8, "eight 64MB photographs cannot all be held");
    }

    /// The picture being shown is held even if it is bigger than the
    /// whole budget, because the alternative is showing nothing.
    #[test]
    fn one_enormous_picture_is_still_shown() {
        let mut cache = Cache::with_budget(mb(8));
        cache.put(Path::new("/huge.jpg"), decoded(4096, 4096));
        assert_eq!(cache.len(), 1);
        assert!(cache.get(Path::new("/huge.jpg")).is_some());
    }

    /// Least *recently used*, not least recently added: paging back and
    /// forth between two pictures must not evict either of them.
    #[test]
    fn the_picture_you_keep_coming_back_to_is_the_last_one_evicted() {
        let mut cache = Cache::with_budget(mb(48));
        cache.put(Path::new("/a.jpg"), decoded(2048, 2048));
        cache.put(Path::new("/b.jpg"), decoded(2048, 2048));

        // Touch a, so b is now the least recently used.
        assert!(cache.get(Path::new("/a.jpg")).is_some());
        cache.put(Path::new("/c.jpg"), decoded(2048, 2048));

        assert!(cache.holds(Path::new("/a.jpg")), "the one we kept using was evicted");
        assert!(cache.holds(Path::new("/c.jpg")));
    }

    #[test]
    fn holding_the_same_picture_twice_does_not_count_it_twice() {
        let mut cache = Cache::default();
        cache.put(Path::new("/a.png"), decoded(512, 512));
        let once = cache.bytes();
        cache.put(Path::new("/a.png"), decoded(512, 512));
        assert_eq!(cache.bytes(), once);
        assert_eq!(cache.len(), 1);
    }

    /// Trashing a picture should not leave it occupying memory.
    #[test]
    fn dropping_a_picture_gives_its_memory_back() {
        let mut cache = Cache::default();
        cache.put(Path::new("/a.png"), decoded(1024, 1024));
        assert!(cache.bytes() > 0);
        cache.remove(Path::new("/a.png"));
        assert_eq!(cache.bytes(), 0);
        assert!(cache.is_empty());
    }
}
