//! A line of a "before → after" preview.
//!
//! Written for Files' bulk rename, and here rather than in Files because
//! nothing about it is about files: any preview of a change to many
//! things at once — Settings importing a hand-written config, a batch of
//! retitled media — is the same two columns and the same three states.
//! See `DESIGN-SYSTEM.md`'s rule about the second user.

use super::scaled_text;
use crate::density;
use crate::theme::{self, spacing, FontScale};
use iced::widget::{column, container, row, text::IntoFragment};
use iced::{Element, Length};

/// What a [`change_row`] is saying about its thing.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Change {
    /// Nothing happens to it. Both sides dim, so the eye skips to the
    /// lines that do change — the row stays, because a preview that
    /// hides rows makes a person wonder whether the rule saw them.
    Unchanged,
    /// It changes, and can.
    Changes,
    /// It would change and cannot; the reason goes under it.
    Refused,
}

/// One thing in a preview: what it is now on the left, what it would be
/// on the right, and — when it cannot — the reason under the right-hand
/// side in the error colour.
///
/// Two equal columns rather than "old → new" run together, so a list of
/// them reads down each side; a long value wraps within its own column,
/// never under the other one. The arrow is text, in the dim colour, at
/// the metadata size: it separates, it does not say anything.
pub fn change_row<'a, Message: 'a>(
    before: impl IntoFragment<'a>,
    after: impl IntoFragment<'a>,
    change: Change,
    reason: Option<&'a str>,
    scale: FontScale,
) -> Element<'a, Message> {
    let (before_colour, after_colour) = match change {
        Change::Unchanged => (theme::text_dim(), theme::text_dim()),
        Change::Changes => (theme::text_dim(), theme::text()),
        Change::Refused => (theme::text_dim(), theme::error()),
    };
    let size = density::ROW_TEXT_BASE * 0.9;
    let wrap = iced::widget::text::Wrapping::WordOrGlyph;
    let mut right = column![scaled_text(after, size, scale).color(after_colour).wrapping(wrap)].spacing(2.0);
    if let (Change::Refused, Some(reason)) = (change, reason) {
        right = right.push(
            scaled_text(reason, density::META_TEXT_BASE * 0.9, scale).color(theme::error()).wrapping(wrap),
        );
    }
    row![
        container(scaled_text(before, size, scale).color(before_colour).wrapping(wrap)).width(Length::FillPortion(1)),
        scaled_text("\u{2192}", density::META_TEXT_BASE, scale).color(theme::text_dim()),
        container(right).width(Length::FillPortion(1)),
    ]
    .spacing(spacing::SM)
    .align_y(iced::Alignment::Start)
    .into()
}
