//! `hyprforge-media`: the window.
//!
//! Everything this draws is decided in the library — which picture is
//! next ([`folder`]), where it sits and how big ([`transform`]), what a
//! key means ([`keys`]), what the inspector says ([`info`]), how the
//! grid falls into days and rows ([`grid`]), what a folder card says
//! ([`library`]). What is here is the part that cannot be asserted
//! without a display: turning those decisions into widgets (`view.rs`),
//! and running the slow work (decoding, reading EXIF, listing folders,
//! trashing, the clipboard, the wallpaper) off the thread that paints.
//!
//! # One window, three modes
//!
//! The design (the `Hyprview Photo Viewer` mockups, `2a`–`2c`) puts the
//! viewer in the file manager's shell — the same header, the same Places
//! sidebar, the same status bar — and makes Photo, Grid and Library
//! *modes* of that one window rather than overlays over a photograph.
//! [`Mode`] is that switch. The folder, the selection and the history
//! are shared by all three, so moving between them never loses your
//! place: the tile selected in the grid is the photograph Photo mode
//! opens on, and the folder the library opens is the one the grid shows.
//!
//! # Pixels, never paths
//!
//! The picture reaches iced as `Handle::from_rgba`, never `image(path)`.
//! A path handle makes iced decode the whole file at full resolution and
//! apply EXIF orientation itself; a 36-megapixel photograph is then
//! 144MB before the renderer's own copy, and `iced_tiny_skia` caches an
//! allocation failure and panics on the next frame. `hyprforge_image`
//! decodes to fit the window instead — and applies the orientation,
//! because the RGBA path does not. Turning a picture is done to those
//! pixels too (`rotation::rotate_rgba`), so the size on screen is the
//! size every zoom calculation assumes.
//!
//! Thumbnails follow the same rule and one more: only the tiles that can
//! be seen are decoded (`grid::visible`), and the store of them is capped
//! at [`MAX_THUMBS`]. A folder of four thousand photographs costs what a
//! screenful of tiles costs.

mod film;
mod model;
mod view;

use hyprforge_image::{decode_to_fit, Budget, Camera, Decoded, Measured, ViewportPixels};
use hyprforge_listing::backend::{FsBackend, StdBackend};
use hyprforge_media::args::{self, Args};
use hyprforge_media::cache::Cache;
use hyprforge_media::folder::{Folder, Media};
use hyprforge_media::history::History;
use hyprforge_media::keys::{Action, Resolved};
use hyprforge_media::library::Summary;
use hyprforge_media::rotation::{rotate_rgba, Turns};
use hyprforge_mesh::camera::{Camera as ModelCamera, ViewPoint};
use hyprforge_mesh::style::DrawMode;
use hyprforge_media::slideshow::{Interval, Show};
use hyprforge_media::transform::{ImageSize, LogicalPoint, Transform, Viewport};
use hyprforge_media::{config, filmstrip, grid, launch, library, order, prefs};
use hyprforge_ui::theme::FontScale;
use hyprforge_ui::widgets::Tint;
use iced::widget::{image, Id};
use iced::{keyboard, mouse, window, Point, Size, Subscription, Task, Theme};
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};

const APP_ID: &str = "hyprforge-media";

/// The Places sidebar, logical pixels — the mockup's 200.
const SIDEBAR_WIDTH: f32 = 200.0;
/// The inspector down the right — mockup `2c`'s 300.
const INSPECTOR_WIDTH: f32 = 300.0;
/// The filmstrip along the bottom of Photo mode — mockup `2a`'s 84.
const FILMSTRIP_HEIGHT: f32 = 84.0;
/// A filmstrip tile, 3:2, and the larger one the current picture gets.
const STRIP_TILE: (f32, f32) = (66.0, 44.0);
const STRIP_CURRENT: (f32, f32) = (78.0, 52.0);
const STRIP_GAP: f32 = 6.0;
/// The status bar — the file manager's 30px, so the two shells match.
const STATUS_HEIGHT: f32 = 30.0;
/// The narrowest a grid tile may be before the grid drops a column.
const TILE_MIN_WIDTH: f32 = 170.0;
/// Padding inside a tile and around the grid, and the gap between tiles.
const TILE_PAD: f32 = 8.0;
const TILE_GAP: f32 = 10.0;
const GRID_PAD: f32 = 16.0;
/// A tile's name line, and a library card's second line under it.
const TILE_LINE: f32 = 18.0;
/// A day's heading in the grid.
const GROUP_HEADER: f32 = 30.0;
const GROUP_GAP: f32 = 18.0;
/// The long edge thumbnails are decoded at, logical pixels. One size for
/// the filmstrip, the grid and the library covers, so one decode serves
/// all three.
const THUMB_EDGE: f32 = 200.0;
/// How many thumbnails are kept. At `THUMB_EDGE` on a 1.6-scale output a
/// 3:2 thumbnail is 320×213 RGBA, about 270KB, so this caps the store near
/// 65MB however large the folder — the grid only ever *wants* a
/// screenful plus a screen of margin either side, well under this.
const MAX_THUMBS: usize = 240;
/// How long a resize has to settle before the window size is saved and a
/// sharper decode is considered — a drag produces a resize per frame.
const RESIZE_SETTLE_MS: u64 = 400;
/// Wheel pixels per zoom step, for a touchpad's smooth scrolling.
const PIXELS_PER_ZOOM_STEP: f32 = 60.0;
/// How often a model on screen is checked for a newer file on disk. A
/// `stat` a second of one path costs nothing and needs no watcher — the
/// file manager polls the files it opened out of archives the same way.
const MODEL_CHECK_EVERY: Duration = Duration::from_secs(1);
/// Scroll units per wheel notch for the model camera, which counts in
/// view3d's (egui's) points; iced's `Lines` are notches.
const MODEL_UNITS_PER_LINE: f32 = 50.0;
/// How long a slideshow's controls stay up after the pointer stops —
/// mockup `1e`'s "controls fade after 2 s".
const SLIDESHOW_CONTROLS_FOR: Duration = Duration::from_secs(2);

fn main() -> iced::Result {
    // `warn` unless RUST_LOG says otherwise — a viewer opened from the
    // file manager has no terminal, and anything quieter would hide the
    // warning for a config file that exists and will not parse.
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("warn")),
        )
        .init();

    // The shared look, resolved before the first frame — the same line
    // every app in the suite has.
    hyprforge_ui::theme::init(hyprforge_appearance::look::resolve());

    let argv: Vec<String> = std::env::args().skip(1).collect();
    let args = args::parse(&argv);

    // Three files, three kinds of trouble, all reported and none fatal:
    // a typo in a binding must not stop the window opening.
    let (config, config_problems) = config::load();
    let ordering = order::load();
    let (prefs, prefs_problem) = match prefs::load() {
        Ok(prefs) => (prefs, None),
        Err(e) => (
            prefs::Prefs::default(),
            Some(format!("Your viewer settings couldn't be read, so defaults are in use: {e}")),
        ),
    };
    let mut problems: Vec<String> = config_problems.iter().map(|p| p.to_string()).collect();
    problems.extend(ordering.problem.clone());
    problems.extend(prefs_problem);

    let size = Size::new(
        (prefs.window_width as f32).max(640.0),
        (prefs.window_height as f32).max(420.0),
    );

    let places = places();

    // A picture opens in Photo mode on that picture; a folder opens as
    // its grid; nothing at all opens the grid of Pictures, because a
    // shell with a sidebar has somewhere better to start than a sentence.
    let (start, focus, mode) = match &args {
        Args::Picture(path) if path.is_dir() => (Some(path.clone()), None, Mode::Grid),
        Args::Picture(path) => (path.parent().map(Path::to_path_buf), Some(path.clone()), Mode::Photo),
        Args::Folder(path) => (Some(path.clone()), None, Mode::Grid),
        Args::Nothing => (places.first().map(|p| p.path.clone()), None, Mode::Grid),
    };

    // A picture opened on its own is a glance: the viewer starts compact —
    // no sidebar, no inspector — and floats itself on Hyprland. A folder,
    // or nothing, is a session in the library and gets the whole shell.
    let compact = matches!(&args, Args::Picture(path) if !path.is_dir()).then_some(Compact { sidebar: false, info: false });
    let float_picture = match &args {
        Args::Picture(path) if !path.is_dir() => Some(path.clone()),
        _ => None,
    };
    // The floating size, worked out before the window exists so it can map
    // floating instead of tiling first — see `float.rs`.
    let planned_float = float_picture.as_deref().filter(|_| hyprforge_media::float::on_hyprland()).and_then(|picture| {
        let scale = FontScale(hyprforge_ui::theme::active().font_scale);
        let mut chrome_h = hyprforge_ui::density::bar_height(scale) + 1.0 + scale.apply(STATUS_HEIGHT) + 1.0;
        if prefs.filmstrip {
            chrome_h += FILMSTRIP_HEIGHT + 1.0;
        }
        hyprforge_media::float::planned_size(picture, chrome_h)
            .inspect_err(|e| tracing::warn!(error = %e, "no floating size; opening tiled"))
            .ok()
    });
    let window_size = planned_float.map_or(size, |(w, h)| Size::new(w as f32, h as f32));

    let mut app = App {
        keymap: config.keymap,
        order: ordering.order,
        prefs,
        mode,
        folder: Folder::default(),
        folder_path: None,
        focus: None,
        loading_folder: false,
        folder_error: None,
        history: History::default(),
        last_selected: HashMap::new(),
        cache: Cache::default(),
        decoding: HashSet::new(),
        failed: HashMap::new(),
        shown: None,
        turns: Turns::none(),
        transform: Transform::default(),
        details: None,
        details_wanted: None,
        window: None,
        window_size: size,
        scale_factor: 1.0,
        fullscreen: false,
        dragging: None,
        cursor: None,
        scroll_pixels: 0.0,
        resize_generation: 0,
        thumbs: HashMap::new(),
        thumbs_wanted: HashSet::new(),
        current_bytes: None,
        places,
        folders: HashMap::new(),
        library_dir: None,
        library_cursor: 0,
        grid_scroll: (0.0, size.height),
        library_scroll: (0.0, size.height),
        menu_open: false,
        show: None,
        model: None,
        model_loading: None,
        model_generation: 0,
        model_drag: None,
        compact,
        float_picture,
        floated: false,
        planned_float,
        video: None,
        video_generation: 0,
        seeking: None,
        thumb_cache: hyprforge_thumbnails::Cache::standard().map(Arc::new),
        notice: problems.first().cloned(),
        undo: None,
        clipboard: Arc::new(hyprforge_clipboard::WaylandWriter::new()),
        font_scale: FontScale(hyprforge_ui::theme::active().font_scale),
    };

    let mut boot_tasks = vec![window::latest().map(Message::WindowFound)];
    if let Some(dir) = start {
        boot_tasks.push(app.load_folder(dir, focus));
    }
    let boot = std::cell::RefCell::new(Some((app, Task::batch(boot_tasks))));

    iced::application(
        move || boot.borrow_mut().take().expect("hyprforge-media boots once"),
        App::update,
        App::view,
    )
    .title(App::title)
    .theme(App::theme)
    .subscription(App::subscription)
    .window(window::Settings {
        size: window_size,
        // Fixed-size while it maps, so Hyprland floats it at once; the
        // ordinary limits come back once it has (`Message::Floated`).
        min_size: Some(if planned_float.is_some() { window_size } else { Size::new(640.0, 420.0) }),
        max_size: planned_float.map(|_| window_size),
        resizable: planned_float.is_none(),
        platform_specific: window::settings::PlatformSpecific {
            application_id: APP_ID.to_string(),
            ..window::settings::PlatformSpecific::default()
        },
        ..window::Settings::default()
    })
    .run()
}

