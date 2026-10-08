//! Quick Look: the focused entry, large, over the window, until the next
//! key — Space, as in the Finder and in Nautilus with Sushi.
//!
//! A child of `browser`, like `searching`, so it can read the listing and
//! the selection without either growing an accessor for one feature.
//!
//! # One pipeline, a bigger picture
//!
//! What it shows is a [`Preview`], built by the same host function as the
//! preview pane's (`hyprforge_files::preview::build`) and drawn from the
//! same parts — a picture, the start of a text file, a folder's first
//! names, the details. The difference is the size it is asked for: the
//! pane decodes for 280 logical pixels, and a glance at a photograph
//! wants the window. So the host is asked with its own outcome,
//! [`Outcome::LoadQuickLook`], and decodes for [`quick_look_edge`] —
//! the card's picture box in physical pixels — rather than this module
//! growing a second set of readers.
//!
//! Its own slot rather than the pane's, because the two are different
//! questions about different entries at different sizes: the pane shows
//! the *selection* when exactly one thing is selected and the setting is
//! on; this shows the keyboard's *focus*, whatever the pane is doing.
//! The latest-wins rule is the pane's, kept here the same way — an
//! answer for anything but the entry on show is dropped
//! ([`Browser::quick_look_loaded`]).
//!
//! # A held arrow key
//!
//! The arrows move the focus with the card open, and the card follows,
//! so holding Down asks for every entry it passes. The browser asks
//! every time — it has no clock, and asking is free — and the *host*
//! coalesces: it waits a moment before starting, starts only the newest
//! request, and runs one at a time (`hyprforge_files::quicklook`). The
//! card meanwhile shows each entry's name, facts and icon at once, from
//! the listing, so moving through a folder still reads as moving.
//!
//! # It never keeps the keyboard
//!
//! The arrows, Space, Escape and Enter are the card's; any other key
//! closes it and then does what it would have, the way the transfers
//! popover behaves. A press on the dim layer closes it too.

use super::{listing_block, many, Browser, IconSet, Message, Outcome};
use crate::action::Action;
use crate::density;
use crate::format::{format_kind, format_modified_at, format_size};
use crate::icon::entry_icon;
use crate::preview::{Excerpt, Preview};
use crate::prefs::ViewMode;
use crate::types::Entry;
use hyprforge_ui::theme::{spacing, FontScale};
use hyprforge_ui::widgets::{meta_text, scaled_text};
use iced::widget::{column, container, row, scrollable};
use iced::{Element, Length};
use std::path::{Path, PathBuf};

/// The card's distance from the window's edges, logical. Enough that the
/// window still frames it, so it reads as a glance over the listing and
/// not as a new place.
const MARGIN: f32 = spacing::XL;

/// The card's own padding.
const PADDING: f32 = spacing::MD;

/// The name's size, base.
const TITLE_BASE: f32 = 17.0;

/// A line of text's height for its size — what the picture box leaves
/// room for above and below it. Generous on purpose: an over-estimate
/// decodes a few pixels smaller than the box, an under-estimate a few
/// larger, and only the second costs memory.
const LINE: f32 = 1.4;

/// The icon drawn while there is nothing better, base.
const ICON_BASE: f32 = 96.0;

/// What a browser holds for Quick Look.
#[derive(Debug, Clone, Default)]
pub(super) struct QuickLook {
    /// Open, and what it shows. `None` when closed.
    glance: Option<Glance>,
    /// Whether the last thing done at the listing was typing into the
    /// search from it — see [`Browser::typing_under_way`].
    typing: bool,
}

/// An open card.
#[derive(Debug, Clone, Default)]
struct Glance {
    /// The entry on show: the keyboard's focus, as of the last update.
    showing: Option<PathBuf>,
    /// What the host found out about it.
    found: Option<Preview>,
    /// Whether the host has answered for `showing` — so "still reading"
    /// and "nothing more to say" draw differently. Answered from the
    /// start inside an archive, where nothing is asked.
    answered: bool,
}

