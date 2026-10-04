//! Every widget the suite shares.
//!
//! The functions in this file are the original set; the modules below
//! arrived with the Settings redesign and Files' path bar and are
//! re-exported, so every widget is `hyprforge_ui::widgets::<name>`
//! wherever it happens to live. `DESIGN-SYSTEM.md` beside this crate's
//! manifest catalogues all of them, and `check.sh` fails if it misses one.

mod anchor;
mod badges;
mod change;
mod controls;
mod field;
mod layout;
mod progress;
mod panel;
mod ring;
mod scrim;
mod selection;
mod suggestions;

pub use anchor::{anchored, Anchored};
pub use badges::{chip, config_line, keycap, removable_chip};
pub use change::{change_row, Change};
pub use controls::{
    dropdown_menu_style, dropdown_style, segment_style, segmented, segmented_choice,
    slider_style, step_index, stepped_slider, toggle, toggle_style, value_slider, SegmentLook,
};
pub use field::{inset_field_style, inset_input_style, search_field, token_field};
pub use layout::{
    hero_card, hint_text, page_header, pending_bar, pending_label, setting_list, setting_row,
    setting_row_style, status_dot, SETTING_ROW_GAP,
};
pub use progress::{popover_card, progress_line, progress_line_style};
pub use panel::{fact, fact_row, panel_tabs};
pub use ring::{countdown_ring, remaining_fraction};
pub use scrim::{scrim, scrim_color, SCRIM_ALPHA};
pub use selection::{section_label, selectable_row_style, spaced_caps, Tint};
pub use suggestions::{suggestions, Suggestion};

use crate::theme::{self, spacing, surface, FontScale};
use iced::widget::{button, column, container, row, text, text::IntoFragment, Text};
use iced::{Background, Border, Color, Element, Length, Theme};

/// `text(content).size(scale.apply(base_size))` — the one place every piece
/// of Hyprforge UI text should go through, so `FontScale` (vision pillar
/// #7: accessibility built into the shared layer, not bolted on per-app)
/// actually reaches the screen instead of being a struct nobody calls.
pub fn scaled_text<'a>(
    content: impl IntoFragment<'a>,
    base_size: f32,
    scale: FontScale,
) -> Text<'a> {
    text(content).size(scale.apply(base_size))
}

/// Dim, secondary-emphasis text — timestamps, hints, IDs. Distinct from
/// `scaled_text` alone so "this is metadata, not content" is visible at a
/// glance rather than relying on size difference only.
pub fn meta_text<'a>(content: impl IntoFragment<'a>, base_size: f32, scale: FontScale) -> Text<'a> {
    scaled_text(content, base_size, scale).color(theme::text_dim())
}

/// The raised-card look used for every section and dialog — a real step
/// up from the root background plus a hairline border, not an auto-derived
/// shade a few percent off the background (which reads as "no elevation
/// at all" on a real monitor).
fn card_style(_theme: &Theme) -> container::Style {
    container::Style {
        background: Some(Background::Color(surface::card())),
        border: Border {
            radius: crate::density::card_radius().into(),
            width: 1.0,
            color: surface::card_border(),
        },
        ..container::Style::default()
    }
}

/// A titled group of content — the standard way every Hyprforge settings
/// screen breaks itself into blocks (vision pillar #2). Sized to its
/// content; callers that need scrolling should cap a `max_height` on the
/// `content` they pass in rather than have the card stretch to fill
/// whatever space happens to be available.
pub fn section<'a, Message: 'a>(
    title: impl AsRef<str>,
    scale: FontScale,
    content: impl Into<Element<'a, Message>>,
) -> Element<'a, Message> {
    column![
        // The same heading every restyled page uses over its groups, so a
        // page still built from cards reads as the same app.
        section_label(title.as_ref(), scale),
        container(content.into())
            .padding(spacing::MD)
            .width(Length::Fill)
            .style(card_style)
    ]
    .spacing(spacing::SM)
    .into()
}

