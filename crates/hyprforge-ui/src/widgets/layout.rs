//! The shapes a settings page is built from: its header, its rows, the
//! card that leads it, and the bar that holds unapplied changes.

use super::{config_line, primary_button, scaled_text, secondary_button, Tint};
use crate::density;
use crate::theme::{self, spacing, surface, FontScale};
use iced::widget::{column, container, row, text::IntoFragment, Column, Container, Space};
use iced::{Background, Border, Element, Length};

// --- the page header -------------------------------------------------------

/// A page title's size at 100%.
const PAGE_TITLE_BASE: f32 = 20.0;

/// A page's title, with a short mono line beside it saying what the page
/// is talking to or what it found — `NetworkManager`,
/// `2 connected · drag to arrange`, `86 binds · 1 conflict`.
///
/// Drawn by the app's shell rather than by each page, so every page has
/// one and they all sit at the same height.
pub fn page_header<'a, Message: 'a>(
    title: impl IntoFragment<'a>,
    subtitle: Option<String>,
    scale: FontScale,
) -> Element<'a, Message> {
    let title = scaled_text(title, PAGE_TITLE_BASE, scale)
        .font(iced::Font { weight: iced::font::Weight::Semibold, ..iced::Font::DEFAULT })
        .color(theme::text());
    let mut header = row![title].spacing(spacing::SM + spacing::XS).align_y(iced::Alignment::Center);
    if let Some(subtitle) = subtitle {
        header = header.push(config_line(subtitle, scale));
    }
    header.into()
}

// --- setting rows ----------------------------------------------------------

/// The gap between setting rows: the mockup's 2px, which with the
/// alternating fill is what makes a stack read as rows without a divider
/// between each.
pub const SETTING_ROW_GAP: f32 = 2.0;

/// A stack of [`setting_row`]s, [`SETTING_ROW_GAP`] apart.
pub fn setting_list<'a, Message: 'a>(
    rows: impl IntoIterator<Item = Element<'a, Message>>,
) -> Column<'a, Message> {
    column(rows).spacing(SETTING_ROW_GAP).width(Length::Fill)
}

/// The fill of the row at `index` in a stack: every even row striped,
/// every odd one clear.
///
/// Even rather than odd so the first row is striped — the mockup's
/// choice, and the one that makes the top of a stack visible as the
/// top of something.
pub fn setting_row_style(index: usize) -> container::Style {
    container::Style {
        background: index.is_multiple_of(2).then(|| Background::Color(theme::row_tint())),
        border: Border { radius: density::card_radius().into(), ..Border::default() },
        ..container::Style::default()
    }
}

/// A dim second line under a row's label — what the setting does, or
/// the config line it writes (use [`config_line`] for the latter).
pub fn hint_text<'a>(content: impl IntoFragment<'a>, scale: FontScale) -> iced::widget::Text<'a> {
    super::meta_text(content, density::META_TEXT_BASE * 0.9, scale)
}

/// One setting: its label on the left (with an optional hint under it),
/// its control on the right, on a card that is striped or clear by
/// `index`.
///
/// The label and control sit at the row's two ends rather than in a
/// fixed 2:3 split. The split put a wide gap between "Description" and
/// its field, so the two read as unrelated; at the ends, the row itself
/// is what joins them, which is the job the stripe is doing anyway.
///
/// At least [`density::setting_row_height`] tall, so a page of one-line
/// rows keeps a steady rhythm, and taller when a hint needs the room —
/// never clipped to fit.
pub fn setting_row<'a, Message: 'a>(
    index: usize,
    label: impl IntoFragment<'a>,
    hint: Option<Element<'a, Message>>,
    control: impl Into<Element<'a, Message>>,
    scale: FontScale,
) -> Element<'a, Message> {
    let mut left = column![scaled_text(label, density::ROW_TEXT_BASE * 0.9, scale).color(theme::text())]
        .spacing(2.0);
    if let Some(hint) = hint {
        left = left.push(hint);
    }
    container(
        row![
            // The floor: a zero-width strut as tall as the least row, so
            // the row can grow past it but never shrink below it. iced's
            // container has a maximum height and no minimum.
            Space::new().width(Length::Fixed(0.0)).height(Length::Fixed(density::setting_row_height(scale))),
            left.width(Length::Fill),
            control.into(),
        ]
        .spacing(spacing::MD)
        .align_y(iced::Alignment::Center),
    )
    .padding([0.0, scale.apply(14.0)])
    .width(Length::Fill)
    .style(move |_t: &iced::Theme| setting_row_style(index))
    .into()
}

// --- the hero card ---------------------------------------------------------

const HERO_FILL_ALPHA: f32 = 0.10;
const HERO_BORDER_ALPHA: f32 = 0.40;

