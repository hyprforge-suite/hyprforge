//! Selection, and the labels that group what can be selected.
//!
//! Moved here from Files when Settings adopted the same design. These
//! are the pieces that carry the suite's one colour rule — purple means
//! *selected*, and nothing else is ever purple — so there must be exactly
//! one copy of each.

use crate::density;
use crate::theme::{self, surface, FontScale};
use iced::widget::{button, Text};
use iced::{Background, Border};

/// A clickable row's look: the accent when selected, a plain elevation
/// step when merely hovered, nothing otherwise.
///
/// Hover must never share the selected colour: a row the pointer happens
/// to be over looking like the one you picked makes "is this selected?"
/// ambiguous the instant the mouse moves.
/// `tests::only_the_selected_row_ever_uses_the_accent_colour` pins
/// this.
///
/// One function for every list of rows — Files' sidebar, its entry list,
/// its application chooser, and Settings' sidebar — because it is one
/// rule, and two copies of "purple means selected" is two places for it
/// to stop being true.
pub fn selectable_row_style(
    theme: &iced::Theme,
    status: button::Status,
    selected: bool,
) -> button::Style {
    let palette = theme.extended_palette();
    let background = if selected {
        Some(Background::Color(palette.primary.weak.color))
    } else {
        match status {
            button::Status::Hovered => Some(Background::Color(surface::row())),
            _ => None,
        }
    };
    button::Style {
        background,
        text_color: theme::text(),
        // The design's 6px inner radius, taken from the Theme — see
        // `density::inner_radius`'s doc.
        border: Border { radius: density::inner_radius().into(), ..Border::default() },
        ..button::Style::default()
    }
}

/// Which of the theme's roles a mark, a chip or a hero card is drawn in.
///
/// A closed set of *roles* rather than a colour, so nothing drawn with it
/// can pick a colour outside the theme. `Accent` is selection; `Info`,
/// `Success`, `Warning` and `Error` are state; `Dim` is neither.
///
/// Files' sidebar borrows the state roles for *identity* — `Warning` on
/// the Pictures row does not mean anything is wrong — which is a real
/// cost worth naming. It is defensible there because a sidebar place
/// carries no state for the colour to be confused with; the same trick
/// on a row that *does* carry state would be a mistake.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Tint {
    Accent,
    Info,
    Success,
    Warning,
    Error,
    Dim,
}

impl Tint {
    /// Resolves to the live theme's colour for this role.
    pub fn color(self) -> hyprforge_look::Color {
        let t = theme::active();
        match self {
            Tint::Accent => t.accent,
            Tint::Info => t.info,
            Tint::Success => t.success,
            Tint::Warning => t.warning,
            Tint::Error => t.error,
            Tint::Dim => t.surfaces.text_dim,
        }
    }

    /// [`Tint::color`], ready for iced.
    pub fn iced(self) -> iced::Color {
        crate::color::to_iced(self.color())
    }
}

/// `title` in capitals with a thin space between the letters.
///
/// iced has no letter-spacing, and the design's heading depends on it:
/// small uppercase text without it reads as a cramped word rather than
/// as a label. Inserting U+2009 THIN SPACE between characters is the
/// approximation available — coarser than real tracking, but it buys
/// most of the effect for a heading that is never more than two words.
pub fn spaced_caps(title: &str) -> String {
    title
        .to_uppercase()
        .chars()
        .map(|c| c.to_string())
        .collect::<Vec<_>>()
        .join("\u{2009}")
}

/// A group's heading in a sidebar or above a stack of rows — `PLACES`,
/// `HYPRLAND`, `BORDERS & FOCUS`.
///
/// Small and dim, because a heading is a signpost rather than something
/// to read: findable when looked for, invisible when not.
pub fn section_label<'a>(title: &str, scale: FontScale) -> Text<'a> {
    super::meta_text(spaced_caps(title), density::SECTION_LABEL_BASE, scale)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The design reserves purple for exactly one meaning: the current
    /// selection. This pins that the accent (`palette.primary.weak`,
    /// what `selectable_row_style` selects on) never appears for any
    /// non-selected status, hover included.
    #[test]
    fn only_the_selected_row_ever_uses_the_accent_colour() {
        let theme = theme::app_theme();
        let accent_bg = theme.extended_palette().primary.weak.color;

        let selected = selectable_row_style(&theme, button::Status::Active, true);
        assert_eq!(selected.background, Some(Background::Color(accent_bg)));

        for status in [
            button::Status::Active,
            button::Status::Hovered,
            button::Status::Pressed,
            button::Status::Disabled,
        ] {
            let unselected = selectable_row_style(&theme, status, false);
            assert_ne!(
                unselected.background,
                Some(Background::Color(accent_bg)),
                "a non-selected row must never render the accent colour ({status:?})"
            );
        }
    }

    /// Hover has to be visibly different from *both* "plain" and
    /// "selected" — a hover that read the same as selection would make a
    /// row the user is merely pointing at look like one they picked.
    #[test]
    fn hover_is_a_distinct_elevation_from_both_plain_and_selected() {
        let theme = theme::app_theme();
        let plain = selectable_row_style(&theme, button::Status::Active, false);
        let hovered = selectable_row_style(&theme, button::Status::Hovered, false);
        let selected = selectable_row_style(&theme, button::Status::Active, true);
        assert_ne!(plain.background, hovered.background);
        assert_ne!(hovered.background, selected.background);
    }

    /// The selected fill derives from the accent, never from a state
    /// colour — the mockup's rule that purple is selection and
    /// cyan/green/orange/red are state only.
    #[test]
    fn a_state_colour_is_never_used_for_selection() {
        let theme = theme::app_theme();
        let Some(Background::Color(selected)) =
            selectable_row_style(&theme, button::Status::Active, true).background
        else {
            panic!("a selected row has a fill");
        };
        for state in [Tint::Info, Tint::Success, Tint::Warning, Tint::Error] {
            let c = state.iced();
            assert_ne!(
                (selected.r, selected.g, selected.b),
                (c.r, c.g, c.b),
                "selection drawn in {state:?}"
            );
        }
    }

    /// Every role is a different colour, or a chip in one could not be
    /// told from a chip in another.
    #[test]
    fn every_tint_is_its_own_colour() {
        let all = [Tint::Accent, Tint::Info, Tint::Success, Tint::Warning, Tint::Error, Tint::Dim];
        for (i, a) in all.iter().enumerate() {
            for b in &all[i + 1..] {
                assert_ne!(a.color(), b.color(), "{a:?} and {b:?}");
            }
        }
    }

    #[test]
    fn a_heading_is_capitals_with_thin_spaces_between_the_letters() {
        assert_eq!(spaced_caps("Wi-Fi"), "W\u{2009}I\u{2009}-\u{2009}F\u{2009}I");
        assert_eq!(spaced_caps(""), "");
    }
}