/// A thin separator between list rows inside a card — the visual grammar
/// used everywhere Hyprforge shows a list of things (profiles, rules) so
/// rows read as distinct items rather than one unbroken block of text.
pub fn divider<'a, Message: 'a>() -> Element<'a, Message> {
    container(column![])
        .width(Length::Fill)
        .height(Length::Fixed(1.0))
        .style(|_theme: &Theme| container::Style {
            background: Some(Background::Color(surface::card_border())),
            ..container::Style::default()
        })
        .into()
}

/// [`divider`] standing up: the same hairline in the same colour, between
/// panes that sit side by side — Files' column view draws one between
/// each folder's pane.
pub fn vertical_divider<'a, Message: 'a>() -> Element<'a, Message> {
    container(column![])
        .width(Length::Fixed(1.0))
        .height(Length::Fill)
        .style(|_theme: &Theme| container::Style {
            background: Some(Background::Color(surface::card_border())),
            ..container::Style::default()
        })
        .into()
}

/// Widest a settings control grows before it stops tracking the row.
///
/// Without a cap, a text input in an 880px content column stretches to
/// ~590px — far wider than any value typed into it, which reads as an
/// unfinished form. Windows and macOS both hold settings controls to
/// roughly this width regardless of window size.
const CONTROL_MAX_WIDTH: f32 = 340.0;

/// A label paired with its control, as a setting row.
///
/// The same row [`setting_row`] draws — label at the left end, control
/// at the right, at least one row tall — so a page not yet built from
/// striped groups still lines up with those that are. It is never
/// striped, because it is not told its place in a list; it used to be a
/// 2:3 split that put a wide gap between a label and its control.
///
/// Takes no `FontScale` of its own, as it never did; it reads the scale
/// from the active theme instead, so its text scales like every row
/// beside it rather than staying at 100%.
pub fn row_field<'a, Message: 'a>(
    label: impl IntoFragment<'a>,
    control: impl Into<Element<'a, Message>>,
) -> Element<'a, Message> {
    let scale = FontScale(theme::active().font_scale);
    setting_row(
        1,
        label,
        None,
        // Fill, capped, and aligned right — not Shrink. A text field
        // fills whatever it is put in, and inside a Shrink container that
        // was nearly nothing: every field on the Window rules editor and
        // Displays' Advanced section drew as a sliver a few pixels wide.
        // A switch or a button still sits at the row's right end.
        container(control.into())
            .max_width(CONTROL_MAX_WIDTH)
            .width(Length::Fill)
            .align_x(iced::alignment::Horizontal::Right),
        scale,
    )
}

fn primary_style(theme: &Theme, status: button::Status) -> button::Style {
    let palette = theme.extended_palette();
    let base = button::Style {
        background: Some(Background::Color(palette.primary.base.color)),
        text_color: palette.primary.base.text,
        border: Border {
            radius: 8.0.into(),
            ..Border::default()
        },
        ..button::Style::default()
    };
    match status {
        button::Status::Hovered => button::Style {
            background: Some(Background::Color(palette.primary.strong.color)),
            ..base
        },
        button::Status::Disabled => button::Style {
            background: base.background.map(|b| match b {
                Background::Color(c) => Background::Color(Color { a: 0.4, ..c }),
                other => other,
            }),
            text_color: Color {
                a: 0.4,
                ..base.text_color
            },
            ..base
        },
        _ => base,
    }
}

fn secondary_style(_theme: &Theme, status: button::Status) -> button::Style {
    let base = button::Style {
        background: Some(Background::Color(surface::row())),
        text_color: crate::theme::text(),
        border: Border {
            radius: 8.0.into(),
            width: 1.0,
            color: surface::card_border(),
        },
        ..button::Style::default()
    };
    match status {
        button::Status::Hovered => button::Style {
            background: Some(Background::Color(surface::card_border())),
            ..base
        },
        button::Status::Disabled => button::Style {
            text_color: Color {
                a: 0.4,
                ..base.text_color
            },
            ..base
        },
        _ => base,
    }
}

