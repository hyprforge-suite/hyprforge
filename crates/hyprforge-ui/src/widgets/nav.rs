//! A sidebar of places: an app's map down its left side.
//!
//! Settings drew its pages this way first, in its own code; the process
//! manager's tabs outgrew a header row (eleven of them, squeezing its
//! search field to a sliver at half a screen's width) and became its
//! second user — which is the moment the suite's rule says the widget
//! belongs here, rather than in a second copy.
//!
//! Groups of rows, each group under a spaced-caps label; a row is an
//! optional mark, its name, and an optional badge at the far end. The
//! chosen row takes the selection fill (`selectable_row_style`). The
//! label is centred in a fill-height container because a button's
//! content sits at its top, and at a fixed row height that put every
//! label above the middle of its own highlight (Settings learned it).

use super::{scaled_text, section_label, selectable_row_style};
use crate::density;
use crate::theme::{self, spacing, surface, FontScale};
use iced::widget::{button, column, container, row, scrollable, Space};
use iced::{Background, Border, Element, Length};

/// One place in the sidebar.
pub struct NavItem<'a, Message> {
    pub label: String,
    /// A mark before the name — a drawn glyph — or none.
    pub mark: Option<Element<'a, Message>>,
    /// Something at the row's far end: a count, a status dot.
    pub badge: Option<Element<'a, Message>>,
    pub selected: bool,
    pub on_press: Message,
}

/// A run of places under one label; `None` for a group with no heading.
pub struct NavGroup<'a, Message> {
    pub label: Option<&'a str>,
    pub items: Vec<NavItem<'a, Message>>,
}

/// One row of a [`nav_sidebar`].
pub fn nav_row<'a, Message: Clone + 'a>(item: NavItem<'a, Message>, scale: FontScale) -> Element<'a, Message> {
    let selected = item.selected;
    let mut line = row![].spacing(spacing::SM + 2.0).align_y(iced::Alignment::Center);
    if let Some(mark) = item.mark {
        line = line.push(mark);
    }
    line = line.push(scaled_text(item.label, density::ROW_TEXT_BASE * 0.9, scale).color(theme::text())).push(Space::new().width(Length::Fill));
    if let Some(badge) = item.badge {
        line = line.push(badge);
    }
    button(container(line).center_y(Length::Fill))
        .width(Length::Fill)
        .height(Length::Fixed(density::row_height(scale) + spacing::XS))
        .padding([0.0, spacing::SM + 2.0])
        .style(move |t: &iced::Theme, status| selectable_row_style(t, status, selected))
        .on_press(item.on_press)
        .into()
}

/// The sidebar: `groups` down a column `width` wide on the sidebar
/// surface, scrolling when it is taller than the window.
pub fn nav_sidebar<'a, Message: Clone + 'a>(groups: Vec<NavGroup<'a, Message>>, width: f32, scale: FontScale) -> Element<'a, Message> {
    let mut nav = column![].spacing(spacing::MD);
    for group in groups {
        let mut col = column![].spacing(1.0);
        if let Some(label) = group.label {
            col = col.push(container(section_label(label, scale)).padding([spacing::XS, spacing::SM + 2.0]));
        }
        for item in group.items {
            col = col.push(nav_row(item, scale));
        }
        nav = nav.push(col);
    }
    container(scrollable(container(nav).padding([spacing::MD, spacing::SM])).height(Length::Fill))
        .width(Length::Fixed(width))
        .height(Length::Fill)
        .style(|_t: &iced::Theme| container::Style { background: Some(Background::Color(surface::sidebar())), ..container::Style::default() })
        .into()
}

/// The sidebar's show/hide control, at the far left of a header where
/// the sidebar begins: the sidebar mark in a square, filled while the
/// sidebar shows — the "filled means on" idiom, so it states what is
/// true now rather than needing a second icon. Files and Media each drew
/// their own copy of exactly this; the process manager is the third
/// user, and the first to take it from here.
pub fn sidebar_toggle<'a, Message: Clone + 'a>(shown: bool, on_press: Message, scale: FontScale) -> Element<'a, Message> {
    let side = density::glyph_button(scale);
    button(container(crate::glyph::sidebar(side, theme::text())).center_x(Length::Fill).center_y(Length::Fill))
        .width(Length::Fixed(side))
        .height(Length::Fixed(side))
        .padding(0)
        .on_press(on_press)
        .style(move |_t: &iced::Theme, status| {
            let hovered = matches!(status, button::Status::Hovered);
            button::Style {
                background: (shown || hovered).then(|| Background::Color(surface::row())),
                text_color: theme::text(),
                border: Border { radius: density::inner_radius().into(), ..Border::default() },
                ..button::Style::default()
            }
        })
        .into()
}