/// The card's size in a window of `window` logical pixels.
fn card_size(window: (f32, f32)) -> (f32, f32) {
    ((window.0 - 2.0 * MARGIN).max(200.0), (window.1 - 2.0 * MARGIN).max(160.0))
}

/// The picture's box inside the card, logical: the card less its padding,
/// the title and facts above and the key hint below.
pub fn picture_box(window: (f32, f32), scale: FontScale) -> (f32, f32) {
    let (w, h) = card_size(window);
    let header = scale.apply(TITLE_BASE) * LINE + scale.apply(density::META_TEXT_BASE) * LINE;
    let footer = scale.apply(density::META_TEXT_BASE) * LINE;
    let gaps = 2.0 * spacing::SM;
    ((w - 2.0 * PADDING).max(1.0), (h - 2.0 * PADDING - header - footer - gaps).max(1.0))
}

/// How many physical pixels the longer side of a Quick Look picture
/// needs, for a window of `window` logical pixels on an output of
/// `scale_factor` — what the host decodes for.
///
/// The box's longer side, not the picture's: a portrait photograph in a
/// landscape box is decoded a little larger than it is drawn. One edge
/// is all [`hyprforge_image::Budget::for_edge`], `pdftoppm -scale-to`
/// and ffmpeg's fit take, and the over-decode is bounded by the window's
/// own proportions.
pub fn quick_look_edge(window: (f32, f32), scale: FontScale, scale_factor: f32) -> u32 {
    let (w, h) = picture_box(window, scale);
    (w.max(h) * scale_factor.max(1.0)).ceil() as u32
}

impl Browser {
    /// Whether the Quick Look card is up.
    pub fn quick_look_open(&self) -> bool {
        self.quick_look.glance.is_some()
    }

    /// Closes the card, if it is up — for a host acting on a key the
    /// browser never sees (a window-scope action: switching tabs, say),
    /// which must not leave a card over a tab nobody is looking at.
    pub fn close_quick_look(&mut self) {
        self.quick_look.glance = None;
    }

    /// Whether a search is being typed at the listing right now: the last
    /// thing done was a letter typed into it from the listing, and the
    /// query is not empty.
    ///
    /// What a host passes to `Keymap::resolve_typing`, so a Space typed
    /// between two words of the query is the space it looks like rather
    /// than Quick Look. Any action, a click on a row, or a change to the
    /// field ends it — after which a Space means Quick Look again, on
    /// the results the query found.
    pub fn typing_under_way(&self) -> bool {
        self.quick_look.typing && !self.search_query.is_empty()
    }

    /// Records whether this update was typing at the listing.
    pub(super) fn note_typing(&mut self, typing: bool) {
        self.quick_look.typing = typing;
    }

    /// Opens the card on the focused entry. The entry itself is filled in
    /// by [`Self::with_quick_look`], which runs after every update.
    pub(super) fn open_quick_look(&mut self) -> Outcome {
        self.quick_look.glance = Some(Glance::default());
        Outcome::None
    }

    /// What a key means with the card up: the action to carry on with —
    /// the one asked for, or the arrow it stands for here — or `None`
    /// when the key was the card's own and nothing more happens.
    pub(super) fn quick_look_key(&mut self, action: Action) -> Option<Action> {
        if self.quick_look.glance.is_none() {
            return Some(action);
        }
        match action {
            Action::QuickLook | Action::ClearSearch => {
                self.quick_look.glance = None;
                None
            }
            // Column view's Left and Right leave the folder; with the card
            // up they would close it on whatever the parent focuses. Here
            // they step through the folder, as in every other view.
            Action::FocusLeft if self.prefs.view_mode == ViewMode::Columns => Some(Action::FocusUp),
            Action::FocusRight if self.prefs.view_mode == ViewMode::Columns => Some(Action::FocusDown),
            Action::FocusUp
            | Action::FocusDown
            | Action::FocusLeft
            | Action::FocusRight
            | Action::ExtendUp
            | Action::ExtendDown => Some(action),
            // Enter included: the card closes and the file opens the
            // ordinary way.
            other => {
                self.quick_look.glance = None;
                Some(other)
            }
        }
    }

