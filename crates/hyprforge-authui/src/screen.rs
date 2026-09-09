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
use iced_runtime::core::{Element, Length, Padding};
use iced_widget::{column, container, row, text, Space};

/// How wide the prompt panel is, and how big the dots are.
const PANEL_WIDTH: f32 = 420.0;
const DOT: f32 = 12.0;
const DOT_GAP: f32 = 10.0;
/// Beyond this many characters the dots stop being countable anyway, and
/// a row that grows without limit would push the panel apart.
const MAX_DOTS: usize = 24;

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
    Renderer: iced_runtime::core::text::Renderer<Font = iced_runtime::core::Font> + 'a,
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

    container(column![clock, Space::new().height(48), panel].spacing(0))
        .width(Length::Fill)
        .height(Length::Fill)
        .center_x(Length::Fill)
        .center_y(Length::Fill)
        .style(move |_: &iced_widget::Theme| container::Style {
            background: Some(to_iced(theme.background).into()),
            ..Default::default()
        })
        .into()
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