/// The three modes of the window — mockup `2a`'s segmented switch.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Mode {
    Photo,
    Grid,
    Library,
}

/// A row under PLACES.
struct Place {
    label: String,
    path: PathBuf,
    tint: Tint,
}

/// Pictures, its Screenshots folder and Downloads — the places a photo
/// viewer is opened on — from the user's own `user-dirs.dirs`, so a
/// machine set up in German lists `Bilder`. Only the ones that exist.
///
/// The tints are the file manager's for the same folders (Pictures in
/// the warning hue, and so on): identity, not state, which is the
/// borrowing `Tint`'s own doc names and defends for a sidebar.
fn places() -> Vec<Place> {
    let Some(home) = std::env::var_os("HOME").map(PathBuf::from) else {
        return Vec::new();
    };
    let dirs = hyprforge_paths::user_dirs::load(&hyprforge_paths::config_home(), &home);
    let label = |p: &Path| p.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    let mut out = Vec::new();
    if let Some(pictures) = dirs.pictures {
        let screenshots = pictures.join("Screenshots");
        out.push(Place { label: label(&pictures), path: pictures, tint: Tint::Warning });
        out.push(Place { label: label(&screenshots), path: screenshots, tint: Tint::Info });
    }
    if let Some(downloads) = dirs.download {
        out.push(Place { label: label(&downloads), path: downloads, tint: Tint::Success });
    }
    out.retain(|p| p.path.is_dir());
    out
}

/// The picture on screen: its pixels as iced takes them, already turned,
/// and the size the transform works in.
struct Shown {
    path: PathBuf,
    decoded: Arc<Decoded>,
    handle: image::Handle,
    /// The picture's own size with the file's orientation *and* the
    /// user's turns applied, in image pixels. Not the decoded size: 1:1
    /// means one pixel of the photograph per logical pixel, however small
    /// the buffer it was decoded into.
    size: ImageSize,
}

/// What the header of a file says, for the inspector and the status bar
/// — read without decoding, so the grid can show it for a tile.
struct Details {
    path: PathBuf,
    measured: Option<Measured>,
    camera: Camera,
}

/// A folder's subfolders, summarised, as the sidebar and the library
/// read them.
enum Listing {
    Loading,
    Loaded(Vec<Summary>),
    Failed(String),
}

/// The 3D model on screen: the mesh, what the inspector says about it,
/// and the camera looking at it. Only the current item's model is ever
/// held — moving to anything else drops it, mesh and GPU copy both.
struct ModelView {
    path: PathBuf,
    mesh: Arc<hyprforge_mesh::Mesh>,
    facts: hyprforge_media::info::ModelFacts,
    generation: u64,
    camera: ModelCamera,
    /// A file that loaded but not entirely — an OBJ whose materials could
    /// not be read, drawn uncoloured.
    warning: Option<String>,
    /// The file's time when it was read, for autoreload.
    modified: Option<std::time::SystemTime>,
}

/// Where the seek bar is being held, or was let go.
///
/// Held after release too, until mpv reports a position near it: the
/// player's last report is from before the seek, and showing it would
/// snap the handle back for a moment before it jumped forward again.
#[derive(Debug, Clone, Copy, PartialEq)]
struct Seeking {
    target: f64,
    dragging: bool,
    since: Instant,
}

/// How close mpv's reported position must come to where the bar was let
/// go before the bar follows mpv again, and how long it waits at most.
const SEEK_SETTLE_SECONDS: f64 = 0.75;
const SEEK_SETTLE_WAIT: Duration = Duration::from_millis(1500);

/// A video in the pane.
struct VideoView {
    path: PathBuf,
    generation: u64,
    /// `None` when it could not start — `error` says why.
    player: Option<hyprforge_video::Player>,
    /// The latest frame and its serial, drawn by `film::FilmProgram`.
    frame: Option<(Arc<hyprforge_video::Frame>, u64)>,
    playback: hyprforge_video::Playback,
    error: Option<String>,
}

/// Thumbnails that cost a process or a mesh load are made a few at a time,
/// so opening a folder of videos does not start forty ffmpegs, nor a
/// folder of models load forty meshes into memory at once.
static EXPENSIVE_THUMBS: tokio::sync::Semaphore = tokio::sync::Semaphore::const_new(2);

/// The compact viewer's panels: off until asked for, and not saved — a
/// panel opened in a glance is not a preference.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Compact {
    sidebar: bool,
    info: bool,
}

/// Which way a drag on a model moves the camera.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ModelDrag {
    Turn,
    Pan,
}

/// A loaded mesh on its way from the blocking pool to the window. By hand
/// rather than derived, so a `Debug` of the message is a line and not every
/// vertex of the model.
#[derive(Clone)]
struct LoadedModel {
    mesh: Arc<hyprforge_mesh::Mesh>,
    warning: Option<String>,
    modified: Option<std::time::SystemTime>,
    /// Read with `modified`, so a reload's size is the new file's and not
    /// the listing's from before it changed.
    bytes: Option<u64>,
}

impl std::fmt::Debug for LoadedModel {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "LoadedModel({} triangles)", self.mesh.tri_count())
    }
}

/// A slideshow in progress.
struct Slideshow {
    show: Show,
    interval: Interval,
    /// When the picture on screen went up.
    since: Instant,
    /// When the pointer last moved, for the fading controls.
    pointer_moved: Instant,
    /// Whether the window was fullscreen before, so ending the show puts
    /// it back the way it was rather than always windowed.
    was_fullscreen: bool,
}

/// Something that has just happened and can be taken back.
enum Undo {
    Trashed { item: hyprforge_fileops::trash::TrashedItem, name: String },
}

struct App {
    keymap: hyprforge_media::keys::Keymap,
    order: hyprforge_listing::order::Order,
    prefs: prefs::Prefs,
    mode: Mode,
    folder: Folder,
    folder_path: Option<PathBuf>,
    /// The picture to select once the folder arrives.
    focus: Option<PathBuf>,
    loading_folder: bool,
    folder_error: Option<String>,
    history: History,
    /// The picture that was selected in each folder when it was left, so
    /// Back returns to where you were rather than to the first picture.
    last_selected: HashMap<PathBuf, PathBuf>,
    cache: Cache,
    /// Decodes in flight, so a picture is never decoded twice at once.
    decoding: HashSet<PathBuf>,
    /// Pictures that could not be decoded, with the sentence to show.
    failed: HashMap<PathBuf, String>,
    shown: Option<Shown>,
    turns: Turns,
    transform: Transform,
    details: Option<Details>,
    details_wanted: Option<PathBuf>,
    window: Option<window::Id>,
    window_size: Size,
    scale_factor: f32,
    fullscreen: bool,
    /// Where the pointer was at the last drag step, while dragging.
    dragging: Option<Point>,
    cursor: Option<Point>,
    /// Smooth-scroll pixels not yet worth a zoom step.
    scroll_pixels: f32,
    resize_generation: u64,
    /// Thumbnails for the filmstrip, the grid and the library covers:
    /// `None` for one that could not be made. Capped at [`MAX_THUMBS`].
    thumbs: HashMap<PathBuf, Option<image::Handle>>,
    thumbs_wanted: HashSet<PathBuf>,
    current_bytes: Option<u64>,
    places: Vec<Place>,
    /// Subfolder summaries, by the folder they are inside.
    folders: HashMap<PathBuf, Listing>,
    /// The folder whose subfolders Library mode is showing.
    library_dir: Option<PathBuf>,
    library_cursor: usize,
    /// Scroll offset and visible height of the grid and the library, as
    /// their scrollables last reported them.
    grid_scroll: (f32, f32),
    library_scroll: (f32, f32),
    /// The header's ⋯ menu.
    menu_open: bool,
    show: Option<Slideshow>,
    model: Option<ModelView>,
    /// The model being read now, so a second request for it waits rather
    /// than reading the file twice.
    model_loading: Option<PathBuf>,
    model_generation: u64,
    /// The drag in progress on a model, and where the pointer last was.
    model_drag: Option<(ModelDrag, Point)>,
    /// `Some` while the window is the compact viewer a picture opened on
    /// its own gets: which of the two panels the user has asked for since.
    /// Showing Grid or Library ends it — that is when the panels are
    /// needed — and the window returns to the shell and its usual size.
    compact: Option<Compact>,
    /// The picture to size the floating window to, until it has been.
    float_picture: Option<PathBuf>,
    /// Whether this window floated itself, so ending compact grows it back.
    floated: bool,
    /// The size the window mapped at to float from its first frame, until
    /// it has settled and taken back an ordinary minimum and maximum.
    planned_float: Option<(u32, u32)>,
    /// The video on screen and its player. Only the current item's — any
    /// other item drops it, which stops the sound.
    video: Option<VideoView>,
    /// Bumped per player, so updates from one already dropped are ignored.
    video_generation: u64,
    /// Where the seek bar is being dragged to, until it is let go.
    seeking: Option<Seeking>,
    /// The shared freedesktop cache, for video frames and model drawings.
    thumb_cache: Option<Arc<hyprforge_thumbnails::Cache>>,
    /// One line in the status bar: a problem to report, or what just
    /// happened. Replaced by the next one; closed by the user.
    notice: Option<String>,
    undo: Option<Undo>,
    clipboard: Arc<hyprforge_clipboard::WaylandWriter>,
    font_scale: FontScale,
}

#[derive(Debug, Clone)]
enum Message {
    WindowFound(Option<window::Id>),
    ScaleFactor(f32),
    FolderRead(PathBuf, Result<Vec<hyprforge_listing::types::Entry>, String>),
    Summaries(PathBuf, Result<Vec<Summary>, String>),
    Decoded(PathBuf, Result<Arc<Decoded>, String>),
    Details(PathBuf, Option<Measured>, Camera),
    Thumb(PathBuf, Option<image::Handle>),
    Key(hyprforge_keys::KeyPress),
    Perform(Action),
    /// A single click on a filmstrip tile or a grid tile.
    Select(usize),
    /// A double click on a grid tile: open it in Photo mode.
    OpenTile(usize),
    SelectCard(usize),
    OpenCard(usize),
    /// A sidebar row, a path crumb, or anything else that goes to a
    /// folder.
    OpenFolder(PathBuf),
    SetMode(Mode),
    GridScrolled(iced::widget::scrollable::Viewport),
    LibraryScrolled(iced::widget::scrollable::Viewport),
    Scrolled(mouse::ScrollDelta),
    PointerMoved(Point),
    DragStart,
    DragEnd,
    ToggleZoom,
    Resized(Size),
    ResizeSettled(u64),
    ToggleMenu,
    MenuAction(Action),
    Tick(Instant),
    /// A model read from disk; `true` when it replaces the one on screen
    /// after its file changed, keeping the view.
    ModelLoaded(PathBuf, bool, Result<LoadedModel, String>),
    ModelPress(ModelDrag),
    ModelRelease,
    ModelPointer(Point),
    ModelScrolled(mouse::ScrollDelta),
    CheckModelFile,
    SetDrawMode(DrawMode),
    /// The window floated (or did not) — logged, never shown.
    Floated(Result<(), String>),
    /// Something from the player numbered `u64`.
    Video(u64, hyprforge_video::Update),
    /// The seek bar dragged to a point, and let go.
    SeekTo(f64),
    SeekRelease,
    SlideshowPause,
    SlideshowShuffle,
    SlideshowLoop,
    SlideshowInterval(Interval),
    Trashed(PathBuf, Result<hyprforge_fileops::trash::TrashedItem, String>),
    UndoPressed,
    Restored(Result<String, String>),
    Done(Result<String, String>),
    DismissNotice,
}