    /// The host's answer for `path`. Dropped unless it is the entry on
    /// show — a slow decode for a picture the arrows have left behind
    /// must not replace the one now in front of the person.
    pub(super) fn quick_look_loaded(&mut self, path: PathBuf, found: Option<Preview>) {
        if let Some(glance) = &mut self.quick_look.glance {
            if glance.showing.as_ref() == Some(&path) {
                glance.found = found;
                glance.answered = true;
            }
        }
    }

    /// Keeps the card on the focused entry, asking the host about each
    /// new one — run after every update and action, like `with_media`, so
    /// no way of moving the focus leaves the card behind.
    ///
    /// Nothing focused, or a focus no longer shown, closes it: there is
    /// nothing to look at, and an empty card would be a dialog about
    /// nothing.
    pub(super) fn with_quick_look(&mut self, outcome: Outcome) -> Outcome {
        if self.quick_look.glance.is_none() {
            return outcome;
        }
        let target = self
            .selection
            .focused()
            .filter(|f| self.listed().iter().any(|e| e.path == *f))
            .map(Path::to_path_buf);
        let in_archive = self.archive.is_some();
        let Some(glance) = &mut self.quick_look.glance else { return outcome };
        let Some(target) = target else {
            self.quick_look.glance = None;
            return outcome;
        };
        if glance.showing.as_ref() == Some(&target) {
            return outcome;
        }
        // A member of an archive has no path another program can read,
        // so there is nothing to ask: the card shows what the listing
        // knows, as it does for any entry before its answer arrives.
        *glance = Glance { showing: Some(target.clone()), found: None, answered: in_archive };
        if in_archive {
            return outcome;
        }
        many(vec![outcome, Outcome::LoadQuickLook(target)])
    }

    /// The entry the card is showing, while it is up — what a host that
    /// draws something live in the card (a playing video) keys it on.
    pub fn quick_look_showing(&self) -> Option<&Path> {
        self.quick_look.glance.as_ref()?.showing.as_deref()
    }

    /// The entry on show, once the host has answered for it — when a
    /// host may start something costly for it, such as a player. The
    /// answer itself waits out a held arrow key (`hyprforge_files::
    /// quicklook`), so this does not change thirty times a second.
    pub fn quick_look_settled(&self) -> Option<&Path> {
        self.quick_look.glance.as_ref().filter(|g| g.answered)?.showing.as_deref()
    }

    /// The card's name-and-facts header and its key hint, for a host
    /// that fills the middle itself — see [`quick_look_card`]. `None`
    /// while the card is closed.
    pub fn quick_look_parts(&self, scale: FontScale) -> Option<(Element<'_, Message>, Element<'_, Message>)> {
        let glance = self.quick_look.glance.as_ref()?;
        let path = glance.showing.as_deref()?;
        let entry = self.listed().iter().find(|e| e.path == path)?;
        let icons = IconSet { icons: &self.icons, folders: &self.folder_icons };
        let header = column![title(entry, icons, scale), facts(entry, glance.found.as_ref(), scale)].spacing(spacing::SM);
        Some((header.into(), keys(&self.quick_look_hint(), scale)))
    }

    fn quick_look_hint(&self) -> String {
        self.config
            .keymap
            .combos_for(Action::QuickLook)
            .first()
            .map_or_else(|| "Esc".to_string(), |c| format!("{c} or Esc"))
    }

    /// The card, when it is up — drawn over everything by the host, from
    /// `menu_overlay`, so a host that already stacks the context menu
    /// needed no change to stack this.
    pub(super) fn quick_look_overlay(&self, scale: FontScale, window: (f32, f32)) -> Option<Element<'_, Message>> {
        let glance = self.quick_look.glance.as_ref()?;
        let path = glance.showing.as_deref()?;
        let entry = self.listed().iter().find(|e| e.path == path)?;
        let icons = IconSet { icons: &self.icons, folders: &self.folder_icons };
        let card = card(entry, glance, icons, &self.quick_look_hint(), scale);
        Some(frame(card, window, Message::QuickLookClose))
    }
}

