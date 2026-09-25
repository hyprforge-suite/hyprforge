//! `hyprforge-photos`: the window.
//!
//! Everything this draws is decided in the library — which picture is
//! next ([`folder`]), where it sits and how big ([`transform`]), what a
//! key means ([`keys`]), what the info panel says ([`info`]). What is
//! here is the part that cannot be asserted without a display: turning
//! those decisions into widgets, and running the slow work (decoding,
//! trashing, the clipboard, the wallpaper) off the thread that paints.
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

use hyprforge_image::{decode_to_fit, Budget, Decoded, ViewportPixels};
use hyprforge_listing::backend::{FsBackend, StdBackend};
use hyprforge_photos::args::{self, Args};
use hyprforge_photos::cache::Cache;
use hyprforge_photos::folder::{Folder, Media};
use hyprforge_photos::keys::{Action, Resolved};
use hyprforge_photos::rotation::{rotate_rgba, Turns};
use hyprforge_photos::transform::{ImageSize, LogicalPoint, Transform, Viewport};
use hyprforge_photos::{config, filmstrip, info, launch, order, prefs};
use hyprforge_ui::theme::{app_theme, spacing, FontScale, BASE_TEXT_SIZE};
use hyprforge_ui::widgets::{meta_text, scaled_text, secondary_button};
use iced::widget::{button, column, container, image, mouse_area, pin, row, scrollable, Space};
use iced::{keyboard, mouse, window, Element, Length, Point, Size, Subscription, Task, Theme};
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::Arc;

const APP_ID: &str = "hyprforge-photos";

/// The toolbar across the top, hidden in fullscreen.
const TOOLBAR_HEIGHT: f32 = 44.0;
/// The filmstrip along the bottom, when shown.
const FILMSTRIP_HEIGHT: f32 = 92.0;
/// One filmstrip tile, logical pixels, square.
const THUMB: f32 = 72.0;
const THUMB_GAP: f32 = 8.0;
/// The info panel down the right, when shown.
const INFO_WIDTH: f32 = 300.0;
/// How long a resize has to settle before the window size is saved and a
/// sharper decode is considered — a drag produces a resize per frame.
const RESIZE_SETTLE_MS: u64 = 400;
/// Wheel pixels per zoom step, for a touchpad's smooth scrolling.
const PIXELS_PER_ZOOM_STEP: f32 = 60.0;

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
        (prefs.window_width as f32).max(480.0),
        (prefs.window_height as f32).max(360.0),
    );

    let (start, focus) = match &args {
        Args::Picture(path) if path.is_dir() => (Some(path.clone()), None),
        Args::Picture(path) => (path.parent().map(Path::to_path_buf), Some(path.clone())),
        Args::Folder(path) => (Some(path.clone()), None),
        Args::Nothing => (None, None),
    };

    let app = App {
        keymap: config.keymap,
        order: ordering.order,
        prefs,
        folder: Folder::default(),
        folder_path: start.clone(),
        focus,
        loading_folder: start.is_some(),
        folder_error: None,
        cache: Cache::default(),
        decoding: HashSet::new(),
        failed: HashMap::new(),
        shown: None,
        turns: Turns::none(),
        transform: Transform::default(),
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
        notice: problems.first().cloned(),
        undo: None,
        clipboard: Arc::new(hyprforge_clipboard::WaylandWriter::new()),
        font_scale: FontScale(hyprforge_ui::theme::active().font_scale),
    };

    let mut boot_tasks = vec![window::latest().map(Message::WindowFound)];
    if let Some(dir) = start {
        boot_tasks.push(read_folder(dir));
    }
    let boot = std::cell::RefCell::new(Some((app, Task::batch(boot_tasks))));

    iced::application(
        move || boot.borrow_mut().take().expect("hyprforge-photos boots once"),
        App::update,
        App::view,
    )
    .title(App::title)
    .theme(App::theme)
    .subscription(App::subscription)
    .window(window::Settings {
        size,
        min_size: Some(Size::new(480.0, 360.0)),
        platform_specific: window::settings::PlatformSpecific {
            application_id: APP_ID.to_string(),
            ..window::settings::PlatformSpecific::default()
        },
        ..window::Settings::default()
    })
    .run()
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

