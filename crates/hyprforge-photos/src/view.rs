//! The window's widgets: the file manager's shell around three modes.
//!
//! Everything here turns state into widgets and decides nothing — the
//! decisions are in the library and in `main.rs`. It is laid out the way
//! the file manager's `browser::render` is, and for the reason that
//! function gives: the header and status bar span the whole window with
//! the sidebar between them, so the header reads as the titlebar a
//! compositor with no titlebars does not draw.
//!
//! Three depth levels, as there: the chrome's plane (`surface::sidebar`),
//! the content recessed between it (`surface::card`, and the photo on the
//! deeper `surface::root`), and raised live controls (`surface::row`).
//! Purple — the theme's accent — means *selected* and nothing else: the
//! current sidebar folder, the current grid tile, the current filmstrip
//! frame. The Photo/Grid/Library switch is drawn [`SegmentLook::Quiet`]
//! for the reason the file manager's own view switch is: a mode is not a
//! selection, and a second purple on the bar would compete with the one
//! that is. The mockup draws it purple; this is the one place the window
//! departs from it on purpose.

use super::{App, Listing, Message, Mode, Tiles, GRID_PAD, INSPECTOR_WIDTH, SIDEBAR_WIDTH};
use super::{FILMSTRIP_HEIGHT, STATUS_HEIGHT, STRIP_CURRENT, STRIP_GAP, STRIP_TILE, TILE_LINE, TILE_PAD};
use hyprforge_photos::folder::Media;
use hyprforge_photos::keys::{self, Action};
use hyprforge_photos::slideshow::Interval;
use hyprforge_photos::{grid, info, library};
use hyprforge_ui::density;
use hyprforge_ui::glyph;
use hyprforge_ui::theme::{self, spacing, surface, FontScale, BASE_TEXT_SIZE};
use hyprforge_ui::widgets::{
    dropdown_menu_style, dropdown_style, inset_field_style, keycap, meta_text, scaled_text, section_label,
    segment_style, segmented, selectable_row_style, SegmentLook, Tint,
};
use iced::widget::{button, column, container, image, mouse_area, pick_list, pin, row, scrollable, stack, text, Space};
use iced::{Background, Border, Element, Length, Theme};
use std::path::{Path, PathBuf};

/// How many folder crumbs the path bar shows before eliding the middle.
const MAX_CRUMBS: usize = 4;
/// Below this window width the path bar shows only the folder you are in:
/// the mode switch and the tools take a fixed 360px or so, and a whole
/// path in what is left runs into the position at the field's end.
const NARROW_HEADER: f32 = 900.0;
const NARROWEST_HEADER: f32 = 720.0;

impl App {
    pub(super) fn view(&self) -> Element<'_, Message> {
        if self.show.is_some() {
            return self.slideshow_view();
        }

        let mut middle = row![].height(Length::Fill);
        if self.sidebar_shown() {
            middle = middle.push(self.sidebar()).push(edge_v());
        }
        middle = middle.push(self.pane());
        if self.inspector_shown() {
            middle = middle.push(edge_v()).push(self.inspector());
        }

        let mut body = column![].width(Length::Fill).height(Length::Fill);
        if !self.fullscreen {
            body = body.push(self.header()).push(edge_h());
        }
        body = body.push(middle);
        if !self.fullscreen {
            body = body.push(edge_h()).push(self.status_bar());
        }
        let window: Element<'_, Message> = container(body)
            .width(Length::Fill)
            .height(Length::Fill)
            .style(|_t: &Theme| plane(surface::card()))
            .into();

