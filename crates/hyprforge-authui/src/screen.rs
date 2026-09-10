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

/// The theme, with a wallpaper the renderer cannot actually draw
/// removed.
///
/// This is not tidiness. `iced_tiny_skia` 0.14 caches a failed image
/// load as "no entry", and its *next* attempt to draw the same handle
/// hits an `expect` and panics. On a lock screen that is the one failure
/// with no recovery: the compositor keeps the session locked and the
/// process that could have unlocked it is gone.
///
/// So the path is checked before it can reach the renderer. Only the
/// header is read — enough to know whether there is a decoder for this
/// format and whether the file is really an image — so the cost does not
/// scale with the size of the picture.
pub fn with_drawable_wallpaper(mut theme: Theme) -> Theme {
    let Some(path) = theme.wallpaper.clone() else {
        return theme;
    };
    let readable = image::ImageReader::open(&path)
        .and_then(|reader| reader.with_guessed_format())
        .ok()
        .and_then(|reader| reader.into_dimensions().ok())
        .is_some_and(|(w, h)| w > 0 && h > 0);

    if !readable {
        theme.wallpaper = None;
    }
    theme
}

/// The theme's font, as iced needs it.
///
/// iced wants a `&'static str` for a family name, and the theme's font
/// comes from gsettings at runtime, so the name is leaked. That is why
/// this is a function with a cache rather than a conversion: it leaks at
/// most once per process, and a lock screen outlives everything else in
/// its own anyway.
///
/// An empty or unresolvable family falls back to iced's default rather
/// than failing. A screen in the wrong font is a cosmetic problem; a
/// screen that refused to start over one would be a machine nobody can
/// get into.
pub fn font(theme: &Theme) -> Font {
    static FAMILY: std::sync::OnceLock<Option<&'static str>> = std::sync::OnceLock::new();
    let family = FAMILY.get_or_init(|| {
        let name = theme.font.trim();
        (!name.is_empty()).then(|| &*Box::leak(name.to_owned().into_boxed_str()))
    });
    match family {
        Some(name) => Font::with_name(name),
        None => Font::DEFAULT,
    }
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
        text(now.format(&theme.clock_format).to_string())
            .size(theme.font_size * 5.0)
            .color(foreground),
        text(now.format(&theme.date_format).to_string())
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
        State::Asking { entered, .. } => entered.chars().count().min(MAX_DOTS),
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

    /// The one thing this screen must never do. Every string it produces
    /// is checked, because a password reaching a label is the same class
    /// of mistake as a password reaching a log — and that one has
    /// already happened in this project.
    #[test]
    fn nothing_the_screen_says_ever_contains_what_was_typed() {
        let secret = "hunter2!";
        let states = [
            State::Asking { prompt: Prompt::secret("Password:"), entered: secret.into() },
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
            let theme = with_drawable_wallpaper(Theme {
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

        let theme = with_drawable_wallpaper(Theme {
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
                entered: entered.into(),
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
            entered: String::new(),
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