/// The card around `content`, over the dimmed window, `close` sent by a
/// press on the dim layer — generic over the message, so a host can fill
/// the card with something of its own ([`Browser::quick_look_parts`])
/// and still draw exactly this card.
pub fn quick_look_card<'a, M: Clone + 'a>(
    header: Element<'a, M>,
    body: Element<'a, M>,
    keys: Element<'a, M>,
    window: (f32, f32),
    close: M,
) -> Element<'a, M> {
    let content = column![
        header,
        container(body).width(Length::Fill).height(Length::Fill).center_x(Length::Fill).center_y(Length::Fill),
        keys,
    ]
    .spacing(spacing::SM);
    frame(content, window, close)
}

/// The card's own frame and the scrim it sits on.
fn frame<'a, M: Clone + 'a>(content: impl Into<Element<'a, M>>, window: (f32, f32), close: M) -> Element<'a, M> {
    let (width, height) = card_size(window);
    let card = container(content)
        .padding(PADDING)
        .width(Length::Fixed(width))
        .height(Length::Fixed(height))
        .style(|_t: &iced::Theme| container::Style {
            background: Some(iced::Background::Color(hyprforge_ui::theme::surface::sidebar())),
            border: iced::Border {
                color: hyprforge_ui::theme::surface::card_border(),
                width: 1.0,
                radius: density::outer_radius().into(),
            },
            ..container::Style::default()
        });
    hyprforge_ui::widgets::scrim(card, Some(close))
}

/// The name, beside its icon.
fn title<'a>(entry: &'a Entry, icons: IconSet<'a>, scale: FontScale) -> Element<'a, Message> {
    row![
        container(entry_icon(entry.kind, icons.for_entry(entry), density::ROW_ICON, scale)),
        scaled_text(entry.name.clone(), TITLE_BASE, scale).wrapping(iced::widget::text::Wrapping::None),
    ]
    .spacing(spacing::SM)
    .align_y(iced::Alignment::Center)
    .into()
}

/// Which keys do what.
fn keys<'a>(hint: &str, scale: FontScale) -> Element<'a, Message> {
    meta_text(
        format!("{hint} closes \u{00b7} the arrows look at the next \u{00b7} Enter opens"),
        density::META_TEXT_BASE,
        scale,
    )
    .into()
}

/// The card's contents: the name and its facts, what the host found, and
/// which keys do what.
fn card<'a>(entry: &'a Entry, glance: &'a Glance, icons: IconSet<'a>, hint: &str, scale: FontScale) -> Element<'a, Message> {
    let found = glance.found.as_ref();

    let body: Element<'a, Message> = if let Some(picture) = found.and_then(|f| f.picture.as_ref()) {
        picture.view_fill()
    } else if let Some(excerpt) = found.and_then(|f| f.text.as_ref()) {
        excerpt_view(excerpt, scale)
    } else if let Some(listing) = found.and_then(|f| f.listing.as_ref()) {
        scrollable(listing_block(listing, scale)).width(Length::Fill).height(Length::Fill).into()
    } else {
        // Nothing yet, or nothing to show: the icon, large. "Reading…"
        // only while an answer is still coming, so a file with nothing
        // more to say does not look like one still loading.
        let mut lone = column![entry_icon(entry.kind, icons.for_entry(entry), ICON_BASE, scale)]
            .spacing(spacing::SM)
            .align_x(iced::Alignment::Center);
        if !glance.answered {
            lone = lone.push(meta_text("Reading\u{2026}", density::META_TEXT_BASE, scale));
        }
        container(lone).center_x(Length::Fill).center_y(Length::Fill).into()
    };

    column![
        title(entry, icons, scale),
        facts(entry, found, scale),
        container(body).width(Length::Fill).height(Length::Fill).center_x(Length::Fill).center_y(Length::Fill),
        keys(hint, scale),
    ]
    .spacing(spacing::SM)
    .into()
}

