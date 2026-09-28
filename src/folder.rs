//! The folder a picture came from, and where we are in it.
//!
//! # Media-aware, though nothing here decodes video
//!
//! A camera folder is mixed. Press Right in one and the next thing must
//! be the next *item* — if clips were invisible to this model, paging
//! through a phone's DCIM would silently skip half of it, and the folder
//! people use most is the one where the viewer would be most wrong.
//!
//! So a clip is a first-class [`Item`] with a [`Media::Clip`] kind. What
//! this app does with one is show what it can (a poster frame from the
//! shared thumbnail cache, when some other program has already made one)
//! and hand it to the user's player on request. It never decodes a frame
//! of video, which is why there is no gstreamer or libmpv anywhere in
//! this crate's dependencies.
//!
//! # The order is the file manager's
//!
//! Not `read_dir` order, and not a local opinion: the entries are
//! filtered and sorted with [`hyprforge_listing`] under the [`Order`] the
//! file manager wrote. Two windows onto one folder that disagree about
//! what comes next is the failure this arrangement exists to prevent.

use hyprforge_listing::order::Order;
use hyprforge_listing::sort;
use hyprforge_listing::types::{Entry, EntryKind};
use std::path::{Path, PathBuf};

/// What kind of thing an item is, as far as a viewer is concerned.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Media {
    /// A picture this build can decode.
    Still,
    /// A video. Shown as a poster frame when one is already cached, and
    /// handed to the user's player when asked — never decoded here.
    Clip,
    /// A 3D model — STL, 3MF or OBJ — loaded by `hyprforge-mesh` and
    /// drawn by the viewer's own renderer (what view3d did on its own).
    Model,
}

/// The videos this viewer claims in its desktop entry: the shared MIME
/// database's canonical types for the five extensions the listing calls
/// video (`mp4 mkv webm avi mov` — `EntryKind::classify`), as that
/// database names them on this machine (`/usr/share/mime/video/`). A
/// type claimed here and not classified there would be a video Files
/// hands over and this window does not page to.
pub const VIDEO_MIME_TYPES: [&str; 5] = ["video/mp4", "video/matroska", "video/webm", "video/vnd.avi", "video/quicktime"];

/// One thing in the folder.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Item {
    pub path: PathBuf,
    pub name: String,
    pub media: Media,
    /// From the listing, never a second `stat`: the grid groups by it and
    /// the library summarises a folder's span of dates with it.
    pub modified: Option<std::time::SystemTime>,
    /// Bytes on disk, from the listing — what the grid's status bar adds
    /// up for a selection.
    pub bytes: Option<u64>,
}

/// The items in a folder, and which one is showing.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Folder {
    items: Vec<Item>,
    cursor: usize,
}

/// Whether an entry is something a viewer shows at all.
///
/// Stills are decided by [`hyprforge_image::format::looks_decodable`] —
/// the build's real decoder list rather than a second one kept here —
/// and clips by the listing's own classification, since nothing decodes
/// them and their extension is all anyone needs. Models by
/// `hyprforge-mesh`'s own format list, for the same reason as stills.
fn media_of(entry: &Entry) -> Option<Media> {
    if entry.is_dir {
        return None;
    }
    if hyprforge_mesh::detect(&entry.path).is_some() {
        return Some(Media::Model);
    }
    if hyprforge_image::format::looks_decodable(&entry.path) {
        return Some(Media::Still);
    }
    if entry.kind == EntryKind::Video {
        return Some(Media::Clip);
    }
    None
}

impl Folder {
    /// Builds the folder's items from a directory listing.
    ///
    /// `directories_first` in the order has no effect here and that is
    /// not an oversight: directories are not items, so there are none to
    /// put first. It stays part of [`Order`] because the *file manager*
    /// means something by it.
    pub fn build(entries: Vec<Entry>, order: &Order) -> Folder {
        let mut entries = hyprforge_listing::filter::filter_hidden(entries, order.show_hidden);
        entries.retain(|e| media_of(e).is_some());
        sort::sort(&mut entries, order.sort_column, order.sort_direction, order.directories_first);

        let items = entries
            .into_iter()
            .map(|e| Item {
                media: media_of(&e).expect("filtered to items above"),
                bytes: match e.size {
                    hyprforge_listing::types::EntrySize::Bytes(b) => Some(b),
                    hyprforge_listing::types::EntrySize::Items(_) => None,
                },
                modified: e.modified,
                name: e.name,
                path: e.path,
            })
            .collect();

        Folder { items, cursor: 0 }
    }

    /// Points the cursor at `path`, if it is here.
    ///
    /// Returns whether it was found. A picture opened from the file
    /// manager is the one the user asked for, so a folder that cannot
    /// find it is worth knowing about rather than silently showing
    /// something else.
    pub fn focus_on(&mut self, path: &Path) -> bool {
        match self.items.iter().position(|i| i.path == path) {
            Some(index) => {
                self.cursor = index;
                true
            }
            None => false,
        }
    }

    pub fn current(&self) -> Option<&Item> {
        self.items.get(self.cursor)
    }

