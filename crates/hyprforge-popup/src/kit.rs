//! The look the suite's pointer popups share — the clipboard history and
//! the emoji picker — and the handful of widgets both draw.
//!
//! They are two separate windows opened by two separate shortcuts, and
//! they are meant to read as one family: the same search field, the same
//! segmented tabs, the same uppercase section labels, the same key hints
//! along the bottom. A second copy of each in each crate is how two
//! things that should look the same stop doing so, which is the failure
//! this suite exists to prevent (see CLAUDE.md on the Settings app and
//! the lock screen growing their own palettes).
//!
//! # Every colour is the Theme's
//!
//! [`Look`] resolves the design's colours from [`hyprforge_look::Theme`]
//! once, and nothing here defines one of its own. Where the design asks
//! for a translucent shade — a selected cell's wash, a key chip's fill —
//! it is a Theme colour at an alpha, never a new hue: the accent at a
//! quarter for "selected", the ramp's lightest grey for chips and
//! dividers. Purple still means selection and nothing else.
//!
//! # Geometry stays with the caller
//!
//! These builders take their heights as arguments rather than choosing
//! them, because each popup's own `geometry` module is the single source
//! for where everything is — see that module in each crate, and
//! [`crate::stack`]. The one exception is [`Tabs`], whose hit-test lives
//! beside its drawing here because both popups draw it identically.

use hyprforge_look::Theme;
use iced_runtime::core::alignment::{Horizontal, Vertical};
use iced_runtime::core::font::Weight;
use iced_runtime::core::text::Wrapping;
use iced_runtime::core::{Border, Color, Element, Font, Length, Padding};
use iced_widget::{container, row, text, Space};

/// A Theme colour as iced wants it.
pub fn to_iced(c: hyprforge_look::Color) -> Color {
    Color::from_rgba8(c.r, c.g, c.b, c.a as f32 / 255.0)
}

/// `c` at `alpha` (0.0–1.0) — how the design's washes are made from
/// Theme colours rather than chosen.
pub fn tint(c: hyprforge_look::Color, alpha: f32) -> Color {
    Color { a: alpha.clamp(0.0, 1.0), ..to_iced(c) }
}

/// The monospace font for `theme` — its own family if it names one,
/// iced's generic monospace otherwise, the same rule `hyprforge-ui`
/// applies (an empty family is how `hyprforge_appearance::look` says the
/// fontconfig alias `Monospace` should not be looked up by name).
///
/// Leaked once: an iced `Font` holds a `&'static str`, and a popup is a
/// short-lived process drawing with one theme for its whole life.
pub fn mono_font(theme: &Theme) -> Font {
    static MONO: std::sync::OnceLock<Font> = std::sync::OnceLock::new();
    *MONO.get_or_init(|| match theme.mono_font.trim() {
        "" => Font::MONOSPACE,
        name => Font::with_name(Box::leak(name.to_owned().into_boxed_str())),
    })
}

/// The design's shades, resolved from one [`Theme`]. Plain `Copy` data
/// so a style closure can capture it by value — the closures outlive the
/// borrow of the theme a `view` call holds.
#[derive(Debug, Clone, Copy)]
pub struct Look {
    /// Behind everything in the popup.
    pub background: Color,
    /// The popup's own outline — the accent, quietly.
    pub outline: Color,
    /// An inset well: the search field and the tab track.
    pub inset: Color,
    pub inset_border: Color,
    pub text: Color,
    pub dim: Color,
    pub accent: Color,
    /// Text drawn *on* the accent — the primary button's label.
    pub on_accent: Color,
    /// The wash under a selected cell, tab or row.
    pub selected: Color,
    /// A key chip's fill, and a badge's.
    pub chip: Color,
    /// A hairline between regions.
    pub divider: Color,
    /// The strip along the bottom.
    pub footer: Color,
    pub info: Color,
    pub success: Color,
    pub warning: Color,
    pub error: Color,
    /// Body text size — the theme's own.
    pub font_size: f32,
    /// The popup's corner radius — the theme's own rounding, bounded.
    pub radius: f32,
    pub mono: Font,
}