impl App {
    fn title(&self) -> String {
        match (self.mode, self.folder.current(), &self.folder_path) {
            (Mode::Photo, Some(item), _) => format!("{} — Media", item.name),
            (_, _, Some(dir)) => format!("{} — Media", display_name(dir)),
            _ => "Media".to_string(),
        }
    }

    fn theme(&self) -> Theme {
        hyprforge_ui::theme::app_theme()
    }

    fn subscription(&self) -> Subscription<Message> {
        let tick = match &self.show {
            // Four times a second: fine enough for the controls to fade
            // on time, coarse enough to cost nothing, and only while a
            // show is running.
            Some(_) => iced::time::every(Duration::from_millis(250)).map(Message::Tick),
            None => Subscription::none(),
        };
        // Only while a model is on screen and the user wants it followed.
        let watch = match (&self.model, self.prefs.model_autoreload) {
            (Some(_), true) => iced::time::every(MODEL_CHECK_EVERY).map(|_| Message::CheckModelFile),
            _ => Subscription::none(),
        };
        Subscription::batch([
            keyboard::listen().filter_map(|event| hyprforge_ui::keys::key_press(&event).map(Message::Key)),
            window::resize_events().map(|(_, size)| Message::Resized(size)),
            tick,
            watch,
        ])
    }

    // --- geometry ------------------------------------------------------

    fn sidebar_shown(&self) -> bool {
        !self.fullscreen && self.compact.map_or(self.prefs.sidebar, |c| c.sidebar)
    }

    fn inspector_shown(&self) -> bool {
        !self.fullscreen && self.compact.map_or(self.prefs.info_panel, |c| c.info)
    }

    /// Whether the inspector is on — what the header's `i` shows lit.
    fn info_on(&self) -> bool {
        self.compact.map_or(self.prefs.info_panel, |c| c.info)
    }

    /// Whether the sidebar is on — what the header's toggle shows filled.
    fn sidebar_on(&self) -> bool {
        self.compact.map_or(self.prefs.sidebar, |c| c.sidebar)
    }

    /// Floats this window and sizes it to the picture it opened on. Once,
    /// off the UI thread, and only on Hyprland — see `float.rs`.
    fn float_to_picture(&mut self) -> Task<Message> {
        let Some(path) = self.float_picture.take() else { return Task::none() };
        if !hyprforge_media::float::on_hyprland() {
            return Task::none();
        }
        // Mapped at its planned size, fixed: it should already be floating,
        // and this only confirms it — dispatching only if some rule tiled
        // it anyway.
        if let Some(size) = self.planned_float {
            return Task::perform(
                async move {
                    tokio::task::spawn_blocking(move || hyprforge_media::float::settle(size))
                        .await
                        .unwrap_or_else(|e| Err(e.to_string()))
                },
                Message::Floated,
            );
        }
        // The chrome around the picture in compact: header, status bar and
        // filmstrip, each with its 1px edge.
        let scale = self.font_scale;
        let mut chrome_h = hyprforge_ui::density::bar_height(scale) + 1.0 + scale.apply(STATUS_HEIGHT) + 1.0;
        if self.prefs.filmstrip {
            chrome_h += FILMSTRIP_HEIGHT + 1.0;
        }
        Task::perform(
            async move {
                tokio::task::spawn_blocking(move || {
                    let picture = hyprforge_image::measure(&path).ok().map(|m| m.display_size());
                    let usable = hyprforge_media::float::focused_monitor()?;
                    let size = hyprforge_media::float::floating_size(picture, usable, (0.0, chrome_h));
                    hyprforge_media::float::float_self(size)
                })
                .await
                .unwrap_or_else(|e| Err(e.to_string()))
            },
            Message::Floated,
        )
    }

    /// Leaves the compact viewer for the whole shell, growing a floating
    /// window to the size the shell was last left at.
    fn expand(&mut self) -> Task<Message> {
        if self.compact.take().is_none() || !self.floated {
            return Task::none();
        }
        let size = (self.prefs.window_width, self.prefs.window_height);
        Task::perform(
            async move {
                tokio::task::spawn_blocking(move || hyprforge_media::float::resize_self(size))
                    .await
                    .unwrap_or_else(|e| Err(e.to_string()))
            },
            Message::Floated,
        )
    }

    fn filmstrip_shown(&self) -> bool {
        self.prefs.filmstrip && !self.fullscreen && self.mode == Mode::Photo
    }

    /// The width between the sidebar and the inspector, logical pixels.
    fn pane_width(&self) -> f32 {
        let mut width = self.window_size.width;
        if self.sidebar_shown() {
            width -= SIDEBAR_WIDTH + 1.0;
        }
        if self.inspector_shown() {
            width -= INSPECTOR_WIDTH + 1.0;
        }
        width.max(1.0)
    }

    /// The area the picture is drawn in, logical pixels: the window less
    /// whatever chrome is showing.
    fn viewport(&self) -> Viewport {
        if self.show.is_some() {
            return Viewport { width: self.window_size.width.max(1.0), height: self.window_size.height.max(1.0) };
        }
        let mut height = self.window_size.height;
        if !self.fullscreen {
            height -= hyprforge_ui::density::bar_height(self.font_scale) + 1.0;
            height -= self.font_scale.apply(STATUS_HEIGHT) + 1.0;
        }
        if self.filmstrip_shown() {
            height -= FILMSTRIP_HEIGHT;
        }
        Viewport { width: self.pane_width(), height: height.max(1.0) }
    }

    /// How much may be decoded for the viewport as it is now.
    fn budget(&self) -> Budget {
        let v = self.viewport();
        Budget::for_viewport(ViewportPixels::from_logical(v.width, v.height, self.scale_factor))
    }

    /// The grid's (or the library's) column count and tile sizes for the
    /// pane as it is now. `lines` is how many lines of text sit under a
    /// tile's picture: one for a photo's name, two for a folder card.
    fn tiles(&self, lines: usize) -> Tiles {
        let inner = (self.pane_width() - 2.0 * GRID_PAD).max(TILE_MIN_WIDTH);
        let columns = (((inner + TILE_GAP) / (TILE_MIN_WIDTH + TILE_GAP)).floor() as usize).max(1);
        let tile = (inner - TILE_GAP * (columns - 1) as f32) / columns as f32;
        let picture = ((tile - 2.0 * TILE_PAD) * 2.0 / 3.0).floor();
        let row = TILE_PAD + picture + TILE_PAD + TILE_LINE * lines as f32 + TILE_PAD;
        Tiles {
            width: tile,
            picture,
            metrics: grid::Metrics {
                columns,
                header: GROUP_HEADER,
                row,
                row_gap: TILE_GAP,
                group_gap: GROUP_GAP,
            },
        }
    }

    fn groups(&self) -> Vec<grid::Group> {
        grid::groups(self.folder.items(), &chrono::Local)
    }

    fn library_summaries(&self) -> &[Summary] {
        match self.library_dir.as_ref().and_then(|d| self.folders.get(d)) {
            Some(Listing::Loaded(list)) => list,
            _ => &[],
        }
    }

    // --- update --------------------------------------------------------