    pub fn items(&self) -> &[Item] {
        &self.items
    }

    pub fn len(&self) -> usize {
        self.items.len()
    }

    pub fn is_empty(&self) -> bool {
        self.items.is_empty()
    }

    /// Which item is showing, counting from zero.
    pub fn cursor(&self) -> usize {
        self.cursor
    }

    /// "3 of 47", as an info panel or a title bar says it — one-based,
    /// because that is how people count pictures.
    pub fn position(&self) -> Option<(usize, usize)> {
        if self.items.is_empty() {
            None
        } else {
            Some((self.cursor + 1, self.items.len()))
        }
    }

    /// The next item, wrapping at the end.
    ///
    /// Wrapping rather than stopping: a viewer is a carousel in every
    /// implementation anyone has used, and stopping dead at the last
    /// picture reads as the program having frozen.
    //
    // Not `Iterator::next`, which clippy suggests by the name alone.
    // This is a cursor, not a sequence: it has a `previous`, it wraps
    // forever, and it hands back a borrow of an item it still owns.
    // Renaming it would cost the one word the domain actually uses for
    // this — the next picture — to satisfy a lint about a trait it
    // cannot implement.
    #[allow(clippy::should_implement_trait)]
    pub fn next(&mut self) -> Option<&Item> {
        if self.items.is_empty() {
            return None;
        }
        self.cursor = (self.cursor + 1) % self.items.len();
        self.current()
    }

    pub fn previous(&mut self) -> Option<&Item> {
        if self.items.is_empty() {
            return None;
        }
        self.cursor = (self.cursor + self.items.len() - 1) % self.items.len();
        self.current()
    }

    pub fn first(&mut self) -> Option<&Item> {
        self.cursor = 0;
        self.current()
    }

    pub fn last(&mut self) -> Option<&Item> {
        self.cursor = self.items.len().saturating_sub(1);
        self.current()
    }

    /// Points the cursor at `index` directly — a click on a tile. Out of
    /// range is ignored rather than clamped: a click on something that
    /// is no longer there should not select something else.
    pub fn select(&mut self, index: usize) -> bool {
        if index < self.items.len() {
            self.cursor = index;
            true
        } else {
            false
        }
    }