        if self.menu_open {
            // A click anywhere outside the menu closes it, the way every
            // popup menu behaves; the catcher sits between the window and
            // the menu so the menu's own buttons still get their clicks.
            let catcher = mouse_area(Space::new().width(Length::Fill).height(Length::Fill)).on_press(Message::ToggleMenu);
            return stack![window, catcher, self.menu()].into();
        }
        window
    }

    // --- header ---------------------------------------------------------

    /// Mockup `2a`'s 46px bar: sidebar toggle and back/forward, the path
    /// field, the mode switch, and the tools.
    fn header(&self) -> Element<'_, Message> {
        let scale = self.font_scale;
        let nav = row![
            sidebar_toggle(self.sidebar_on(), scale),
            nav_button(glyph::Nav::Back, self.history.can_go_back().then_some(Message::Perform(Action::Back)), true, scale),
            nav_button(glyph::Nav::Forward, self.history.can_go_forward().then_some(Message::Perform(Action::Forward)), false, scale),
        ]
        .spacing(spacing::XS)
        .align_y(iced::Alignment::Center);

        let modes = segmented([Mode::Photo, Mode::Grid, Mode::Library].map(|mode| {
            let label = match mode {
                Mode::Photo => "Photo",
                Mode::Grid => "Grid",
                Mode::Library => "Library",
            };
            button(scaled_text(label, density::META_TEXT_BASE, scale))
                .padding([2.0, scale.apply(10.0)])
                .on_press(Message::SetMode(mode))
                .style(segment_style(SegmentLook::Quiet, self.mode == mode))
                .into()
        }));

        let has_picture = self.folder.current().is_some();
        let photo = self.mode == Mode::Photo && has_picture;
        let tools = row![
            glyph_button("⟲", photo.then_some(Message::Perform(Action::RotateLeft)), false, scale),
            // Edit (mockup `1d`) is not built: it writes to somebody's
            // original, which `photos-plan.md` keeps as a separate
            // decision. Drawn disabled rather than left out, the way the
            // file manager shows its Columns view — a control that is
            // visibly waiting is a smaller surprise than one that appears.
            glyph_button("✎", None, false, scale),
            glyph_button("i", Some(Message::Perform(Action::ToggleInfo)), self.info_on(), scale),
            glyph_button("⋯", Some(Message::ToggleMenu), self.menu_open, scale),
        ]
        .spacing(spacing::XS)
        .align_y(iced::Alignment::Center);

        container(
            row![nav, self.path_bar(), modes, tools]
                .spacing(spacing::SM)
                .align_y(iced::Alignment::Center),
        )
        .height(Length::Fixed(density::bar_height(scale)))
        .center_y(Length::Fixed(density::bar_height(scale)))
        .padding([0, spacing::SM as u16])
        .width(Length::Fill)
        .style(|_t: &Theme| plane(surface::sidebar()))
        .into()
    }

    /// The path, in the mono font: home in the accent, folders as
    /// clickable crumbs, and in Photo mode the file itself in full text
    /// colour. The position — "12 / 48", or "48 photos" — sits at the
    /// field's right end, as in the mockup.
    fn path_bar(&self) -> Element<'_, Message> {
        let scale = self.font_scale;
        let size = density::META_TEXT_BASE;
        let mono = theme::mono_font();
        let mut crumbs = row![].spacing(0).align_y(iced::Alignment::Center);

        if let Some(dir) = &self.folder_path {
            let mut segments = crumbs_of(dir);
            // A narrow window — the compact viewer sized to a small
            // picture — has room for where you are, not how you got
            // there: the folder alone, still clickable, and the file.
            if self.window_size.width < NARROW_HEADER {
                segments = segments.split_off(segments.len().saturating_sub(1));
            }
            // At the window's minimum even that crowds the name out, and
            // in Photo mode the name is the thing to keep.
            if self.window_size.width < NARROWEST_HEADER && self.mode == Mode::Photo {
                segments.clear();
            }
            let last = segments.len().saturating_sub(1);
            for (i, (label, path)) in segments.into_iter().enumerate() {
                let is_home = i == 0 && label == "~";
                let is_here = i == last && self.mode != Mode::Photo;
                let color = if is_home {
                    accent()
                } else if is_here {
                    theme::text()
                } else {
                    theme::text_dim()
                };
                let crumb = text(label).font(mono).size(scale.apply(size)).color(color);
                crumbs = match path {
                    Some(path) => crumbs.push(button(crumb).padding(0).style(bare_style).on_press(Message::OpenFolder(path))),
                    None => crumbs.push(crumb),
                };
                if i < last || self.mode == Mode::Photo {
                    crumbs = crumbs.push(text("/").font(mono).size(scale.apply(size)).color(theme::text_dim()));
                }
            }
            if self.mode == Mode::Photo {
                if let Some(item) = self.folder.current() {
                    crumbs = crumbs.push(text(item.name.clone()).font(mono).size(scale.apply(size)).color(theme::text()));
                }
            }
        }

        let position = match self.mode {
            Mode::Photo => self.folder.position().map(|(at, of)| format!("{at} / {of}")),
            Mode::Grid => (!self.loading_folder).then(|| grid::describe(self.folder.items().iter().map(|i| i.media))),
            Mode::Library => {
                let n = self.library_summaries().len();
                Some(if n == 1 { "1 folder".to_string() } else { format!("{n} folders") })
            }
        };

        container(
            row![
                container(crumbs).width(Length::Fill).clip(true),
                text(position.unwrap_or_default()).font(mono).size(scale.apply(density::META_TEXT_BASE * 0.85)).color(theme::text_dim()),
            ]
            .spacing(spacing::SM)
            .align_y(iced::Alignment::Center),
        )
        .width(Length::Fill)
        .height(Length::Fixed(density::field_height(scale)))
        .center_y(Length::Fixed(density::field_height(scale)))
        .padding([0, spacing::SM as u16 + 4])
        .style(inset_field_style)
        .into()
    }

    // --- sidebar --------------------------------------------------------

    /// PLACES, then the folders beside this one under their parent's
    /// name — mockup `2a`'s "2026" section, which is Library mode's
    /// cards as rows.
    fn sidebar(&self) -> Element<'_, Message> {
        let scale = self.font_scale;
        let current = self.folder_path.as_deref();
        let mut list = column![].spacing(spacing::MD).width(Length::Fill);

        if !self.places.is_empty() {
            let mut group = column![heading("Places", scale)].spacing(2.0);
            for place in &self.places {
                group = group.push(sidebar_row(
                    place.label.clone(),
                    None,
                    place.tint,
                    current == Some(place.path.as_path()),
                    place.path.clone(),
                    scale,
                ));
            }
            list = list.push(group);
        }

        if let Some(parent) = current.and_then(Path::parent) {
            if let Some(Listing::Loaded(siblings)) = self.folders.get(parent) {
                if !siblings.is_empty() {
                    let title = if Some(parent) == home().as_deref() { "Home".to_string() } else { super::display_name(parent) };
                    let mut group = column![heading(&title, scale)].spacing(2.0);
                    for s in siblings {
                        group = group.push(sidebar_row(
                            s.name.clone(),
                            Some((s.photos + s.models).to_string()),
                            Tint::Accent,
                            current == Some(s.path.as_path()),
                            s.path.clone(),
                            scale,
                        ));
                    }
                    list = list.push(group);
                }
            }
        }

        container(scrollable(list))
            .padding([14, 10])
            .width(Length::Fixed(SIDEBAR_WIDTH))
            .height(Length::Fill)
            .style(|_t: &Theme| plane(surface::sidebar()))
            .into()
    }

    // --- the pane between -------------------------------------------------

    fn pane(&self) -> Element<'_, Message> {
        let scale = self.font_scale;
        let centred = |said: String| -> Element<'_, Message> {
            container(meta_text(said, BASE_TEXT_SIZE, scale)).center(Length::Fill).into()
        };
        if let Some(e) = &self.folder_error {
            return centred(format!("Couldn't read the folder: {e}"));
        }
        if self.folder_path.is_none() {
            return centred("Open a picture from Files, or name one on the command line.".to_string());
        }
        match self.mode {
            Mode::Photo => self.photo_mode(),
            Mode::Grid if self.loading_folder => centred("Loading…".to_string()),
            Mode::Grid if self.folder.is_empty() => centred("There are no pictures in this folder.".to_string()),
            Mode::Grid => self.grid_mode(),
            Mode::Library => self.library_mode(),
        }
    }

    /// Mockup `2a`: the photograph on the deepest plane, the overlay
    /// arrows and zoom pill over it, and the filmstrip below.
    fn photo_mode(&self) -> Element<'_, Message> {
        let mut col = column![self.canvas()].width(Length::Fill).height(Length::Fill);
        if self.filmstrip_shown() {
            col = col.push(edge_h()).push(self.filmstrip());
        }
        col.into()
    }

    /// The picture itself, placed by the transform and clipped to the
    /// viewport; or a sentence where there is no picture to place.
    fn canvas(&self) -> Element<'_, Message> {
        let scale = self.font_scale;
        let viewport = self.viewport();
        let centred = |said: String| -> Element<'_, Message> {
            container(meta_text(said, BASE_TEXT_SIZE, scale)).center(Length::Fill).into()
        };

        let content: Element<'_, Message> = if self.loading_folder {
            centred("Loading…".to_string())
        } else if let Some(item) = self.folder.current() {
            if item.media == Media::Model {
                return self.model_canvas(true);
            }
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

        let picture = mouse_area(
            container(content)
                .width(Length::Fixed(viewport.width))
                .height(Length::Fixed(viewport.height))
                .clip(true)
                .style(|_t: &Theme| plane(surface::root())),
        )
        .on_scroll(Message::Scrolled)
        .on_move(Message::PointerMoved)
        .on_press(Message::DragStart)
        .on_release(Message::DragEnd)
        .on_double_click(Message::ToggleZoom);

        if self.folder.is_empty() {
            return picture.into();
        }

        // The overlay controls: previous and next at the edges, the zoom
        // pill in the corner — the mockup's, over the photograph rather
        // than in a toolbar, so the header stays the window's.
        let arrows = container(
            row![
                overlay_arrow("‹", Message::Perform(Action::Previous), scale),
                Space::new().width(Length::Fill),
                overlay_arrow("›", Message::Perform(Action::Next), scale),
            ]
            .align_y(iced::Alignment::Center),
        )
        .center_y(Length::Fill)
        .width(Length::Fill)
        .padding([0, 14]);

        let pill = container(self.zoom_pill()).align_right(Length::Fill).align_bottom(Length::Fill).padding([12, 14]);

        stack![picture, arrows, pill].width(Length::Fixed(viewport.width)).height(Length::Fixed(viewport.height)).into()
    }

    /// A 3D model in the pane: view3d's renderer in a `shader` widget, with
    /// drag to turn, right-drag to pan and scroll to zoom about the
    /// pointer. `controls` adds the overlay arrows and the style pill —
    /// the slideshow draws the model bare.
    fn model_canvas(&self, controls: bool) -> Element<'_, Message> {
        let scale = self.font_scale;
        let viewport = self.viewport();
        let centred = |said: String| -> Element<'_, Message> {
            container(meta_text(said, BASE_TEXT_SIZE, scale))
                .center(Length::Fill)
                .style(|_t: &Theme| plane(surface::root()))
                .into()
        };
        let Some(item) = self.folder.current() else { return centred(String::new()) };
        let body: Element<'_, Message> = match &self.model {
            Some(model) if model.path == item.path => {
                let program = super::model::ModelProgram {
                    mesh: model.mesh.clone(),
                    generation: model.generation,
                    camera: model.camera,
                    style: super::model::Style {
                        draw_mode: self.prefs.model_draw_mode(),
                        axes: self.prefs.model_axes,
                        backdrop: surface::root(),
                    },
                };
                let scene = mouse_area(
                    iced::widget::shader(program)
                        .width(Length::Fixed(viewport.width))
                        .height(Length::Fixed(viewport.height)),
                )
                .on_press(Message::ModelPress(super::ModelDrag::Turn))
                .on_release(Message::ModelRelease)
                .on_right_press(Message::ModelPress(super::ModelDrag::Pan))
                .on_right_release(Message::ModelRelease)
                .on_move(Message::ModelPointer)
                .on_scroll(Message::ModelScrolled)
                .interaction(if self.model_drag.is_some() {
                    iced::mouse::Interaction::Grabbing
                } else {
                    iced::mouse::Interaction::Grab
                });
                // Behind the scene, and covered by its backdrop whenever
                // wgpu is drawing — see `model`'s module doc.
                let fallback = container(meta_text(
                    "3D models need GPU rendering, and this window is drawing without it.",
                    BASE_TEXT_SIZE,
                    scale,
                ))
                .center(Length::Fill)
                .style(|_t: &Theme| plane(surface::root()));
                stack![fallback, scene].into()
            }
            _ => match self.failed.get(&item.path) {
                Some(e) => centred(e.clone()),
                None => centred(format!("Loading {}…", item.name)),
            },
        };
        if !controls || self.model.as_ref().is_none_or(|m| m.path != item.path) {
            return container(body).width(Length::Fixed(viewport.width)).height(Length::Fixed(viewport.height)).into();
        }
        let arrows = container(
            row![
                overlay_arrow("‹", Message::Perform(Action::Previous), scale),
                Space::new().width(Length::Fill),
                overlay_arrow("›", Message::Perform(Action::Next), scale),
            ]
            .align_y(iced::Alignment::Center),
        )
        .center_y(Length::Fill)
        .width(Length::Fill)
        .padding([0, 14]);
        let pill = container(self.model_pill()).align_right(Length::Fill).align_bottom(Length::Fill).padding([12, 14]);
        stack![body, arrows, pill].width(Length::Fixed(viewport.width)).height(Length::Fixed(viewport.height)).into()
    }

    /// The model's counterpart of the zoom pill: the five draw modes, then
    /// perspective/orthographic and the axes — everything view3d's View
    /// menu held, one click away and each with its key in the menu.
    fn model_pill(&self) -> Element<'_, Message> {
        let scale = self.font_scale;
        let current = self.prefs.model_draw_mode();
        let segment = |label: &'static str, message: Message, lit: bool| -> Element<'_, Message> {
            button(text(label).font(theme::mono_font()).size(scale.apply(density::META_TEXT_BASE * 0.85)))
                .padding([3.0, 8.0])
                .on_press(message)
                .style(segment_style(SegmentLook::Quiet, lit))
                .into()
        };
        let mut modes = row![].spacing(3);
        for mode in hyprforge_mesh::style::DrawMode::ALL {
            modes = modes.push(segment(mode.short_label(), Message::SetDrawMode(mode), mode == current));
        }
        let ortho = self.prefs.model_projection() == hyprforge_mesh::style::Projection::Orthographic;
        container(
            row![
                modes,
                container(Space::new()).width(Length::Fixed(1.0)).height(Length::Fixed(16.0)).style(|_t: &Theme| plane(surface::card_border())),
                segment(if ortho { "Ortho" } else { "Persp" }, Message::Perform(Action::ToggleProjection), false),
                segment("Axes", Message::Perform(Action::ToggleAxes), self.prefs.model_axes),
            ]
            .spacing(6)
            .align_y(iced::Alignment::Center),
        )
        .padding(3)
        .style(|_t: &Theme| floating())
        .into()
    }

    /// Fit · 100% · − · +, with whichever of the first two is true now
    /// lit — the same "filled means on" idiom as the header's buttons.
    fn zoom_pill(&self) -> Element<'_, Message> {
        let scale = self.font_scale;
        let fit = self.transform.zoom == hyprforge_photos::transform::Zoom::Fit;
        let actual = self.transform.zoom == hyprforge_photos::transform::Zoom::Factor(1.0);
        let segment = |label: &'static str, action: Action, lit: bool| -> Element<'_, Message> {
            button(text(label).font(theme::mono_font()).size(scale.apply(density::META_TEXT_BASE * 0.85)))
                .padding([3.0, 8.0])
                .on_press(Message::Perform(action))
                .style(segment_style(SegmentLook::Quiet, lit))
                .into()
        };
        container(
            row![
                segment("Fit", Action::ZoomFit, fit),
                segment("100%", Action::ZoomActual, actual),
                segment("−", Action::ZoomOut, false),
                segment("+", Action::ZoomIn, false),
            ]
            .spacing(3),
        )
        .padding(3)
        .style(|_t: &Theme| floating())
        .into()
    }

    /// Mockup `2a`'s strip: 3:2 frames, dimmed, with the current one
    /// larger and ringed in the accent.
    fn filmstrip(&self) -> Element<'_, Message> {
        let scale = self.font_scale;
        let mut strip = row![].spacing(STRIP_GAP).align_y(iced::Alignment::Center);
        for index in self.strip_window().indices() {
            let Some(item) = self.folder.items().get(index) else { continue };
            let current = index == self.folder.cursor();
            let (w, h) = if current { STRIP_CURRENT } else { STRIP_TILE };
            let picture: Element<'_, Message> = match self.thumbs.get(&item.path) {
                Some(Some(handle)) => image(handle.clone())
                    .width(Length::Fixed(w))
                    .height(Length::Fixed(h))
                    .content_fit(iced::ContentFit::Cover)
                    .border_radius(5.0)
                    // Everything but the current frame recedes, so the
                    // eye finds where it is before reading anything.
                    .opacity(if current { 1.0_f32 } else { 0.72_f32 })
                    .into(),
                Some(None) if item.media != Media::Still => container(meta_text(badge(item.media), 14.0, scale))
                    .center_x(Length::Fixed(w))
                    .center_y(Length::Fixed(h))
                    .style(|_t: &Theme| rounded(surface::row(), 5.0))
                    .into(),
                _ => container(Space::new())
                    .width(Length::Fixed(w))
                    .height(Length::Fixed(h))
                    .style(|_t: &Theme| rounded(surface::row(), 5.0))
                    .into(),
            };
            let framed = container(picture).padding(if current { 2 } else { 0 }).style(move |t: &Theme| {
                if current {
                    container::Style {
                        border: Border { radius: 7.0.into(), width: 2.0, color: t.palette().primary },
                        ..container::Style::default()
                    }
                } else {
                    container::Style::default()
                }
            });
            strip = strip.push(button(framed).padding(0).style(bare_style).on_press(Message::Select(index)));
        }
        container(strip)
            .center_x(Length::Fill)
            .center_y(Length::Fixed(FILMSTRIP_HEIGHT))
            .padding([0, 14])
            .style(|_t: &Theme| plane(surface::sidebar()))
            .into()
    }

    /// Mockup `2b`: the folder as tiles under the day they were taken.
    fn grid_mode(&self) -> Element<'_, Message> {
        let scale = self.font_scale;
        let tiles = self.tiles(1);
        let groups = self.groups();
        let today = chrono::Local::now().date_naive();
        let items = self.folder.items();

        let mut body = column![].spacing(0).width(Length::Fill);
        for band in grid::bands(&groups, tiles.metrics) {
            match band {
                grid::Band::Header { group, .. } => {
                    let g = &groups[group];
                    if group > 0 {
                        body = body.push(Space::new().height(Length::Fixed(tiles.metrics.group_gap)));
                    }
                    body = body.push(
                        container(
                            row![
                                scaled_text(grid::heading(g.day, today), 13.0, scale).font(semibold()),
                                text(grid::describe(g.indices.iter().filter_map(|&i| items.get(i)).map(|i| i.media)))
                                    .font(theme::mono_font())
                                    .size(scale.apply(11.0))
                                    .color(theme::text_dim()),
                            ]
                            .spacing(10)
                            .align_y(iced::Alignment::Center),
                        )
                        .height(Length::Fixed(tiles.metrics.header))
                        .padding([0, 8])
                        .center_y(Length::Fixed(tiles.metrics.header)),
                    );
                }
                grid::Band::Row { indices, top } => {
                    // A row gap before every row that is not a day's first
                    // — exactly where `grid::bands` counted one.
                    let first_in_group = body_needs_no_gap(&groups, &indices);
                    if !first_in_group {
                        body = body.push(Space::new().height(Length::Fixed(tiles.metrics.row_gap)));
                    }
                    let _ = top;
                    let mut line = row![].spacing(super::TILE_GAP);
                    for index in indices {
                        let Some(item) = items.get(index) else { continue };
                        line = line.push(self.tile(index, &item.name, Some(&item.path), (item.media != Media::Still).then(|| badge(item.media)), None, index == self.folder.cursor(), tiles));
                    }
                    body = body.push(line);
                }
            }
        }

        scrollable(container(body).padding(GRID_PAD))
            .id(super::grid_id())
            .on_scroll(Message::GridScrolled)
            .width(Length::Fill)
            .height(Length::Fill)
            .into()
    }

    /// Mockup `1h`: a folder's subfolders as cards — the library as the
    /// directory tree you already keep.
    fn library_mode(&self) -> Element<'_, Message> {
        let scale = self.font_scale;
        let centred = |said: String| -> Element<'_, Message> {
            container(meta_text(said, BASE_TEXT_SIZE, scale)).center(Length::Fill).into()
        };
        let Some(dir) = &self.library_dir else {
            return centred("There is no folder to show the library of.".to_string());
        };
        let cards = match self.folders.get(dir) {
            Some(Listing::Loaded(cards)) => cards,
            Some(Listing::Failed(e)) => return centred(format!("Couldn't read {}: {e}", dir.display())),
            _ => return centred("Loading…".to_string()),
        };

        let total = library::describe_counts(cards.iter().map(|c| c.photos).sum(), cards.iter().map(|c| c.models).sum());
        let folders = if cards.len() == 1 { "1 folder".to_string() } else { format!("{} folders", cards.len()) };
        let up: Element<'_, Message> = match dir.parent() {
            Some(parent) => button(meta_text(format!("{} ›", super::display_name(parent)), 13.0, scale))
                .padding(0)
                .style(bare_style)
                .on_press(Message::Perform(Action::Up))
                .into(),
            None => Space::new().into(),
        };
        let header = container(
            row![
                up,
                scaled_text(super::display_name(dir), 13.0, scale).font(semibold()),
                text(format!("{folders} · {total}"))
                    .font(theme::mono_font())
                    .size(scale.apply(11.0))
                    .color(theme::text_dim()),
            ]
            .spacing(8)
            .align_y(iced::Alignment::Center),
        )
        .padding([0, 8])
        .height(Length::Fixed(super::GROUP_HEADER))
        .center_y(Length::Fixed(super::GROUP_HEADER));

        if cards.is_empty() {
            return column![container(header).padding([GRID_PAD, GRID_PAD]), centred("No folders here.".to_string())].into();
        }

        let tiles = self.tiles(2);
        let today = chrono::Local::now().date_naive();
        let mut body = column![].spacing(super::TILE_GAP).width(Length::Fill);
        for (r, chunk) in cards.chunks(tiles.metrics.columns).enumerate() {
            let mut line = row![].spacing(super::TILE_GAP);
            for (c, card) in chunk.iter().enumerate() {
                let index = r * tiles.metrics.columns + c;
                let second = library::card_line(card, &chrono::Local, today);
                line = line.push(self.tile(index, &card.name, card.cover.as_deref(), None, Some(second), index == self.library_cursor, tiles));
            }
            body = body.push(line);
        }

        column![
            container(header).padding([GRID_PAD as u16 / 2, GRID_PAD as u16]),
            scrollable(container(body).padding([0.0, GRID_PAD]))
                .id(super::library_id())
                .on_scroll(Message::LibraryScrolled)
                .width(Length::Fill)
                .height(Length::Fill),
        ]
        .into()
    }

    /// One tile: the picture (or a folder's cover), its name, and for a
    /// folder card a second line. A click selects, a double click opens —
    /// the file manager's grammar — which is why this is a `mouse_area`
    /// and not a `button`: a button takes the press, and the double click
    /// would never reach anything around it.
    #[allow(clippy::too_many_arguments)]
    fn tile<'a>(
        &'a self,
        index: usize,
        name: &str,
        picture: Option<&Path>,
        badge: Option<&'static str>,
        second: Option<String>,
        selected: bool,
        tiles: Tiles,
    ) -> Element<'a, Message> {
        let scale = self.font_scale;
        let inner = tiles.width - 2.0 * TILE_PAD;
        let thumb: Element<'a, Message> = match picture.and_then(|p| self.thumbs.get(p)) {
            Some(Some(handle)) => image(handle.clone())
                .width(Length::Fixed(inner))
                .height(Length::Fixed(tiles.picture))
                .content_fit(iced::ContentFit::Cover)
                .border_radius(6.0)
                .into(),
            _ => container(meta_text(badge.unwrap_or(""), 20.0, scale))
                .center_x(Length::Fixed(inner))
                .center_y(Length::Fixed(tiles.picture))
                .style(|_t: &Theme| rounded(surface::row(), 6.0))
                .into(),
        };
        // The lines under the picture sit flush in a column of their own:
        // `App::tiles` counts one `TILE_PAD` between picture and text and
        // `TILE_LINE` per line, and a gap between the lines would push the
        // second one out of the tile's fixed height.
        let mut lines = column![container(scaled_text(name.to_string(), 12.5, scale).wrapping(text::Wrapping::None))
            .width(Length::Fixed(inner))
            .height(Length::Fixed(TILE_LINE))
            .clip(true)];
        if let Some(second) = second {
            lines = lines.push(
                container(meta_text(second, 11.5, scale).wrapping(text::Wrapping::None))
                    .width(Length::Fixed(inner))
                    .height(Length::Fixed(TILE_LINE))
                    .clip(true),
            );
        }
        let body = column![thumb, lines].spacing(TILE_PAD);
        let is_card = self.mode == Mode::Library;
        let (on_press, on_double) = if is_card {
            (Message::SelectCard(index), Message::OpenCard(index))
        } else {
            (Message::Select(index), Message::OpenTile(index))
        };
        mouse_area(
            container(body)
                .padding(TILE_PAD)
                .width(Length::Fixed(tiles.width))
                .height(Length::Fixed(tiles.metrics.row))
                .style(move |t: &Theme| tile_style(t, selected)),
        )
        .interaction(iced::mouse::Interaction::Pointer)
        .on_press(on_press)
        .on_double_click(on_double)
        .into()
    }

    // --- inspector ------------------------------------------------------

    /// Mockup `2c`: the name and when it was taken, then FILE, CAMERA and
    /// LOCATION — whichever have something to say.
    fn inspector(&self) -> Element<'_, Message> {
        let scale = self.font_scale;
        let body: Element<'_, Message> = match self.folder.current() {
            None => meta_text("Select a photo to see it here.", BASE_TEXT_SIZE, scale).into(),
            Some(item) => {
                let details = self.details.as_ref().filter(|d| d.path == item.path);
                let measured = self
                    .shown
                    .as_ref()
                    .filter(|s| s.path == item.path)
                    .map(|s| &s.decoded.measured)
                    .or(details.and_then(|d| d.measured.as_ref()));
                let camera = details.map(|d| &d.camera);
                let turns = if self.mode == Mode::Photo { self.turns } else { hyprforge_photos::rotation::Turns::none() };

                let mut col = column![column![
                    scaled_text(item.name.clone(), 14.0, scale)
                        .font(semibold())
                        .wrapping(text::Wrapping::WordOrGlyph),
                    text(info::when(camera, item.modified, &chrono::Local).unwrap_or_default())
                        .font(theme::mono_font())
                        .size(scale.apply(11.0))
                        .color(theme::text_dim()),
                ]
                .spacing(2)]
                .spacing(spacing::MD);

                let model = self.model.as_ref().filter(|m| m.path == item.path).map(|m| &m.facts);
                for section in info::sections(item, measured, camera, model, turns, self.current_bytes) {
                    let mut rows = column![container(section_label(section.title, scale)).padding([0, 0])].spacing(6);
                    for r in section.rows {
                        rows = rows.push(
                            row![
                                scaled_text(r.label, 12.5, scale).color(theme::text_dim()).width(Length::Fixed(scale.apply(84.0))),
                                text(r.value)
                                    .font(theme::mono_font())
                                    .size(scale.apply(12.0))
                                    .color(theme::text())
                                    .wrapping(text::Wrapping::WordOrGlyph),
                            ]
                            .spacing(spacing::MD),
                        );
                    }
                    col = col.push(rows);
                }
                col.into()
            }
        };
        container(scrollable(body))
            .padding(spacing::MD)
            .width(Length::Fixed(INSPECTOR_WIDTH))
            .height(Length::Fill)
            .style(|_t: &Theme| plane(surface::sidebar()))
            .into()
    }

    // --- status bar -------------------------------------------------------

    /// What is here on the left, the keys worth knowing on the right —
    /// read from the keymap, so a rebound key is the one named.
    fn status_bar(&self) -> Element<'_, Message> {
        let scale = self.font_scale;
        let size = density::META_TEXT_BASE * 0.9;
        let hint = |action: Action, what: &str| keys::hint(&self.keymap, action).map(|k| format!("{k} {what}"));

        let mut left = row![].spacing(18).align_y(iced::Alignment::Center);
        if let Some(notice) = &self.notice {
            left = left.push(scaled_text(notice.clone(), size, scale));
            if self.undo.is_some() {
                left = left.push(link("Undo", Message::UndoPressed, scale));
            }
            left = left.push(link("Dismiss", Message::DismissNotice, scale));
        } else {
            match self.mode {
                Mode::Photo => {
                    if let Some(item) = self.folder.current().filter(|i| i.media == Media::Model) {
                        let model = self.model.as_ref().filter(|m| m.path == item.path);
                        left = left.push(meta_text(info::model_status_line(item, model.map(|m| &m.facts), self.current_bytes), size, scale));
                        if let Some(warning) = model.and_then(|m| m.warning.clone()) {
                            left = left.push(scaled_text(warning, size, scale));
                        }
                    } else if let Some(item) = self.folder.current() {
                        let measured = self.shown.as_ref().filter(|s| s.path == item.path).map(|s| &s.decoded.measured);
                        let details = self.details.as_ref().filter(|d| d.path == item.path);
                        let measured = measured.or(details.and_then(|d| d.measured.as_ref()));
                        left = left.push(meta_text(info::status_line(item, measured, self.current_bytes), size, scale));
                        if let Some(summary) = details.and_then(|d| d.camera.summary()) {
                            left = left.push(scaled_text(summary, size, scale));
                        }
                    }
                }
                Mode::Grid => {
                    let mut parts = vec![grid::describe(self.folder.items().iter().map(|i| i.media))];
                    if let Some(item) = self.folder.current() {
                        parts.push("1 selected".to_string());
                        if let Some(bytes) = item.bytes {
                            parts.push(info::human_size(bytes));
                        }
                    }
                    left = left.push(meta_text(parts.join(" · "), size, scale));
                }
                Mode::Library => {
                    let cards = self.library_summaries();
                    let total = library::describe_counts(cards.iter().map(|c| c.photos).sum(), cards.iter().map(|c| c.models).sum());
                    let folders = if cards.len() == 1 { "1 folder".to_string() } else { format!("{} folders", cards.len()) };
                    left = left.push(meta_text(format!("{folders} · {total}"), size, scale));
                }
            }
        }

        let hints: Vec<String> = match self.mode {
            Mode::Photo if self.model_on_screen() => {
                vec![Some("drag turn · right-drag pan".to_string()), hint(Action::CycleDrawMode, "style"), hint(Action::ToggleInfo, "info")]
            }
            Mode::Photo if self.info_on() => {
                vec![hint(Action::ToggleInfo, "close info"), hint(Action::ToggleSidebar, "sidebar")]
            }
            Mode::Photo => {
                let browse = match (keys::hint(&self.keymap, Action::Previous), keys::hint(&self.keymap, Action::Next)) {
                    (Some(p), Some(n)) => Some(format!("{p} {n} browse")),
                    _ => None,
                };
                vec![browse, hint(Action::ShowGrid, "grid"), hint(Action::ToggleInfo, "info")]
            }
            Mode::Grid => vec![hint(Action::Activate, "open"), hint(Action::Trash, "trash")],
            Mode::Library => vec![hint(Action::Activate, "open folder"), hint(Action::Up, "up")],
        }
        .into_iter()
        .flatten()
        .collect();

        container(
            row![
                container(left).width(Length::Fill).clip(true),
                text(hints.join(" · ")).font(theme::mono_font()).size(scale.apply(size)).color(theme::text_dim()),
            ]
            .spacing(spacing::MD)
            .align_y(iced::Alignment::Center),
        )
        .height(Length::Fixed(scale.apply(STATUS_HEIGHT)))
        .center_y(Length::Fixed(scale.apply(STATUS_HEIGHT)))
        .padding([0, spacing::MD as u16])
        .width(Length::Fill)
        .style(|_t: &Theme| plane(surface::sidebar()))
        .into()
    }

    // --- the ⋯ menu --------------------------------------------------------

    /// What does not earn a button on the bar, each with its key.
    fn menu(&self) -> Element<'_, Message> {
        let scale = self.font_scale;
        let has = self.folder.current().is_some();
        let entry = |label: String, action: Action, enabled: bool| -> Element<'_, Message> {
            let key = keys::hint(&self.keymap, action).unwrap_or_default();
            button(
                row![
                    scaled_text(label, BASE_TEXT_SIZE, scale).width(Length::Fill),
                    text(key).font(theme::mono_font()).size(scale.apply(density::META_TEXT_BASE * 0.85)).color(theme::text_dim()),
                ]
                .spacing(spacing::LG)
                .align_y(iced::Alignment::Center),
            )
            .width(Length::Fill)
            .padding([6, 10])
            .on_press_maybe(enabled.then_some(Message::MenuAction(action)))
            .style(|t: &Theme, status| selectable_row_style(t, status, false))
            .into()
        };
        let filmstrip = if self.prefs.filmstrip { "Hide Filmstrip" } else { "Show Filmstrip" };
        let sidebar = if self.sidebar_on() { "Hide Sidebar" } else { "Show Sidebar" };
        let menu = column![
            entry(Action::Slideshow.label().to_string(), Action::Slideshow, !self.folder.is_empty()),
            entry(Action::SetWallpaper.label().to_string(), Action::SetWallpaper, has),
            entry(Action::Copy.label().to_string(), Action::Copy, has),
            entry(Action::OpenExternally.label().to_string(), Action::OpenExternally, has),
            entry(Action::ShowInFiles.label().to_string(), Action::ShowInFiles, has),
            entry(filmstrip.to_string(), Action::ToggleFilmstrip, true),
            entry(sidebar.to_string(), Action::ToggleSidebar, true),
            entry(Action::ToggleFullscreen.label().to_string(), Action::ToggleFullscreen, true),
            entry(Action::Trash.label().to_string(), Action::Trash, has),
        ]
        .spacing(1)
        .width(Length::Fixed(scale.apply(250.0)));

        container(container(menu).padding(4).style(|_t: &Theme| floating()))
            .align_right(Length::Fill)
            .padding(iced::Padding { top: density::bar_height(scale) + 2.0, right: spacing::SM, bottom: 0.0, left: 0.0 })
            .into()
    }

    // --- slideshow ----------------------------------------------------------

    /// Mockup `1e`: the photograph alone on the deepest plane, and a row
    /// of controls that fades two seconds after the pointer stops.
    fn slideshow_view(&self) -> Element<'_, Message> {
        let scale = self.font_scale;
        let picture: Element<'_, Message> = match &self.shown {
            // Through the same Fit as Photo mode — never past 1:1 — so a
            // small picture in a slideshow is shown at its size rather
            // than as a blur stretched across the screen.
            Some(shown) if self.current_path().as_deref() == Some(shown.path.as_path()) => {
                let viewport = self.viewport();
                let fit = hyprforge_photos::transform::Transform::default();
                let (w, h) = fit.drawn_size(shown.size, viewport);
                container(
                    image(shown.handle.clone())
                        .width(Length::Fixed(w))
                        .height(Length::Fixed(h))
                        .content_fit(iced::ContentFit::Fill),
                )
                .center(Length::Fill)
                .into()
            }
            _ => match self.folder.current() {
                Some(item) if item.media == Media::Model => self.model_canvas(false),
                Some(item) if item.media == Media::Clip => {
                    container(meta_text(format!("{} is a video.", item.name), BASE_TEXT_SIZE, scale)).center(Length::Fill).into()
                }
                _ => Space::new().width(Length::Fill).height(Length::Fill).into(),
            },
        };
        let backdrop = mouse_area(container(picture).width(Length::Fill).height(Length::Fill).style(|_t: &Theme| plane(surface::root())))
            .on_move(Message::PointerMoved);

        let Some(show) = &self.show else { return backdrop.into() };
        if !self.slideshow_controls_visible() {
            return backdrop.into();
        }

        let (at, of) = show.show.position();
        let place = self.folder_path.as_deref().map(super::display_name).unwrap_or_default();
        let key = |action: Action| keys::hint(&self.keymap, action);
        let control = |label: &str, key: Option<String>, message: Message, lit: bool| -> Element<'_, Message> {
            let mut content = row![scaled_text(label.to_string(), density::META_TEXT_BASE, scale)].spacing(6).align_y(iced::Alignment::Center);
            if let Some(key) = key {
                content = content.push(keycap(key, scale));
            }
            button(content).padding([4, 10]).on_press(message).style(segment_style(SegmentLook::Quiet, lit)).into()
        };
        let controls = row![
            text(format!("{at} / {of} · {place}")).font(theme::mono_font()).size(scale.apply(11.0)).color(theme::text_dim()),
            control("‹", key(Action::Previous), Message::Perform(Action::Previous), false),
            control(if show.show.paused { "Play" } else { "Pause" }, None, Message::SlideshowPause, show.show.paused),
            control("›", key(Action::Next), Message::Perform(Action::Next), false),
            pick_list(Interval::ALL, Some(show.interval), Message::SlideshowInterval)
                .text_size(scale.apply(density::META_TEXT_BASE))
                .style(dropdown_style)
                .menu_style(dropdown_menu_style),
            control("Shuffle", None, Message::SlideshowShuffle, show.show.is_shuffled()),
            control("Loop", None, Message::SlideshowLoop, show.show.looping),
            control("Exit", Some("Esc".to_string()), Message::Perform(Action::Close), false),
        ]
        .spacing(spacing::SM)
        .align_y(iced::Alignment::Center);

        let bar = container(container(controls).padding([6, 12]).style(|_t: &Theme| floating()))
            .center_x(Length::Fill)
            .align_bottom(Length::Fill)
            .padding(28);
        stack![backdrop, bar].into()
    }
}

