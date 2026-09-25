//! The controls a setting row holds: a switch, a slider with its value
//! beside it, a dropdown, and a segmented choice.
//!
//! Each is iced's own widget with the suite's look applied, not a
//! reimplementation — the behaviour (keyboard, dragging, the open menu)
//! is iced's and stays iced's.

use super::field::inset_field_style;
use crate::density;
use crate::theme::{self, surface, FontScale};
use iced::widget::overlay::menu;
use iced::widget::{button, container, pick_list, row, slider, text, toggler, Toggler};
use iced::{Background, Border, Element, Length};

// --- toggle ----------------------------------------------------------------

/// A switch's height at 100% scale; iced draws it twice as wide.
///
/// The mockup's switch is about 38×22. iced's `size` is the height, and
/// the track is `2 × size` wide, so 20 lands within a couple of pixels of
/// both.
const TOGGLE_BASE: f32 = 20.0;

/// An on/off switch — what every boolean setting is, rather than a
/// checkbox.
///
/// A checkbox reads as "include this in something", which is right for
/// picking rows and wrong for a setting that is simply on or off. The
/// caller attaches `on_toggle`; a switch without one is drawn disabled,
/// which is how iced says "you cannot change this here".
pub fn toggle<'a, Message: 'a>(is_on: bool, scale: FontScale) -> Toggler<'a, Message> {
    toggler(is_on).size(scale.apply(TOGGLE_BASE)).style(toggle_style)
}

/// The switch's look: the accent when on, the border grey when off, a
/// dark knob either way.
///
/// On is the accent because "on" is one of the two things purple means
/// here (the other is "selected"). Off is not a state colour — off is
/// not an error — so it is the lightest grey in the ramp.
pub fn toggle_style(t: &iced::Theme, status: toggler::Status) -> toggler::Style {
    let palette = t.extended_palette();
    let (is_on, hovered, disabled) = match status {
        toggler::Status::Active { is_toggled } => (is_toggled, false, false),
        toggler::Status::Hovered { is_toggled } => (is_toggled, true, false),
        toggler::Status::Disabled { is_toggled } => (is_toggled, false, true),
    };
    let track = match (is_on, hovered) {
        (true, false) => palette.primary.base.color,
        (true, true) => palette.primary.strong.color,
        (false, _) => surface::card_border(),
    };
    let fade = |c: iced::Color| if disabled { iced::Color { a: c.a * 0.5, ..c } } else { c };
    toggler::Style {
        background: Background::Color(fade(track)),
        // Off and hovered gets an outline, so pointing at a switch that
        // is off still says "this responds" without borrowing the accent
        // that would make it look on.
        background_border_width: if hovered && !is_on { 1.0 } else { 0.0 },
        background_border_color: theme::text_dim(),
        foreground: Background::Color(fade(surface::sidebar())),
        foreground_border_width: 0.0,
        foreground_border_color: iced::Color::TRANSPARENT,
        text_color: Some(theme::text()),
        border_radius: None,
        padding_ratio: 0.14,
    }
}

// --- sliders ---------------------------------------------------------------

/// How wide a slider's track is at 100%. The mockup's 150px: long enough
/// to place a value by eye, short enough to leave the label its room.
const SLIDER_WIDTH_BASE: f32 = 150.0;

/// Room for the readout beside a slider — "12 px", "1.60×" — so the
/// track does not shuffle sideways as the number gains a digit.
const READOUT_WIDTH_BASE: f32 = 56.0;

/// A slider with its value written beside it in the mono font.
///
/// The readout is what makes a slider usable for a setting: dragging to
/// "about there" is fine for volume and useless for a gap in pixels,
/// where the number is the point. The caller formats it, because only
/// the caller knows whether it is pixels, a scale or a percentage.
///
/// `on_release` is for settings that write a file: sending every
/// intermediate value of a drag to disk is a rewrite per pixel.
pub fn value_slider<'a, Message: Clone + 'a>(
    range: std::ops::RangeInclusive<f32>,
    value: f32,
    step: f32,
    on_change: impl Fn(f32) -> Message + 'a,
    on_release: Option<Message>,
    readout: String,
    scale: FontScale,
) -> Element<'a, Message> {
    let mut track = slider(range, value, on_change)
        .step(step)
        .width(Length::Fixed(scale.apply(SLIDER_WIDTH_BASE)))
        .style(slider_style);
    if let Some(release) = on_release {
        track = track.on_release(release);
    }
    row![
        track,
        text(readout)
            .font(theme::mono_font())
            .size(scale.apply(density::META_TEXT_BASE))
            .color(theme::text())
            .width(Length::Fixed(scale.apply(READOUT_WIDTH_BASE)))
            .align_x(iced::alignment::Horizontal::Right),
    ]
    .spacing(crate::theme::spacing::SM)
    .align_y(iced::Alignment::Center)
    .into()
}