/// What the pane says about an entry, on one line that wraps when the
/// window is narrow: Kind, Size and Modified from the listing, then
/// whatever the host found — Dimensions, Duration, Pages, Artist.
fn facts<'a>(entry: &Entry, found: Option<&Preview>, scale: FontScale) -> Element<'a, Message> {
    let now = chrono::Local::now();
    let mut pairs = vec![
        ("Kind".to_string(), format_kind(entry)),
        ("Size".to_string(), format_size(entry.size)),
        ("Modified".to_string(), format_modified_at(entry.modified, now)),
    ];
    pairs.extend(found.map(|f| f.details.clone()).unwrap_or_default());
    let mut line = row![].spacing(spacing::MD);
    for (label, value) in pairs.into_iter().filter(|(_, v)| !v.is_empty()) {
        line = line.push(
            row![
                meta_text(label, density::META_TEXT_BASE, scale),
                scaled_text(value, density::META_TEXT_BASE, scale),
            ]
            .spacing(spacing::XS),
        );
    }
    line.wrap().vertical_spacing(spacing::XS).into()
}

/// A text file's start, as written: monospace, never wrapped — a wrapped
/// line of code reads as two lines that are not in the file — and
/// scrollable both ways, since the card is a window onto it rather than
/// a column it has to fit.
fn excerpt_view<'a>(excerpt: &'a Excerpt, scale: FontScale) -> Element<'a, Message> {
    // The listing's own size, not the pane's smaller one: the card has
    // the room, and this is the thing being looked at.
    let mut lines = column![scaled_text(&excerpt.text, density::ROW_TEXT_BASE, scale)
        .font(hyprforge_ui::theme::mono_font())
        .wrapping(iced::widget::text::Wrapping::None)];
    if excerpt.truncated {
        lines = lines.push(meta_text("\u{2026}", density::META_TEXT_BASE, scale));
    }
    let scrolled = scrollable(container(lines).padding(spacing::SM))
        .direction(scrollable::Direction::Both {
            vertical: scrollable::Scrollbar::default(),
            horizontal: scrollable::Scrollbar::default(),
        })
        .width(Length::Fill)
        .height(Length::Fill);
    container(scrolled)
        .width(Length::Fill)
        .height(Length::Fill)
        .style(|_t: &iced::Theme| container::Style {
            background: Some(iced::Background::Color(hyprforge_ui::theme::surface::card())),
            border: iced::Border { radius: density::nested_radius().into(), ..iced::Border::default() },
            ..container::Style::default()
        })
        .into()
}

#[cfg(test)]
mod tests {
    use super::super::tests_support::loaded;
    use super::*;

    fn asks(outcome: &Outcome) -> Vec<PathBuf> {
        match outcome {
            Outcome::LoadQuickLook(path) => vec![path.clone()],
            Outcome::Many(parts) => parts.iter().flat_map(asks).collect(),
            _ => Vec::new(),
        }
    }

    fn focused_on(names: &[&str], at: usize) -> Browser {
        let rows: Vec<(&str, bool)> = names.iter().map(|n| (*n, false)).collect();
        let mut browser = loaded(&rows);
        browser.update(Message::EntryClicked { index: at, ctrl: false, shift: false });
        browser
    }

    fn showing(browser: &Browser) -> Option<PathBuf> {
        browser.quick_look.glance.as_ref().and_then(|g| g.showing.clone())
    }

    /// Space opens the card on the focused entry and asks the host about
    /// that entry — and only that one.
    #[test]
    fn quick_look_asks_about_the_focused_entry() {
        let mut browser = focused_on(&["a.png", "b.png"], 1);
        let outcome = browser.perform(Action::QuickLook);
        assert!(browser.quick_look_open());
        assert_eq!(asks(&outcome), [PathBuf::from("/dir/b.png")]);
    }

    /// With nothing focused there is nothing to look at, and the card
    /// does not open on nothing.
    #[test]
    fn quick_look_with_nothing_focused_does_nothing() {
        let mut browser = loaded(&[("a.png", false)]);
        assert_eq!(asks(&browser.perform(Action::QuickLook)), Vec::<PathBuf>::new());
        assert!(!browser.quick_look_open());
    }