// --- pieces -------------------------------------------------------------------

/// Whether a grid row is the first of its day, where `grid::bands` puts
/// no row gap above it.
fn body_needs_no_gap(groups: &[grid::Group], indices: &[usize]) -> bool {
    indices.first().is_some_and(|first| groups.iter().any(|g| g.indices.first() == Some(first)))
}

/// A folder's path as crumbs: `~` for home, then each folder with the
/// path it goes to — the middle elided to one unclickable `…` past
/// [`MAX_CRUMBS`], for the reason the file manager's path bar gives.
fn crumbs_of(dir: &Path) -> Vec<(String, Option<PathBuf>)> {
    let home = home();
    let mut out: Vec<(String, Option<PathBuf>)> = Vec::new();
    let mut from = PathBuf::new();
    let rest = match home.as_deref().and_then(|h| dir.strip_prefix(h).ok().map(|rest| (h, rest))) {
        Some((h, rest)) => {
            out.push(("~".to_string(), Some(h.to_path_buf())));
            from.push(h);
            rest.to_path_buf()
        }
        None => {
            out.push(("".to_string(), Some(PathBuf::from("/"))));
            from.push("/");
            dir.strip_prefix("/").map(Path::to_path_buf).unwrap_or_else(|_| dir.to_path_buf())
        }
    };
    let mut folders = Vec::new();
    for part in rest.components() {
        from.push(part);
        folders.push((part.as_os_str().to_string_lossy().into_owned(), Some(from.clone())));
    }
    if folders.len() > MAX_CRUMBS {
        let tail = folders.split_off(folders.len() - (MAX_CRUMBS - 1));
        folders.truncate(1);
        folders.push(("…".to_string(), None));
        folders.extend(tail);
    }
    out.extend(folders);
    out
}