    /// Drops the item at the cursor — what trashing one leaves behind.
    ///
    /// The cursor stays put so the *next* picture slides into view,
    /// which is what makes deleting several in a row feel like anything
    /// at all. At the end of the folder it steps back instead, and an
    /// empty folder leaves it at zero.
    pub fn remove_current(&mut self) {
        if self.items.is_empty() {
            return;
        }
        self.items.remove(self.cursor);
        if self.cursor >= self.items.len() {
            self.cursor = self.items.len().saturating_sub(1);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use hyprforge_listing::types::{EntrySize, ItemCount};
    use std::time::{Duration, SystemTime};

    fn entry(name: &str, modified_secs: u64) -> Entry {
        let is_dir = !name.contains('.');
        Entry {
            name: name.to_string(),
            path: PathBuf::from("/photos").join(name),
            is_dir,
            size: if is_dir {
                EntrySize::Items(ItemCount::Known(0))
            } else {
                EntrySize::Bytes(1024)
            },
            modified: Some(SystemTime::UNIX_EPOCH + Duration::from_secs(modified_secs)),
            is_symlink: false,
            link_broken: false,
            hidden: name.starts_with('.'),
            kind: EntryKind::classify(is_dir, name),
            mode: 0o644,
            // A file on disk, not a member of an archive.
            packed: None,
            uid: 1000,
            owner: Some("someone".to_string()),
            origin: None,
        }
    }

    fn folder(names: &[&str]) -> Folder {
        let entries = names.iter().enumerate().map(|(i, n)| entry(n, i as u64)).collect();
        Folder::build(entries, &Order::default())
    }

    #[test]
    fn only_things_a_viewer_can_show_become_items() {
        let f = folder(&["a.png", "notes.txt", "b.jpg", "somedir", "c.tar.gz"]);
        let names: Vec<&str> = f.items().iter().map(|i| i.name.as_str()).collect();
        assert_eq!(names, ["a.png", "b.jpg"]);
    }

    /// The DCIM case: a clip is an item, not a gap.
    #[test]
    fn a_clip_is_an_item_you_can_page_past() {
        let mut f = folder(&["a.jpg", "b.mp4", "c.jpg"]);
        assert_eq!(f.len(), 3);
        assert_eq!(f.current().unwrap().media, Media::Still);
        assert_eq!(f.next().unwrap().media, Media::Clip);
        assert_eq!(f.next().unwrap().name, "c.jpg");
    }

    /// A folder of prints: the models page with the pictures, in the
    /// same order, rather than being skipped as files a viewer cannot
    /// show.
    #[test]
    fn a_model_is_an_item_you_can_page_to() {
        let mut f = folder(&["a.jpg", "bracket.stl", "case.3mf", "notes.txt", "teapot.obj"]);
        let kinds: Vec<Media> = f.items().iter().map(|i| i.media).collect();
        assert_eq!(kinds, [Media::Still, Media::Model, Media::Model, Media::Model]);
        assert_eq!(f.next().unwrap().name, "bracket.stl");
    }

    #[test]
    fn an_item_carries_what_the_listing_already_knew() {
        let f = folder(&["a.jpg"]);
        let item = f.current().unwrap();
        assert_eq!(item.bytes, Some(1024));
        assert_eq!(item.modified, Some(SystemTime::UNIX_EPOCH));
    }

    #[test]
    fn selecting_past_the_end_selects_nothing_else() {
        let mut f = folder(&["a.jpg", "b.jpg"]);
        assert!(f.select(1));
        assert_eq!(f.current().unwrap().name, "b.jpg");
        assert!(!f.select(5));
        assert_eq!(f.current().unwrap().name, "b.jpg");
    }

    #[test]
    fn next_at_the_end_of_the_folder_wraps_to_the_first() {
        let mut f = folder(&["a.jpg", "b.jpg"]);
        assert_eq!(f.next().unwrap().name, "b.jpg");
        assert_eq!(f.next().unwrap().name, "a.jpg");
    }

    #[test]
    fn previous_at_the_start_wraps_to_the_last() {
        let mut f = folder(&["a.jpg", "b.jpg", "c.jpg"]);
        assert_eq!(f.previous().unwrap().name, "c.jpg");
    }

    /// The order is the file manager's, not `read_dir`'s: reversing the
    /// sort direction reverses what "next" means.
    #[test]
    fn the_order_follows_the_one_the_file_manager_was_showing() {
        let entries: Vec<Entry> =
            ["b.jpg", "a.jpg", "c.jpg"].iter().enumerate().map(|(i, n)| entry(n, i as u64)).collect();

        let ascending = Folder::build(entries.clone(), &Order::default());
        assert_eq!(ascending.current().unwrap().name, "a.jpg");

        let descending = Folder::build(
            entries,
            &Order {
                sort_direction: hyprforge_listing::sort::SortDirection::Descending,
                ..Order::default()
            },
        );
        assert_eq!(descending.current().unwrap().name, "c.jpg");
    }

    /// And sorting by date really does reorder — the case that makes an
    /// independently-sorted viewer wrong, since a folder of photographs
    /// is usually sorted by when they were taken.
    #[test]
    fn sorting_by_date_is_honoured_and_differs_from_by_name() {
        let entries = vec![entry("a.jpg", 300), entry("b.jpg", 100), entry("c.jpg", 200)];
        let by_date = Folder::build(
            entries,
            &Order {
                sort_column: hyprforge_listing::sort::SortColumn::Modified,
                ..Order::default()
            },
        );
        let names: Vec<&str> = by_date.items().iter().map(|i| i.name.as_str()).collect();
        assert_eq!(names, ["b.jpg", "c.jpg", "a.jpg"]);
    }

    #[test]
    fn hidden_pictures_are_left_out_unless_the_file_manager_shows_them() {
        let hidden = folder(&["a.jpg", ".secret.jpg"]);
        assert_eq!(hidden.len(), 1);

        let entries = vec![entry("a.jpg", 0), entry(".secret.jpg", 1)];
        let shown = Folder::build(entries, &Order { show_hidden: true, ..Order::default() });
        assert_eq!(shown.len(), 2);
    }

    #[test]
    fn focusing_on_the_picture_that_was_opened_finds_it() {
        let mut f = folder(&["a.jpg", "b.jpg", "c.jpg"]);
        assert!(f.focus_on(Path::new("/photos/c.jpg")));
        assert_eq!(f.current().unwrap().name, "c.jpg");
        assert_eq!(f.position(), Some((3, 3)));
    }

    #[test]
    fn focusing_on_something_that_is_not_here_says_so() {
        let mut f = folder(&["a.jpg"]);
        assert!(!f.focus_on(Path::new("/photos/elsewhere.jpg")));
    }

    /// Every navigation on an empty folder is a no-op rather than a
    /// panic — an empty folder is an ordinary thing to open.
    #[test]
    fn an_empty_folder_navigates_without_panicking() {
        let mut f = folder(&["notes.txt"]);
        assert!(f.is_empty());
        assert_eq!(f.next(), None);
        assert_eq!(f.previous(), None);
        assert_eq!(f.first(), None);
        assert_eq!(f.last(), None);
        assert_eq!(f.position(), None);
        f.remove_current();
        assert!(f.is_empty());
    }

    /// Trashing one picture should bring the next into view, so that
    /// clearing out a folder is one keypress per picture.
    #[test]
    fn removing_the_current_picture_shows_the_next_one() {
        let mut f = folder(&["a.jpg", "b.jpg", "c.jpg"]);
        f.next();
        assert_eq!(f.current().unwrap().name, "b.jpg");
        f.remove_current();
        assert_eq!(f.current().unwrap().name, "c.jpg");
    }

    #[test]
    fn removing_the_last_picture_steps_back_instead() {
        let mut f = folder(&["a.jpg", "b.jpg"]);
        f.last();
        f.remove_current();
        assert_eq!(f.current().unwrap().name, "a.jpg");
    }
}