impl Look {
    pub fn new(theme: &Theme) -> Look {
        let s = &theme.surfaces;
        Look {
            background: to_iced(s.root),
            outline: tint(theme.accent, 0.28),
            inset: to_iced(s.sidebar),
            inset_border: to_iced(s.card_border),
            text: to_iced(s.text),
            dim: to_iced(s.text_dim),
            accent: to_iced(theme.accent),
            on_accent: to_iced(s.root),
            selected: tint(theme.accent, 0.25),
            chip: tint(s.card_border, 0.7),
            divider: tint(s.card_border, 0.6),
            footer: tint(s.sidebar, 0.35),
            info: to_iced(theme.info),
            success: to_iced(theme.success),
            warning: to_iced(theme.warning),
            error: to_iced(theme.error),
            font_size: theme.font_size,
            radius: theme.corner_radius(),
            mono: mono_font(theme),
        }
    }

    /// The design's small sizes as fractions of the body size, so the
    /// whole popup follows the desktop's font setting together.
    pub fn small(&self) -> f32 {
        self.font_size * 0.88
    }

    /// Section labels and key chips.
    pub fn label(&self) -> f32 {
        self.font_size * 0.81
    }

    /// The design's radius for an inner element, `wanted`, but never
    /// rounder than the theme itself asks for — a theme with square
    /// corners gets square fields too — and never more than half of
    /// `side`: a larger one is a degenerate shape, and `tiny_skia` answers
    /// those with `None`, which iced unwraps — the reason
    /// `Theme::corner_radius` is bounded at all.
    pub fn radius_for(&self, side: f64, wanted: f32) -> f32 {
        wanted.min(self.radius).min((side / 2.0) as f32).max(0.0)
    }
}

/// The body font at a heavier weight.
pub fn strong() -> Font {
    Font { weight: Weight::Semibold, ..Font::DEFAULT }
}

/// The popup's outer frame: its background, its outline and its corners.
pub fn frame<'a, Message: 'a, Renderer>(
    content: impl Into<Element<'a, Message, iced_widget::Theme, Renderer>>,
    look: &Look,
) -> Element<'a, Message, iced_widget::Theme, Renderer>
where
    Renderer: iced_runtime::core::Renderer + 'a,
{
    let look = *look;
    container(content)
        .width(Length::Fill)
        .height(Length::Fill)
        .clip(true)
        .style(move |_: &iced_widget::Theme| container::Style {
            background: Some(look.background.into()),
            border: Border { radius: look.radius.into(), width: 1.0, color: look.outline },
            ..Default::default()
        })
        .into()
}

/// The search field: a drawn magnifier ring and either what was typed or
/// the placeholder, in an inset well exactly `height` tall.
///
/// The ring is drawn rather than a glyph for the reason `hyprforge-ui`'s
/// own search field gives: the theme's font is whatever the desktop
/// chose, and it may have no magnifier at all.
pub fn search_field<'a, Message: 'a, Renderer>(
    value: &str,
    placeholder: &str,
    height: f64,
    look: &Look,
) -> Element<'a, Message, iced_widget::Theme, Renderer>
where
    Renderer: iced_runtime::core::text::Renderer<Font = Font> + 'a,
{
    let look = *look;
    let ring_side = (look.font_size * 0.77).max(8.0);
    let ring = container(Space::new()).width(Length::Fixed(ring_side)).height(Length::Fixed(ring_side)).style(
        move |_: &iced_widget::Theme| container::Style {
            border: Border { radius: (ring_side / 2.0).into(), width: 1.6, color: look.dim },
            ..Default::default()
        },
    );
    let label = if value.is_empty() {
        text(placeholder.to_string()).color(look.dim)
    } else {
        text(value.to_string()).color(look.text)
    };
    let label = label.size(look.font_size).wrapping(Wrapping::None);
    let radius = look.radius_for(height, 8.0);
    container(row![ring, label].spacing(9).align_y(Vertical::Center))
        .width(Length::Fill)
        .height(Length::Fixed(height as f32))
        .padding(Padding { top: 0.0, right: 10.0, bottom: 0.0, left: 10.0 })
        .align_y(Vertical::Center)
        .clip(true)
        .style(move |_: &iced_widget::Theme| container::Style {
            background: Some(tint_color(look.inset, 0.6).into()),
            border: Border { radius: radius.into(), width: 1.0, color: look.inset_border },
            ..Default::default()
        })
        .into()
}