/// The mark a tile without a thumbnail carries: what kind of thing it is.
fn badge(media: Media) -> &'static str {
    match media {
        Media::Still => "",
        Media::Clip => "▶",
        Media::Model => "3D",
    }
}

fn home() -> Option<PathBuf> {
    std::env::var_os("HOME").map(PathBuf::from)
}

fn semibold() -> iced::Font {
    iced::Font { weight: iced::font::Weight::Semibold, ..iced::Font::DEFAULT }
}

fn heading<'a>(title: &str, scale: FontScale) -> Element<'a, Message> {
    container(section_label(title, scale)).padding([4, 10]).into()
}

/// A sidebar row: a folder mark in its tint, the name, and a count — the
/// count in the accent on the selected row, as the mockup has it.
fn sidebar_row<'a>(
    label: String,
    count: Option<String>,
    tint: Tint,
    selected: bool,
    path: PathBuf,
    scale: FontScale,
) -> Element<'a, Message> {
    let color = tint.iced();
    let mark = container(Space::new())
        .width(Length::Fixed(scale.apply(13.0)))
        .height(Length::Fixed(scale.apply(11.0)))
        .style(move |_t: &Theme| rounded(color, 2.0));
    let mut content = row![mark, scaled_text(label, density::ROW_TEXT_BASE * 0.9, scale).width(Length::Fill)]
        .spacing(10)
        .align_y(iced::Alignment::Center);
    if let Some(count) = count {
        let count_color = if selected { accent() } else { theme::text_dim() };
        content = content.push(text(count).font(theme::mono_font()).size(scale.apply(10.5)).color(count_color));
    }
    button(content)
        .width(Length::Fill)
        .padding([7, 10])
        .on_press(Message::OpenFolder(path))
        .style(move |t: &Theme, status| selectable_row_style(t, status, selected))
        .into()
}