fn danger_style(theme: &Theme, status: button::Status) -> button::Style {
    let palette = theme.extended_palette();
    let base = button::Style {
        background: Some(Background::Color(surface::row())),
        text_color: palette.danger.base.color,
        border: Border {
            radius: 8.0.into(),
            width: 1.0,
            color: palette.danger.base.color,
        },
        ..button::Style::default()
    };
    match status {
        // The stronger accent, not the danger colour it used to turn on
        // hover: red is a state colour, and a primary button is Apply,
        // Save, Keep — the opposite of danger. Pointing at one must not
        // make it look like the Delete button.
        button::Status::Hovered => button::Style {
            background: Some(Background::Color(palette.primary.strong.color)),
            text_color: palette.primary.strong.text,
            ..base
        },
        _ => base,
    }
}

/// The default action button — filled with the accent color. Use for the
/// one primary action in a row or dialog (Apply, Save, Confirm).
pub fn primary_button<'a, Message: Clone + 'a>(
    label: impl IntoFragment<'a>,
) -> button::Button<'a, Message> {
    button(text(label)).style(primary_style)
}

/// A quiet, outlined button for secondary actions (Rename, Cancel, Edit) —
/// visible without competing with the primary action in the same row.
pub fn secondary_button<'a, Message: Clone + 'a>(
    label: impl IntoFragment<'a>,
) -> button::Button<'a, Message> {
    button(text(label)).style(secondary_style)
}

/// A button styled for destructive/irreversible actions. Reserve for
/// things that actually destroy data (Delete) — a plain Cancel/Close is
/// `secondary_button`, not this.
pub fn danger_button<'a, Message: Clone + 'a>(
    label: &'a str,
    on_press: Message,
) -> Element<'a, Message> {
    button(text(label))
        .on_press(on_press)
        .style(danger_style)
        .into()
}

/// A three-way "unset / true / false" selector, for the settings that have
/// a real difference between "don't care" and "explicitly false" — a window
/// -rule matcher of `floating = false` selects tiled windows, which a plain
/// checkbox can't express separately from not matching on it at all.
pub fn tri_state<'a, Message: Clone + 'a>(
    value: Option<bool>,
    on_select: impl Fn(Option<bool>) -> Message,
) -> Element<'a, Message> {
    let option = |label: &'a str, choice: Option<bool>| {
        let button = if value == choice {
            primary_button(label)
        } else {
            secondary_button(label)
        };
        button.on_press(on_select(choice))
    };
    row![
        option("Any", None),
        option("Yes", Some(true)),
        option("No", Some(false)),
    ]
    .spacing(spacing::XS)
    .into()
}

/// The standard "this is about to touch a file you own" confirmation dialog
/// (vision pillar #4: nothing requires faith that a change worked — show the
/// exact diff before writing).
pub fn confirm_dialog<'a, Message: Clone + 'a>(
    title: &'a str,
    body: &'a str,
    diff_text: &'a str,
    on_confirm: Message,
    on_cancel: Message,
) -> Element<'a, Message> {
    let diff = container(text(diff_text).font(crate::theme::mono_font()).size(13))
        .padding(spacing::SM)
        .width(Length::Fill)
        .style(|_theme: &Theme| container::Style {
            background: Some(Background::Color(surface::row())),
            border: Border {
                radius: 6.0.into(),
                width: 1.0,
                color: surface::card_border(),
            },
            ..container::Style::default()
        });

    container(
        column![
            text(title).size(18),
            text(body),
            diff,
            row![
                secondary_button("Cancel").on_press(on_cancel),
                primary_button("Confirm").on_press(on_confirm),
            ]
            .spacing(spacing::SM),
        ]
        .spacing(spacing::MD)
        .padding(spacing::LG)
        .max_width(560.0),
    )
    .style(card_style)
    .into()
}



#[cfg(test)]
mod tests {
    use super::*;

    /// Pointing at Apply must not paint it the colour of Delete. It did,
    /// from the first polish pass until the Settings redesign: hover
    /// turned the primary button the danger red.
    #[test]
    fn a_primary_button_stays_the_accent_when_hovered() {
        let t = theme::app_theme();
        let palette = t.extended_palette();
        for status in [button::Status::Active, button::Status::Hovered] {
            let style = primary_style(&t, status);
            assert_ne!(
                style.background,
                Some(Background::Color(palette.danger.base.color)),
                "{status:?}"
            );
        }
        assert_eq!(
            primary_style(&t, button::Status::Hovered).background,
            Some(Background::Color(palette.primary.strong.color))
        );
    }
}