    /// The arrows move the focus with the card up, and the card follows.
    #[test]
    fn the_card_follows_the_arrows() {
        let mut browser = focused_on(&["a.png", "b.png", "c.png"], 0);
        browser.perform(Action::QuickLook);
        let outcome = browser.perform(Action::FocusDown);
        assert!(browser.quick_look_open(), "an arrow keeps it up");
        assert_eq!(asks(&outcome), [PathBuf::from("/dir/b.png")]);
        assert_eq!(showing(&browser), Some(PathBuf::from("/dir/b.png")));
    }

    /// Latest wins: an answer for an entry the arrows have already left
    /// is dropped, and the one for the entry on show is kept.
    #[test]
    fn an_answer_for_an_entry_already_passed_is_dropped() {
        let mut browser = focused_on(&["a.png", "b.png"], 0);
        browser.perform(Action::QuickLook);
        browser.perform(Action::FocusDown);
        let preview = Preview { details: vec![("Pages".into(), "3".into())], ..Preview::default() };
        browser.update(Message::QuickLookLoaded(PathBuf::from("/dir/a.png"), Some(preview.clone())));
        assert!(browser.quick_look.glance.as_ref().unwrap().found.is_none(), "a.png's answer landed on b.png");
        browser.update(Message::QuickLookLoaded(PathBuf::from("/dir/b.png"), Some(preview.clone())));
        assert_eq!(browser.quick_look.glance.as_ref().unwrap().found, Some(preview));
    }

    /// Space and Escape close it; neither does anything else.
    #[test]
    fn space_or_escape_closes_the_card_and_nothing_more() {
        for key in [Action::QuickLook, Action::ClearSearch] {
            let mut browser = focused_on(&["a.png"], 0);
            browser.perform(Action::QuickLook);
            // The pane behind catching up is all: no search cleared, no
            // file opened.
            let outcome = browser.perform(key);
            assert!(
                matches!(outcome, Outcome::None | Outcome::LoadPreview(_)),
                "{key:?}: {outcome:?}"
            );
            assert!(!browser.quick_look_open(), "{key:?}");
        }
    }

    /// Enter closes the card and opens the file the ordinary way.
    #[test]
    fn enter_closes_the_card_and_opens_the_file() {
        let mut browser = focused_on(&["a.png"], 0);
        browser.perform(Action::QuickLook);
        let outcome = browser.perform(Action::Open);
        assert!(!browser.quick_look_open());
        let opened = match outcome {
            Outcome::Activated(path) => Some(path),
            Outcome::Many(parts) => parts.into_iter().find_map(|p| match p {
                Outcome::Activated(path) => Some(path),
                _ => None,
            }),
            _ => None,
        };
        assert_eq!(opened, Some(PathBuf::from("/dir/a.png")));
    }

    /// It never keeps the keyboard: any other key closes it and then
    /// does what it would have.
    #[test]
    fn any_other_key_closes_the_card_and_still_does_its_job() {
        let mut browser = focused_on(&["a.png"], 0);
        browser.perform(Action::QuickLook);
        let outcome = browser.perform(Action::CopyPath);
        assert!(!browser.quick_look_open());
        let copied = match &outcome {
            Outcome::CopyText(t) => Some(t.clone()),
            Outcome::Many(parts) => parts.iter().find_map(|p| match p {
                Outcome::CopyText(t) => Some(t.clone()),
                _ => None,
            }),
            _ => None,
        };
        assert_eq!(copied.as_deref(), Some("/dir/a.png"), "{outcome:?}");
    }

    /// Column view's Left would leave the folder; with the card up it
    /// steps back through it instead.
    #[test]
    fn in_column_view_left_and_right_step_through_the_folder() {
        let mut browser = focused_on(&["a.png", "b.png"], 1);
        browser.prefs.view_mode = ViewMode::Columns;
        browser.perform(Action::QuickLook);
        browser.perform(Action::FocusLeft);
        assert_eq!(showing(&browser), Some(PathBuf::from("/dir/a.png")));
        assert_eq!(browser.current_dir, PathBuf::from("/dir"), "still in the same folder");
    }