fn tint_color(c: Color, alpha: f32) -> Color {
    Color { a: alpha, ..c }
}

/// Where a row of equal segmented tabs is, and which one a point lands
/// on — the one place both the drawing ([`tabs`]) and a click read.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Tabs {
    pub x: f64,
    pub y: f64,
    pub width: f64,
    pub height: f64,
    pub count: usize,
}

impl Tabs {
    /// The track's inner padding, and the gap between two segments.
    pub const INSET: f64 = 3.0;

    fn segment_width(&self) -> f64 {
        let n = self.count.max(1) as f64;
        ((self.width - Self::INSET * 2.0 - Self::INSET * (n - 1.0)) / n).max(0.0)
    }

    /// The segment under `position`, or `None` outside the track or in a
    /// gap between two segments.
    pub fn tab_at(&self, position: (f64, f64)) -> Option<usize> {
        let (x, y) = (position.0 - self.x - Self::INSET, position.1 - self.y);
        if self.count == 0 || x < 0.0 || y < 0.0 || y > self.height {
            return None;
        }
        let stride = self.segment_width() + Self::INSET;
        let index = (x / stride) as usize;
        (index < self.count && x - index as f64 * stride <= self.segment_width()).then_some(index)
    }
}

/// Segmented tabs filling their track, `active` washed in the accent.
/// Drawn to the geometry [`Tabs`] hit-tests: `INSET` padding, `INSET`
/// between segments, every segment `Fill` so they split the width evenly.
pub fn tabs<'a, Message: 'a, Renderer>(
    labels: &[&str],
    active: usize,
    height: f64,
    look: &Look,
) -> Element<'a, Message, iced_widget::Theme, Renderer>
where
    Renderer: iced_runtime::core::text::Renderer<Font = Font> + 'a,
{
    let look = *look;
    let segment_height = (height - Tabs::INSET * 2.0).max(0.0);
    let radius = look.radius_for(segment_height, 6.0);
    let segments = labels.iter().enumerate().map(|(index, label)| {
        let on = index == active;
        let label = text(label.to_string())
            .size(look.font_size * 0.96)
            .font(if on { strong() } else { Font::DEFAULT })
            .color(if on { look.text } else { look.dim })
            .wrapping(Wrapping::None);
        container(label)
            .width(Length::Fill)
            .height(Length::Fixed(segment_height as f32))
            .align_x(Horizontal::Center)
            .align_y(Vertical::Center)
            .style(move |_: &iced_widget::Theme| container::Style {
                background: on.then_some(look.selected.into()),
                border: Border { radius: radius.into(), ..Default::default() },
                ..Default::default()
            })
            .into()
    });
    let track_radius = look.radius_for(height, 8.0);
    container(iced_widget::Row::with_children(segments).spacing(Tabs::INSET as f32))
        .width(Length::Fill)
        .height(Length::Fixed(height as f32))
        .padding(Padding::from(Tabs::INSET as f32))
        .style(move |_: &iced_widget::Theme| container::Style {
            background: Some(tint_color(look.inset, 0.55).into()),
            border: Border { radius: track_radius.into(), ..Default::default() },
            ..Default::default()
        })
        .into()
}