/// A slider over a list of values, where only the listed ones exist.
///
/// A monitor's scale is the case this is for: Hyprland accepts 1.0,
/// 1.25, 1.6 and not 1.4, so a continuous slider would offer values that
/// get refused. This one slides over *positions* in the list, so there is
/// no position between two valid values to land on. The caller keeps the
/// list and looks the value up by index.
///
/// A list of one or none has nothing to slide between, so it is drawn as
/// its readout alone rather than as a track with its handle stuck.
pub fn stepped_slider<'a, Message: Clone + 'a>(
    count: usize,
    index: usize,
    on_change: impl Fn(usize) -> Message + 'a,
    on_release: Option<Message>,
    readout: String,
    scale: FontScale,
) -> Element<'a, Message> {
    if count < 2 {
        return text(readout)
            .font(theme::mono_font())
            .size(scale.apply(density::META_TEXT_BASE))
            .into();
    }
    let last = (count - 1) as f32;
    value_slider(
        0.0..=last,
        index.min(count - 1) as f32,
        1.0,
        move |raw| on_change(step_index(count, raw)),
        on_release,
        readout,
        scale,
    )
}

/// The list position a slider's raw value means: the nearest one, never
/// past either end.
///
/// iced's `step` already rounds, but a drag can deliver the range's own
/// end as a float that is not quite an integer, and a value from outside
/// the range must not index past the list. `NaN` is the first position,
/// because it is the only answer that is always valid.
pub fn step_index(count: usize, raw: f32) -> usize {
    if count == 0 || !raw.is_finite() {
        return 0;
    }
    (raw.round().max(0.0) as usize).min(count - 1)
}

/// The track: accent up to the handle, border grey after it, and a
/// light handle — the mockup's rail.
pub fn slider_style(t: &iced::Theme, status: slider::Status) -> slider::Style {
    let palette = t.extended_palette();
    let filled = match status {
        slider::Status::Active => palette.primary.base.color,
        slider::Status::Hovered | slider::Status::Dragged => palette.primary.strong.color,
    };
    slider::Style {
        rail: slider::Rail {
            backgrounds: (Background::Color(filled), Background::Color(surface::card_border())),
            width: 4.0,
            border: Border { radius: 2.0.into(), ..Border::default() },
        },
        handle: slider::Handle {
            shape: slider::HandleShape::Circle { radius: 7.0 },
            background: Background::Color(theme::text()),
            border_width: 0.0,
            border_color: iced::Color::TRANSPARENT,
        },
    }
}

// --- dropdown --------------------------------------------------------------

/// A closed `pick_list`'s look: the inset field's, so a dropdown beside a
/// text field reads as the same kind of control.
///
/// Apply as `.style(dropdown_style).menu_style(dropdown_menu_style)`.
/// Style functions rather than a wrapper widget, because `pick_list` is
/// generic over what it lists and how, and a wrapper would have to
/// re-expose every one of those parameters to add nothing.
pub fn dropdown_style(t: &iced::Theme, status: pick_list::Status) -> pick_list::Style {
    let inset = inset_field_style(t);
    let border = match status {
        pick_list::Status::Active => inset.border,
        // Hover and open outline in the dim text colour, not the accent:
        // an open menu is not a selection.
        pick_list::Status::Hovered | pick_list::Status::Opened { .. } => {
            Border { color: theme::text_dim(), ..inset.border }
        }
    };
    pick_list::Style {
        text_color: theme::text(),
        placeholder_color: theme::text_dim(),
        handle_color: theme::text_dim(),
        background: inset.background.unwrap_or(Background::Color(surface::card())),
        border,
    }
}

/// An open dropdown's list. The highlighted option is the selection
/// colour, because it is the option you are about to select.
pub fn dropdown_menu_style(t: &iced::Theme) -> menu::Style {
    let palette = t.extended_palette();
    menu::Style {
        background: Background::Color(surface::card()),
        border: Border {
            radius: density::inner_radius().into(),
            width: 1.0,
            color: surface::card_border(),
        },
        text_color: theme::text(),
        selected_text_color: theme::text(),
        selected_background: Background::Color(palette.primary.weak.color),
        shadow: iced::Shadow::default(),
    }
}

// --- segmented -------------------------------------------------------------

/// What the lit segment of a segmented control means.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SegmentLook {
    /// A mode of the window — Files' list/grid switch. The lit segment is
    /// an elevation step, not the accent: the mode is not a selection,
    /// and a second purple on the bar would compete with the one that
    /// means "selected".
    Quiet,
    /// The value of a setting — Dwindle or Master, Off/Always/Fullscreen.
    /// The lit segment *is* the chosen value, so it is the accent.
    Choice,
}