/// The show/hide control for the sidebar, filled while it shows — the
/// file manager's `sidebar_toggle`, idiom for idiom.
fn sidebar_toggle<'a>(shown: bool, scale: FontScale) -> Element<'a, Message> {
    let side = density::glyph_button(scale);
    button(container(glyph::sidebar(side, theme::text())).center_x(Length::Fill).center_y(Length::Fill))
        .width(Length::Fixed(side))
        .height(Length::Fixed(side))
        .padding(0)
        .on_press(Message::Perform(Action::ToggleSidebar))
        .style(move |_t: &Theme, status| filled_button(shown, status, true))
        .into()
}

/// Back and forward, drawn marks in fixed squares. `fills` is whether the
/// live state is shown as a fill — Back's, as in the file manager, so the
/// fill means "there is somewhere to go back to".
fn nav_button<'a>(kind: glyph::Nav, message: Option<Message>, fills: bool, scale: FontScale) -> Element<'a, Message> {
    let enabled = message.is_some();
    let side = density::glyph_button(scale);
    let color = if enabled { theme::text() } else { theme::text_dim() };
    button(container(glyph::nav(kind, side, color)).center_x(Length::Fill).center_y(Length::Fill))
        .width(Length::Fixed(side))
        .height(Length::Fixed(side))
        .padding(0)
        .on_press_maybe(message)
        .style(move |_t: &Theme, status| filled_button(fills && enabled, status, enabled))
        .into()
}

