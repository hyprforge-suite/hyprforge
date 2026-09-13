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
//! is an ordinary window. Software rendering is not a limitation here —
//! it is the point. This is the surface between a locked machine and its
//! user, and it has no business depending on a GPU being in a good mood.
//!
//! Every colour comes from [`hyprforge_look::Theme`] explicitly rather
//! than from iced's own theming, so the screen looks the same whichever
//! host renders it and whatever iced's defaults happen to be.

use crate::conversation::State;
use chrono::{DateTime, Local};
use hyprforge_look::Theme;
use hyprforge_ui::color::to_iced;
use iced_runtime::core::{Element, Font, Length, Padding};
use iced_widget::{column, container, row, text, Space};

/// How wide the prompt panel is, and how big the dots are.
const PANEL_WIDTH: f32 = 420.0;
const DOT: f32 = 12.0;
const DOT_GAP: f32 = 10.0;
/// Beyond this many characters the dots stop being countable anyway, and
/// a row that grows without limit would push the panel apart.
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
        let drawable = image::ImageReader::open(&path)
            .and_then(|reader| reader.with_guessed_format())
            .ok()
            .and_then(|reader| reader.into_dimensions().ok())
            .is_some_and(|(w, h)| w > 0 && h > 0);
        if !drawable {
            theme.wallpaper = None;
        }
    }
    theme
}

/// The theme's font, as iced needs it.
///
/// iced wants a `&'static str` for a family name and the theme's font
/// arrives from gsettings at runtime, so the name has to be leaked.
/// Leaking is bounded by caching per family rather than per call: a
/// process ends up holding one small string per distinct font it was
/// ever asked for, which for a lock screen is one.
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
    static FAMILIES: std::sync::Mutex<Option<std::collections::HashMap<String, &'static str>>> =
        std::sync::Mutex::new(None);

    let name = theme.font.trim();
    if name.is_empty() {
        return Font::DEFAULT;
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

/// The whole screen, for a conversation in `state`.
///
/// `now` is passed in rather than read here so the clock is testable and
/// so both hosts show the same time for the same frame.
pub fn view<'a, Message, Renderer>(
    state: &'a State,
    username: &'a str,
    theme: &'a Theme,
    now: DateTime<Local>,
    caps_lock: bool,
) -> Element<'a, Message, iced_widget::Theme, Renderer>
where
    Message: 'a,
    Renderer: iced_runtime::core::text::Renderer<Font = iced_runtime::core::Font>
        + iced_runtime::core::image::Renderer<Handle = iced_runtime::core::image::Handle>
        + 'a,
{
    let foreground = to_iced(theme.foreground);
    let dim = to_iced(theme.surfaces.text_dim);

    let clock = column![
        text(formatted(&now, &theme.clock_format, "%H:%M"))
            .size(theme.font_size * 5.0)
            .color(foreground),
        text(formatted(&now, &theme.date_format, "%A, %e %B"))
            .size(theme.font_size * 1.2)
            .color(dim),
    ]
    .spacing(4);

    let panel = container(
        column![
            text(username).size(theme.font_size * 1.3).color(foreground),
            Space::new().height(4),
            prompt_line(state, theme),
            dots(state, theme),
            status_line(state, theme),
            caps_lock_warning(caps_lock, theme),
        ]
        .spacing(10),
    )
    .width(Length::Fixed(PANEL_WIDTH))
    .padding(Padding::from(24))
    .style(move |_: &iced_widget::Theme| container::Style {
        background: Some(to_iced(theme.surface).into()),
        border: iced_runtime::core::Border {
            radius: (theme.rounding as f32).into(),
            ..Default::default()
        },
        ..Default::default()
    });

    let content = container(column![clock, Space::new().height(48), panel].spacing(0))
        .width(Length::Fill)
        .height(Length::Fill)
        .center_x(Length::Fill)
        .center_y(Length::Fill);

    match &theme.wallpaper {
        Some(path) => iced_widget::stack![
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
            container(Space::new().width(Length::Fill).height(Length::Fill)).style(move |_: &iced_widget::Theme| {
                container::Style {
                    background: Some(
                        iced_runtime::core::Color {
                            a: theme.dim.clamp(0.0, 1.0),
                            ..to_iced(theme.background)
                        }
                        .into(),
                    ),
                    ..Default::default()
                }
            }),
            content,
        ]
        .into(),
        // No wallpaper is not a failure — it is the normal case on a
        // fresh install, and the flat background is legible on its own.
        None => content
            .style(move |_: &iced_widget::Theme| container::Style {
                background: Some(to_iced(theme.background).into()),
                ..Default::default()
            })
            .into(),
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

/// What is being asked, in PAM's own words.
///
/// PAM's prompts are not always "Password:" — a fingerprint reader or a
/// 2FA challenge says something else, and showing the real text is the
/// difference between a user knowing what to do and guessing.
fn prompt_line<'a, Message, Renderer>(
    state: &'a State,
    theme: &'a Theme,
) -> Element<'a, Message, iced_widget::Theme, Renderer>
where
    Message: 'a,
    Renderer: iced_runtime::core::text::Renderer<Font = iced_runtime::core::Font> + 'a,
{
    text(prompt_label(state))
        .size(theme.font_size)
        .color(to_iced(theme.surfaces.text_dim))
        .into()
}

/// Says so when Caps Lock is on.
///
/// Not a nicety. Without it the password is simply wrong, over and over,
/// with nothing on screen to explain why — and where `pam_faillock` is
/// configured (it is, on the machine this was written on, at three
/// attempts) that turns a stuck key into a locked *account*, which is a
/// much worse afternoon than a locked screen. hyprlock and swaylock both
/// show this, and they are right to.
fn caps_lock_warning<'a, Message, Renderer>(
    on: bool,
    theme: &'a Theme,
) -> Element<'a, Message, iced_widget::Theme, Renderer>
where
    Message: 'a,
    Renderer: iced_runtime::core::text::Renderer<Font = iced_runtime::core::Font> + 'a,
{
    text(if on { "Caps Lock is on" } else { "" })
        .size(theme.font_size * 0.9)
        .color(to_iced(theme.warning))
        .into()
}

