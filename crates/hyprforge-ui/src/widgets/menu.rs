//! A context menu: what a right click opens, at the pointer.
//!
//! Files drew the first one with its own code; the process manager was
//! the second app to want one, so it moved here — the look, the sizes it
//! is placed by, the rule for flipping it away from an edge, and the
//! overlay that closes it on a click anywhere else. What goes in a menu,
//! and what choosing a row does, stays the caller's.
//!
//! The menu is placed before iced lays it out, so its size is computed
//! ([`menu_size`]) from the same numbers [`context_menu`] draws with:
//! the box placed is the box drawn.

use crate::density;
use crate::theme::{self, spacing, surface, FontScale};
use crate::widgets::{divider, meta_text, scaled_text};
use iced::widget::{button, column, container, mouse_area, opaque, pin, row, stack, Space};
use iced::{Element, Length};

/// One row of a menu.
#[derive(Debug, Clone, PartialEq)]
pub enum MenuRow<Message> {
    /// Something to choose: its label, an optional dim note at the right
    /// edge (a shortcut, usually), and what choosing it sends — `None`
    /// draws it dimmed and unchoosable, never hidden, so the rows don't
    /// move under the pointer from one menu to the next.
    Item { label: String, hint: Option<String>, on: Option<Message> },
    /// A line between groups.
    Separator,
}

impl<Message> MenuRow<Message> {
    /// A row that sends `on` when chosen.
    pub fn item(label: impl Into<String>, on: Message) -> Self {
        MenuRow::Item { label: label.into(), hint: None, on: Some(on) }
    }

    /// A row shown but not choosable here.
    pub fn disabled(label: impl Into<String>) -> Self {
        MenuRow::Item { label: label.into(), hint: None, on: None }
    }

    /// The same row with a note at its right edge.
    pub fn hint(self, note: impl Into<String>) -> Self {
        match self {
            MenuRow::Item { label, on, .. } => MenuRow::Item { label, hint: Some(note.into()), on },
            separator => separator,
        }
    }
}

/// Wide enough for Files' longest label beside its shortcut — "Open in
/// New Tab" and "Ctrl+Enter" — at 100%.
pub const MENU_WIDTH: f32 = 260.0;
/// A separator's whole slot, the line centred in it, at 100%.
pub const MENU_SEPARATOR: f32 = 9.0;

/// The space inside a menu's border.
pub fn menu_padding(scale: FontScale) -> f32 {
    scale.apply(spacing::XS)
}

/// A separator's slot at this scale.
pub fn menu_separator_height(scale: FontScale) -> f32 {
    scale.apply(MENU_SEPARATOR)
}

/// A menu's `(width, height)` before it is drawn, from what each row is.
/// Takes only whether each row is a separator, so a caller with its own
/// row type can ask without building the rows first.
pub fn menu_size(separators: impl IntoIterator<Item = bool>, scale: FontScale) -> (f32, f32) {
    let rows: f32 = separators.into_iter().map(|sep| if sep { menu_separator_height(scale) } else { density::row_height(scale) }).sum();
    // `menu_padding` all round, plus the one-pixel border either side.
    let chrome = 2.0 * menu_padding(scale) + 2.0;
    (scale.apply(MENU_WIDTH), rows + chrome)
}

/// Where a menu's top-left corner goes, given where it was asked for,
/// its own size and the window's.
///
/// It opens down and to the right of the pointer, the way menus do,
/// unless that would run off the window — then it flips to open up, or
/// left, from the same point. A menu too large to fit either way is
/// pinned to the edge it overflows least, so its top-left is always on
/// screen and the first items are always reachable.
pub fn place_menu(at: (f32, f32), menu: (f32, f32), window: (f32, f32)) -> (f32, f32) {
    fn axis(at: f32, size: f32, limit: f32) -> f32 {
        if at + size <= limit {
            at
        } else if at - size >= 0.0 {
            at - size
        } else {
            (limit - size).max(0.0)
        }
    }
    (axis(at.0, menu.0, window.0), axis(at.1, menu.1, window.1))
}

/// The menu itself: the floating list's surface (`sidebar`, a 1px
/// `card_border`, `inner_radius`), one row per item. `highlighted` is the
/// row the keyboard is on, lit the way the pointer lights one.
pub fn context_menu<'a, Message: Clone + 'a>(rows: Vec<MenuRow<Message>>, highlighted: Option<usize>, scale: FontScale, width: f32) -> Element<'a, Message> {
    let row_h = density::row_height(scale);
    let mut list = column![].width(Length::Fill);
    for (index, item) in rows.into_iter().enumerate() {
        match item {
            MenuRow::Separator => {
                let slot = menu_separator_height(scale);
                list = list.push(container(divider()).height(Length::Fixed(slot)).center_y(Length::Fixed(slot)).padding([0, spacing::SM as u16]));
            }
            MenuRow::Item { label, hint, on } => {
                let enabled = on.is_some();
                let lit = highlighted == Some(index);
                let colour = if enabled { theme::text() } else { theme::text_dim() };
                // One line, whatever the label: `menu_size` counted one
                // row per item, and a caller's label can be longer than
                // any this crate ships.
                let mut content = row![scaled_text(label, density::ROW_TEXT_BASE, scale)
                    .color(colour)
                    .wrapping(iced::widget::text::Wrapping::None)
                    .width(Length::Fill)]
                .align_y(iced::Alignment::Center)
                .spacing(spacing::MD);
                if let Some(hint) = hint {
                    content = content.push(meta_text(hint, density::META_TEXT_BASE, scale));
                }
                list = list.push(
                    button(content)
                        .on_press_maybe(on)
                        .width(Length::Fill)
                        .height(Length::Fixed(row_h))
                        .padding([0, spacing::SM as u16])
                        .style(move |t: &iced::Theme, status| menu_item_style(t, status, lit, enabled)),
                );
            }
        }
    }
    container(list)
        .padding(menu_padding(scale))
        .width(Length::Fixed(width))
        .style(|_t: &iced::Theme| container::Style {
            background: Some(iced::Background::Color(surface::sidebar())),
            border: iced::Border { color: surface::card_border(), width: 1.0, radius: density::inner_radius().into() },
            ..container::Style::default()
        })
        .into()
}

