//! The screen itself — the part a person actually looks at.
//!
//! This is what makes a greeter and a lock screen one system rather than
//! two that were styled to match. They do not share a *look*; they share
//! this function. Neither host can drift, because neither host draws
//! anything.
//!
//! Built as an iced `Element` so it renders two ways from one
//! description: the lock screen hands it to `iced_tiny_skia` and paints
//! the result into the buffer the compositor gave it, while the greeter
//! is an ordinary window — drawn by iced's tiny-skia backend too, never
//! wgpu, because wgpu blends translucency in linear light and the glass
//! would come out differently on each (see the greeter's `main`).
//! Software rendering is not a limitation here — it is the point. This is the surface between a locked machine and its
//! user, and it has no business depending on a GPU being in a good mood.
//!
//! Every colour comes from [`hyprforge_look::Theme`] explicitly rather
//! than from iced's own theming, so the screen looks the same whichever
//! host renders it and whatever iced's defaults happen to be. The
//! mockup's translucent glass is the theme's own colours at an alpha,
//! never a colour of this file's choosing.
//!
//! Sizes are the mockup's, in logical pixels, multiplied by [`unit`] so
//! a larger desktop font makes a larger lock screen. The mockup was drawn
//! against a 10pt desktop font, so at 10pt this is the mockup exactly.

use crate::conversation::State;
use crate::scene::{
    self, Action, Battery, Fingerprint, Media, Mode, NotificationCount, PowerMenu,
    Role, Scene,
};
use chrono::{DateTime, Local};
use hyprforge_look::Theme;
use hyprforge_ui::color::to_iced;
use iced_runtime::core::{
    alignment, gradient, mouse, Alignment, Background, Border, Color, Element, Font, Length,
    Padding, Point, Radians, Rectangle, Shadow, Vector,
};
use iced_widget::{button, canvas, column, container, row, stack, text, Space};

/// Everything a renderer has to be able to do to draw this screen: text,
/// the wallpaper, and the drawn glyphs. Both `iced_tiny_skia` (the lock)
/// and iced's own renderer (the greeter) are all three.
pub trait ScreenRenderer:
    iced_runtime::core::text::Renderer<Font = Font>
    + iced_runtime::core::image::Renderer<Handle = iced_runtime::core::image::Handle>
    + iced_widget::graphics::geometry::Renderer
{
}

impl<T> ScreenRenderer for T where
    T: iced_runtime::core::text::Renderer<Font = Font>
        + iced_runtime::core::image::Renderer<Handle = iced_runtime::core::image::Handle>
        + iced_widget::graphics::geometry::Renderer
{
}

type El<'a, R> = Element<'a, Action, iced_widget::Theme, R>;

/// Beyond this many characters the dots stop being countable anyway, and
/// a row that grows without limit would push the field apart.
const MAX_DOTS: usize = 24;

/// The smallest and largest text this screen will render at.
///
/// Not taste — survival. `cosmic-text` asserts that a line height is
/// non-zero, so a font size of zero panics the renderer outright, and a
/// size in the millions makes it try to lay out glyphs the size of a
/// building.
const MIN_FONT: f32 = 6.0;
const MAX_FONT: f32 = 96.0;
/// Corner radius and blur are bounded for the same reason: the geometry
/// they feed ends in `tiny_skia` path builders that return `None` for
/// degenerate shapes, and iced unwraps those.
const MAX_ROUNDING: u32 = 64;
const MAX_BLUR: u32 = 64;

/// A theme that cannot panic the renderer.
///
/// `lock.toml` is a file people edit by hand, and one that a program
/// writes without a schema. Anything can be in it: a zero font size, a
/// NaN, a wallpaper that is really a text file. The renderer underneath
/// this screen is not defensive — `iced_tiny_skia` and `cosmic-text` are
/// full of `expect`s on geometry, and one of them fires on a font size
/// of zero.
///
/// A panic here is the failure with no recovery. The compositor keeps
/// the session locked whatever happens to this process, so a crash
/// leaves a machine that cannot be unlocked without another TTY. Worse,
/// the font system is behind a global mutex, so the first panic poisons
/// it and every later attempt to draw fails too — there is no retrying
/// out of it.
///
/// So every value is brought into a range the renderer will accept,
/// rather than trusted or validated-and-rejected. Rejecting would mean
/// refusing to show a lock screen, which is the same lockout by a
/// politer route.
pub fn renderable(mut theme: Theme) -> Theme {
    let finite = |value: f32, fallback: f32| if value.is_finite() { value } else { fallback };

    theme.font_size = finite(theme.font_size, Theme::default().font_size).clamp(MIN_FONT, MAX_FONT);
    theme.font_scale = finite(theme.font_scale, 1.0).clamp(0.5, 3.0);
    theme.dim = finite(theme.dim, 0.0).clamp(0.0, 1.0);
    theme.rounding = theme.rounding.min(MAX_ROUNDING);
    theme.blur = theme.blur.min(MAX_BLUR);

    // A wallpaper the renderer cannot decode is its own hazard:
    // iced_tiny_skia 0.14 caches a failed load as "no entry", and its
    // next draw of the same handle hits an `expect`. Only the header is
    // read, so the cost does not scale with the size of the picture.
    if let Some(path) = theme.wallpaper.clone() {
        if !drawable(&path) {
            theme.wallpaper = None;
        }
    }
    theme
}

/// Whether an image file is one the renderer can draw — decodable, by
/// its header, and not zero-sized. The same guard the wallpaper gets,
/// for the same reason, and the avatar needs it too.
pub fn drawable(path: &std::path::Path) -> bool {
    image::ImageReader::open(path)
        .and_then(|reader| reader.with_guessed_format())
        .ok()
        .and_then(|reader| reader.into_dimensions().ok())
        .is_some_and(|(w, h)| w > 0 && h > 0)
}

/// The theme's font, as iced needs it.
///
/// iced wants a `&'static str` for a family name and the theme's font
/// arrives from gsettings at runtime, so the name has to be leaked.
/// Leaking is bounded by caching per family rather than per call: a
/// process ends up holding one small string per distinct font it was
/// ever asked for, which for a lock screen is two — this and the mono.
///
/// Deliberately not a `OnceLock` of a single name. That would make the
/// *first* theme's font win for the life of the process, so a greeter
/// or a reloaded theme would silently render in the wrong font with
/// nothing to explain why.
///
/// An empty or unresolvable family falls back to iced's default rather
/// than failing. A screen in the wrong font is a cosmetic problem; a
/// screen that refused to start over one would be a machine nobody can
/// get into.
pub fn font(theme: &Theme) -> Font {
    named(&theme.font, Font::DEFAULT)
}

/// The theme's monospace font — the status line, the layout, "attempt
/// 2" — or iced's own monospace when the theme names none. See
/// [`Theme::mono_font`] for why empty is the right default.
pub fn mono_font(theme: &Theme) -> Font {
    named(&theme.mono_font, Font::MONOSPACE)
}

fn named(name: &str, fallback: Font) -> Font {
    static FAMILIES: std::sync::Mutex<Option<std::collections::HashMap<String, &'static str>>> =
        std::sync::Mutex::new(None);

    let name = name.trim();
    if name.is_empty() {
        return fallback;
    }
    // A poisoned lock is recovered from rather than propagated: the only
    // thing in here is a font-name cache, and panicking a lock screen
    // over it would be trading a cosmetic problem for a lockout.
    let mut guard = FAMILIES.lock().unwrap_or_else(|e| e.into_inner());
    let cache = guard.get_or_insert_with(std::collections::HashMap::new);
    if let Some(existing) = cache.get(name) {
        return Font::with_name(existing);
    }
    let leaked: &'static str = Box::leak(name.to_owned().into_boxed_str());
    cache.insert(name.to_owned(), leaked);
    Font::with_name(leaked)
}

/// The mockup's pixel, for this theme.
///
/// Bounded both ways: a 96pt theme font must not turn the card into
/// something wider than the screen, and a 6pt one must not make the
/// field too small to see a dot in.
pub fn unit(theme: &Theme) -> f32 {
    let size = if theme.font_size.is_finite() { theme.font_size } else { 10.0 };
    (size / 10.0).clamp(0.8, 1.6)
}

/// The same colour, at `alpha` of its own opacity.
fn fade(color: hyprforge_look::Color, alpha: f32) -> Color {
    let c = to_iced(color);
    Color { a: c.a * alpha.clamp(0.0, 1.0), ..c }
}

/// `color` moved `amount` of the way towards `toward` — the lighter red
/// of "Shut down", the paler orange of the battery warning. Derived from
/// two theme colours rather than written down, so a theme with a
/// different red gets a matching lighter one.
fn mix(color: hyprforge_look::Color, toward: hyprforge_look::Color, amount: f32) -> Color {
    let (a, b, t) = (to_iced(color), to_iced(toward), amount.clamp(0.0, 1.0));
    Color {
        r: a.r + (b.r - a.r) * t,
        g: a.g + (b.g - a.g) * t,
        b: a.b + (b.b - a.b) * t,
        a: a.a,
    }
}

/// A font size that cannot reach the renderer as zero or as nonsense.
fn size(value: f32) -> f32 {
    if value.is_finite() {
        value.clamp(MIN_FONT, 400.0)
    } else {
        MIN_FONT
    }
}