    fn update(&mut self, message: Message) -> Task<Message> {
        match message {
            Message::WindowFound(id) => {
                self.window = id;
                let float = self.float_to_picture();
                match id {
                    Some(id) => Task::batch([window::scale_factor(id).map(Message::ScaleFactor), float]),
                    None => float,
                }
            }
            Message::Video(generation, update) => {
                let Some(video) = self.video.as_mut().filter(|v| v.generation == generation) else {
                    return Task::none();
                };
                match update {
                    hyprforge_video::Update::Frame(frame) => {
                        let serial = video.frame.as_ref().map_or(0, |(_, s)| s + 1);
                        video.frame = Some((frame, serial));
                    }
                    hyprforge_video::Update::Playback(playback) => {
                        // Width and height arrive as separate properties,
                        // so "known" means both — (1280, 0) is not a shape.
                        let shaped = |p: &hyprforge_video::Playback| p.video_size.is_some_and(|(w, h)| w > 0 && h > 0);
                        let first_size = !shaped(&video.playback) && shaped(&playback);
                        if let Some(seek) = self.seeking.filter(|s| !s.dragging) {
                            if (playback.position - seek.target).abs() < SEEK_SETTLE_SECONDS
                                || seek.since.elapsed() > SEEK_SETTLE_WAIT
                            {
                                self.seeking = None;
                            }
                        }
                        video.playback = playback;
                        // Now the shape is known, frames can be drawn at it
                        // and mpv adds no bars of its own.
                        if first_size {
                            self.size_video();
                        }
                    }
                    hyprforge_video::Update::Failed(e) => video.error = Some(e),
                }
                Task::none()
            }
            // Scrubbing: the picture follows the bar while it is dragged,
            // by fast keyframe seeks — the player collapses a run of them
            // into the last — and lands exactly where it is let go.
            Message::SeekTo(seconds) => {
                self.seeking = Some(Seeking { target: seconds, dragging: true, since: Instant::now() });
                if let Some(player) = self.video.as_ref().and_then(|v| v.player.as_ref()) {
                    player.send(hyprforge_video::Command::Seek { seconds, relative: false, exact: false });
                }
                Task::none()
            }
            Message::SeekRelease => {
                if let Some(seek) = self.seeking {
                    if let Some(player) = self.video.as_ref().and_then(|v| v.player.as_ref()) {
                        player.send(hyprforge_video::Command::Seek { seconds: seek.target, relative: false, exact: true });
                    }
                    self.seeking = Some(Seeking { dragging: false, since: Instant::now(), ..seek });
                }
                Task::none()
            }
            Message::Floated(result) => {
                match result {
                    Ok(()) => self.floated = true,
                    Err(e) => tracing::warn!(error = %e, "the viewer could not float itself"),
                }
                // Floating now, or never going to be: either way the
                // window takes back its ordinary limits and can be resized.
                match (self.planned_float.take(), self.window) {
                    (Some(_), Some(id)) => Task::batch([
                        window::set_resizable(id, true),
                        window::set_max_size(id, None),
                        window::set_min_size(id, Some(Size::new(640.0, 420.0))),
                    ]),
                    _ => Task::none(),
                }
            }
            Message::ScaleFactor(scale) => {
                if (scale - self.scale_factor).abs() > f32::EPSILON {
                    self.scale_factor = scale;
                    // Everything held was decoded for a density this
                    // window does not have; the picture on screen stays
                    // up until its sharper replacement arrives.
                    self.cache = Cache::default();
                    self.thumbs.clear();
                    return Task::batch([self.redecode_current(), self.request_thumbs()]);
                }
                Task::none()
            }
            Message::FolderRead(dir, result) => {
                // A folder the user has since left: its listing is of no
                // use, and applying it would show the wrong pictures.
                if self.folder_path.as_deref() != Some(dir.as_path()) {
                    return Task::none();
                }
                self.loading_folder = false;
                match result {
                    Ok(entries) => {
                        self.folder = Folder::build(entries, &self.order);
                        if let Some(path) = self.focus.take() {
                            // A remembered selection that has since gone
                            // (trashed, renamed) is simply not there; only
                            // a file that exists and cannot be shown is
                            // worth a sentence.
                            if !self.folder.focus_on(&path) && path.exists() {
                                self.notice =
                                    Some(format!("{} isn't a picture this viewer can show.", path.display()));
                            }
                        }
                        self.after_selection_moved()
                    }
                    Err(e) => {
                        self.folder_error = Some(e);
                        Task::none()
                    }
                }
            }
            Message::Summaries(dir, result) => {
                let listing = match result {
                    Ok(list) => {
                        if self.library_dir.as_deref() == Some(dir.as_path()) {
                            if let Some(i) = self.folder_path.as_ref().and_then(|f| list.iter().position(|s| &s.path == f)) {
                                self.library_cursor = i;
                            }
                        }
                        Listing::Loaded(list)
                    }
                    Err(e) => Listing::Failed(e),
                };
                self.folders.insert(dir, listing);
                self.request_thumbs()
            }
            Message::Decoded(path, result) => {
                self.decoding.remove(&path);
                match result {
                    Ok(decoded) => {
                        self.cache.put(&path, decoded.clone());
                        if self.current_path().as_deref() == Some(path.as_path()) {
                            self.present(path, decoded);
                        }
                    }
                    Err(e) => {
                        self.failed.insert(path, e);
                    }
                }
                Task::none()
            }
            Message::Details(path, measured, camera) => {
                if self.details_wanted.as_deref() == Some(path.as_path()) {
                    self.details_wanted = None;
                }
                if self.current_path().as_deref() == Some(path.as_path()) {
                    self.details = Some(Details { path, measured, camera });
                }
                Task::none()
            }
            Message::Thumb(path, handle) => {
                self.thumbs_wanted.remove(&path);
                self.thumbs.insert(path, handle);
                Task::none()
            }
            Message::Key(press) => match self.keymap.resolve(&press) {
                Some(Resolved::Action(action)) => self.perform(action),
                // Nowhere here takes typing — see `keys::BARE_KEYS`.
                Some(Resolved::Text(_)) | None => Task::none(),
            },
            Message::Perform(action) => self.perform(action),
            Message::Select(index) => {
                if self.folder.select(index) {
                    return self.after_selection_moved();
                }
                Task::none()
            }
            Message::OpenTile(index) => {
                if self.folder.select(index) {
                    return self.set_mode(Mode::Photo);
                }
                Task::none()
            }
            Message::SelectCard(index) => {
                self.library_cursor = index;
                Task::none()
            }
            Message::OpenCard(index) => {
                self.library_cursor = index;
                self.open_card()
            }
            Message::OpenFolder(dir) => {
                self.menu_open = false;
                self.open_folder(dir, None)
            }
            Message::SetMode(mode) => self.set_mode(mode),
            Message::GridScrolled(viewport) => {
                self.grid_scroll = (viewport.absolute_offset().y, viewport.bounds().height);
                self.request_thumbs()
            }
            Message::LibraryScrolled(viewport) => {
                self.library_scroll = (viewport.absolute_offset().y, viewport.bounds().height);
                self.request_thumbs()
            }
            Message::Scrolled(delta) => {
                let steps = match delta {
                    mouse::ScrollDelta::Lines { y, .. } => y.signum() as i32,
                    mouse::ScrollDelta::Pixels { y, .. } => {
                        self.scroll_pixels += y;
                        let steps = (self.scroll_pixels / PIXELS_PER_ZOOM_STEP).trunc();
                        self.scroll_pixels -= steps * PIXELS_PER_ZOOM_STEP;
                        steps as i32
                    }
                };
                if steps != 0 {
                    if let Some(shown) = &self.shown {
                        let viewport = self.viewport();
                        let at = self.cursor.unwrap_or(Point::new(viewport.width / 2.0, viewport.height / 2.0));
                        self.transform.zoom_about(steps, LogicalPoint { x: at.x, y: at.y }, shown.size, viewport);
                    }
                }
                Task::none()
            }
            Message::PointerMoved(point) => {
                if let (Some(last), Some(shown)) = (self.dragging, &self.shown) {
                    let viewport = self.viewport();
                    self.transform.pan_by(point.x - last.x, point.y - last.y, shown.size, viewport);
                    self.dragging = Some(point);
                }
                self.cursor = Some(point);
                if let Some(show) = &mut self.show {
                    show.pointer_moved = Instant::now();
                }
                Task::none()
            }
            Message::DragStart => {
                self.menu_open = false;
                self.dragging = self.cursor;
                Task::none()
            }
            Message::DragEnd => {
                self.dragging = None;
                Task::none()
            }
            Message::ToggleZoom => {
                if self.transform == Transform::default() {
                    self.transform.actual_size();
                } else {
                    self.transform.fit();
                }
                Task::none()
            }
            Message::Resized(size) => {
                self.window_size = size;
                self.size_video();
                if let Some(shown) = &self.shown {
                    self.transform.clamp_pan(shown.size, self.viewport());
                }
                self.resize_generation += 1;
                let generation = self.resize_generation;
                Task::perform(
                    tokio::time::sleep(Duration::from_millis(RESIZE_SETTLE_MS)),
                    move |_| Message::ResizeSettled(generation),
                )
            }
            Message::ResizeSettled(generation) => {
                if generation != self.resize_generation || self.fullscreen {
                    return Task::none();
                }
                // The compact viewer's size is the picture's, not a choice
                // about the shell — saving it would open the next library
                // session at the shape of the last photograph.
                if self.compact.is_none() {
                    let (width, height) = (self.window_size.width as u32, self.window_size.height as u32);
                    save_pref(|p| {
                        p.window_width = width;
                        p.window_height = height;
                    });
                }
                Task::batch([self.sharpen_if_needed(), self.request_thumbs()])
            }
            Message::ToggleMenu => {
                self.menu_open = !self.menu_open;
                Task::none()
            }
            Message::MenuAction(action) => {
                self.menu_open = false;
                self.perform(action)
            }
            Message::Tick(now) => self.slideshow_tick(now),
            Message::ModelLoaded(path, reload, result) => {
                if self.model_loading.as_deref() == Some(path.as_path()) {
                    self.model_loading = None;
                }
                // A model the user has since moved past is dropped here
                // rather than held: only the current item's mesh is kept.
                if self.current_path().as_deref() != Some(path.as_path()) {
                    return Task::none();
                }
                match result {
                    Ok(loaded) => {
                        let format = hyprforge_mesh::detect(&path).unwrap_or(hyprforge_mesh::Format::Stl);
                        let mut camera = match (&self.model, reload) {
                            (Some(old), true) if old.path == path => old.camera,
                            _ => ModelCamera::default(),
                        };
                        camera.perspective = self.prefs.model_projection().value();
                        let b = loaded.mesh.bounds;
                        camera.fit(b.min, b.max, reload, true);
                        self.model_generation += 1;
                        self.failed.remove(&path);
                        if loaded.bytes.is_some() {
                            self.current_bytes = loaded.bytes;
                        }
                        self.model = Some(ModelView {
                            facts: hyprforge_media::info::ModelFacts::of(format, &loaded.mesh),
                            path,
                            mesh: loaded.mesh,
                            generation: self.model_generation,
                            camera,
                            warning: loaded.warning,
                            modified: loaded.modified,
                        });
                    }
                    Err(e) => {
                        // A reload that fails mid-save keeps the last good
                        // model up: a slicer writing the file in pieces
                        // must not blank the preview between them.
                        if !reload {
                            self.model = None;
                            self.failed.insert(path, e);
                        }
                    }
                }
                Task::none()
            }
            Message::ModelPress(drag) => {
                self.menu_open = false;
                self.model_drag = self.cursor.map(|at| (drag, at));
                Task::none()
            }
            Message::ModelRelease => {
                self.model_drag = None;
                Task::none()
            }
            Message::ModelPointer(point) => {
                let viewport = self.viewport();
                if let (Some((drag, last)), Some(model)) = (self.model_drag, self.model.as_mut()) {
                    let (w, h) = (viewport.width, viewport.height);
                    match drag {
                        ModelDrag::Turn => model.camera.rotate([last.x, last.y], [point.x, point.y], w, h),
                        ModelDrag::Pan => model.camera.pan([point.x - last.x, point.y - last.y], w, h),
                    }
                    self.model_drag = Some((drag, point));
                }
                self.cursor = Some(point);
                if let Some(show) = &mut self.show {
                    show.pointer_moved = Instant::now();
                }
                Task::none()
            }
            Message::ModelScrolled(delta) => {
                let viewport = self.viewport();
                let units = match delta {
                    mouse::ScrollDelta::Lines { y, .. } => y * MODEL_UNITS_PER_LINE,
                    mouse::ScrollDelta::Pixels { y, .. } => y,
                };
                if let Some(model) = self.model.as_mut() {
                    let at = self.cursor.map_or([viewport.width / 2.0, viewport.height / 2.0], |p| [p.x, p.y]);
                    model.camera.zoom_at(at, units, false, viewport.width, viewport.height);
                }
                Task::none()
            }
            Message::CheckModelFile => {
                let Some(model) = &self.model else { return Task::none() };
                if self.model_loading.is_some() {
                    return Task::none();
                }
                let now = std::fs::metadata(&model.path).and_then(|m| m.modified()).ok();
                if now.is_some() && now != model.modified {
                    let path = model.path.clone();
                    return self.load_model(path, true);
                }
                Task::none()
            }
            Message::SetDrawMode(mode) => {
                self.set_draw_mode(mode);
                Task::none()
            }
            Message::SlideshowPause => {
                if let Some(show) = &mut self.show {
                    show.show.paused = !show.show.paused;
                    show.since = Instant::now();
                }
                Task::none()
            }
            Message::SlideshowShuffle => {
                if let Some(show) = &mut self.show {
                    let seed = std::time::SystemTime::now()
                        .duration_since(std::time::UNIX_EPOCH)
                        .map(|d| d.as_nanos() as u64)
                        .unwrap_or(1);
                    let on = !show.show.is_shuffled();
                    show.show.set_shuffled(on, seed);
                }
                Task::none()
            }
            Message::SlideshowLoop => {
                if let Some(show) = &mut self.show {
                    show.show.looping = !show.show.looping;
                    let on = show.show.looping;
                    self.prefs.slideshow_loop = on;
                    save_pref(|p| p.slideshow_loop = on);
                }
                Task::none()
            }
            Message::SlideshowInterval(interval) => {
                if let Some(show) = &mut self.show {
                    show.interval = interval;
                }
                self.prefs.slideshow_seconds = interval.seconds();
                save_pref(|p| p.slideshow_seconds = interval.seconds());
                Task::none()
            }
            Message::Trashed(path, result) => match result {
                Ok(item) => {
                    let name = self.folder.current().map(|i| i.name.clone()).unwrap_or_default();
                    if self.current_path().as_deref() == Some(path.as_path()) {
                        self.folder.remove_current();
                    }
                    self.cache.remove(&path);
                    self.thumbs.remove(&path);
                    self.notice = Some(format!("Moved {name} to the trash."));
                    self.undo = Some(Undo::Trashed { item, name });
                    // The sidebar's count for this folder is now one high.
                    self.refresh_parent_summaries();
                    self.after_selection_moved()
                }
                Err(e) => {
                    self.notice = Some(e);
                    Task::none()
                }
            },
            Message::UndoPressed => {
                let Some(Undo::Trashed { item, name }) = self.undo.take() else {
                    return Task::none();
                };
                Task::perform(
                    async move {
                        tokio::task::spawn_blocking(move || {
                            hyprforge_fileops::trash::restore(&item)
                                .map(|()| name.clone())
                                .map_err(|e| format!("Couldn't put {name} back: {e}"))
                        })
                        .await
                        .unwrap_or_else(|e| Err(e.to_string()))
                    },
                    Message::Restored,
                )
            }
            Message::Restored(result) => match result {
                Ok(name) => {
                    self.notice = Some(format!("Put {name} back."));
                    self.refresh_parent_summaries();
                    match self.folder_path.clone() {
                        Some(dir) => {
                            // Re-read rather than re-insert: where it
                            // belongs in the order is the listing's
                            // question, not this window's.
                            let focus = self.current_path();
                            self.focus = focus;
                            self.loading_folder = true;
                            read_folder(dir)
                        }
                        None => Task::none(),
                    }
                }
                Err(e) => {
                    self.notice = Some(e);
                    Task::none()
                }
            },
            Message::Done(result) => {
                self.notice = Some(match result {
                    Ok(said) | Err(said) => said,
                });
                Task::none()
            }
            Message::DismissNotice => {
                self.notice = None;
                self.undo = None;
                Task::none()
            }
        }
    }