/// An uppercase section label — "PINNED", "SMILEYS & EMOTION" — with an
/// optional note at the right, in a line exactly `height` tall.
///
/// `opaque` paints the popup's own background behind it, which is what a
/// header pinned over scrolling content needs so the rows passing under
/// it do not show through.
pub fn section_label<'a, Message: 'a, Renderer>(
    title: &str,
    note: Option<&str>,
    height: f64,
    opaque: bool,
    look: &Look,
) -> Element<'a, Message, iced_widget::Theme, Renderer>
where
    Renderer: iced_runtime::core::text::Renderer<Font = Font> + 'a,
{
    let look = *look;
    let title = text(title.to_uppercase()).size(look.label()).font(strong()).color(look.dim).wrapping(Wrapping::None);
    let mut line = row![title].align_y(Vertical::Bottom);
    if let Some(note) = note {
        line = line.push(Space::new().width(Length::Fill)).push(
            text(note.to_string()).size(look.label() * 1.05).font(look.mono).color(look.dim).wrapping(Wrapping::None),
        );
    }
    container(line)
        .width(Length::Fill)
        .height(Length::Fixed(height as f32))
        .padding(Padding { top: 0.0, right: 6.0, bottom: 5.0, left: 6.0 })
        .align_y(Vertical::Bottom)
        .style(move |_: &iced_widget::Theme| container::Style {
            background: opaque.then_some(look.background.into()),
            ..Default::default()
        })
        .into()
}

/// A key chip and what it does — `[Enter] paste`.
pub fn key_hint<'a, Message: 'a, Renderer>(key: &str, action: &str, look: &Look) -> Element<'a, Message, iced_widget::Theme, Renderer>
where
    Renderer: iced_runtime::core::text::Renderer<Font = Font> + 'a,
{
    let look = *look;
    let chip = container(text(key.to_string()).size(look.label()).font(look.mono).color(look.text).wrapping(Wrapping::None))
        .padding(Padding { top: 1.0, right: 5.0, bottom: 1.0, left: 5.0 })
        .style(move |_: &iced_widget::Theme| container::Style {
            background: Some(look.chip.into()),
            border: Border { radius: 4.0.into(), ..Default::default() },
            ..Default::default()
        });
    row![chip, text(action.to_string()).size(look.small()).color(look.dim).wrapping(Wrapping::None)]
        .spacing(6)
        .align_y(Vertical::Center)
        .into()
}

/// A hairline `width` wide (or `Fill`), in the divider shade.
pub fn divider<'a, Message: 'a, Renderer>(horizontal: bool, look: &Look) -> Element<'a, Message, iced_widget::Theme, Renderer>
where
    Renderer: iced_runtime::core::Renderer + 'a,
{
    let color = look.divider;
    let line = container(Space::new()).style(move |_: &iced_widget::Theme| container::Style {
        background: Some(color.into()),
        ..Default::default()
    });
    if horizontal {
        line.width(Length::Fill).height(Length::Fixed(1.0)).into()
    } else {
        line.width(Length::Fixed(1.0)).height(Length::Fill).into()
    }
}

/// The scrollbar thumb as an overlay layer, or `None` when everything
/// already fits — a scrollbar that cannot scroll is noise. `bar` is the
/// caller's own [`crate::Scrollbar`], the same value its drag handling
/// hit-tests, so the drawn thumb and the dragged one are one thumb.
pub fn scrollbar_layer<'a, Message: 'a, Renderer>(
    bar: &crate::Scrollbar,
    content_height: f64,
    offset: f64,
    look: &Look,
) -> Option<Element<'a, Message, iced_widget::Theme, Renderer>>
where
    Renderer: iced_runtime::core::Renderer + 'a,
{
    if !bar.is_needed(content_height) {
        return None;
    }
    let color = Color { a: 0.55, ..look.accent };
    let width = bar.width as f32;
    let thumb = container(Space::new())
        .width(Length::Fixed(width))
        .height(Length::Fixed(bar.thumb_height(content_height) as f32))
        .style(move |_: &iced_widget::Theme| container::Style {
            background: Some(color.into()),
            border: Border { radius: (width / 2.0).into(), ..Default::default() },
            ..Default::default()
        });
    Some(
        container(thumb)
            .padding(Padding {
                top: bar.thumb_top(content_height, offset) as f32,
                left: bar.track_x as f32,
                right: 0.0,
                bottom: 0.0,
            })
            .into(),
    )
}