/// The frosted panel every raised thing on this screen sits on: the
/// theme's background at half strength, a hairline of foreground, and a
/// soft shadow underneath.
///
/// Not blurred. The mockup's `backdrop-filter` has no equivalent in a
/// software renderer that draws each widget once, and blurring the whole
/// wallpaper under a card means decoding it twice — on a lock screen
/// where one oversized wallpaper already cost 296MB. The tint does most
/// of the work of making text legible over a photograph.
fn glass(theme: &Theme, radius: f32) -> container::Style {
    container::Style {
        background: Some(fade(theme.background, 0.55).into()),
        border: Border {
            color: fade(theme.foreground, 0.10),
            width: 1.0,
            radius: radius.into(),
        },
        shadow: Shadow {
            color: fade(theme.background, 0.45),
            offset: Vector::new(0.0, 20.0),
            blur_radius: 50.0,
        },
        ..Default::default()
    }
}

/// Text that stays legible over any wallpaper: the text, with a copy of
/// itself a pixel lower in the background colour behind it.
///
/// The mockup uses a CSS text shadow. iced has none, and a real blur per
/// glyph would be expensive in software; one offset copy is the part of
/// the shadow that does the work.
fn lifted<'a, R: ScreenRenderer + 'a>(
    content: String,
    px: f32,
    font: Font,
    color: Color,
    theme: &Theme,
) -> El<'a, R> {
    let shade = fade(theme.background, 0.65 * color.a);
    stack![
        container(text(content.clone()).size(size(px)).font(font).color(shade))
            .padding(Padding { top: 1.5, left: 0.0, right: 0.0, bottom: 0.0 }),
        text(content).size(size(px)).font(font).color(color),
    ]
    .into()
}

/// The whole screen, for one frame.
pub fn view<'a, R: ScreenRenderer + 'a>(scene: Scene<'a>) -> El<'a, R> {
    let theme = scene.theme;
    let content: El<'a, R> = match scene.role {
        Role::Secondary => centred(clock(&scene, true), theme),
        Role::Primary => primary(scene.clone()),
    };

    if let Some(backdrop) = scene.backdrop {
        // Already the output's size, already dimmed: drawn one-to-one,
        // and nearest-neighbour because at exactly one source pixel per
        // buffer pixel there is nothing to filter.
        return stack![
            iced_widget::image(backdrop.clone())
                .width(Length::Fill)
                .height(Length::Fill)
                .content_fit(iced_runtime::core::ContentFit::Fill)
                .filter_method(iced_runtime::core::image::FilterMethod::Nearest),
            content,
        ]
        .into();
    }

    match &theme.wallpaper {
        Some(path) => stack![
            // `Cover` rather than `Contain`: letterbox bars around a lock
            // screen look like a rendering fault, and the wallpaper is
            // backdrop rather than something being examined.
            iced_widget::image(path)
                .width(Length::Fill)
                .height(Length::Fill)
                .content_fit(iced_runtime::core::ContentFit::Cover),
            // Dimming is what keeps the prompt readable over an
            // arbitrary photograph. Without it the whole screen depends
            // on the user having chosen a dark wallpaper.
            container(Space::new().width(Length::Fill).height(Length::Fill)).style(move |_| {
                container::Style {
                    background: Some(fade(theme.background, theme.dim.clamp(0.0, 1.0)).into()),
                    ..Default::default()
                }
            }),
            content,
        ]
        .into(),
        // No wallpaper is not a failure — it is the normal case on a
        // fresh install, and the flat background is legible on its own.
        None => container(content)
            .width(Length::Fill)
            .height(Length::Fill)
            .style(move |_| container::Style {
                background: Some(to_iced(theme.background).into()),
                ..Default::default()
            })
            .into(),
    }
}