    fn current_path(&self) -> Option<PathBuf> {
        self.folder.current().map(|i| i.path.clone())
    }

    // --- folders and modes -----------------------------------------------

    /// Goes to `dir` as a new step in the history.
    fn open_folder(&mut self, dir: PathBuf, focus: Option<PathBuf>) -> Task<Message> {
        if self.folder_path.as_deref() == Some(dir.as_path()) && focus.is_none() {
            return Task::none();
        }
        if let Some(from) = self.folder_path.clone() {
            self.history.leave(from);
        }
        self.load_folder(dir, focus)
    }

    /// Shows `dir`, without touching the history — what Back and Forward
    /// use, and the first folder at startup.
    fn load_folder(&mut self, dir: PathBuf, focus: Option<PathBuf>) -> Task<Message> {
        if let (Some(from), Some(selected)) = (self.folder_path.clone(), self.current_path()) {
            self.last_selected.insert(from, selected);
        }
        let focus = focus.or_else(|| self.last_selected.get(&dir).cloned());
        self.folder_path = Some(dir.clone());
        self.folder = Folder::default();
        self.focus = focus;
        self.loading_folder = true;
        self.folder_error = None;
        self.shown = None;
        self.details = None;
        self.grid_scroll.0 = 0.0;
        let mut tasks = vec![
            read_folder(dir.clone()),
            iced::widget::operation::scroll_to(grid_id(), iced::widget::operation::AbsoluteOffset { x: None, y: Some(0.0) }),
        ];
        if let Some(parent) = dir.parent() {
            tasks.push(self.ensure_summaries(parent.to_path_buf()));
        }
        // The library follows the folder: a new folder means its parent's
        // cards, with it selected, the next time Library is shown — and
        // now, if Library is what is showing.
        self.library_dir = None;
        if self.mode == Mode::Library {
            tasks.push(self.set_mode(Mode::Library));
        }
        Task::batch(tasks)
    }

    /// Reads `dir`'s subfolder summaries if nothing has asked yet.
    fn ensure_summaries(&mut self, dir: PathBuf) -> Task<Message> {
        if matches!(self.folders.get(&dir), Some(Listing::Loading | Listing::Loaded(_))) {
            return Task::none();
        }
        self.folders.insert(dir.clone(), Listing::Loading);
        read_summaries(dir, self.order)
    }

    /// The counts beside the current folder's siblings have changed.
    fn refresh_parent_summaries(&mut self) {
        if let Some(parent) = self.folder_path.as_ref().and_then(|d| d.parent()) {
            self.folders.remove(parent);
        }
    }

    fn set_mode(&mut self, mode: Mode) -> Task<Message> {
        self.menu_open = false;
        self.mode = mode;
        // Grid and Library are where the sidebar is needed.
        let grow = if mode == Mode::Photo { Task::none() } else { self.expand() };
        let shown = self.set_mode_inner(mode);
        Task::batch([grow, shown])
    }

    fn set_mode_inner(&mut self, mode: Mode) -> Task<Message> {
        // Nothing plays behind the grid or the library.
        if mode != Mode::Photo {
            self.video = None;
            self.seeking = None;
        }
        match mode {
            Mode::Photo => self.show_current(),
            Mode::Grid => {
                let mut tasks = vec![self.request_thumbs()];
                if let Some(index) = (!self.folder.is_empty()).then_some(self.folder.cursor()) {
                    tasks.push(self.scroll_grid_to(index));
                }
                Task::batch(tasks)
            }
            Mode::Library => {
                let dir = match &self.library_dir {
                    Some(dir) => dir.clone(),
                    None => {
                        let Some(current) = &self.folder_path else { return Task::none() };
                        let dir = current.parent().map(Path::to_path_buf).unwrap_or_else(|| current.clone());
                        // Start on the folder you came from. The sidebar
                        // has usually loaded this listing already, so the
                        // same choice made when it arrives would never run.
                        self.library_cursor = match self.folders.get(&dir) {
                            Some(Listing::Loaded(list)) => list.iter().position(|s| &s.path == current).unwrap_or(0),
                            _ => 0,
                        };
                        self.library_dir = Some(dir.clone());
                        dir
                    }
                };
                Task::batch([self.ensure_summaries(dir), self.request_thumbs()])
            }
        }
    }

    /// Enter on a library card, or a double click: a folder with pictures
    /// opens as its grid; one with none is a level of the tree, and the
    /// library goes down into it instead.
    fn open_card(&mut self) -> Task<Message> {
        let Some(card) = self.library_summaries().get(self.library_cursor).cloned() else {
            return Task::none();
        };
        if card.photos + card.models == 0 {
            self.library_dir = Some(card.path.clone());
            self.library_cursor = 0;
            return Task::batch([self.ensure_summaries(card.path), self.request_thumbs()]);
        }
        self.mode = Mode::Grid;
        self.open_folder(card.path, None)
    }

    /// Library mode one level up, keeping the folder you came from
    /// selected.
    fn library_up(&mut self) -> Task<Message> {
        let Some(dir) = self.library_dir.clone() else {
            return self.set_mode(Mode::Library);
        };
        let Some(parent) = dir.parent().map(Path::to_path_buf) else { return Task::none() };
        self.library_dir = Some(parent.clone());
        self.library_cursor = match self.folders.get(&parent) {
            Some(Listing::Loaded(list)) => list.iter().position(|s| s.path == dir).unwrap_or(0),
            _ => 0,
        };
        Task::batch([self.ensure_summaries(parent), self.request_thumbs()])
    }

    /// Everything that follows the selection moving, in whichever mode.
    fn after_selection_moved(&mut self) -> Task<Message> {
        match self.mode {
            Mode::Photo => self.show_current(),
            Mode::Grid | Mode::Library => {
                let mut tasks = vec![self.request_details(), self.request_thumbs()];
                if self.mode == Mode::Grid && !self.folder.is_empty() {
                    tasks.push(self.scroll_grid_to(self.folder.cursor()));
                }
                Task::batch(tasks)
            }
        }
    }

    fn scroll_grid_to(&self, index: usize) -> Task<Message> {
        let tiles = self.tiles(1);
        let bands = grid::bands(&self.groups(), tiles.metrics);
        let (top, height) = self.grid_scroll;
        match grid::scroll_to_show(&bands, tiles.metrics, index, top, height - 2.0 * GRID_PAD) {
            Some(y) => iced::widget::operation::scroll_to(
                grid_id(),
                iced::widget::operation::AbsoluteOffset { x: None, y: Some(y) },
            ),
            None => Task::none(),
        }
    }

    /// The new picture in Photo mode (from the cache, or decoded), its
    /// neighbours made ready, the filmstrip's thumbnails, and the
    /// details for the inspector and the status bar.
    fn show_current(&mut self) -> Task<Message> {
        self.transform = Transform::default();
        self.turns = Turns::none();
        let Some(item) = self.folder.current().cloned() else {
            self.shown = None;
            return Task::none();
        };
        self.current_bytes = item.bytes;

        // A video plays in the pane; anything else lets go of the player,
        // which stops its sound.
        if item.media == Media::Clip {
            self.shown = None;
            self.model = None;
            let mut tasks = vec![self.request_details(), self.request_thumbs()];
            if self.video.as_ref().is_none_or(|v| v.path != item.path) {
                tasks.push(self.open_video(item.path.clone()));
            }
            return Task::batch(tasks);
        }
        self.video = None;
        self.seeking = None;

        // A model is drawn by the model view, not decoded: hold its mesh,
        // and let go of any other — one model's geometry at a time.
        if item.media == Media::Model {
            self.shown = None;
            let mut tasks = vec![self.request_details(), self.request_thumbs()];
            if self.model.as_ref().is_none_or(|m| m.path != item.path) {
                self.model = None;
                tasks.push(self.load_model(item.path.clone(), false));
            }
            return Task::batch(tasks);
        }
        self.model = None;
        self.model_drag = None;

        let mut tasks = Vec::new();
        match self.cache.get(&item.path) {
            Some(decoded) => self.present(item.path.clone(), decoded),
            None => {
                // The previous picture stays up until this one is ready,
                // unless it is a clip, which has nothing to show.
                if item.media == Media::Clip {
                    self.shown = None;
                }
                tasks.push(self.decode(item.path.clone()));
            }
        }
        // The pictures either side, so paging does not wait on a decode.
        for neighbour in self.neighbours() {
            if self.cache.get(&neighbour).is_none() {
                tasks.push(self.decode(neighbour));
            }
        }
        tasks.push(self.request_details());
        tasks.push(self.request_thumbs());
        Task::batch(tasks)
    }