/// A typed glyph in the same square as the drawn ones — ⟲ ✎ i ⋯.
fn glyph_button<'a>(glyph: &'a str, message: Option<Message>, on: bool, scale: FontScale) -> Element<'a, Message> {
    let enabled = message.is_some();
    let side = density::glyph_button(scale);
    button(
        container(text(glyph).size(scale.apply(15.0)).color(if enabled { theme::text() } else { theme::text_dim() }))
            .center_x(Length::Fill)
            .center_y(Length::Fill),
    )
    .width(Length::Fixed(side))
    .height(Length::Fixed(side))
    .padding(0)
    .on_press_maybe(message)
    .style(move |_t: &Theme, status| filled_button(on, status, enabled))
    .into()
}

/// "Filled means on": the one idiom for a live control across the bar.
fn filled_button(on: bool, status: button::Status, enabled: bool) -> button::Style {
    let hovered = enabled && matches!(status, button::Status::Hovered);
    button::Style {
        background: (on || hovered).then(|| Background::Color(surface::row())),
        text_color: if enabled { theme::text() } else { theme::text_dim() },
        border: Border { radius: density::inner_radius().into(), ..Border::default() },
        ..button::Style::default()
    }
}

/// The previous/next squares floating over the photograph.
fn overlay_arrow<'a>(glyph: &'a str, message: Message, scale: FontScale) -> Element<'a, Message> {
    button(container(text(glyph).font(theme::mono_font()).size(scale.apply(15.0))).center(Length::Fill))
        .width(Length::Fixed(32.0))
        .height(Length::Fixed(32.0))
        .padding(0)
        .on_press(message)
        .style(|_t: &Theme, status| {
            let mut style = floating_button(status);
            style.border.radius = 8.0.into();
            style
        })
        .into()
}