/// Centres one thing on the screen, nudged up by the mockup's 40px so
/// the eye's centre and the screen's agree.
fn centred<'a, R: ScreenRenderer + 'a>(inner: El<'a, R>, theme: &Theme) -> El<'a, R> {
    container(inner)
        .width(Length::Fill)
        .height(Length::Fill)
        .padding(Padding { bottom: 40.0 * unit(theme), ..Padding::ZERO })
        .center_x(Length::Fill)
        .center_y(Length::Fill)
        .into()
}

/// The output with the keyboard: everything.
fn primary<'a, R: ScreenRenderer + 'a>(scene: Scene<'a>) -> El<'a, R> {
    let theme = scene.theme;
    let u = unit(theme);
    let idle = scene.mode == Mode::Idle;

    let mut middle = column![clock(&scene, idle)].align_x(Alignment::Center).spacing(40.0 * u);
    if idle {
        if let Some(extras) = idle_extras(&scene) {
            middle = middle.push(extras);
        }
    } else {
        middle = middle.push(card(&scene));
    }

    let mut layers = stack![centred(middle.into(), theme)];
    layers = layers.push(
        container(status_row(&scene))
            .width(Length::Fill)
            .padding(Padding { top: 22.0 * u, right: 26.0 * u, ..Padding::ZERO })
            .align_right(Length::Fill),
    );
    if idle {
        let hint = scene::idle_hint(scene.fingerprint).to_string();
        layers = layers.push(
            container(lifted(hint, 13.0 * u, font(theme), fade(theme.foreground, 0.62), theme))
                .width(Length::Fill)
                .height(Length::Fill)
                .padding(Padding { bottom: 30.0 * u, ..Padding::ZERO })
                .center_x(Length::Fill)
                .align_bottom(Length::Fill),
        );
    }
    if let Some(menu) = scene.power {
        layers = layers.push(
            container(power_button(theme))
                .width(Length::Fill)
                .height(Length::Fill)
                .padding(Padding { right: 26.0 * u, bottom: 24.0 * u, ..Padding::ZERO })
                .align_right(Length::Fill)
                .align_bottom(Length::Fill),
        );
        if let Some(menu) = menu {
            layers = layers.push(
                container(power_menu(menu, theme))
                    .width(Length::Fill)
                    .height(Length::Fill)
                    .padding(Padding { right: 26.0 * u, bottom: 80.0 * u, ..Padding::ZERO })
                    .align_right(Length::Fill)
                    .align_bottom(Length::Fill),
            );
        }
    }
    layers.into()
}

/// How big the clock is: the mockup's 180px while idle and 120px with
/// the card up, but never more than a fifth of the output — a portrait
/// monitor or a small window would otherwise be all clock.
fn clock_size(theme: &Theme, idle: bool, now_fits: (f32, f32)) -> f32 {
    let u = unit(theme);
    let (w, h) = now_fits;
    let (ideal, of_height, of_width) = if idle { (180.0, 0.2, 0.17) } else { (120.0, 0.14, 0.13) };
    let bound = if w > 0.0 && h > 0.0 { (h * of_height).min(w * of_width) } else { f32::MAX };
    size((ideal * u).min(bound))
}

/// The time and the date.
///
/// Sized against [`Scene::output`], which the host fills in because the
/// view itself cannot know it: iced lays out after this returns.
fn clock<'a, R: ScreenRenderer + 'a>(scene: &Scene<'a>, idle: bool) -> El<'a, R> {
    let theme = scene.theme;
    let u = unit(theme);
    let fg = to_iced(theme.foreground);
    let heavy = Font { weight: iced_runtime::core::font::Weight::Semibold, ..font(theme) };
    let medium = Font { weight: iced_runtime::core::font::Weight::Medium, ..font(theme) };

    let time = formatted(&scene.now, &theme.clock_format, "%H:%M");
    let date = tracked(&formatted(&scene.now, &theme.date_format, "%A, %e %B").to_uppercase());
    let px = clock_size(theme, idle, scene.output);

    let mut parts = column![].align_x(Alignment::Center).spacing(6.0 * u);
    if !time.is_empty() {
        parts = parts.push(lifted(time, px, heavy, fg, theme));
    }
    if !date.is_empty() {
        parts = parts.push(lifted(date, 15.0 * u, medium, fg, theme));
    }
    parts.into()
}

/// Letter-spaced, the way the mockup sets the date: a hair space between
/// letters. iced has no tracking control, and this is the whole of what
/// `letter-spacing: .14em` does to a line of capitals.
///
/// Only for text that is plain Latin letters and spaces. Anything else —
/// a date in a script that joins, or combining marks — would be broken
/// apart by spaces inserted between its characters.
fn tracked(text: &str) -> String {
    let text = text.split_whitespace().collect::<Vec<_>>().join(" ");
    if !text.chars().all(|c| c.is_ascii_alphanumeric() || c == ' ' || c == ',' || c == '.') {
        return text;
    }
    let mut out = String::with_capacity(text.len() * 4);
    for (i, c) in text.chars().enumerate() {
        if i > 0 {
            out.push('\u{200a}');
        }
        out.push(c);
    }
    out
}

/// The card: an optional battery warning, the avatar and the name, the
/// prompt, and the line under it.
fn card<'a, R: ScreenRenderer + 'a>(scene: &Scene<'a>) -> El<'a, R> {
    let theme = scene.theme;
    let u = unit(theme);
    let typed = typed_count(scene.state);

    let mut body = column![].align_x(Alignment::Center).spacing(18.0 * u);
    if let Some(battery) = scene.status.battery.filter(|b| b.low && !b.charging) {
        body = body.push(battery_warning(battery, theme));
    }
    body = body.push(identity(scene));

    if scene::offers_fingerprint(scene.state, typed, scene.fingerprint) {
        body = body.push(fingerprint(scene.fingerprint, theme));
    } else {
        body = body.push(field(scene));
        body = body.push(under_field(scene));
    }

    container(body)
        .width(Length::Fixed(380.0 * u))
        .padding(Padding { top: 26.0 * u, right: 28.0 * u, bottom: 22.0 * u, left: 28.0 * u })
        .center_x(Length::Fixed(380.0 * u))
        .style(move |_| glass(theme, 18.0 * u))
        .into()
}

/// The avatar and the name above the prompt.
fn identity<'a, R: ScreenRenderer + 'a>(scene: &Scene<'a>) -> El<'a, R> {
    let theme = scene.theme;
    let u = unit(theme);
    let side = 72.0 * u;

    // Checked here, every frame, rather than trusted from the host: a
    // path that does not decode panics `iced_tiny_skia` on its next draw
    // ("Image should be allocated"), which the sweep below proved on the
    // first run. A header read a second is nothing next to that.
    let avatar: El<'a, R> = match scene.avatar.filter(|path| drawable(path)) {
        Some(path) => iced_widget::image(iced_runtime::core::image::Handle::from_path(path))
            .width(Length::Fixed(side))
            .height(Length::Fixed(side))
            .content_fit(iced_runtime::core::ContentFit::Cover)
            .border_radius(side / 2.0)
            .into(),
        None => {
            let accent = theme.accent;
            let background = theme.background;
            container(
                text(scene::initial(scene.username))
                    .size(size(30.0 * u))
                    .font(Font { weight: iced_runtime::core::font::Weight::Semibold, ..font(theme) })
                    .color(fade(theme.foreground, 0.92)),
            )
            .width(Length::Fixed(side))
            .height(Length::Fixed(side))
            .center_x(Length::Fixed(side))
            .center_y(Length::Fixed(side))
            .style(move |_| container::Style {
                // The accent falling away into the background, which is
                // the mockup's pink-to-purple-to-grey in whatever colours
                // this theme actually has.
                background: Some(Background::Gradient(
                    gradient::Linear::new(Radians(std::f32::consts::FRAC_PI_4 * 3.0))
                        .add_stop(0.0, mix(accent, hyprforge_look::Color::rgba(0xff, 0xff, 0xff, 0xff), 0.25))
                        .add_stop(0.55, to_iced(accent))
                        .add_stop(1.0, mix(accent, background, 0.55))
                        .into(),
                )),
                border: Border { radius: (side / 2.0).into(), ..Default::default() },
                shadow: Shadow {
                    color: fade(background, 0.45),
                    offset: Vector::new(0.0, 6.0),
                    blur_radius: 20.0,
                },
                ..Default::default()
            })
            .into()
        }
    };

    let name = Font { weight: iced_runtime::core::font::Weight::Medium, ..font(theme) };
    column![
        avatar,
        lifted(scene::display_name(scene.username), 17.0 * u, name, to_iced(theme.foreground), theme),
    ]
    .align_x(Alignment::Center)
    .spacing(10.0 * u)
    .into()
}

/// The field: dots for a secret, the text itself for an answer that is
/// not one, and a caret while something is being asked.
///
/// Red, and swinging, while a rejection is being shown.
fn field<'a, R: ScreenRenderer + 'a>(scene: &Scene<'a>) -> El<'a, R> {
    let theme = scene.theme;
    let u = unit(theme);
    let rejected = scene.rejection;
    let ink = if rejected.is_some() { to_iced(theme.error) } else { to_iced(theme.foreground) };

    let (dots, visible) = match scene.state {
        State::Asking { prompt, entered } if !prompt.secret => (0, Some(entered.expose().clone())),
        State::Asking { .. } => (typed_count(scene.state), None),
        // Checking: the attempt stays on screen as it was submitted.
        State::Working => (scene.submitted.min(MAX_DOTS), None),
        State::Failed { .. } => (rejected.map_or(0, |r| r.dots.min(MAX_DOTS)), None),
        _ => (0, None),
    };
    let asking = scene.state.accepts_input();

    let dot = 9.0 * u;
    let mut marks = row![].spacing(9.0 * u).align_y(Alignment::Center);
    for _ in 0..dots {
        marks = marks.push(
            container(Space::new().width(dot).height(dot)).style(move |_| container::Style {
                background: Some(ink.into()),
                border: Border { radius: (dot / 2.0).into(), ..Default::default() },
                ..Default::default()
            }),
        );
    }
    if let Some(answer) = visible.filter(|a| !a.is_empty()) {
        marks = marks.push(text(answer).size(size(15.0 * u)).font(font(theme)).color(ink));
    }
    // An empty field says what it is waiting for, in PAM's own words —
    // "Password", "Verification code" — rather than nothing at all.
    let empty = dots == 0 && typed_count(scene.state) == 0;
    if asking {
        marks = marks.push(
            container(Space::new().width(2.0 * u).height(18.0 * u)).style(move |_| container::Style {
                background: Some(Color { a: 0.8, ..ink }.into()),
                ..Default::default()
            }),
        );
        if empty {
            let label = prompt_label(scene.state).trim_end_matches(':').trim().to_string();
            marks = marks.push(text(label).size(size(13.0 * u)).font(font(theme)).color(fade(theme.foreground, 0.45)));
        }
    }

    let (fill, edge) = match rejected {
        Some(_) => (fade(theme.error, 0.14), fade(theme.error, 0.65)),
        None => (fade(theme.background, 0.5), fade(theme.foreground, 0.14)),
    };
    let boxed = container(marks)
        .width(Length::Fixed(300.0 * u))
        .height(Length::Fixed(46.0 * u))
        .center_x(Length::Fixed(300.0 * u))
        .center_y(Length::Fixed(46.0 * u))
        .clip(true)
        .style(move |_| container::Style {
            background: Some(fill.into()),
            border: Border { color: edge, width: 1.0, radius: (12.0 * u).into() },
            ..Default::default()
        });

    // The shake: the same box, with the slack on one side or the other.
    // Padding rather than a transform, because iced has no transform —
    // and a box that moved by changing its own width would visibly
    // squash.
    let offset = rejected.map_or(0.0, |r| r.offset).clamp(-12.0, 12.0) * u;
    let slack = 12.0 * u;
    container(boxed)
        .padding(Padding { left: slack + offset, right: slack - offset, ..Padding::ZERO })
        .into()
}

/// The line under the field: what went wrong, what PAM said, or — when
/// nothing is wrong — the layout and Caps Lock, the two things that make
/// a correct password come out wrong.
fn under_field<'a, R: ScreenRenderer + 'a>(scene: &Scene<'a>) -> El<'a, R> {
    let theme = scene.theme;
    let u = unit(theme);
    let mono = mono_font(theme);
    let fg = to_iced(theme.foreground);
    let medium = Font { weight: iced_runtime::core::font::Weight::Medium, ..font(theme) };

    let (message, failed) = status_text(scene.state);
    if !message.is_empty() {
        let colour = if failed { to_iced(theme.error) } else { fg };
        let mut line = row![text(message)
            .size(size(13.0 * u))
            .font(medium)
            .color(colour)
            .align_x(alignment::Horizontal::Center)]
        .spacing(10.0 * u)
        .align_y(Alignment::Center);
        if let Some(rejection) = scene.rejection.filter(|r| r.attempt > 0) {
            line = line.push(
                text(format!("attempt {}", rejection.attempt))
                    .size(size(11.0 * u))
                    .font(mono)
                    .color(mix(theme.error, theme.foreground, 0.45)),
            );
        }
        return line.into();
    }
    if matches!(scene.state, State::Working) {
        return text("Checking…").size(size(11.0 * u)).font(mono).color(fade(theme.foreground, 0.7)).into();
    }

    let mut line = row![].spacing(14.0 * u).align_y(Alignment::Center);
    if let Some(layout) = &scene.status.layout {
        line = line.push(text(format!("{layout} layout")).size(size(11.0 * u)).font(mono).color(fg));
    }
    // Not a nicety. Without it a stuck key is indistinguishable from a
    // forgotten password, and where `pam_faillock` is configured (it is,
    // on the machine this was written on, at three attempts) that turns
    // a stuck key into a locked *account*.
    let (caps, caps_colour) = caps_lock_line(scene.caps_lock, theme);
    line = line.push(text(caps).size(size(11.0 * u)).font(mono).color(caps_colour));
    if matches!(scene.fingerprint, Fingerprint::Exhausted) {
        line = line.push(
            text("fingerprint off").size(size(11.0 * u)).font(mono).color(fade(theme.foreground, 0.6)),
        );
    }
    line.into()
}

/// The Caps Lock half of the line under the field.
fn caps_lock_line(on: bool, theme: &Theme) -> (&'static str, Color) {
    if on {
        ("Caps Lock on", to_iced(theme.warning))
    } else {
        ("Caps Lock off", to_iced(theme.foreground))
    }
}

/// The fingerprint prompt, in place of the field.
fn fingerprint<'a, R: ScreenRenderer + 'a>(reader: &Fingerprint, theme: &Theme) -> El<'a, R> {
    let u = unit(theme);
    let info = theme.info;
    let ring = |side: f32, alpha: f32, inner: El<'a, R>| -> El<'a, R> {
        container(inner)
            .width(Length::Fixed(side))
            .height(Length::Fixed(side))
            .center_x(Length::Fixed(side))
            .center_y(Length::Fixed(side))
            .style(move |_| container::Style {
                border: Border { color: fade(info, alpha), width: 2.0, radius: (side / 2.0).into() },
                ..Default::default()
            })
            .into()
    };
    let core = ring(12.0 * u, 0.5, Space::new().into());
    let middle = ring(34.0 * u, 0.7, core);
    let outer = ring(58.0 * u, 1.0, middle);
    // The glow: the mockup's 8px spread shadow, which is a ring of the
    // same colour at a seventh of its strength.
    let glow = container(outer)
        .padding(8.0 * u)
        .style(move |_| container::Style {
            background: Some(fade(info, 0.14).into()),
            border: Border { radius: (37.0 * u).into(), ..Default::default() },
            ..Default::default()
        });

    let (said, colour) = match reader {
        Fingerprint::Retry(why) => (why.clone(), to_iced(theme.warning)),
        _ => ("Place your finger on the sensor".to_string(), to_iced(theme.foreground)),
    };
    let medium = Font { weight: iced_runtime::core::font::Weight::Medium, ..font(theme) };
    column![
        glow,
        lifted(said, 14.0 * u, medium, colour, theme),
        text("fprintd · or start typing a password")
            .size(size(11.0 * u))
            .font(mono_font(theme))
            .color(to_iced(theme.foreground)),
    ]
    .align_x(Alignment::Center)
    .spacing(12.0 * u)
    .into()
}

/// "Battery 7% · plug in soon", as a pill at the top of the card.
fn battery_warning<'a, R: ScreenRenderer + 'a>(battery: Battery, theme: &Theme) -> El<'a, R> {
    let u = unit(theme);
    let warning = theme.warning;
    let dot = 7.0 * u;
    container(
        row![
            container(Space::new().width(dot).height(dot)).style(move |_| container::Style {
                background: Some(to_iced(warning).into()),
                border: Border { radius: (dot / 2.0).into(), ..Default::default() },
                ..Default::default()
            }),
            text(format!("Battery {}% · plug in soon", battery.percent))
                .size(size(12.5 * u))
                .font(Font { weight: iced_runtime::core::font::Weight::Medium, ..font(theme) })
                .color(mix(theme.warning, theme.foreground, 0.4)),
        ]
        .spacing(8.0 * u)
        .align_y(Alignment::Center),
    )
    .padding(Padding { top: 7.0 * u, bottom: 7.0 * u, left: 12.0 * u, right: 12.0 * u })
    .style(move |_| container::Style {
        background: Some(fade(warning, 0.18).into()),
        border: Border { radius: 999.0.into(), ..Default::default() },
        ..Default::default()
    })
    .into()
}

/// Layout, network, battery — top right.
fn status_row<'a, R: ScreenRenderer + 'a>(scene: &Scene<'a>) -> El<'a, R> {
    let theme = scene.theme;
    let u = unit(theme);
    let mono = mono_font(theme);
    let fg = to_iced(theme.foreground);
    let status = scene.status;

    let mut line = row![].spacing(18.0 * u).align_y(Alignment::Center);
    if let Some(layout) = &status.layout {
        line = line.push(lifted(layout.clone(), 12.0 * u, mono, fg, theme));
    }
    if let Some(network) = &status.network {
        line = line.push(lifted(network.clone(), 12.0 * u, mono, fg, theme));
    }
    if let Some(battery) = status.battery {
        // The low state turns the whole reading the warning colour,
        // outline included — the mockup's "status turns orange".
        let ink = if battery.low && !battery.charging { to_iced(theme.warning) } else { fg };
        let mut reading = row![battery_icon(battery, ink, u)].spacing(7.0 * u).align_y(Alignment::Center);
        reading = reading.push(lifted(format!("{}%", battery.percent), 12.0 * u, mono, ink, theme));
        if battery.charging {
            reading = reading.push(glyph(Glyph::Bolt, 12.0 * u, ink));
        }
        line = line.push(reading);
    }
    line.into()
}

/// A battery drawn as an outline with a fill, the mockup's 20×10.
fn battery_icon<'a, R: ScreenRenderer + 'a>(battery: Battery, ink: Color, u: f32) -> El<'a, R> {
    let inner = 14.0 * u;
    let fill = (inner * f32::from(battery.percent.min(100)) / 100.0).max(1.0);
    container(
        container(Space::new().width(fill).height(Length::Fill)).style(move |_| container::Style {
            background: Some(ink.into()),
            border: Border { radius: 1.0.into(), ..Default::default() },
            ..Default::default()
        }),
    )
    .width(Length::Fixed(20.0 * u))
    .height(Length::Fixed(10.0 * u))
    .padding(2.0 * u)
    .style(move |_| container::Style {
        border: Border { color: ink, width: 1.5 * u, radius: (3.0 * u).into() },
        ..Default::default()
    })
    .into()
}

/// The idle screen's extras: what is playing and who has been in touch.
/// `None` when there is neither, so the clock sits alone.
fn idle_extras<'a, R: ScreenRenderer + 'a>(scene: &Scene<'a>) -> Option<El<'a, R>> {
    let theme = scene.theme;
    let u = unit(theme);
    if scene.media.is_none() && scene.notifications.is_empty() {
        return None;
    }
    let mut extras = column![].align_x(Alignment::Center).spacing(12.0 * u);
    if let Some(media) = scene.media {
        extras = extras.push(media_card(media, theme));
    }
    if !scene.notifications.is_empty() {
        extras = extras.push(notification_pills(scene.notifications, theme));
    }
    // The mockup gives the extras a little more air than the card.
    Some(container(extras).padding(Padding { top: 10.0 * u, ..Padding::ZERO }).into())
}

/// Now playing, with previous / play-pause / next.
fn media_card<'a, R: ScreenRenderer + 'a>(media: &Media, theme: &'a Theme) -> El<'a, R> {
    let u = unit(theme);
    let fg = to_iced(theme.foreground);
    let (accent, error, background) = (theme.accent, theme.error, theme.background);

    let art = container(Space::new().width(48.0 * u).height(48.0 * u)).style(move |_| container::Style {
        background: Some(Background::Gradient(
            gradient::Linear::new(Radians(std::f32::consts::PI * 150.0 / 180.0))
                .add_stop(0.0, mix(error, accent, 0.35))
                .add_stop(0.6, to_iced(accent))
                .add_stop(1.0, mix(accent, background, 0.5))
                .into(),
        )),
        border: Border { radius: (8.0 * u).into(), ..Default::default() },
        ..Default::default()
    });

    let byline = [media.artist.as_str(), media.player.as_str()]
        .into_iter()
        .filter(|s| !s.is_empty())
        .collect::<Vec<_>>()
        .join(" · ");
    let mut words = column![
        text(media.title.clone())
            .size(size(13.5 * u))
            .font(Font { weight: iced_runtime::core::font::Weight::Medium, ..font(theme) })
            .color(fg)
            .wrapping(iced_runtime::core::text::Wrapping::None),
        text(byline)
            .size(size(11.0 * u))
            .font(mono_font(theme))
            .color(fg)
            .wrapping(iced_runtime::core::text::Wrapping::None),
    ]
    .spacing(3.0 * u)
    .width(Length::Fill);
    if let Some(progress) = media.progress.filter(|p| p.is_finite()) {
        // Proportions rather than a pixel width: the track is as wide as
        // whatever the title and the controls leave, which the view does
        // not know until iced lays it out.
        let done = (progress.clamp(0.0, 1.0) * 1000.0).round() as u16;
        let track = container(
            row![
                container(Space::new().height(3.0 * u))
                    .width(Length::FillPortion(done.max(1)))
                    .style(move |_| container::Style {
                        background: Some(fg.into()),
                        border: Border { radius: (2.0 * u).into(), ..Default::default() },
                        ..Default::default()
                    }),
                Space::new().width(Length::FillPortion((1000 - done).max(1))),
            ]
            .width(Length::Fill),
        )
        .width(Length::Fill)
        .style(move |_| container::Style {
            background: Some(Color { a: 0.2, ..fg }.into()),
            border: Border { radius: (2.0 * u).into(), ..Default::default() },
            ..Default::default()
        });
        words = words.push(container(track).padding(Padding { top: 5.0 * u, ..Padding::ZERO }));
    }

    let side = 32.0 * u;
    let control = |kind: Glyph, action: Option<Action>, filled: bool| -> El<'a, R> {
        let mark = container(glyph(kind, 14.0 * u, if action.is_some() { fg } else { Color { a: 0.35, ..fg } }))
            .width(Length::Fixed(side))
            .height(Length::Fixed(side))
            .center_x(Length::Fixed(side))
            .center_y(Length::Fixed(side))
            .style(move |_| container::Style {
                background: filled.then(|| Color { a: 0.16, ..fg }.into()),
                border: Border { radius: (side / 2.0).into(), ..Default::default() },
                ..Default::default()
            });
        bare(mark.into(), action)
    };
    let controls = row![
        control(Glyph::Previous, media.can_previous.then_some(Action::MediaPrevious), false),
        control(
            if media.playing { Glyph::Pause } else { Glyph::Play },
            Some(Action::MediaPlayPause),
            true
        ),
        control(Glyph::Next, media.can_next.then_some(Action::MediaNext), false),
    ]
    .spacing(4.0 * u)
    .align_y(Alignment::Center);

    container(row![art, words, controls].spacing(12.0 * u).align_y(Alignment::Center))
        .width(Length::Fixed(380.0 * u))
        .padding(12.0 * u)
        .style(move |_| glass(theme, 14.0 * u))
        .into()
}

/// "Signal 3 · Mail 12" — application and count, never the text.
fn notification_pills<'a, R: ScreenRenderer + 'a>(counts: &[NotificationCount], theme: &'a Theme) -> El<'a, R> {
    let u = unit(theme);
    let fg = to_iced(theme.foreground);
    let accent = theme.accent;
    let mut pills = row![].spacing(8.0 * u);
    for NotificationCount { app, count } in counts.iter().take(4) {
        let badge = container(
            text(count.to_string())
                .size(size(11.0 * u))
                .font(Font { weight: iced_runtime::core::font::Weight::Semibold, ..mono_font(theme) })
                .color(fg),
        )
        .padding(Padding { top: 1.0 * u, bottom: 1.0 * u, left: 6.0 * u, right: 6.0 * u })
        .style(move |_| container::Style {
            background: Some(fade(accent, 0.35).into()),
            border: Border { radius: 999.0.into(), ..Default::default() },
            ..Default::default()
        });
        pills = pills.push(
            container(
                row![
                    text(app.clone())
                        .size(size(12.5 * u))
                        .font(Font { weight: iced_runtime::core::font::Weight::Medium, ..font(theme) })
                        .color(fg),
                    badge,
                ]
                .spacing(8.0 * u)
                .align_y(Alignment::Center),
            )
            .padding(Padding { top: 7.0 * u, bottom: 7.0 * u, left: 12.0 * u, right: 12.0 * u })
            .style(move |_| glass(theme, 999.0)),
        );
    }
    pills.into()
}

/// The ⏻ in the corner.
fn power_button<'a, R: ScreenRenderer + 'a>(theme: &'a Theme) -> El<'a, R> {
    let u = unit(theme);
    let side = 44.0 * u;
    let mark = container(glyph(Glyph::Power, 18.0 * u, to_iced(theme.foreground)))
        .width(Length::Fixed(side))
        .height(Length::Fixed(side))
        .center_x(Length::Fixed(side))
        .center_y(Length::Fixed(side))
        .style(move |_| glass(theme, side / 2.0));
    bare(mark.into(), Some(Action::TogglePowerMenu))
}

/// The power menu, opened from ⏻.
fn power_menu<'a, R: ScreenRenderer + 'a>(menu: &PowerMenu, theme: &'a Theme) -> El<'a, R> {
    let u = unit(theme);
    let accent = theme.accent;
    let mut rows = column![];
    for (index, action) in menu.actions.iter().copied().enumerate() {
        let selected = index == menu.selected;
        let colour = if action.dangerous() {
            mix(theme.error, theme.foreground, 0.3)
        } else {
            to_iced(theme.foreground)
        };
        let weight = if selected {
            iced_runtime::core::font::Weight::Medium
        } else {
            iced_runtime::core::font::Weight::Normal
        };
        let line = container(
            row![
                text(action.label()).size(size(13.5 * u)).font(Font { weight, ..font(theme) }).color(colour),
                Space::new().width(Length::Fill),
                text(action.key().to_ascii_uppercase().to_string())
                    .size(size(11.0 * u))
                    .font(mono_font(theme))
                    .color(to_iced(theme.surfaces.text_dim)),
            ]
            .align_y(Alignment::Center),
        )
        .width(Length::Fill)
        .height(Length::Fixed(38.0 * u))
        .padding(Padding { left: 12.0 * u, right: 12.0 * u, ..Padding::ZERO })
        .center_y(Length::Fixed(38.0 * u))
        .style(move |_| container::Style {
            background: selected.then(|| fade(accent, 0.22).into()),
            border: Border { radius: (9.0 * u).into(), ..Default::default() },
            ..Default::default()
        });
        rows = rows.push(bare(line.into(), Some(Action::Power(action))));
    }
    container(rows)
        .width(Length::Fixed(230.0 * u))
        .padding(6.0 * u)
        .style(move |_| glass(theme, 14.0 * u))
        .into()
}

/// A click target with no look of its own — the content is the look.
/// `None` makes it inert, which iced draws exactly the same.
fn bare<'a, R: ScreenRenderer + 'a>(content: El<'a, R>, action: Option<Action>) -> El<'a, R> {
    button(content)
        .padding(0)
        .on_press_maybe(action)
        .style(|_, _| button::Style { background: None, ..button::Style::default() })
        .into()
}

/// The marks this screen draws rather than types. A font's `⏻` or `‹`
/// is whatever the desktop's font thinks it is — often missing, the
/// power symbol especially — and a tofu box in the corner of a lock
/// screen reads as something broken.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Glyph {
    Power,
    Previous,
    Next,
    Play,
    Pause,
    Bolt,
}

fn glyph<'a, R: ScreenRenderer + 'a>(kind: Glyph, side: f32, color: Color) -> El<'a, R> {
    canvas(Mark { kind, color })
        .width(Length::Fixed(side.max(1.0)))
        .height(Length::Fixed(side.max(1.0)))
        .into()
}

struct Mark {
    kind: Glyph,
    color: Color,
}

impl<R: ScreenRenderer> canvas::Program<Action, iced_widget::Theme, R> for Mark {
    type State = ();

    fn draw(
        &self,
        _state: &(),
        renderer: &R,
        _theme: &iced_widget::Theme,
        bounds: Rectangle,
        _cursor: mouse::Cursor,
    ) -> Vec<R::Geometry> {
        use iced_widget::graphics::geometry::{Frame, LineCap, LineJoin, Path, Stroke, Style};

        let mut frame = Frame::new(renderer, bounds.size());
        let s = bounds.width.min(bounds.height);
        let c = Point::new(bounds.width / 2.0, bounds.height / 2.0);
        let stroke = Stroke {
            style: Style::Solid(self.color),
            width: (s * 0.11).max(1.0),
            line_cap: LineCap::Round,
            line_join: LineJoin::Round,
            ..Stroke::default()
        };
        match self.kind {
            Glyph::Power => {
                // An arc open at the top, with the stroke through the gap.
                let r = s * 0.36;
                let arc = Path::new(|b| {
                    b.arc(iced_widget::graphics::geometry::path::Arc {
                        center: c,
                        radius: r,
                        start_angle: Radians(-std::f32::consts::FRAC_PI_2 + 0.75),
                        end_angle: Radians(std::f32::consts::PI * 1.5 - 0.75),
                    })
                });
                let line = Path::line(Point::new(c.x, c.y - s * 0.46), Point::new(c.x, c.y - s * 0.02));
                frame.stroke(&arc, stroke);
                frame.stroke(&line, stroke);
            }
            Glyph::Previous | Glyph::Next => {
                let dir = if self.kind == Glyph::Next { 1.0 } else { -1.0 };
                let e = s * 0.26;
                let chevron = Path::new(|b| {
                    b.move_to(Point::new(c.x - dir * e * 0.5, c.y - e));
                    b.line_to(Point::new(c.x + dir * e * 0.5, c.y));
                    b.line_to(Point::new(c.x - dir * e * 0.5, c.y + e));
                });
                frame.stroke(&chevron, stroke);
            }
            Glyph::Play => {
                let e = s * 0.3;
                let triangle = Path::new(|b| {
                    b.move_to(Point::new(c.x - e * 0.7, c.y - e));
                    b.line_to(Point::new(c.x + e, c.y));
                    b.line_to(Point::new(c.x - e * 0.7, c.y + e));
                    b.close();
                });
                frame.fill(&triangle, self.color);
            }
            Glyph::Pause => {
                let (w, h) = (s * 0.14, s * 0.56);
                for x in [c.x - s * 0.17, c.x + s * 0.03] {
                    frame.fill(
                        &Path::rounded_rectangle(
                            Point::new(x, c.y - h / 2.0),
                            iced_runtime::core::Size::new(w, h),
                            (w * 0.3).into(),
                        ),
                        self.color,
                    );
                }
            }
            Glyph::Bolt => {
                let bolt = Path::new(|b| {
                    b.move_to(Point::new(c.x + s * 0.12, c.y - s * 0.46));
                    b.line_to(Point::new(c.x - s * 0.22, c.y + s * 0.06));
                    b.line_to(Point::new(c.x + s * 0.02, c.y + s * 0.06));
                    b.line_to(Point::new(c.x - s * 0.12, c.y + s * 0.46));
                    b.line_to(Point::new(c.x + s * 0.22, c.y - s * 0.06));
                    b.line_to(Point::new(c.x - s * 0.02, c.y - s * 0.06));
                    b.close();
                });
                frame.fill(&bolt, self.color);
            }
        }
        vec![frame.into_geometry()]
    }
}

/// Formats the time, without trusting the format string.
///
/// `chrono`'s `format(..).to_string()` **panics** on an invalid
/// specifier — `%E`, `%O`, a trailing `%-` — because `to_string` unwraps
/// a `Display` that returned an error. The format strings here come out
/// of a config file, so a typo would take the lock screen down, and on a
/// lock screen that is a machine you need another TTY to get into.
///
/// Writing through `fmt::Write` surfaces that as a `Result` instead, so
/// a bad format degrades to the default and then, if even that fails, to
/// no clock at all. A missing clock is a cosmetic loss; a panic is not.
fn formatted(now: &DateTime<Local>, format: &str, fallback: &str) -> String {
    use std::fmt::Write;
    let mut out = String::new();
    if write!(out, "{}", now.format(format)).is_ok() {
        return out;
    }
    let mut out = String::new();
    if write!(out, "{}", now.format(fallback)).is_ok() {
        return out;
    }
    String::new()
}

/// What is being asked, in PAM's own words, for each state.
///
/// PAM's prompts are not always "Password:" — a fingerprint reader or a
/// 2FA challenge says something else, and showing the real text is the
/// difference between knowing what to do and guessing. Shown inside the
/// empty field, where a placeholder goes.
fn prompt_label(state: &State) -> String {
    match state {
        State::Asking { prompt, .. } => prompt.text.trim().to_string(),
        State::Working => "Checking…".to_string(),
        State::Authenticated => "Welcome back".to_string(),
        // A failure or a message replaces the question until it is
        // acknowledged, so the prompt stays quiet rather than competing
        // with it.
        State::Failed { .. } | State::Telling { .. } => String::new(),
    }
}

/// How many characters are in the field. Never the characters, never a
/// number in words.
fn typed_count(state: &State) -> usize {
    match state {
        State::Asking { entered, .. } => entered.expose().chars().count().min(MAX_DOTS),
        _ => 0,
    }
}

/// The line under the field's text, and whether it reads as a failure.
fn status_text(state: &State) -> (String, bool) {
    match state {
        State::Failed { reason } => (reason.clone(), true),
        State::Telling { text, error } => (text.clone(), *error),
        State::Authenticated => ("Welcome back".into(), false),
        _ => (String::new(), false),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::conversation::Prompt;
    use crate::scene::{Media, NotificationCount, PowerAction, PowerMenu, Rejection, Status};
    use hyprforge_look::Color;
    use iced_runtime::core::Pixels;
    use iced_runtime::user_interface::{Cache, UserInterface};

    /// The one thing this screen must never do. Every string it produces
    /// is checked, because a password reaching a label is the same class
    /// of mistake as a password reaching a log — and that one has
    /// already happened in this project.
    #[test]
    fn nothing_the_screen_says_ever_contains_what_was_typed() {
        let secret = "hunter2!";
        let states = [
            State::Asking { prompt: Prompt::secret("Password:"), entered: secret.to_string().into() },
            State::Working,
            State::Authenticated,
            State::Failed { reason: "Incorrect password".into() },
            State::Telling { text: "Password expires in 3 days".into(), error: false },
        ];
        for state in states {
            assert!(!prompt_label(&state).contains(secret), "{state:?}");
            assert!(!status_text(&state).0.contains(secret), "{state:?}");
        }
    }

    /// Every extra switched on at once, so one render covers every
    /// widget this file can build.
    struct Everything {
        status: Status,
        media: Media,
        notifications: Vec<NotificationCount>,
        menu: PowerMenu,
        fingerprint: Fingerprint,
    }

    impl Everything {
        fn new() -> Everything {
            Everything {
                status: Status {
                    layout: Some("us".into()),
                    network: Some("studio-5G".into()),
                    battery: Some(Battery { percent: 7, charging: false, low: true }),
                },
                media: Media {
                    title: "Nightcall".into(),
                    artist: "Kavinsky".into(),
                    player: "Spotify".into(),
                    playing: true,
                    progress: Some(0.38),
                    can_previous: true,
                    can_next: false,
                },
                notifications: vec![
                    NotificationCount { app: "Signal".into(), count: 3 },
                    NotificationCount { app: "Mail".into(), count: 12 },
                ],
                menu: PowerMenu { actions: PowerAction::ALL.to_vec(), selected: 1 },
                fingerprint: Fingerprint::Ready,
            }
        }
    }

    /// Renders once, reporting a panic rather than propagating it.
    fn try_render(state: &State, theme: &Theme, w: u32, h: u32) -> Result<(), String> {
        try_render_with(state, theme, w, h, |_| {})
    }

    fn try_render_with(
        state: &State,
        theme: &Theme,
        w: u32,
        h: u32,
        tweak: impl FnOnce(&mut Scene<'_>) + std::panic::UnwindSafe,
    ) -> Result<(), String> {
        let state = state.clone();
        let theme = theme.clone();
        std::panic::catch_unwind(move || {
            // Exactly what the hosts do: nothing reaches the renderer
            // without passing through `renderable` first.
            let theme = renderable(theme);
            let everything = Everything::new();
            let mut renderer = iced_tiny_skia::Renderer::new(font(&theme), Pixels(theme.font_size));
            let size = iced_runtime::core::Size::new(w as f32, h as f32);
            let mut scene = Scene::new(&state, "apost", &theme, chrono::Local::now());
            scene.output = (w as f32, h as f32);
            scene.status = &everything.status;
            scene.fingerprint = &everything.fingerprint;
            scene.media = Some(&everything.media);
            scene.notifications = &everything.notifications;
            scene.power = Some(Some(&everything.menu));
            tweak(&mut scene);
            let mut ui = UserInterface::<Action, iced_widget::Theme, iced_tiny_skia::Renderer>::build(
                view(scene),
                size,
                Cache::default(),
                &mut renderer,
            );
            ui.draw(
                &mut renderer,
                &iced_widget::Theme::Dark,
                &iced_runtime::core::renderer::Style {
                    text_color: iced_runtime::core::Color::WHITE,
                },
                iced_runtime::core::mouse::Cursor::Unavailable,
            );
            let mut pixmap = tiny_skia::Pixmap::new(w, h).expect("pixmap");
            let mut mask = tiny_skia::Mask::new(w, h).expect("mask");
            renderer.draw(
                &mut pixmap.as_mut(),
                &mut mask,
                &iced_tiny_skia::graphics::Viewport::with_physical_size(
                    iced_runtime::core::Size::new(w, h),
                    1.0,
                ),
                &[iced_runtime::core::Rectangle::with_size(size)],
                iced_runtime::core::Color::BLACK,
            );
        })
        .map_err(|e| {
            e.downcast_ref::<&str>()
                .map(|s| s.to_string())
                .or_else(|| e.downcast_ref::<String>().cloned())
                .unwrap_or_else(|| "non-string panic".into())
        })
    }

    /// Every hostile theme worth worrying about.
    ///
    /// `lock.toml` is a file a person edits by hand and a program writes
    /// atomically but not transactionally. Any value in it can be
    /// nonsense, and the renderer this screen uses is full of `expect`s
    /// that fire on degenerate geometry. A panic here is the failure
    /// with no recovery: the compositor keeps the session locked and the
    /// process that could unlock it is gone.
    fn hostile_themes() -> Vec<(&'static str, Theme)> {
        let base = Theme::default();
        vec![
            ("default", base.clone()),
            ("font_size 0", Theme { font_size: 0.0, ..base.clone() }),
            ("font_size negative", Theme { font_size: -20.0, ..base.clone() }),
            ("font_size NaN", Theme { font_size: f32::NAN, ..base.clone() }),
            ("font_size inf", Theme { font_size: f32::INFINITY, ..base.clone() }),
            ("font_size enormous", Theme { font_size: 1.0e9, ..base.clone() }),
            ("dim negative", Theme { dim: -3.0, ..base.clone() }),
            ("dim over one", Theme { dim: 9.0, ..base.clone() }),
            ("dim NaN", Theme { dim: f32::NAN, ..base.clone() }),
            ("rounding max", Theme { rounding: u32::MAX, ..base.clone() }),
            ("empty font", Theme { font: String::new(), ..base.clone() }),
            ("empty mono font", Theme { mono_font: String::new(), ..base.clone() }),
            ("blur max", Theme { blur: u32::MAX, ..base.clone() }),
            (
                "empty formats",
                Theme { clock_format: String::new(), date_format: String::new(), ..base.clone() },
            ),
            (
                "absurd clock format",
                Theme { clock_format: "%".repeat(200), ..base.clone() },
            ),
            (
                "transparent everything",
                Theme {
                    background: Color::rgba(0, 0, 0, 0),
                    surface: Color::rgba(0, 0, 0, 0),
                    foreground: Color::rgba(0, 0, 0, 0),
                    ..base.clone()
                },
            ),
        ]
    }

    /// Sizes a compositor can hand over, including the ones it hands
    /// over before it knows the real answer.
    const HOSTILE_SIZES: &[(u32, u32)] = &[
        (1, 1),
        (1, 800),
        (800, 1),
        (2, 2),
        (64, 64),
        (771, 906),
        (3840, 2160),
    ];

    /// Input is the other thing this screen does not control.
    ///
    /// A passphrase can be any length and any script; PAM's prompts and
    /// failure text come from modules this project did not write; and a
    /// keyboard can deliver control characters.
    fn hostile_states() -> Vec<State> {
        vec![
            State::Working,
            State::Authenticated,
            State::Asking { prompt: Prompt::secret("Password:"), entered: "hunter2".to_string().into() },
            State::Asking { prompt: Prompt::secret("Password:"), entered: String::new().into() },
            State::Failed { reason: "Incorrect password".into() },
            // A passphrase far longer than the dot row can show.
            State::Asking { prompt: Prompt::secret("Password:"), entered: "x".repeat(10_000).into() },
            // Scripts that shape and combine, plus an emoji with a
            // zero-width joiner — the kind of thing that has broken text
            // layout engines before.
            State::Asking {
                prompt: Prompt::visible("رمز المرور:"),
                entered: "\u{1f9d1}\u{200d}\u{1f680}é\u{200d}\u{915}\u{94d}".repeat(20).into(),
            },
            // Control characters, which the key handler filters but a PAM
            // module's own text is not obliged to.
            State::Telling { text: "line\u{0}one\ttwo\r\n".into(), error: true },
            // Text long enough to need wrapping inside a fixed card.
            State::Asking { prompt: Prompt::secret("y".repeat(4_000)), entered: String::new().into() },
            State::Failed { reason: "z".repeat(4_000) },
        ]
    }

    /// Nothing in a theme file, a surface size, or a conversation can
    /// panic the renderer.
    ///
    /// Swept additively rather than as a full cross product: every
    /// hostile theme against one representative size, every size against
    /// one theme, every state against one of each. The multiplicative
    /// version covered ~1000 combinations and cost half a minute, which
    /// is too slow to run on every change — and a guard nobody runs is
    /// not a guard.
    #[test]
    fn nothing_can_panic_the_renderer() {
        let ordinary_size = (771, 906);
        let ordinary_state = State::Asking {
            prompt: Prompt::secret("Password:"),
            entered: "hunter2".to_string().into(),
        };
        let ordinary_theme = Theme::default();

        let mut failures = Vec::new();
        {
            let mut check = |what: String, state: &State, theme: &Theme, (w, h): (u32, u32)| {
                if let Err(why) = try_render(state, theme, w, h) {
                    failures.push(format!("{what}: {why}"));
                }
            };

            for (label, theme) in hostile_themes() {
                check(format!("theme {label}"), &ordinary_state, &theme, ordinary_size);
            }
            for &size in HOSTILE_SIZES {
                check(format!("size {size:?}"), &ordinary_state, &ordinary_theme, size);
            }
            for state in hostile_states() {
                check(format!("state {state:?}"), &state, &ordinary_theme, ordinary_size);
                // And once at a single pixel, which is what a compositor
                // can hand over before it knows the real size.
                check(format!("state {state:?} at 1x1"), &state, &ordinary_theme, (1, 1));
            }
        }

        // Each shape the scene can take beyond the conversation, once.
        type Tweak = fn(&mut Scene<'_>);
        let shapes: [(&str, Tweak); 6] = [
            ("caps lock on", |s| s.caps_lock = true),
            ("idle", |s| s.mode = Mode::Idle),
            ("secondary", |s| s.role = Role::Secondary),
            ("rejection mid-shake", |s| {
                s.rejection = Some(Rejection { dots: 30, offset: 400.0, attempt: 9 })
            }),
            ("no extras", |s| {
                s.media = None;
                s.notifications = &[];
                s.power = None;
            }),
            ("a missing avatar", |s| s.avatar = Some(std::path::Path::new("/nonexistent.png"))),
        ];
        for (label, tweak) in shapes {
            for (w, h) in [ordinary_size, (1, 1)] {
                if let Err(why) = try_render_with(&ordinary_state, &ordinary_theme, w, h, tweak) {
                    failures.push(format!("{label} at {w}x{h}: {why}"));
                }
            }
        }

        assert!(failures.is_empty(), "{} panic(s):\n{}", failures.len(), failures.join("\n"));
    }

    /// Renders `element` alone at `w`×`h` on black and returns the RGBA
    /// pixels, for tests that ask what actually reached the buffer.
    fn pixels_of(element: El<'_, iced_tiny_skia::Renderer>, w: u32, h: u32) -> Vec<u8> {
        let theme = Theme::default();
        let mut renderer = iced_tiny_skia::Renderer::new(font(&theme), Pixels(theme.font_size));
        let size = iced_runtime::core::Size::new(w as f32, h as f32);
        let mut ui = UserInterface::<Action, iced_widget::Theme, iced_tiny_skia::Renderer>::build(
            element,
            size,
            Cache::default(),
            &mut renderer,
        );
        ui.draw(
            &mut renderer,
            &iced_widget::Theme::Dark,
            &iced_runtime::core::renderer::Style { text_color: iced_runtime::core::Color::WHITE },
            iced_runtime::core::mouse::Cursor::Unavailable,
        );
        let mut pixmap = tiny_skia::Pixmap::new(w, h).unwrap();
        let mut mask = tiny_skia::Mask::new(w, h).unwrap();
        renderer.draw(
            &mut pixmap.as_mut(),
            &mut mask,
            &iced_tiny_skia::graphics::Viewport::with_physical_size(iced_runtime::core::Size::new(w, h), 1.0),
            &[iced_runtime::core::Rectangle::with_size(size)],
            iced_runtime::core::Color::BLACK,
        );
        pixmap.data().to_vec()
    }

    /// The drawn marks have to reach the pixels. A glyph that silently
    /// draws nothing leaves an empty circle in the corner — which is
    /// what the first version of this file shipped, with every other
    /// test passing.
    #[test]
    fn every_drawn_glyph_puts_ink_on_the_screen() {
        for kind in [Glyph::Power, Glyph::Previous, Glyph::Next, Glyph::Play, Glyph::Pause, Glyph::Bolt] {
            let white = iced_runtime::core::Color::WHITE;
            let data = pixels_of(glyph(kind, 32.0, white), 32, 32);
            let lit = data.chunks_exact(4).filter(|px| px[1] > 128).count();
            assert!(lit > 12, "{kind:?} drew {lit} bright pixel(s)");
        }
    }

    /// A glyph anywhere but the top-left corner still draws.
    ///
    /// `iced_tiny_skia` 0.14.0 applied a canvas's translation to its clip
    /// rectangle twice, so any glyph not at the origin was clipped away
    /// entirely — the test above passed, because it draws at the origin,
    /// while every glyph on the real screen was invisible. 0.14.1 fixed
    /// it; this fails if the lockfile ever goes back.
    #[test]
    fn a_glyph_away_from_the_origin_is_not_clipped_away() {
        let white = iced_runtime::core::Color::WHITE;
        let moved = container(glyph(Glyph::Power, 32.0, white)).padding(60);
        let lit = pixels_of(moved.into(), 160, 160).chunks_exact(4).filter(|px| px[1] > 128).count();
        assert!(lit > 12, "an offset glyph drew {lit} bright pixel(s)");
    }

    /// Clicks at `at` on a full scene and returns what the screen asked
    /// for — through iced's own hit-testing, exactly as the lock does.
    fn click(scene: Scene<'_>, (w, h): (f32, f32), at: iced_runtime::core::Point) -> Vec<Action> {
        let theme = scene.theme.clone();
        let mut renderer = iced_tiny_skia::Renderer::new(font(&theme), Pixels(theme.font_size));
        let mut ui = UserInterface::<Action, iced_widget::Theme, iced_tiny_skia::Renderer>::build(
            view(scene),
            iced_runtime::core::Size::new(w, h),
            Cache::default(),
            &mut renderer,
        );
        use iced_runtime::core::{mouse, Event};
        let events = [
            Event::Mouse(mouse::Event::CursorMoved { position: at }),
            Event::Mouse(mouse::Event::ButtonPressed(mouse::Button::Left)),
            Event::Mouse(mouse::Event::ButtonReleased(mouse::Button::Left)),
        ];
        let mut actions = Vec::new();
        ui.update(&events, mouse::Cursor::Available(at), &mut renderer, &mut iced_runtime::core::clipboard::Null, &mut actions);
        actions
    }

    /// The ⏻ is where the mockup puts it — 26px in, 24px up, 44 across —
    /// and a click there opens the menu, while a click on the wallpaper
    /// asks for nothing. The lock hands pointer events to exactly this.
    #[test]
    fn a_click_on_the_power_button_opens_the_menu_and_one_elsewhere_does_nothing() {
        let theme = Theme { font_size: 10.0, ..Theme::default() };
        let state = State::Asking { prompt: Prompt::secret("Password:"), entered: String::new().into() };
        let (w, h) = (1440.0, 900.0);
        let scene = |menu: Option<&'static PowerMenu>| {
            let mut s = Scene::new(Box::leak(Box::new(state.clone())), "apost", Box::leak(Box::new(theme.clone())), Local::now());
            s.power = Some(menu);
            s
        };
        let button = iced_runtime::core::Point::new(w - 26.0 - 22.0, h - 24.0 - 22.0);
        assert_eq!(click(scene(None), (w, h), button), vec![Action::TogglePowerMenu]);
        assert!(click(scene(None), (w, h), iced_runtime::core::Point::new(100.0, 100.0)).is_empty());

        // Each menu row is a target of its own, and the first row is the
        // first action: 80px up from the bottom, the menu's 6px padding,
        // then rows of 38.
        let menu: &'static PowerMenu =
            Box::leak(Box::new(PowerMenu { actions: vec![PowerAction::Suspend, PowerAction::PowerOff], selected: 0 }));
        let bottom_row = iced_runtime::core::Point::new(w - 26.0 - 115.0, h - 80.0 - 6.0 - 19.0);
        assert_eq!(click(scene(Some(menu)), (w, h), bottom_row), vec![Action::Power(PowerAction::PowerOff)]);
    }

    /// Two themes must not share one font.
    ///
    /// The first version cached a single name in a `OnceLock`, so
    /// whichever theme asked first won for the life of the process and
    /// every later one silently rendered in the wrong font.
    #[test]
    fn each_theme_gets_its_own_font_family() {
        let of = |name: &str| font(&Theme { font: name.into(), ..Theme::default() });
        let sans = of("DejaVu Sans");
        let mono = of("DejaVu Sans Mono");
        assert_ne!(sans.family, mono.family);
        // And asking again is stable rather than leaking a new name.
        assert_eq!(of("DejaVu Sans").family, sans.family);
        // An unnamed font is iced's default, not an empty family.
        assert_eq!(of("   ").family, Font::DEFAULT.family);
        // And an unnamed mono is iced's monospace, not the sans.
        assert_eq!(mono_font(&Theme::default()).family, Font::MONOSPACE.family);
    }

    /// A typo in a clock format must not take the machine down.
    ///
    /// `chrono::format(..).to_string()` panics on an invalid specifier,
    /// and these strings come from a config file. The realistic formats
    /// are fine — `%-I:%M %p` works — but `%E` is one slip away, and the
    /// consequence of that slip is needing another TTY to log in.
    #[test]
    fn an_invalid_clock_format_falls_back_instead_of_panicking() {
        let now = Local::now();
        for bad in ["%", "%-", "%E", "%O", "%Q", &"%".repeat(200)] {
            let shown = formatted(&now, bad, "%H:%M");
            assert!(
                !shown.is_empty(),
                "{bad:?} produced nothing; the fallback should have run"
            );
        }
    }

    /// And a valid format is still used as written, or the fallback
    /// would be silently replacing everyone's clock.
    #[test]
    fn a_valid_clock_format_is_used_as_written() {
        let now = Local::now();
        assert_eq!(formatted(&now, "%Y", "%H:%M"), now.format("%Y").to_string());
        // The 12-hour form people actually write.
        assert!(formatted(&now, "%-I:%M %p", "%H:%M").contains(':'));
    }

    /// The specific value that panics `cosmic-text`, pinned separately
    /// from the sweep.
    ///
    /// A font size of zero fails an assertion deep in text layout —
    /// "line height cannot be 0" — and that poisons the global font
    /// mutex, so every later attempt to draw fails too. There is no
    /// recovering from it inside the process, which on a lock screen
    /// means a machine that cannot be unlocked.
    #[test]
    fn a_zero_font_size_never_reaches_the_renderer() {
        for bad in [0.0, -1.0, -1.0e9, f32::NAN, f32::NEG_INFINITY] {
            let theme = renderable(Theme { font_size: bad, ..Theme::default() });
            assert!(
                theme.font_size >= MIN_FONT && theme.font_size <= MAX_FONT,
                "{bad} became {}",
                theme.font_size
            );
            assert!(size(bad) >= MIN_FONT, "a derived size must be bounded too");
        }
    }

    /// Everything else the renderer unwraps geometry from.
    ///
    /// Two different treatments, deliberately. A value that is merely
    /// too large is clamped, because the intent is legible — someone
    /// wanted big text. A value that is not a number at all carries no
    /// intent, so it falls back to the default rather than to whichever
    /// end of the range it happens to clamp toward.
    #[test]
    fn every_out_of_range_theme_value_is_brought_back_into_range() {
        let clamped = renderable(Theme {
            font_size: 1.0e9,
            dim: 12.0,
            rounding: u32::MAX,
            blur: u32::MAX,
            ..Theme::default()
        });
        assert_eq!(clamped.font_size, MAX_FONT);
        assert_eq!(clamped.dim, 1.0);
        assert_eq!(clamped.rounding, MAX_ROUNDING);
        assert_eq!(clamped.blur, MAX_BLUR);

        let nonsense = renderable(Theme {
            font_size: f32::INFINITY,
            font_scale: f32::NAN,
            dim: f32::NAN,
            ..Theme::default()
        });
        assert_eq!(nonsense.font_size, Theme::default().font_size);
        assert_eq!(nonsense.font_scale, 1.0);
        assert_eq!(nonsense.dim, 0.0, "an undimmed wallpaper is legible; a NaN one is not");

        // And a sane theme is left exactly as it was, or the guard would
        // be quietly restyling everyone's lock screen.
        let sane = Theme::default();
        assert_eq!(renderable(sane.clone()), sane);
    }

    /// A wallpaper the renderer cannot draw must never reach it.
    ///
    /// iced_tiny_skia 0.14 caches a failed load as "no entry" and then
    /// panics on the next draw of the same handle. On a lock screen that
    /// is unrecoverable, so anything that is not a decodable image is
    /// dropped here and the screen falls back to its flat background.
    #[test]
    fn a_wallpaper_the_renderer_cannot_draw_is_dropped() {
        let dir = tempfile::tempdir().unwrap();

        let not_an_image = dir.path().join("notes.png");
        std::fs::write(&not_an_image, b"this is not a PNG").unwrap();
        let missing = dir.path().join("gone.png");

        for path in [not_an_image, missing, dir.path().to_path_buf()] {
            let theme = renderable(Theme {
                wallpaper: Some(path.clone()),
                ..Theme::default()
            });
            assert_eq!(theme.wallpaper, None, "{}", path.display());
        }
    }

    /// A real image survives, or the wallpaper feature does nothing at
    /// all and the guard above would be indistinguishable from a bug.
    #[test]
    fn a_real_image_is_kept() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("wall.png");
        let pixel = image::RgbaImage::from_pixel(2, 2, image::Rgba([1, 2, 3, 255]));
        pixel.save(&path).unwrap();

        let theme = renderable(Theme {
            wallpaper: Some(path.clone()),
            ..Theme::default()
        });
        assert_eq!(theme.wallpaper, Some(path));
    }

    /// One dot per character, so the count is the only feedback — and it
    /// stops growing before a long passphrase can push the field apart.
    #[test]
    fn dots_count_characters_and_stop_at_a_sensible_limit() {
        let dots = |entered: &str| {
            typed_count(&State::Asking {
                prompt: Prompt::secret("Password:"),
                entered: entered.to_string().into(),
            })
        };
        assert_eq!(dots(""), 0);
        assert_eq!(dots("abc"), 3);
        // Characters, not bytes: a multi-byte character is one dot.
        assert_eq!(dots("é😀"), 2);
        assert_eq!(dots(&"x".repeat(500)), MAX_DOTS);
    }

    /// Only a question has anything typed into it — otherwise a stale
    /// row would sit there while PAM worked.
    #[test]
    fn only_a_question_counts_as_typed() {
        for state in [State::Working, State::Authenticated, State::Failed { reason: "no".into() }] {
            assert_eq!(typed_count(&state), 0, "{state:?}");
        }
    }

    /// PAM asks for more than passwords. Showing its own wording is why
    /// this is a conversation and not a password box.
    #[test]
    fn the_prompt_is_pams_own_words() {
        let state = State::Asking {
            prompt: Prompt::visible("Verification code:"),
            entered: String::new().into(),
        };
        assert_eq!(prompt_label(&state), "Verification code:");
    }

    /// The failure the hand-drawn screen could not express: it had no
    /// font renderer, so "incorrect password" and "your password expires
    /// in three days" were both a coloured bar.
    #[test]
    fn a_failure_says_what_went_wrong_and_reads_as_a_failure() {
        let (message, failed) = status_text(&State::Failed {
            reason: "Password accepted, but the account check failed".into(),
        });
        assert!(message.contains("account check"), "{message}");
        assert!(failed);

        let (notice, failed) = status_text(&State::Telling {
            text: "Password expires in 3 days".into(),
            error: false,
        });
        assert_eq!(notice, "Password expires in 3 days");
        assert!(!failed, "a notice is not a failure");
    }

    /// The idle clock is the mockup's 180px and the card's 120px at the
    /// mockup's own font size — and neither may take over a small or
    /// portrait output.
    #[test]
    fn the_clock_is_the_mockups_size_but_never_more_than_the_output_allows() {
        let theme = Theme { font_size: 10.0, ..Theme::default() };
        assert_eq!(clock_size(&theme, true, (1440.0, 900.0)), 180.0);
        assert_eq!(clock_size(&theme, false, (1440.0, 900.0)), 120.0);
        let portrait = clock_size(&theme, true, (900.0, 1600.0));
        assert!((150.0..180.0).contains(&portrait), "{portrait}");
        assert_eq!(clock_size(&theme, true, (1.0, 1.0)), MIN_FONT);
    }

    /// Letter-spacing is only for text it cannot break.
    #[test]
    fn only_plain_latin_is_letter_spaced() {
        assert_eq!(tracked("AB C"), "A\u{200a}B\u{200a} \u{200a}C");
        assert_eq!(tracked("THURSDAY,  4 SEPTEMBER"), tracked("THURSDAY, 4 SEPTEMBER"), "%e pads");
        assert_eq!(tracked("الخميس"), "الخميس");
        assert_eq!(tracked("JEUDI 24 SEPTEMBRE"), tracked("JEUDI 24 SEPTEMBRE"));
    }

    #[test]
    fn caps_lock_on_reads_as_a_warning() {
        let theme = Theme::default();
        assert_eq!(caps_lock_line(true, &theme), ("Caps Lock on", to_iced(theme.warning)));
        assert_eq!(caps_lock_line(false, &theme).0, "Caps Lock off");
    }

    /// The mockup's sizes hold at the font size it was drawn against, and
    /// a font size of nonsense does not make a card the size of a wall.
    #[test]
    fn one_mockup_pixel_follows_the_desktop_font_within_bounds() {
        assert_eq!(unit(&Theme { font_size: 10.0, ..Theme::default() }), 1.0);
        assert_eq!(unit(&Theme { font_size: 96.0, ..Theme::default() }), 1.6);
        assert_eq!(unit(&Theme { font_size: f32::NAN, ..Theme::default() }), 1.0);
    }
}