    /// Starts playing `path` in the pane. A machine without libmpv gets a
    /// sentence where the video would be, and everything else still works.
    fn open_video(&mut self, path: PathBuf) -> Task<Message> {
        self.video_generation += 1;
        let generation = self.video_generation;
        let root = hyprforge_ui::theme::surface::root();
        let to_byte = |v: f32| (v.clamp(0.0, 1.0) * 255.0).round() as u8;
        let options = hyprforge_video::Options {
            background: [to_byte(root.r), to_byte(root.g), to_byte(root.b)],
            size: self.video_pixels(),
            // A slideshow plays each video through; a video opened by
            // paging to it plays too — it is what paging to a video means.
            paused: false,
            muted: false,
        };
        let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
        match hyprforge_video::Player::open(&path, options, move |update| {
            let _ = tx.send(update);
        }) {
            Ok(player) => {
                self.video = Some(VideoView {
                    path,
                    generation,
                    player: Some(player),
                    frame: None,
                    playback: hyprforge_video::Playback::default(),
                    error: None,
                });
                // Until the player is dropped and its thread lets go of
                // the sender, which ends the stream.
                let stream = iced::futures::stream::poll_fn(move |cx| rx.poll_recv(cx));
                Task::run(stream, move |update| Message::Video(generation, update))
            }
            Err(e) => {
                let said = match e {
                    hyprforge_video::MpvError::Missing => {
                        "Videos play through mpv, which isn't installed here. Shift+Enter opens this one in another player."
                            .to_string()
                    }
                    other => other.to_string(),
                };
                self.video = Some(VideoView {
                    path,
                    generation,
                    player: None,
                    frame: None,
                    playback: hyprforge_video::Playback::default(),
                    error: Some(said),
                });
                Task::none()
            }
        }
    }

    /// The pane's size in physical pixels — what the player draws frames at.
    fn video_pixels(&self) -> (u32, u32) {
        let v = self.viewport();
        ((v.width * self.scale_factor).round() as u32, (v.height * self.scale_factor).round() as u32)
    }

    /// Tells the player the pane changed size: frames drawn at the video's
    /// own shape fitted inside the pane, once the shape is known, so mpv
    /// letterboxes nothing (its bars are black whatever it is told) and
    /// the themed pane shows around the picture instead.
    fn size_video(&self) {
        let Some(video) = &self.video else { return };
        let Some(player) = &video.player else { return };
        let (pw, ph) = self.video_pixels();
        let (w, h) = match video.playback.video_size.filter(|(w, h)| *w > 0 && *h > 0) {
            Some((vw, vh)) => {
                let k = (pw as f32 / vw as f32).min(ph as f32 / vh as f32);
                ((vw as f32 * k).round() as u32, (vh as f32 * k).round() as u32)
            }
            None => (pw, ph),
        };
        player.send(hyprforge_video::Command::Size(w.max(16), h.max(1)));
    }

    fn video_on_screen(&self) -> bool {
        self.mode == Mode::Photo && self.folder.current().is_some_and(|i| i.media == Media::Clip)
    }

    /// Space on a video: pause, play, or from the start again at the end.
    fn play_pause(&mut self) {
        if let Some(video) = &self.video {
            if let Some(player) = &video.player {
                player.send(if video.playback.ended {
                    hyprforge_video::Command::Restart
                } else {
                    hyprforge_video::Command::TogglePause
                });
            }
        }
    }

    /// Reads a model off the UI thread. `reload` keeps the view when the
    /// same file is read again.
    fn load_model(&mut self, path: PathBuf, reload: bool) -> Task<Message> {
        if self.model_loading.as_deref() == Some(path.as_path()) || (!reload && self.failed.contains_key(&path)) {
            return Task::none();
        }
        self.model_loading = Some(path.clone());
        Task::perform(
            async move {
                let for_task = path.clone();
                let result = tokio::task::spawn_blocking(move || {
                    let meta = std::fs::metadata(&for_task).ok();
                    let modified = meta.as_ref().and_then(|m| m.modified().ok());
                    let bytes = meta.map(|m| m.len());
                    hyprforge_mesh::load(&for_task, true)
                        .map(|(mesh, warning)| LoadedModel { mesh: Arc::new(mesh), warning, modified, bytes })
                        .map_err(|e| format!("Couldn't read this model: {e:#}"))
                })
                .await
                .unwrap_or_else(|e| Err(format!("The model loader stopped unexpectedly: {e}")));
                (path, reload, result)
            },
            |(path, reload, result)| Message::ModelLoaded(path, reload, result),
        )
    }

    /// Whether the item on screen is a model — the question every model
    /// key asks before meaning what it means on a model.
    fn model_on_screen(&self) -> bool {
        self.mode == Mode::Photo && self.folder.current().is_some_and(|i| i.media == Media::Model)
    }

    fn set_draw_mode(&mut self, mode: DrawMode) {
        self.prefs.model_draw_mode = mode.id().to_string();
        save_pref(|p| p.model_draw_mode = mode.id().to_string());
    }

    /// The model keys: viewpoints, zoom, style. `None` when the action is
    /// not one a model answers, so the caller carries on as for a picture.
    fn perform_on_model(&mut self, action: Action) -> Option<Task<Message>> {
        let viewport = self.viewport();
        let model = self.model.as_mut();
        let view = |model: Option<&mut ModelView>, v: ViewPoint| {
            if let Some(m) = model {
                m.camera.set_viewpoint(v);
            }
        };
        match action {
            // `0`/`F`: iso and back in the middle — "fit" for a model.
            Action::ZoomFit => {
                if let Some(m) = model {
                    m.camera.set_viewpoint(ViewPoint::Iso);
                    m.camera.set_viewpoint(ViewPoint::Center);
                }
            }
            // `1`: straight down — "actual size" has no meaning without
            // pixels, and top is the view that shows a model flat.
            Action::ZoomActual => view(model, ViewPoint::Top),
            Action::ViewBottom => view(model, ViewPoint::Bottom),
            Action::ViewFront => view(model, ViewPoint::Front),
            Action::ViewBack => view(model, ViewPoint::Back),
            Action::ViewLeft => view(model, ViewPoint::Left),
            Action::ViewRight => view(model, ViewPoint::Right),
            Action::ViewCenter => view(model, ViewPoint::Center),
            Action::ZoomIn | Action::ZoomOut => {
                if let Some(m) = model {
                    let units = if action == Action::ZoomIn { MODEL_UNITS_PER_LINE } else { -MODEL_UNITS_PER_LINE };
                    let centre = [viewport.width / 2.0, viewport.height / 2.0];
                    m.camera.zoom_at(centre, units, false, viewport.width, viewport.height);
                }
            }
            Action::CycleDrawMode => {
                let next = self.prefs.model_draw_mode().next();
                self.set_draw_mode(next);
            }
            Action::ToggleProjection => {
                let next = self.prefs.model_projection().toggled();
                self.prefs.model_projection = next.id().to_string();
                save_pref(|p| p.model_projection = next.id().to_string());
                if let Some(m) = self.model.as_mut() {
                    m.camera.perspective = next.value();
                }
            }
            Action::ToggleAxes => {
                self.prefs.model_axes = !self.prefs.model_axes;
                let on = self.prefs.model_axes;
                save_pref(|p| p.model_axes = on);
            }
            Action::Reload => {
                let path = self.current_path()?;
                self.failed.remove(&path);
                return Some(self.load_model(path, true));
            }
            // Turning a model is the drag's job; the picture's quarter
            // turns do not apply.
            Action::RotateLeft | Action::RotateRight => {}
            _ => return None,
        }
        Some(Task::none())
    }

    fn neighbours(&self) -> Vec<PathBuf> {
        let items = self.folder.items();
        let cursor = self.folder.cursor();
        [cursor.checked_sub(1), Some(cursor + 1)]
            .into_iter()
            .flatten()
            .filter_map(|i| items.get(i))
            .filter(|i| i.media == Media::Still)
            .map(|i| i.path.clone())
            .collect()
    }

    /// The header and EXIF of the selected file, read without decoding.
    fn request_details(&mut self) -> Task<Message> {
        let Some(item) = self.folder.current().cloned() else { return Task::none() };
        self.current_bytes = item.bytes;
        if self.details.as_ref().is_some_and(|d| d.path == item.path)
            || self.details_wanted.as_deref() == Some(item.path.as_path())
        {
            return Task::none();
        }
        self.details_wanted = Some(item.path.clone());
        let path = item.path;
        let still = item.media == Media::Still;
        Task::perform(
            async move {
                let for_task = path.clone();
                let (measured, camera) = tokio::task::spawn_blocking(move || {
                    if !still {
                        return (None, Camera::default());
                    }
                    (hyprforge_image::measure(&for_task).ok(), hyprforge_image::camera::read(&for_task))
                })
                .await
                .unwrap_or((None, Camera::default()));
                (path, measured, camera)
            },
            |(path, measured, camera)| Message::Details(path, measured, camera),
        )
    }

    fn decode(&mut self, path: PathBuf) -> Task<Message> {
        let still = self.folder.items().iter().any(|i| i.path == path && i.media == Media::Still);
        if !still || self.decoding.contains(&path) || self.failed.contains_key(&path) {
            return Task::none();
        }
        self.decoding.insert(path.clone());
        let budget = self.budget();
        Task::perform(
            async move {
                let for_task = path.clone();
                let result = tokio::task::spawn_blocking(move || {
                    decode_to_fit(&for_task, &budget).map(Arc::new).map_err(|e| e.to_string())
                })
                .await
                .unwrap_or_else(|e| Err(format!("The decoder stopped unexpectedly: {e}")));
                (path, result)
            },
            |(path, result)| Message::Decoded(path, result),
        )
    }

    /// Replaces the picture on screen, applying the user's turns.
    fn present(&mut self, path: PathBuf, decoded: Arc<Decoded>) {
        let (pixels, width, height) =
            rotate_rgba(&decoded.pixels, decoded.size.width, decoded.size.height, self.turns);
        let (source_w, source_h) =
            hyprforge_media::rotation::presented_size(decoded.measured.display_size(), self.turns);
        self.shown = Some(Shown {
            path,
            handle: image::Handle::from_rgba(width, height, pixels),
            decoded,
            size: ImageSize { width: source_w as f32, height: source_h as f32 },
        });
    }

    fn redecode_current(&mut self) -> Task<Message> {
        if self.mode != Mode::Photo {
            return Task::none();
        }
        match self.current_path() {
            Some(path) => {
                self.cache.remove(&path);
                self.failed.remove(&path);
                self.decode(path)
            }
            None => Task::none(),
        }
    }

    /// After the window grows, decodes the current picture again if the
    /// buffer on screen is now smaller than the budget allows and smaller
    /// than the picture itself — the only case a sharper decode would
    /// show anything more.
    fn sharpen_if_needed(&mut self) -> Task<Message> {
        let Some(shown) = &self.shown else { return Task::none() };
        let wanted = self.budget().fit(shown.decoded.measured.source);
        let have = shown.decoded.size;
        if wanted.width > have.width || wanted.height > have.height {
            return self.redecode_current();
        }
        Task::none()
    }

    // --- thumbnails ------------------------------------------------------