/// One dot per typed character.
///
/// Never the characters themselves and never a count in words: the
/// number of dots is already the only feedback a password field should
/// give.
fn dots<'a, Message, Renderer>(
    state: &'a State,
    theme: &'a Theme,
) -> Element<'a, Message, iced_widget::Theme, Renderer>
where
    Message: 'a,
    Renderer: iced_runtime::core::text::Renderer<Font = iced_runtime::core::Font> + 'a,
{
    let typed = dot_count(state);
    let filled = to_iced(theme.accent);

    let mut marks = row![].spacing(DOT_GAP);
    for _ in 0..typed {
        marks = marks.push(
            container(Space::new().width(DOT))
                .height(DOT)
                .style(move |_: &iced_widget::Theme| container::Style {
                    background: Some(filled.into()),
                    border: iced_runtime::core::Border {
                        radius: (DOT / 2.0).into(),
                        ..Default::default()
                    },
                    ..Default::default()
                }),
        );
    }
    // A fixed height whether or not anything is typed, so the panel
    // doesn't jump the moment the first key lands.
    container(marks).height(DOT + 8.0).into()
}

/// The line that says what went wrong, or what PAM wanted to tell you.
///
/// This is the line the hand-drawn screen could not have: it had no font
/// renderer, so "incorrect password" and "your password expires in three
/// days" were both a red bar.
fn status_line<'a, Message, Renderer>(
    state: &'a State,
    theme: &'a Theme,
) -> Element<'a, Message, iced_widget::Theme, Renderer>
where
    Message: 'a,
    Renderer: iced_runtime::core::text::Renderer<Font = iced_runtime::core::Font> + 'a,
{
    let (message, failed) = status_text(state);
    text(message)
        .size(theme.font_size)
        .color(if failed { to_iced(theme.error) } else { to_iced(theme.foreground) })
        .into()
}