fn floating_button(status: button::Status) -> button::Style {
    let fill = match status {
        button::Status::Hovered | button::Status::Pressed => surface::row(),
        _ => with_alpha(surface::sidebar(), 0.75),
    };
    button::Style {
        background: Some(Background::Color(fill)),
        text_color: theme::text(),
        border: Border { radius: density::inner_radius().into(), width: 1.0, color: surface::card_border() },
        ..button::Style::default()
    }
}

/// A status-bar control that reads as text until pointed at.
fn link<'a>(label: &'a str, message: Message, scale: FontScale) -> Element<'a, Message> {
    button(scaled_text(label, density::META_TEXT_BASE * 0.9, scale))
        .padding([0, spacing::XS as u16])
        .on_press(message)
        .style(|t: &Theme, status| button::Style {
            background: None,
            text_color: match status {
                button::Status::Hovered | button::Status::Pressed => t.palette().primary,
                _ => theme::text(),
            },
            ..button::Style::default()
        })
        .into()
}

/// A button with no look of its own — crumbs and filmstrip frames.
fn bare_style(_t: &Theme, _status: button::Status) -> button::Style {
    button::Style { background: None, text_color: theme::text(), ..button::Style::default() }
}

/// A grid tile or library card: the accent's weak fill and a border in
/// it when selected — mockup `2b` — and nothing otherwise.
fn tile_style(t: &Theme, selected: bool) -> container::Style {
    let palette = t.extended_palette();
    container::Style {
        background: selected.then_some(Background::Color(palette.primary.weak.color)),
        border: Border {
            radius: density::card_radius().into(),
            width: 1.0,
            color: if selected { palette.primary.base.color } else { iced::Color::TRANSPARENT },
        },
        ..container::Style::default()
    }
}