    /// The pane behind the card does not decode what the card is already
    /// decoding larger — one full-size decode at a time — and catches up
    /// when the card closes.
    #[test]
    fn the_pane_waits_while_the_card_covers_it() {
        let asks_pane = |outcome: &Outcome| -> bool {
            match outcome {
                Outcome::LoadPreview(_) => true,
                Outcome::Many(parts) => parts.iter().any(|p| matches!(p, Outcome::LoadPreview(_))),
                _ => false,
            }
        };
        let mut browser = focused_on(&["a.png", "b.png"], 0);
        browser.prefs.preview_pane = true;
        browser.perform(Action::QuickLook);
        assert!(!asks_pane(&browser.perform(Action::FocusDown)), "the card is in front of it");
        assert!(asks_pane(&browser.perform(Action::QuickLook)), "closed: the pane catches up");
    }

    /// A press on the dim layer closes it, from the view's own message.
    #[test]
    fn a_press_outside_the_card_closes_it() {
        let mut browser = focused_on(&["a.png"], 0);
        browser.perform(Action::QuickLook);
        browser.update(Message::QuickLookClose);
        assert!(!browser.quick_look_open());
    }

    /// A member of an archive has no path another program can read, so
    /// nothing is asked — the card shows what the listing knows.
    #[test]
    fn inside_an_archive_nothing_is_asked_of_the_host() {
        let mut browser = focused_on(&["a.png"], 0);
        browser.set_archive(Some("zip".to_string()));
        let outcome = browser.perform(Action::QuickLook);
        assert!(browser.quick_look_open());
        assert!(asks(&outcome).is_empty(), "{outcome:?}");
        assert!(browser.quick_look.glance.as_ref().unwrap().answered, "and it does not claim to be reading");
    }

    /// The listing re-read without the entry on show closes the card,
    /// rather than leaving it up over something that is gone.
    #[test]
    fn the_card_closes_when_its_entry_goes() {
        let mut browser = focused_on(&["a.png", "b.png"], 0);
        browser.perform(Action::QuickLook);
        let b = loaded(&[("b.png", false)]).entries;
        browser.update(Message::DirLoaded(PathBuf::from("/dir"), Ok(b)));
        assert!(!browser.quick_look_open());
    }

    /// A Space typed between two words of a type-to-search query is the
    /// query's — and once anything else is done, Space is Quick Look
    /// again.
    #[test]
    fn typing_at_the_listing_is_under_way_until_something_else_is_done() {
        let mut browser = focused_on(&["a.png", "b.png"], 0);
        assert!(!browser.typing_under_way(), "nothing typed");
        browser.update(Message::TypeToSearch('a'));
        assert!(browser.typing_under_way());
        browser.perform(Action::FocusDown);
        assert!(!browser.typing_under_way(), "an arrow ends it");
        browser.update(Message::TypeToSearch('.'));
        browser.update(Message::EntryClicked { index: 0, ctrl: false, shift: false });
        assert!(!browser.typing_under_way(), "a click ends it");
    }

    /// The edge the host decodes for: the card's picture box, in physical
    /// pixels — twice as many on a 2x output, and bounded by the window,
    /// so a 36-megapixel photograph is never kept at its own size.
    #[test]
    fn the_decode_edge_follows_the_window_and_the_output_scale() {
        let scale = FontScale::default();
        let at_1x = quick_look_edge((1400.0, 900.0), scale, 1.0);
        let at_2x = quick_look_edge((1400.0, 900.0), scale, 2.0);
        assert!(at_1x < 1400, "inside the window: {at_1x}");
        assert!(at_1x > 1000, "but most of it: {at_1x}");
        assert!(at_2x.abs_diff(at_1x * 2) <= 1, "{at_1x} at 1x, {at_2x} at 2x");
        assert!(quick_look_edge((800.0, 600.0), scale, 1.0) < at_1x, "a smaller window, a smaller decode");
    }
}