/// What the prompt line says, for each state.
///
/// PAM's own words when it is asking, because its prompts are not always
/// "Password:" — a fingerprint reader or a 2FA challenge says something
/// else, and showing the real text is the difference between knowing
/// what to do and guessing.
fn prompt_label(state: &State) -> String {
    match state {
        State::Asking { prompt, .. } => prompt.text.trim().to_string(),
        State::Working => "Checking…".to_string(),
        State::Authenticated => "Welcome back".to_string(),
        // A failure or a message replaces the question until it is
        // acknowledged, so the prompt line stays quiet rather than
        // competing with it.
        State::Failed { .. } | State::Telling { .. } => String::new(),
    }
}

/// How many dots to draw. Never the characters, never a number in words.
fn dot_count(state: &State) -> usize {
    match state {
        State::Asking { entered, .. } => entered.expose().chars().count().min(MAX_DOTS),
        _ => 0,
    }
}

/// The status line's text, and whether it reads as a failure.
fn status_text(state: &State) -> (String, bool) {
    match state {
        State::Failed { reason } => (reason.clone(), true),
        State::Telling { text, error } => (text.clone(), *error),
        _ => (String::new(), false),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::conversation::Prompt;
    use hyprforge_look::Color;
    use iced_runtime::core::Pixels;
    use iced_runtime::user_interface::{Cache, UserInterface};

    /// The screen sends no messages.
    #[derive(Debug, Clone)]
    enum Nothing {}

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

    /// Renders once, reporting a panic rather than propagating it.
    fn try_render(state: &State, theme: &Theme, w: u32, h: u32) -> Result<(), String> {
        try_render_with_caps(state, theme, w, h, false)
    }

    fn try_render_with_caps(
        state: &State,
        theme: &Theme,
        w: u32,
        h: u32,
        caps: bool,
    ) -> Result<(), String> {
        let state = state.clone();
        let theme = theme.clone();
        std::panic::catch_unwind(move || {
            // Exactly what the hosts do: nothing reaches the renderer
            // without passing through `renderable` first.
            let theme = renderable(theme);
            let mut renderer = iced_tiny_skia::Renderer::new(font(&theme), Pixels(theme.font_size));
            let size = iced_runtime::core::Size::new(w as f32, h as f32);
            let mut ui = UserInterface::<Nothing, iced_widget::Theme, iced_tiny_skia::Renderer>::build(
                view(&state, "apost", &theme, chrono::Local::now(), caps),
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
            let msg = e
                .downcast_ref::<&str>()
                .map(|s| s.to_string())
                .or_else(|| e.downcast_ref::<String>().cloned())
                .unwrap_or_else(|| "non-string panic".into());
            msg
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
            // Text long enough to need wrapping inside a fixed panel.
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

        // Caps Lock adds a line to the panel, so it is its own shape.
        if let Err(why) = try_render_with_caps(&ordinary_state, &ordinary_theme, 771, 906, true) {
            failures.push(format!("caps lock on: {why}"));
        }

        assert!(failures.is_empty(), "{} panic(s):\n{}", failures.len(), failures.join("\n"));
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
        for size in [0.0, -1.0, -1.0e9, f32::NAN, f32::NEG_INFINITY] {
            let theme = renderable(Theme { font_size: size, ..Theme::default() });
            assert!(
                theme.font_size >= MIN_FONT && theme.font_size <= MAX_FONT,
                "{size} became {}",
                theme.font_size
            );
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
    /// stops growing before a long passphrase can push the panel apart.
    #[test]
    fn dots_count_characters_and_stop_at_a_sensible_limit() {
        let dots = |entered: &str| {
            dot_count(&State::Asking {
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

    /// Only a question has anything to type into, so nothing else shows
    /// dots — otherwise a stale row would sit there while PAM worked.
    #[test]
    fn only_a_question_shows_dots() {
        for state in [State::Working, State::Authenticated, State::Failed { reason: "no".into() }] {
            assert_eq!(dot_count(&state), 0, "{state:?}");
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
}