/// The card that leads a page with the one thing it is about — the
/// network you are on, the battery's charge.
///
/// Tinted by what that thing's *state* is: `Success` for charging,
/// `Accent` for "this is the connected one". Faintly, so the content
/// stays the loudest thing on it.
pub fn hero_card<'a, Message: 'a>(
    tint: Tint,
    content: impl Into<Element<'a, Message>>,
) -> Container<'a, Message> {
    let color = tint.iced();
    container(content)
        .padding(spacing::MD)
        .width(Length::Fill)
        .style(move |_t: &iced::Theme| container::Style {
            background: Some(Background::Color(iced::Color { a: HERO_FILL_ALPHA, ..color })),
            border: Border {
                radius: density::card_radius().into(),
                width: 1.0,
                color: iced::Color { a: HERO_BORDER_ALPHA, ..color },
            },
            ..container::Style::default()
        })
}

/// A small filled circle in `tint` — "connected", "pending", "muted".
pub fn status_dot<'a, Message: 'a>(tint: Tint, scale: FontScale) -> Element<'a, Message> {
    let side = scale.apply(7.0);
    let color = tint.iced();
    container(Space::new())
        .width(Length::Fixed(side))
        .height(Length::Fixed(side))
        .style(move |_t: &iced::Theme| container::Style {
            background: Some(Background::Color(color)),
            border: Border { radius: (side / 2.0).into(), ..Border::default() },
            ..container::Style::default()
        })
        .into()
}

// --- pending changes -------------------------------------------------------

/// "N pending changes", in words.
pub fn pending_label(count: usize) -> String {
    match count {
        1 => "1 pending change".to_string(),
        n => format!("{n} pending changes"),
    }
}

/// The bar along the foot of a page that has changes nobody has applied
/// yet: what they are, the setting they will write, and Discard/Apply.
///
/// `summary` is usually [`pending_label`]'s count, but not always — a
/// layout edited on a canvas is one change with no count to give.
/// `on_discard` is optional because not every page can take its drafts
/// back, and a Discard button that does nothing is worse than none.
///
/// One bar for every page, drawn by the shell, rather than each page's
/// own wording of "N fields typed but not applied" — nothing is written
/// until Apply, and the bar is what says so, so it has to look the same
/// everywhere or a user learns it on one page and misses it on the next.
///
/// The preview is the line that will actually change, not a summary of
/// it: the suite's rule is that nothing silently rewrites a dotfile, and
/// showing the line is how "silently" is ruled out.
pub fn pending_bar<'a, Message: Clone + 'a>(
    summary: String,
    preview: Option<String>,
    on_discard: Option<Message>,
    on_apply: Message,
    scale: FontScale,
) -> Element<'a, Message> {
    let mut info = row![
        status_dot(Tint::Warning, scale),
        scaled_text(summary, density::META_TEXT_BASE, scale).color(theme::text()),
    ]
    .spacing(spacing::SM)
    .align_y(iced::Alignment::Center);
    if let Some(preview) = preview {
        info = info.push(config_line(preview, scale));
    }
    let mut actions = row![].spacing(spacing::SM).align_y(iced::Alignment::Center);
    if let Some(discard) = on_discard {
        actions = actions.push(secondary_button("Discard").on_press(discard));
    }
    actions = actions.push(primary_button("Apply").on_press(on_apply));
    container(
        row![info.width(Length::Fill), actions]
            .spacing(spacing::SM)
            .align_y(iced::Alignment::Center),
    )
    .padding([spacing::SM, spacing::MD])
    .width(Length::Fill)
    .style(|_t: &iced::Theme| container::Style {
        background: Some(Background::Color(surface::sidebar())),
        border: Border { width: 1.0, color: surface::card_border(), ..Border::default() },
        ..container::Style::default()
    })
    .into()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A stripe and a gap, and nothing else: the two row looks must
    /// share their shape exactly, or alternating rows would read as two
    /// different kinds of row rather than one list.
    #[test]
    fn alternating_rows_differ_only_in_fill() {
        let striped = setting_row_style(0);
        let clear = setting_row_style(1);
        assert_eq!(striped.border, clear.border);
        assert_eq!(striped.text_color, clear.text_color);
        assert_eq!(striped.shadow, clear.shadow);
        assert_eq!(striped.background, Some(Background::Color(theme::row_tint())));
        assert_eq!(clear.background, None);
        assert_eq!(setting_row_style(2), striped, "the pattern repeats");
    }

    #[test]
    fn a_single_pending_change_is_not_plural() {
        assert_eq!(pending_label(1), "1 pending change");
        assert_eq!(pending_label(2), "2 pending changes");
        assert_eq!(pending_label(0), "0 pending changes");
    }
}