/// A menu item's look.
///
/// Unlike a listing row, hover takes the accent here. In a menu the
/// pointer *is* the choice being made — the item under it is what a
/// click will run — so hover and the keyboard highlight are the same
/// state and look the same. A disabled item never lights up.
pub fn menu_item_style(theme: &iced::Theme, status: button::Status, highlighted: bool, enabled: bool) -> button::Style {
    let hovered = matches!(status, button::Status::Hovered | button::Status::Pressed);
    let lit = enabled && (highlighted || hovered);
    button::Style {
        background: lit.then(|| iced::Background::Color(theme.extended_palette().primary.weak.color)),
        text_color: theme::text(),
        border: iced::Border { radius: density::nested_radius().into(), ..iced::Border::default() },
        ..button::Style::default()
    }
}

/// A menu over the whole window, opened at `at` (window coordinates).
///
/// Two layers. Underneath, a transparent area the size of the window
/// that sends `close` when clicked anywhere, left or right — and takes
/// that click, so it does not also land on whatever was under it. On
/// top, the menu, `opaque` so a click on it never falls through to the
/// closing layer, and `pin`ned where it was asked for, flipped away from
/// any edge it would run off ([`place_menu`]). Stack it over the window's
/// content.
pub fn menu_overlay<'a, Message: Clone + 'a>(rows: Vec<MenuRow<Message>>, at: (f32, f32), window: (f32, f32), close: Message, scale: FontScale) -> Element<'a, Message> {
    let size = menu_size(rows.iter().map(|r| matches!(r, MenuRow::Separator)), scale);
    let (x, y) = place_menu(at, size, window);
    let away = mouse_area(Space::new().width(Length::Fill).height(Length::Fill)).on_press(close.clone()).on_right_press(close);
    let placed = pin(opaque(context_menu(rows, None, scale, size.0))).x(x).y(y).width(Length::Fill).height(Length::Fill);
    stack![away, placed].width(Length::Fill).height(Length::Fill).into()
}

/// Where the pointer last was, for opening a menu at it: iced's
/// `on_right_press` says that a press happened and not where.
pub mod pointer {
    use iced::{event, mouse, Event, Subscription};
    use std::sync::atomic::{AtomicU32, Ordering};

    static X: AtomicU32 = AtomicU32::new(0);
    static Y: AtomicU32 = AtomicU32::new(0);

    /// Listens for pointer movement and never produces a message. Add it
    /// to the application's subscriptions.
    pub fn track<Message: 'static + Send>() -> Subscription<Message> {
        event::listen_with(|event, _status, _window| {
            if let Event::Mouse(mouse::Event::CursorMoved { position }) = event {
                X.store(position.x.to_bits(), Ordering::Relaxed);
                Y.store(position.y.to_bits(), Ordering::Relaxed);
            }
            None
        })
    }

    /// The last pointer position seen, in window coordinates.
    pub fn last() -> (f32, f32) {
        (f32::from_bits(X.load(Ordering::Relaxed)), f32::from_bits(Y.load(Ordering::Relaxed)))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_menu_near_an_edge_flips_to_open_the_other_way() {
        let window = (800.0, 600.0);
        let menu = (260.0, 200.0);
        assert_eq!(place_menu((100.0, 100.0), menu, window), (100.0, 100.0), "room: down and right");
        assert_eq!(place_menu((700.0, 100.0), menu, window), (440.0, 100.0), "the right edge: opens left");
        assert_eq!(place_menu((100.0, 550.0), menu, window), (100.0, 350.0), "the bottom: opens up");
    }

    #[test]
    fn a_menu_too_big_for_either_side_keeps_its_first_rows_on_screen() {
        assert_eq!(place_menu((50.0, 50.0), (260.0, 900.0), (800.0, 600.0)), (50.0, 0.0));
    }

    #[test]
    fn the_size_placed_counts_each_row_as_it_is_drawn() {
        let scale = FontScale::default();
        let (w, h) = menu_size([false, true, false], scale);
        assert_eq!(w, MENU_WIDTH);
        assert_eq!(h, 2.0 * density::row_height(scale) + MENU_SEPARATOR + 2.0 * spacing::XS + 2.0);
    }

    #[test]
    fn a_row_without_a_message_is_shown_but_not_choosable() {
        let row: MenuRow<u8> = MenuRow::disabled("End as administrator").hint("needs Show everything");
        assert_eq!(row, MenuRow::Item { label: "End as administrator".into(), hint: Some("needs Show everything".into()), on: None });
    }
}