    fn strip_window(&self) -> filmstrip::Window {
        let visible = filmstrip::fits(self.pane_width() - 28.0, STRIP_TILE.0, STRIP_GAP);
        filmstrip::window(self.folder.len(), self.folder.cursor(), visible)
    }

    /// The paths whose thumbnails the window can show right now.
    fn wanted_thumbs(&self) -> Vec<PathBuf> {
        let items = self.folder.items();
        let path_of = |i: usize| items.get(i).map(|item| item.path.clone());
        match self.mode {
            Mode::Photo if self.filmstrip_shown() => self.strip_window().indices().filter_map(path_of).collect(),
            Mode::Photo => Vec::new(),
            Mode::Grid => {
                let tiles = self.tiles(1);
                let bands = grid::bands(&self.groups(), tiles.metrics);
                let (top, height) = self.grid_scroll;
                grid::visible(&bands, tiles.metrics, top, height, height).into_iter().filter_map(path_of).collect()
            }
            Mode::Library => {
                let cards = self.library_summaries();
                let one = [grid::Group { day: None, indices: (0..cards.len()).collect() }];
                let tiles = self.tiles(2);
                let metrics = grid::Metrics { header: 0.0, ..tiles.metrics };
                let (top, height) = self.library_scroll;
                grid::visible(&grid::bands(&one, metrics), metrics, top, height, height)
                    .into_iter()
                    .filter_map(|i| cards.get(i).and_then(|c| c.cover.clone()))
                    .collect()
            }
        }
    }

    fn request_thumbs(&mut self) -> Task<Message> {
        let wanted = self.wanted_thumbs();
        // Over the cap, keep only what is wanted now. The cap is on what
        // is held, so this runs before anything new is asked for.
        if self.thumbs.len() > MAX_THUMBS {
            let keep: HashSet<&PathBuf> = wanted.iter().collect();
            self.thumbs.retain(|path, _| keep.contains(path));
        }
        let edge = (THUMB_EDGE * self.scale_factor).ceil() as u32;
        // Clips and models are thumbnailed through the shared cache
        // (`thumbs.rs`); stills are decoded straight to size, below.
        let kinds: HashMap<&Path, Media> =
            self.folder.items().iter().filter(|i| i.media != Media::Still).map(|i| (i.path.as_path(), i.media)).collect();
        let mut tasks = Vec::new();
        for path in wanted {
            if self.thumbs.contains_key(&path) || self.thumbs_wanted.contains(&path) {
                continue;
            }
            self.thumbs_wanted.insert(path.clone());
            if let Some(&media) = kinds.get(path.as_path()) {
                let cache = self.thumb_cache.clone();
                tasks.push(Task::perform(
                    async move {
                        let _slot = EXPENSIVE_THUMBS.acquire().await;
                        let for_task = path.clone();
                        let handle = tokio::task::spawn_blocking(move || {
                            hyprforge_media::thumbs::of(cache.as_deref(), &for_task, media)
                                .map(|t| image::Handle::from_rgba(t.width, t.height, t.pixels))
                        })
                        .await
                        .ok()
                        .flatten();
                        (path, handle)
                    },
                    |(path, handle)| Message::Thumb(path, handle),
                ));
                continue;
            }
            tasks.push(Task::perform(
                async move {
                    let for_task = path.clone();
                    let handle = tokio::task::spawn_blocking(move || {
                        decode_to_fit(&for_task, &Budget::for_edge(edge))
                            .ok()
                            .map(|d| image::Handle::from_rgba(d.size.width, d.size.height, d.pixels))
                    })
                    .await
                    .ok()
                    .flatten();
                    (path, handle)
                },
                |(path, handle)| Message::Thumb(path, handle),
            ));
        }
        Task::batch(tasks)
    }

    // --- actions -----------------------------------------------------------

    fn perform(&mut self, action: Action) -> Task<Message> {
        if self.show.is_some() {
            return self.perform_in_slideshow(action);
        }
        if self.model_on_screen() {
            if let Some(task) = self.perform_on_model(action) {
                return task;
            }
        }
        if self.video_on_screen() {
            let player = self.video.as_ref().and_then(|v| v.player.as_ref());
            match action {
                Action::PlayPause | Action::Activate => {
                    self.play_pause();
                    return Task::none();
                }
                Action::SeekForward | Action::SeekBack => {
                    if let Some(player) = player {
                        let seconds = if action == Action::SeekForward { 5.0 } else { -5.0 };
                        player.send(hyprforge_video::Command::Seek { seconds, relative: true, exact: true });
                    }
                    return Task::none();
                }
                Action::ToggleMute => {
                    // mpv's own toggle, not "the opposite of what it last
                    // said": a press before its report of the one before
                    // used to send the same state twice, and do nothing.
                    if let Some(player) = player {
                        player.send(hyprforge_video::Command::ToggleMute);
                    }
                    return Task::none();
                }
                _ => {}
            }
        }
        let viewport = self.viewport();
        match action {
            Action::Next | Action::Previous | Action::Below | Action::Above | Action::First | Action::Last => {
                return self.move_selection(action);
            }
            Action::ZoomIn | Action::ZoomOut => {
                if let Some(shown) = &self.shown {
                    let steps = if action == Action::ZoomIn { 1 } else { -1 };
                    self.transform.zoom_by(steps, shown.size, viewport);
                }
            }
            Action::ZoomFit => self.transform.fit(),
            Action::ZoomActual => self.transform.actual_size(),
            Action::RotateLeft | Action::RotateRight => {
                self.turns = if action == Action::RotateRight { self.turns.right() } else { self.turns.left() };
                self.transform.fit();
                if let Some(shown) = self.shown.take() {
                    self.present(shown.path, shown.decoded);
                }
            }
            Action::ToggleFullscreen => {
                self.fullscreen = !self.fullscreen;
                return self.set_window_mode();
            }
            Action::ToggleInfo => {
                match &mut self.compact {
                    Some(c) => c.info = !c.info,
                    None => {
                        self.prefs.info_panel = !self.prefs.info_panel;
                        let on = self.prefs.info_panel;
                        save_pref(|p| p.info_panel = on);
                    }
                }
                self.size_video();
                return Task::batch([self.request_details(), self.request_thumbs()]);
            }
            Action::ToggleFilmstrip => {
                self.prefs.filmstrip = !self.prefs.filmstrip;
                let on = self.prefs.filmstrip;
                save_pref(|p| p.filmstrip = on);
                self.size_video();
                return self.request_thumbs();
            }
            Action::ToggleSidebar => {
                match &mut self.compact {
                    Some(c) => c.sidebar = !c.sidebar,
                    None => {
                        self.prefs.sidebar = !self.prefs.sidebar;
                        let on = self.prefs.sidebar;
                        save_pref(|p| p.sidebar = on);
                    }
                }
                self.size_video();
                return self.request_thumbs();
            }
            Action::ShowPhoto => return self.set_mode(Mode::Photo),
            Action::ShowGrid => return self.set_mode(Mode::Grid),
            Action::ShowLibrary => return self.set_mode(Mode::Library),
            Action::Activate => {
                return match self.mode {
                    Mode::Grid if !self.folder.is_empty() => self.set_mode(Mode::Photo),
                    Mode::Library => self.open_card(),
                    Mode::Photo => self.open_externally(),
                    Mode::Grid => Task::none(),
                };
            }
            Action::Up => {
                return match self.mode {
                    Mode::Library => self.library_up(),
                    // One level up from a folder's pictures is the folders
                    // beside it — mockup `1h`'s Backspace.
                    Mode::Photo | Mode::Grid => self.set_mode(Mode::Library),
                };
            }
            Action::Back => {
                if let Some(current) = self.folder_path.clone() {
                    if let Some(to) = self.history.back(current) {
                        return self.load_folder(to, None);
                    }
                }
            }
            Action::Forward => {
                if let Some(current) = self.folder_path.clone() {
                    if let Some(to) = self.history.forward(current) {
                        return self.load_folder(to, None);
                    }
                }
            }
            Action::Slideshow => return self.start_slideshow(),
            // On a picture or a model, Space is what it always was here:
            // the next one.
            Action::PlayPause => return self.move_selection(Action::Next),
            Action::SeekForward | Action::SeekBack | Action::ToggleMute => {}
            // Model keys pressed with no model on screen: nothing to do.
            Action::ViewBottom
            | Action::ViewFront
            | Action::ViewBack
            | Action::ViewLeft
            | Action::ViewRight
            | Action::ViewCenter
            | Action::CycleDrawMode
            | Action::ToggleProjection
            | Action::ToggleAxes
            | Action::Reload => {}
            Action::OpenExternally => return self.open_externally(),
            Action::ShowInFiles => {
                if let Some(path) = self.current_path() {
                    if let Err(e) = launch::show_in_files(&path) {
                        tracing::warn!(error = %e, "show in files");
                        self.notice = Some(e);
                    }
                }
            }
            Action::Copy => return self.copy(),
            Action::SetWallpaper => return self.set_wallpaper(),
            Action::Trash => return self.trash(),
            Action::Close => {
                // Escape unwinds one layer at a time — the menu, then
                // fullscreen, then the grid or library back to the
                // photograph — and closes only from a plain photo. The
                // keymap cannot know which state the window is in, so
                // this decides.
                if self.menu_open {
                    self.menu_open = false;
                    return Task::none();
                }
                if self.fullscreen {
                    self.fullscreen = false;
                    return self.set_window_mode();
                }
                if self.mode != Mode::Photo && !self.folder.is_empty() {
                    return self.set_mode(Mode::Photo);
                }
                return match self.window {
                    Some(id) => window::close(id),
                    None => iced::exit(),
                };
            }
        }
        Task::none()
    }

    /// The arrows, Home and End, which mean different things per mode.
    fn move_selection(&mut self, action: Action) -> Task<Message> {
        match self.mode {
            Mode::Photo => {
                let moved = match action {
                    Action::Next | Action::Below => self.folder.next().is_some(),
                    Action::Previous | Action::Above => self.folder.previous().is_some(),
                    Action::First => self.folder.first().is_some(),
                    _ => self.folder.last().is_some(),
                };
                if moved {
                    return self.show_current();
                }
                Task::none()
            }
            Mode::Grid => {
                let movement = grid_move(action);
                let tiles = self.tiles(1);
                match grid::step(&self.groups(), tiles.metrics.columns, self.folder.cursor(), movement) {
                    Some(index) if self.folder.select(index) => self.after_selection_moved(),
                    _ => Task::none(),
                }
            }
            Mode::Library => {
                let count = self.library_summaries().len();
                let one = [grid::Group { day: None, indices: (0..count).collect() }];
                let columns = self.tiles(2).metrics.columns;
                if let Some(index) = grid::step(&one, columns, self.library_cursor, grid_move(action)) {
                    self.library_cursor = index;
                }
                Task::none()
            }
        }
    }

    fn set_window_mode(&mut self) -> Task<Message> {
        self.transform.fit();
        self.size_video();
        let redecode = self.sharpen_if_needed();
        let mode = match self.window {
            Some(id) => window::set_mode(
                id,
                if self.fullscreen { window::Mode::Fullscreen } else { window::Mode::Windowed },
            ),
            None => Task::none(),
        };
        Task::batch([mode, redecode])
    }