/// An axis-aligned rectangle in popup-surface coordinates — for the
/// handful of fixed controls (a button, the tone picker's trigger) whose
/// drawn box and clickable box must be the same box.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Rect {
    pub x: f64,
    pub y: f64,
    pub width: f64,
    pub height: f64,
}

impl Rect {
    pub fn contains(&self, position: (f64, f64)) -> bool {
        position.0 >= self.x && position.0 <= self.x + self.width && position.1 >= self.y && position.1 <= self.y + self.height
    }

    pub fn bottom(&self) -> f64 {
        self.y + self.height
    }

    pub fn right(&self) -> f64 {
        self.x + self.width
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_tab_is_hit_in_the_middle_of_where_it_is_drawn() {
        let tabs = Tabs { x: 10.0, y: 48.0, width: 580.0, height: 26.0, count: 5 };
        let segment = tabs.segment_width();
        for index in 0..5 {
            let x = tabs.x + Tabs::INSET + index as f64 * (segment + Tabs::INSET) + segment / 2.0;
            assert_eq!(tabs.tab_at((x, 60.0)), Some(index));
        }
    }

    #[test]
    fn a_point_outside_the_track_or_between_segments_hits_no_tab() {
        let tabs = Tabs { x: 10.0, y: 48.0, width: 340.0, height: 26.0, count: 3 };
        assert_eq!(tabs.tab_at((5.0, 60.0)), None);
        assert_eq!(tabs.tab_at((100.0, 40.0)), None);
        assert_eq!(tabs.tab_at((100.0, 80.0)), None);
        let gap = tabs.x + Tabs::INSET + tabs.segment_width() + Tabs::INSET / 2.0;
        assert_eq!(tabs.tab_at((gap, 60.0)), None);
    }

    /// The segments and the gaps between them must add up to the track —
    /// otherwise the last tab is drawn somewhere its hit box is not.
    #[test]
    fn the_segments_and_gaps_fill_the_track_exactly() {
        let tabs = Tabs { x: 0.0, y: 0.0, width: 340.0, height: 26.0, count: 3 };
        let total = 3.0 * tabs.segment_width() + 2.0 * Tabs::INSET + 2.0 * Tabs::INSET;
        assert!((total - 340.0).abs() < 1e-9);
    }

    /// Purple means selection: the wash under a selected tab or cell is
    /// the theme's accent, and nothing else in the look is.
    #[test]
    fn the_selection_wash_is_the_themes_accent_and_nothing_else_is() {
        let theme = Theme::default();
        let look = Look::new(&theme);
        let accent = to_iced(theme.accent);
        assert_eq!((look.selected.r, look.selected.g, look.selected.b), (accent.r, accent.g, accent.b));
        for other in [look.chip, look.divider, look.inset, look.footer] {
            assert_ne!((other.r, other.g, other.b), (accent.r, accent.g, accent.b));
        }
    }

    #[test]
    fn a_radius_never_exceeds_half_the_side_it_rounds() {
        let theme = Theme { rounding: u32::MAX, ..Theme::default() };
        let look = Look::new(&theme);
        assert!(look.radius_for(20.0, 8.0) <= 10.0);
        assert!(look.radius_for(0.0, 8.0) >= 0.0);
    }
}