/// Something that has just happened and can be taken back.
enum Undo {
    Trashed { item: hyprforge_fileops::trash::TrashedItem, name: String },
}

struct App {
    keymap: hyprforge_photos::keys::Keymap,
    order: hyprforge_listing::order::Order,
    prefs: prefs::Prefs,
    folder: Folder,
    folder_path: Option<PathBuf>,
    /// The picture argv named, focused once the folder arrives.
    focus: Option<PathBuf>,
    loading_folder: bool,
    folder_error: Option<String>,
    cache: Cache,
    /// Decodes in flight, so a picture is never decoded twice at once.
    decoding: HashSet<PathBuf>,
    /// Pictures that could not be decoded, with the sentence to show.
    failed: HashMap<PathBuf, String>,
    shown: Option<Shown>,
    turns: Turns,
    transform: Transform,
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
    /// Filmstrip thumbnails: `None` for one that could not be made.
    thumbs: HashMap<PathBuf, Option<image::Handle>>,
    thumbs_wanted: HashSet<PathBuf>,
    current_bytes: Option<u64>,
    /// One line across the top: a problem to report, or what just
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
    Decoded(PathBuf, Result<Arc<Decoded>, String>),
    Thumb(PathBuf, Option<image::Handle>),
    Key(hyprforge_keys::KeyPress),
    Perform(Action),
    Select(usize),
    Scrolled(mouse::ScrollDelta),
    PointerMoved(Point),
    DragStart,
    DragEnd,
    ToggleZoom,
    Resized(Size),
    ResizeSettled(u64),
    Trashed(PathBuf, Result<hyprforge_fileops::trash::TrashedItem, String>),
    UndoPressed,
    Restored(Result<String, String>),
    Done(Result<String, String>),
    DismissNotice,
}

impl App {
    fn title(&self) -> String {
        match self.folder.current() {
            Some(item) => format!("{} — Photos", item.name),
            None => "Photos".to_string(),
        }
    }

    fn theme(&self) -> Theme {
        app_theme()
    }

    fn subscription(&self) -> Subscription<Message> {
        Subscription::batch([
            keyboard::listen().filter_map(|event| hyprforge_ui::keys::key_press(&event).map(Message::Key)),
            window::resize_events().map(|(_, size)| Message::Resized(size)),
        ])
    }

    /// The area the picture is drawn in, logical pixels: the window less
    /// whatever chrome is showing.
    fn viewport(&self) -> Viewport {
        let mut width = self.window_size.width;
        let mut height = self.window_size.height;
        if !self.fullscreen {
            height -= TOOLBAR_HEIGHT;
        }
        if self.prefs.filmstrip && !self.fullscreen {
            height -= FILMSTRIP_HEIGHT;
        }
        if self.prefs.info_panel && !self.fullscreen {
            width -= INFO_WIDTH;
        }
        Viewport { width: width.max(1.0), height: height.max(1.0) }
    }

    /// How much may be decoded for the viewport as it is now.
    fn budget(&self) -> Budget {
        let v = self.viewport();
        Budget::for_viewport(ViewportPixels::from_logical(v.width, v.height, self.scale_factor))
    }