    // --- slideshow ---------------------------------------------------------

    fn start_slideshow(&mut self) -> Task<Message> {
        if self.folder.is_empty() {
            return Task::none();
        }
        self.menu_open = false;
        let now = Instant::now();
        self.show = Some(Slideshow {
            show: Show::new(self.folder.len(), self.folder.cursor(), self.prefs.slideshow_loop),
            interval: Interval::from_seconds(self.prefs.slideshow_seconds),
            since: now,
            pointer_moved: now,
            was_fullscreen: self.fullscreen,
        });
        self.mode = Mode::Photo;
        self.fullscreen = true;
        Task::batch([self.set_window_mode(), self.show_current()])
    }

    fn stop_slideshow(&mut self) -> Task<Message> {
        let Some(show) = self.show.take() else { return Task::none() };
        self.fullscreen = show.was_fullscreen;
        Task::batch([self.set_window_mode(), self.show_current()])
    }

    fn perform_in_slideshow(&mut self, action: Action) -> Task<Message> {
        let Some(show) = &mut self.show else { return Task::none() };
        let target = match action {
            Action::Next | Action::Below => show.show.advance(),
            Action::Previous | Action::Above => show.show.back(),
            Action::Close | Action::Slideshow => return self.stop_slideshow(),
            // Space pauses the show; on a video it pauses the video too.
            Action::PlayPause => {
                show.show.paused = !show.show.paused;
                show.since = Instant::now();
                self.play_pause();
                return Task::none();
            }
            _ => return Task::none(),
        };
        show.since = Instant::now();
        match target {
            Some(index) if self.folder.select(index) => self.show_current(),
            Some(_) => Task::none(),
            None => self.stop_slideshow(),
        }
    }

    fn slideshow_tick(&mut self, now: Instant) -> Task<Message> {
        let Some(show) = &mut self.show else { return Task::none() };
        if show.show.paused || now.duration_since(show.since) < Duration::from_secs(show.interval.seconds()) {
            return Task::none();
        }
        // A video plays through before the show moves on; one that could
        // not play is passed like a picture.
        if self.video.as_ref().is_some_and(|v| v.player.is_some() && v.error.is_none() && !v.playback.ended) {
            return Task::none();
        }
        // The next picture only once this one is actually on screen: a
        // slow decode must not be skipped past before anyone saw it.
        let current = self.folder.current().map(|i| &i.path);
        let on_screen = self.shown.as_ref().map(|s| &s.path) == current
            || self.model.as_ref().map(|m| &m.path) == current
            || self.folder.current().is_some_and(|i| i.media == Media::Clip || self.failed.contains_key(&i.path));
        if !on_screen {
            return Task::none();
        }
        show.since = now;
        match show.show.advance() {
            Some(index) if self.folder.select(index) => self.show_current(),
            Some(_) => Task::none(),
            None => self.stop_slideshow(),
        }
    }

    fn slideshow_controls_visible(&self) -> bool {
        self.show.as_ref().is_some_and(|s| s.show.paused || s.pointer_moved.elapsed() < SLIDESHOW_CONTROLS_FOR)
    }

    // --- things that touch files ---------------------------------------------

    fn open_externally(&mut self) -> Task<Message> {
        let Some(path) = self.current_path() else { return Task::none() };
        Task::perform(
            async move {
                tokio::task::spawn_blocking(move || {
                    let db = hyprforge_mime::MimeDb::load();
                    let Some(mime) = db.type_of(&path).map(str::to_string) else {
                        return Err("Couldn't tell what kind of file this is, so there's no program to hand it to."
                            .to_string());
                    };
                    let candidates = db.apps_for(&mime);
                    match launch::other_app(db.default_for(&mime), &candidates) {
                        Some(app) => launch::open_with(&app.path, &path).map(|()| format!("Opened in {}.", app.name)),
                        None => Err(format!("No other program is registered to open {mime}.")),
                    }
                })
                .await
                .unwrap_or_else(|e| Err(e.to_string()))
            },
            Message::Done,
        )
    }

    /// The file on the clipboard two ways: its own bytes under its own
    /// type, for an application that pastes pictures, and its location,
    /// for one that pastes files. The clipboard *library*, never the
    /// history daemon — which need not be installed. What this cannot do
    /// is outlive the window: a Wayland selection belongs to whoever set
    /// it, so closing the viewer ends the copy unless something else (the
    /// clipboard daemon, when it is running) has taken it up.
    fn copy(&mut self) -> Task<Message> {
        let Some(path) = self.current_path() else { return Task::none() };
        let writer = self.clipboard.clone();
        Task::perform(
            async move {
                tokio::task::spawn_blocking(move || {
                    let bytes = std::fs::read(&path).map_err(|e| format!("Couldn't read the picture to copy it: {e}"))?;
                    let mime = hyprforge_mime::MimeDb::load()
                        .type_of(&path)
                        .unwrap_or("application/octet-stream")
                        .to_string();
                    let uri = format!("file://{}", path.display());
                    let offers = hyprforge_clipboard::Offers::new(vec![
                        (hyprforge_clipboard::Mime::new(mime), bytes),
                        (hyprforge_clipboard::Mime::new("text/uri-list"), format!("{uri}\r\n").into_bytes()),
                        (hyprforge_clipboard::Mime::new("text/plain"), path.display().to_string().into_bytes()),
                    ]);
                    writer
                        .set_offers(offers)
                        // The source stays alive on the writer's own
                        // thread for as long as it is the selection; the
                        // waiter only says when that ends, and nothing
                        // here needs to know.
                        .map(|_waiter| "Copied.".to_string())
                        .map_err(|e| format!("Couldn't put it on the clipboard: {e}"))
                })
                .await
                .unwrap_or_else(|e| Err(e.to_string()))
            },
            Message::Done,
        )
    }

    /// Through the same wallpaper settings Settings' Wallpaper page edits,
    /// never around them: saved first, then pushed to hyprpaper, so the
    /// choice survives a restart and Settings shows what is on screen.
    fn set_wallpaper(&mut self) -> Task<Message> {
        let Some(path) = self.current_path() else { return Task::none() };
        Task::perform(
            async move {
                tokio::task::spawn_blocking(move || set_wallpaper(&path))
                    .await
                    .unwrap_or_else(|e| Err(e.to_string()))
            },
            Message::Done,
        )
    }

    fn trash(&mut self) -> Task<Message> {
        let Some(path) = self.current_path() else { return Task::none() };
        Task::perform(
            async move {
                let for_task = path.clone();
                let result = tokio::task::spawn_blocking(move || {
                    hyprforge_fileops::trash::trash(&for_task).map_err(|e| format!("Couldn't move it to the trash: {e}"))
                })
                .await
                .unwrap_or_else(|e| Err(e.to_string()));
                (path, result)
            },
            |(path, result)| Message::Trashed(path, result),
        )
    }
}

/// The grid's (or library's) tile geometry for the pane as it is now.
#[derive(Debug, Clone, Copy)]
struct Tiles {
    width: f32,
    picture: f32,
    metrics: grid::Metrics,
}

fn grid_move(action: Action) -> grid::Move {
    match action {
        Action::Next => grid::Move::Next,
        Action::Previous => grid::Move::Previous,
        Action::Below => grid::Move::Down,
        Action::Above => grid::Move::Up,
        Action::First => grid::Move::First,
        _ => grid::Move::Last,
    }
}

fn grid_id() -> Id {
    Id::new("media-grid")
}

fn library_id() -> Id {
    Id::new("media-library")
}

/// A folder's name as the title and headings show it; the root is `/`.
fn display_name(dir: &Path) -> String {
    dir.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_else(|| dir.display().to_string())
}

fn read_folder(dir: PathBuf) -> Task<Message> {
    Task::perform(
        async move {
            let for_task = dir.clone();
            let result = tokio::task::spawn_blocking(move || StdBackend.read_dir(&for_task).map_err(|e| e.to_string()))
                .await
                .unwrap_or_else(|e| Err(e.to_string()));
            (dir, result)
        },
        |(dir, result)| Message::FolderRead(dir, result),
    )
}

/// Lists `dir`'s subfolders and summarises each from its own listing —
/// one directory read per subfolder, capped at `library::MOST_FOLDERS`.
/// A subfolder that cannot be read still gets its card, with no
/// pictures: it is there, and hiding it would be a claim that it is not.
fn read_summaries(dir: PathBuf, order: hyprforge_listing::order::Order) -> Task<Message> {
    Task::perform(
        async move {
            let for_task = dir.clone();
            let result = tokio::task::spawn_blocking(move || {
                let entries = StdBackend.read_dir(&for_task).map_err(|e| e.to_string())?;
                Ok(library::subfolders(entries, &order)
                    .into_iter()
                    .map(|(name, path)| {
                        let entries = StdBackend.read_dir(&path).unwrap_or_default();
                        library::summarise(name, path, entries, &order)
                    })
                    .collect())
            })
            .await
            .unwrap_or_else(|e| Err(e.to_string()));
            (dir, result)
        },
        |(dir, result)| Message::Summaries(dir, result),
    )
}

fn save_pref(change: impl FnOnce(&mut prefs::Prefs)) {
    if let Err(e) = prefs::update(change) {
        tracing::warn!(error = %e, "viewer setting not saved");
    }
}

/// Saves `picture` as the wallpaper on every monitor and pushes it to
/// hyprpaper — see `hyprforge_ecosystem::wallpaper::Settings::set_everywhere`
/// for what "every monitor" means when some already have their own.
///
/// A settings file that exists and will not parse is reported and left
/// alone: overwriting it would replace whatever the user configured with
/// one picture, which is the collapse CLAUDE.md's first rules exist to
/// stop.
fn set_wallpaper(picture: &Path) -> Result<String, String> {
    use hyprforge_ecosystem::apply::{self, Applied};
    use hyprforge_ecosystem::wallpaper;

    let settings_path = wallpaper::settings_path();
    let mut settings: wallpaper::Settings = hyprforge_ecosystem::storage::load(&settings_path)
        .map_err(|e| format!("Your wallpaper settings couldn't be read, so nothing was changed: {e}"))?;
    settings.set_everywhere(&picture.display().to_string());
    hyprforge_ecosystem::storage::save(&settings_path, &settings)
        .map_err(|e| format!("Couldn't save the wallpaper setting: {e}"))?;

    match apply::wallpapers(&wallpaper::generated_path(), &wallpaper::hyprpaper_conf_path(), &settings) {
        Ok(Applied::Live) => Ok("Set as your wallpaper.".to_string()),
        Ok(Applied::DaemonNotRunning) => Ok(
            "Saved as your wallpaper. hyprpaper isn't running, so it will show when it next starts."
                .to_string(),
        ),
        Ok(Applied::PartlyRefused(refused)) => Err(format!(
            "Saved, but hyprpaper wouldn't show it on: {}",
            refused.join(", ")
        )),
        Ok(other) => Ok(format!("Saved as your wallpaper ({other:?}).")),
        Err(e) => Err(format!("Couldn't apply the wallpaper: {e}")),
    }
}