fn plane(color: iced::Color) -> container::Style {
    container::Style { background: Some(Background::Color(color)), ..container::Style::default() }
}

fn rounded(color: iced::Color, radius: f32) -> container::Style {
    container::Style {
        background: Some(Background::Color(color)),
        border: Border { radius: radius.into(), ..Border::default() },
        ..container::Style::default()
    }
}

/// Anything floating over content: the zoom pill, the menu, the
/// slideshow's controls.
fn floating() -> container::Style {
    container::Style {
        background: Some(Background::Color(with_alpha(surface::sidebar(), 0.92))),
        border: Border { radius: density::inner_radius().into(), width: 1.0, color: surface::card_border() },
        ..container::Style::default()
    }
}

/// The theme's accent — Hyprland's own active-border colour.
fn accent() -> iced::Color {
    hyprforge_ui::color::to_iced(theme::active().accent)
}

fn with_alpha(mut color: iced::Color, alpha: f32) -> iced::Color {
    color.a = alpha;
    color
}

/// The 1px line between the chrome and what it frames — its own element
/// because iced's border draws on all four sides.
fn edge_h<'a>() -> Element<'a, Message> {
    container(Space::new()).width(Length::Fill).height(Length::Fixed(1.0)).style(|_t: &Theme| plane(surface::root())).into()
}

fn edge_v<'a>() -> Element<'a, Message> {
    container(Space::new()).width(Length::Fixed(1.0)).height(Length::Fill).style(|_t: &Theme| plane(surface::root())).into()
}