    fn update(&mut self, message: Message) -> Task<Message> {
        match message {
            Message::WindowFound(id) => {
                self.window = id;
                match id {
                    Some(id) => window::scale_factor(id).map(Message::ScaleFactor),
                    None => Task::none(),
                }
            }
            Message::ScaleFactor(scale) => {
                if (scale - self.scale_factor).abs() > f32::EPSILON {
                    self.scale_factor = scale;
                    // Everything held was decoded for a density this
                    // window does not have; the picture on screen stays
                    // up until its sharper replacement arrives.
                    self.cache = Cache::default();
                    return self.redecode_current();
                }
                Task::none()
            }
            Message::FolderRead(dir, result) => {
                self.loading_folder = false;
                match result {
                    Ok(entries) => {
                        self.folder = Folder::build(entries, &self.order);
                        if let Some(path) = self.focus.take() {
                            if !self.folder.focus_on(&path) {
                                self.notice =
                                    Some(format!("{} isn't a picture this viewer can show.", path.display()));
                            }
                        }
                        self.folder_path = Some(dir);
                        self.show_current()
                    }
                    Err(e) => {
                        self.folder_error = Some(e);
                        Task::none()
                    }
                }
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
                if self.folder.items().get(index).is_some() {
                    while self.folder.cursor() < index {
                        self.folder.next();
                    }
                    while self.folder.cursor() > index {
                        self.folder.previous();
                    }
                    return self.show_current();
                }
                Task::none()
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
                        self.transform.zoom_about(
                            steps,
                            LogicalPoint { x: at.x, y: at.y },
                            shown.size,
                            viewport,
                        );
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
                Task::none()
            }
            Message::DragStart => {
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
                if let Some(shown) = &self.shown {
                    self.transform.clamp_pan(shown.size, self.viewport());
                }
                self.resize_generation += 1;
                let generation = self.resize_generation;
                Task::perform(
                    tokio::time::sleep(std::time::Duration::from_millis(RESIZE_SETTLE_MS)),
                    move |_| Message::ResizeSettled(generation),
                )
            }
            Message::ResizeSettled(generation) => {
                if generation != self.resize_generation || self.fullscreen {
                    return Task::none();
                }
                let (width, height) = (self.window_size.width as u32, self.window_size.height as u32);
                if let Err(e) = prefs::update(|p| {
                    p.window_width = width;
                    p.window_height = height;
                }) {
                    tracing::warn!(error = %e, "window size not saved");
                }
                self.sharpen_if_needed()
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
                    self.show_current()
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
                    match self.folder_path.clone() {
                        Some(dir) => {
                            // Re-read rather than re-insert: where it
                            // belongs in the order is the listing's
                            // question, not this window's.
                            self.focus = self.current_path();
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

    /// Everything that follows the cursor moving: the new picture (from
    /// the cache, or decoded), its neighbours made ready, the filmstrip's
    /// thumbnails, and the size shown in the info panel.
    fn show_current(&mut self) -> Task<Message> {
        self.transform = Transform::default();
        self.turns = Turns::none();
        let Some(item) = self.folder.current().cloned() else {
            self.shown = None;
            return Task::none();
        };
        self.current_bytes = std::fs::metadata(&item.path).ok().map(|m| m.len());

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
        tasks.push(self.request_thumbs());
        Task::batch(tasks)
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
        let (source_w, source_h) = hyprforge_photos::rotation::presented_size(
            decoded.measured.display_size(),
            self.turns,
        );
        self.shown = Some(Shown {
            path,
            handle: image::Handle::from_rgba(width, height, pixels),
            decoded,
            size: ImageSize { width: source_w as f32, height: source_h as f32 },
        });
    }

    fn redecode_current(&mut self) -> Task<Message> {
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

    fn visible_thumbs(&self) -> filmstrip::Window {
        let visible = filmstrip::fits(self.window_size.width, THUMB, THUMB_GAP);
        filmstrip::window(self.folder.len(), self.folder.cursor(), visible)
    }

    fn request_thumbs(&mut self) -> Task<Message> {
        if !self.prefs.filmstrip {
            return Task::none();
        }
        let edge = (THUMB * self.scale_factor).ceil() as u32;
        let mut tasks = Vec::new();
        for index in self.visible_thumbs().indices() {
            let Some(item) = self.folder.items().get(index) else { continue };
            if self.thumbs.contains_key(&item.path) || self.thumbs_wanted.contains(&item.path) {
                continue;
            }
            if item.media == Media::Clip {
                self.thumbs.insert(item.path.clone(), None);
                continue;
            }
            self.thumbs_wanted.insert(item.path.clone());
            let path = item.path.clone();
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

    fn perform(&mut self, action: Action) -> Task<Message> {
        let viewport = self.viewport();
        match action {
            Action::Next => {
                if self.folder.next().is_some() {
                    return self.show_current();
                }
            }
            Action::Previous => {
                if self.folder.previous().is_some() {
                    return self.show_current();
                }
            }
            Action::First => {
                if self.folder.first().is_some() {
                    return self.show_current();
                }
            }
            Action::Last => {
                if self.folder.last().is_some() {
                    return self.show_current();
                }
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
                self.prefs.info_panel = !self.prefs.info_panel;
                let on = self.prefs.info_panel;
                save_pref(|p| p.info_panel = on);
            }
            Action::ToggleFilmstrip => {
                self.prefs.filmstrip = !self.prefs.filmstrip;
                let on = self.prefs.filmstrip;
                save_pref(|p| p.filmstrip = on);
                return self.request_thumbs();
            }
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
                // Escape leaves fullscreen first — the keymap cannot
                // know which state the window is in, so this decides.
                if self.fullscreen {
                    self.fullscreen = false;
                    return self.set_window_mode();
                }
                return match self.window {
                    Some(id) => window::close(id),
                    None => iced::exit(),
                };
            }
        }
        Task::none()
    }

    fn set_window_mode(&mut self) -> Task<Message> {
        self.transform.fit();
        match self.window {
            Some(id) => window::set_mode(
                id,
                if self.fullscreen { window::Mode::Fullscreen } else { window::Mode::Windowed },
            ),
            None => Task::none(),
        }
    }

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

    fn view(&self) -> Element<'_, Message> {
        let mut body = column![].width(Length::Fill).height(Length::Fill);
        if !self.fullscreen {
            body = body.push(self.toolbar());
        }

        let mut middle = row![self.canvas()].height(Length::Fill);
        if self.prefs.info_panel && !self.fullscreen {
            middle = middle.push(self.info_panel());
        }
        body = body.push(middle);

        if self.prefs.filmstrip && !self.fullscreen {
            body = body.push(self.filmstrip());
        }
        body.into()
    }

    fn toolbar(&self) -> Element<'_, Message> {
        let scale = self.font_scale;
        let position = match self.folder.position() {
            Some((at, of)) => format!("{at} of {of}"),
            None => String::new(),
        };
        let name = self.folder.current().map(|i| i.name.clone()).unwrap_or_default();
        let tool = |label: &'static str, action: Action| secondary_button(label).on_press(Message::Perform(action));

        let mut bar = row![
            tool("‹", Action::Previous),
            tool("›", Action::Next),
            column![scaled_text(name, BASE_TEXT_SIZE, scale), meta_text(position, 12.0, scale)]
                .width(Length::Fill),
        ]
        .spacing(spacing::SM)
        .align_y(iced::Alignment::Center);

        if let Some(notice) = &self.notice {
            bar = bar.push(meta_text(notice.clone(), 13.0, scale));
            if self.undo.is_some() {
                bar = bar.push(secondary_button("Undo").on_press(Message::UndoPressed));
            }
            bar = bar.push(secondary_button("×").on_press(Message::DismissNotice));
        }

        bar = bar
            .push(tool("−", Action::ZoomOut))
            .push(tool("Fit", Action::ZoomFit))
            .push(tool("+", Action::ZoomIn))
            .push(tool("⟲", Action::RotateLeft))
            .push(tool("⟳", Action::RotateRight))
            .push(tool("Wallpaper", Action::SetWallpaper))
            .push(tool("Info", Action::ToggleInfo));

        container(bar)
            .padding([spacing::XS, spacing::SM])
            .height(Length::Fixed(TOOLBAR_HEIGHT))
            .width(Length::Fill)
            .into()
    }

    /// The picture itself, placed by the transform and clipped to the
    /// viewport; or a sentence where there is no picture to place.
    fn canvas(&self) -> Element<'_, Message> {
        let scale = self.font_scale;
        let viewport = self.viewport();
        let centred = |text: String| -> Element<'_, Message> {
            container(meta_text(text, BASE_TEXT_SIZE, scale)).center(Length::Fill).into()
        };

        let content: Element<'_, Message> = if let Some(e) = &self.folder_error {
            centred(format!("Couldn't read the folder: {e}"))
        } else if self.folder_path.is_none() {
            centred("Open a picture from Files, or name one on the command line.".to_string())
        } else if self.loading_folder {
            centred("Loading…".to_string())
        } else if let Some(item) = self.folder.current() {
            if item.media == Media::Clip {
                centred(format!("{} is a video. Press Enter to open it in a video player.", item.name))
            } else if let Some(e) = self.failed.get(&item.path) {
                centred(e.clone())
            } else if let Some(shown) = &self.shown {
                let (w, h) = self.transform.drawn_size(shown.size, viewport);
                let at = self.transform.top_left(shown.size, viewport);
                pin(image(shown.handle.clone())
                    .width(Length::Fixed(w))
                    .height(Length::Fixed(h))
                    .content_fit(iced::ContentFit::Fill))
                .x(at.x)
                .y(at.y)
                .width(Length::Fill)
                .height(Length::Fill)
                .into()
            } else {
                centred("Loading…".to_string())
            }
        } else {
            centred("There are no pictures in this folder.".to_string())
        };

        mouse_area(
            container(content)
                .width(Length::Fixed(viewport.width))
                .height(Length::Fixed(viewport.height))
                .clip(true),
        )
        .on_scroll(Message::Scrolled)
        .on_move(Message::PointerMoved)
        .on_press(Message::DragStart)
        .on_release(Message::DragEnd)
        .on_double_click(Message::ToggleZoom)
        .into()
    }

    fn info_panel(&self) -> Element<'_, Message> {
        let scale = self.font_scale;
        let rows = match self.folder.current() {
            Some(item) => info::rows(
                item,
                self.shown.as_ref().filter(|s| s.path == item.path).map(|s| &s.decoded.measured),
                self.turns,
                self.folder.position(),
                self.current_bytes,
            ),
            None => Vec::new(),
        };
        let mut list = column![].spacing(spacing::SM);
        for row in rows {
            list = list.push(column![meta_text(row.label, 12.0, scale), scaled_text(row.value, BASE_TEXT_SIZE, scale)]);
        }
        container(scrollable(list).height(Length::Fill))
            .padding(spacing::MD)
            .width(Length::Fixed(INFO_WIDTH))
            .height(Length::Fill)
            .into()
    }

    fn filmstrip(&self) -> Element<'_, Message> {
        let scale = self.font_scale;
        let window = self.visible_thumbs();
        let mut strip = row![].spacing(THUMB_GAP).align_y(iced::Alignment::Center);
        for index in window.indices() {
            let Some(item) = self.folder.items().get(index) else { continue };
            let tile: Element<'_, Message> = match self.thumbs.get(&item.path) {
                Some(Some(handle)) => image(handle.clone())
                    .width(Length::Fixed(THUMB))
                    .height(Length::Fixed(THUMB))
                    .content_fit(iced::ContentFit::Contain)
                    .into(),
                Some(None) if item.media == Media::Clip => container(meta_text("▶", 18.0, scale))
                    .center(Length::Fixed(THUMB))
                    .into(),
                _ => Space::new().width(Length::Fixed(THUMB)).height(Length::Fixed(THUMB)).into(),
            };
            let selected = index == self.folder.cursor();
            strip = strip.push(
                button(tile)
                    .padding(2)
                    .style(move |theme: &Theme, status| {
                        let mut style = button::secondary(theme, status);
                        if selected {
                            style.border.color = theme.palette().primary;
                            style.border.width = 2.0;
                        }
                        style
                    })
                    .on_press(Message::Select(index)),
            );
        }
        container(strip)
            .center_x(Length::Fill)
            .height(Length::Fixed(FILMSTRIP_HEIGHT))
            .padding(spacing::XS)
            .into()
    }
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