/// One segment's look, for a button placed inside [`segmented`].
pub fn segment_style(
    look: SegmentLook,
    active: bool,
) -> impl Fn(&iced::Theme, button::Status) -> button::Style {
    move |t, status| {
        let palette = t.extended_palette();
        let hovered = matches!(status, button::Status::Hovered);
        let (background, text_color) = match (look, active) {
            (SegmentLook::Choice, true) => {
                (Some(palette.primary.base.color), palette.primary.base.text)
            }
            (SegmentLook::Quiet, true) => (Some(surface::row()), theme::text()),
            (_, false) => (hovered.then(surface::row), theme::text_dim()),
        };
        button::Style {
            background: background.map(Background::Color),
            text_color,
            border: Border { radius: density::nested_radius().into(), ..Border::default() },
            ..button::Style::default()
        }
    }
}

/// The groove a row of segments sits in.
///
/// Filled with the sidebar's colour rather than the inset fields', so it
/// reads as a groove cut into the surface rather than as a raised control.
pub fn segmented<'a, Message: 'a>(
    segments: impl IntoIterator<Item = Element<'a, Message>>,
) -> Element<'a, Message> {
    container(row(segments).spacing(3.0).align_y(iced::Alignment::Center))
        .padding(3.0)
        .style(|_t: &iced::Theme| container::Style {
            background: Some(Background::Color(surface::sidebar())),
            border: Border {
                radius: density::inner_radius().into(),
                width: 1.0,
                color: surface::card_border(),
            },
            ..container::Style::default()
        })
        .into()
}

/// A setting whose value is one of a few named options, shown all at
/// once — the mockup's `Off | Always | Fullscreen`.
///
/// For three or four short options. Past that the row runs out of room
/// and a dropdown is the honest control.
pub fn segmented_choice<'a, T, Message>(
    options: &[T],
    selected: Option<&T>,
    label: impl Fn(&T) -> String,
    on_select: impl Fn(T) -> Message,
    scale: FontScale,
) -> Element<'a, Message>
where
    T: Clone + PartialEq,
    Message: Clone + 'a,
{
    segmented(options.iter().map(|option| {
        let active = selected == Some(option);
        button(text(label(option)).size(scale.apply(density::META_TEXT_BASE)))
            .padding([2.0, scale.apply(10.0)])
            .on_press(on_select(option.clone()))
            .style(segment_style(SegmentLook::Choice, active))
            .into()
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A monitor's scales are a list, not a range: no drag may produce a
    /// position between two of them, or past either end.
    #[test]
    fn the_scale_slider_cannot_land_between_valid_scales() {
        let scales = [1.0, 1.25, 1.6, 2.0];
        let n = scales.len();
        let mut raw = -1.0_f32;
        while raw <= n as f32 + 1.0 {
            let i = step_index(n, raw);
            assert!(i < n, "{raw} indexed past the list");
            assert!(scales.contains(&scales[i]));
            raw += 0.05;
        }
        assert_eq!(step_index(n, 1.49), 1);
        assert_eq!(step_index(n, 1.51), 2);
        assert_eq!(step_index(n, f32::NAN), 0);
        assert_eq!(step_index(0, 3.0), 0, "an empty list still answers");
    }

    /// On is the accent and off is not — and off is never a state
    /// colour, because a setting being off is not an error.
    #[test]
    fn a_switch_is_the_accent_when_on_and_plain_grey_when_off() {
        let t = theme::app_theme();
        let accent = t.extended_palette().primary.base.color;
        let on = toggle_style(&t, toggler::Status::Active { is_toggled: true });
        let off = toggle_style(&t, toggler::Status::Active { is_toggled: false });
        assert_eq!(on.background, Background::Color(accent));
        assert_eq!(off.background, Background::Color(surface::card_border()));
        for state in [theme::error(), theme::warning(), theme::success(), theme::info()] {
            assert_ne!(off.background, Background::Color(state));
        }
    }

    /// Only a `Choice` segment lights up in the accent. Files' view-mode
    /// switch is a mode, not a value, and a second purple on its bar
    /// would read as a second selection.
    #[test]
    fn only_a_value_choice_lights_its_segment_in_the_accent() {
        let t = theme::app_theme();
        let accent = Some(Background::Color(t.extended_palette().primary.base.color));
        let choice = segment_style(SegmentLook::Choice, true)(&t, button::Status::Active);
        let quiet = segment_style(SegmentLook::Quiet, true)(&t, button::Status::Active);
        assert_eq!(choice.background, accent);
        assert_ne!(quiet.background, accent);
        for look in [SegmentLook::Choice, SegmentLook::Quiet] {
            let hovered = segment_style(look, false)(&t, button::Status::Hovered);
            assert_ne!(hovered.background, accent, "hovering is not choosing ({look:?})");
        }
    }
}
