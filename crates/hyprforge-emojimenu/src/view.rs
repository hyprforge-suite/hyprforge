//! The picker's widget tree, built fresh each frame from a [`Model`].
//!
//! Every region is sized from [`Layout`] — the numbers
//! `geometry::Layout::hit` measures with — and the grid's lines from the
//! model's [`hyprforge_popup::Stack`]. Nothing here picks a height of its
//! own; see `geometry.rs`'s module doc for why that is the whole
//! contract. Every colour comes from the theme through
//! [`hyprforge_popup::kit::Look`]; CLAUDE.md is explicit that no app may
//! define its own colour constant.
//!
//! Nothing routes through iced's own click handling (every `Element` is
//! `Infallible`-messaged): `popup_app.rs` resolves clicks from raw
//! pointer coordinates against `geometry.rs`.

use crate::geometry::Layout;
use crate::model::{self, Item, Line, Model, Tab, ToneMode};
use hyprforge_look::Theme;
use hyprforge_popup::kit::{self, Look};
use iced_runtime::core::alignment::{Horizontal, Vertical};
use iced_runtime::core::text::{LineHeight, Wrapping};
use iced_runtime::core::{Border, Color, Element, Font, Length, Padding};
use iced_widget::{column, container, row, text, Column, Row, Space, Stack};

type El<'a, Message, Renderer> = Element<'a, Message, iced_widget::Theme, Renderer>;

/// The font emoji are drawn in, named explicitly rather than left to the
/// theme's UI font.
///
/// Without this, roughly half the grid renders as monochrome glyphs. It
/// is not a data problem — the table carries `U+FE0F` wherever it is
/// needed — but font selection: plenty of ordinary UI fonts carry their
/// own black-and-white ✔ ☺ ⚙, so a renderer asked for the theme's font
/// finds a glyph there and never falls back to the colour font beside it.
/// `noto-fonts-emoji` is a hard dependency of this package for exactly
/// this reason (see `packaging/arch/PKGBUILD`); without it cells render as
/// tofu, which is the honest failure rather than a half-monochrome grid.
const EMOJI_FONT: Font = Font::with_name("Noto Color Emoji");

/// How much of a cell an emoji fills — the design's 20px glyph in a 34px
/// cell. Emoji are square with no descenders, so they sit centred in the
/// box without leaving room for a line's leading.
const GLYPH_FILL: f64 = 0.6;

pub fn view<'a, Message, Renderer>(model: &'a Model, theme: &'a Theme) -> El<'a, Message, Renderer>
where
    Message: 'a,
    Renderer: iced_runtime::core::text::Renderer<Font = Font> + 'a,
{
    let layout = Layout::for_font_size(theme.font_size);
    let look = Look::new(theme);

    let labels: Vec<&str> = Tab::ALL.iter().map(|t| t.label()).collect();
    let header = container(column![
        row![
            kit::search_field(model.filter_text(), model.tab().placeholder(), layout.search.height, &look),
            Space::new().width((layout.tone_button.x - layout.search.right()) as f32),
            tone_button(model, &layout, &look),
        ],
        Space::new().height((layout.tabs.y - layout.search.bottom()) as f32),
        kit::tabs(&labels, model.tab().index(), layout.tabs.height, &look),
    ])
    .width(Length::Fill)
    .height(Length::Fixed(layout.grid.y as f32))
    .padding(Padding { top: layout.padding as f32, right: layout.padding as f32, bottom: 0.0, left: layout.padding as f32 });

    let content = column![
        header,
        container(grid(model, &layout, &look)).width(Length::Fill).height(Length::Fixed(layout.grid.height as f32)).padding(Padding {
            top: 0.0,
            right: layout.padding as f32,
            bottom: 0.0,
            left: layout.padding as f32
        }),
        kit::divider(true, &look),
        footer(model, &layout, &look),
    ];

    let mut layers: Vec<El<'a, Message, Renderer>> = vec![kit::frame(content, &look)];

    // The label of the section scrolling past, pinned over the top of the
    // grid — drawn opaque so the rows passing under it do not show
    // through. `Layout::hit` treats the same band as the label's.
    if let Some(section) = model.sticky_section().and_then(|s| model.sections().get(s)) {
        layers.push(
            container(kit::section_label(&section.title, section_note(model, section).as_deref(), layout.header_height, true, &look))
                .padding(Padding { top: layout.grid.y as f32, left: layout.padding as f32, right: layout.padding as f32, bottom: 0.0 })
                .width(Length::Fill)
                .into(),
        );
    }
    if let Some(bar) = kit::scrollbar_layer(&layout.scrollbar(), model.stack().content_height(), model.scroll_offset(), &look) {
        layers.push(bar);
    }
    if let Some(picker) = tone_picker(model, &layout, &look) {
        layers.push(picker);
    }
    Stack::with_children(layers).width(Length::Fill).height(Length::Fill).into()
}

/// A section label's right-hand note: the search's "best match first",
/// or — on People & Body, where nearly every emoji has tones — which tone
/// they are all being shown in.
fn section_note(model: &Model, section: &model::Section) -> Option<String> {
    if let Some(note) = &section.note {
        return Some(note.clone());
    }
    (section.title == "People & Body" && model.default_tone().is_some())
        .then(|| format!("default tone: {}", crate::config::tone_label(model.default_tone())))
}

/// The ✋ button: a raised hand in the default tone, and a ▾ saying it
/// opens something.
fn tone_button<'a, Message: 'a, Renderer>(model: &Model, layout: &Layout, look: &Look) -> El<'a, Message, Renderer>
where
    Renderer: iced_runtime::core::text::Renderer<Font = Font> + 'a,
{
    let look = *look;
    let fill = Color { a: 0.45, ..look.chip };
    let radius = look.radius_for(layout.tone_button.height, 8.0);
    let glyph_size = (layout.tone_button.height * 0.6) as f32;
    container(
        row![
            glyph(model.default_tone_glyph(), glyph_size),
            text("\u{25be}").size(look.label()).color(look.dim),
        ]
        .spacing(4)
        .align_y(Vertical::Center),
    )
    .width(Length::Fixed(layout.tone_button.width as f32))
    .height(Length::Fixed(layout.tone_button.height as f32))
    .align_x(Horizontal::Center)
    .align_y(Vertical::Center)
    .style(move |_: &iced_widget::Theme| container::Style {
        background: Some(fill.into()),
        border: Border { radius: radius.into(), ..Default::default() },
        ..Default::default()
    })
    .into()
}

/// An emoji glyph at `size`, in the emoji font. `LineHeight::Absolute`
/// makes the text box exactly the glyph rather than a line box with
/// leading above and below, which a centring container would otherwise
/// centre high.
fn glyph<'a, Message: 'a, Renderer>(value: &str, size: f32) -> El<'a, Message, Renderer>
where
    Renderer: iced_runtime::core::text::Renderer<Font = Font> + 'a,
{
    text(value.to_string())
        .font(EMOJI_FONT)
        .size(size)
        .line_height(LineHeight::Absolute(size.into()))
        .align_x(Horizontal::Center)
        .align_y(Vertical::Center)
        .wrapping(Wrapping::None)
        .into()
}

/// The scrolling grid: the visible lines, at exactly the positions the
/// model's stack gives them.
fn grid<'a, Message, Renderer>(model: &'a Model, layout: &Layout, look: &Look) -> El<'a, Message, Renderer>
where
    Message: 'a,
    Renderer: iced_runtime::core::text::Renderer<Font = Font> + 'a,
{
    if model.items().is_empty() {
        return container(text("No matches").size(look.font_size).color(look.dim)).padding(12).into();
    }
    let tab = model.tab();
    let lines = model.lines();
    let stack = model.stack();
    let offset = model.scroll_offset();
    let visible = stack.visible(offset, layout.grid.height);
    let selected = model.selected_index();
    let drawn: Vec<El<'a, Message, Renderer>> = visible
        .clone()
        .map(|i| match lines[i] {
            Line::Header(s) => {
                let section = &model.sections()[s];
                kit::section_label(&section.title, section_note(model, section).as_deref(), stack.height(i), false, look)
            }
            Line::Cells { start, end, .. } => Row::with_children(
                (start..end).map(|index| cell(model, model.items()[index], index == selected, tab, layout, look)),
            )
            .spacing(layout.column_gap(tab) as f32)
            .into(),
        })
        .collect();
    // Shifted up by however far the first drawn line sits above the
    // grid's top — the same offset `Layout::hit` adds back — and clipped,
    // so a half-scrolled row reads as half-scrolled.
    let shift = offset - stack.top(visible.start);
    container(Column::with_children(drawn).spacing(stack.spacing() as f32))
        .padding(Padding { top: -(shift as f32), right: 0.0, bottom: 0.0, left: 0.0 })
        .width(Length::Fill)
        .height(Length::Fixed(layout.grid.height as f32))
        .clip(true)
        .into()
}

/// One cell, at exactly `Layout`'s cell size for the tab — never left to
/// shrink or grow around its glyph, because the hit-test assumes that
/// size.
fn cell<'a, Message: 'a, Renderer>(model: &Model, item: Item, selected: bool, tab: Tab, layout: &Layout, look: &Look) -> El<'a, Message, Renderer>
where
    Renderer: iced_runtime::core::text::Renderer<Font = Font> + 'a,
{
    let look = *look;
    let (width, height) = (layout.cell_width(tab), layout.cell_height(tab));
    let value = model.display(item);
    let (content, rest, radius): (El<'a, Message, Renderer>, Option<Color>, f32) = match (item, tab) {
        (Item::Extra(_), Tab::Kaomoji) => (
            text(value.to_string()).size(look.font_size * 1.08).color(look.text).wrapping(Wrapping::None).into(),
            Some(Color { a: 0.3, ..look.chip }),
            8.0,
        ),
        (Item::Extra(_), _) => (
            text(value.to_string())
                .size((height * 0.55) as f32)
                .line_height(LineHeight::Absolute(((height * 0.7) as f32).into()))
                .color(look.text)
                .wrapping(Wrapping::None)
                .into(),
            None,
            7.0,
        ),
        (Item::Emoji(_), _) => (glyph(value, (height * GLYPH_FILL) as f32), None, 7.0),
    };
    let fill = if selected { Some(look.selected) } else { rest };
    let radius = look.radius_for(height.min(width), radius);
    container(content)
        .width(Length::Fixed(width as f32))
        .height(Length::Fixed(height as f32))
        .align_x(Horizontal::Center)
        .align_y(Vertical::Center)
        .clip(true)
        .style(move |_: &iced_widget::Theme| container::Style {
            background: fill.map(Into::into),
            border: Border { radius: radius.into(), ..Default::default() },
            ..Default::default()
        })
        .into()
}

/// The six-cell tone picker, floating at the rectangle
/// `popup_app::picker_rect` gives it — the one the hit-test uses.
fn tone_picker<'a, Message: 'a, Renderer>(model: &Model, layout: &Layout, look: &Look) -> Option<El<'a, Message, Renderer>>
where
    Renderer: iced_runtime::core::text::Renderer<Font = Font> + 'a,
{
    let state = model.tone_picker()?;
    let glyphs = model.tone_glyphs()?;
    let picker = crate::popup_app::picker_rect(model, layout, state.mode);
    let look = *look;
    let cells = glyphs.into_iter().enumerate().map(|(i, g)| {
        let on = i == state.cursor;
        let radius = look.radius_for(picker.cell, 7.0);
        container(glyph(g, (picker.cell * GLYPH_FILL) as f32))
            .width(Length::Fixed(picker.cell as f32))
            .height(Length::Fixed(picker.cell as f32))
            .align_x(Horizontal::Center)
            .align_y(Vertical::Center)
            .style(move |_: &iced_widget::Theme| container::Style {
                background: on.then_some(look.selected.into()),
                border: Border { radius: radius.into(), ..Default::default() },
                ..Default::default()
            })
            .into()
    });
    let outline = Color { a: 0.35, ..look.accent };
    let radius = look.radius_for(picker.rect.height, 11.0);
    let bar = container(Row::with_children(cells).spacing(picker.gap as f32))
        .width(Length::Fixed(picker.rect.width as f32))
        .height(Length::Fixed(picker.rect.height as f32))
        .padding(picker.padding as f32)
        .style(move |_: &iced_widget::Theme| container::Style {
            background: Some(look.raised.into()),
            border: Border { radius: radius.into(), width: 1.0, color: outline },
            ..Default::default()
        });
    Some(container(bar).padding(Padding { top: picker.rect.y as f32, left: picker.rect.x as f32, right: 0.0, bottom: 0.0 }).into())
}

/// The strip along the bottom: what is selected, larger, with its name —
/// and what the keys do right now.
fn footer<'a, Message: 'a, Renderer>(model: &Model, layout: &Layout, look: &Look) -> El<'a, Message, Renderer>
where
    Renderer: iced_runtime::core::text::Renderer<Font = Font> + 'a,
{
    let look_copy = *look;
    let picker = model.tone_picker();
    let subject: Option<(&'static str, String, String)> = match (picker, model.selected_item()) {
        (Some(state), _) => model.tone_glyphs().zip(model.tone_subject()).map(|(glyphs, emoji)| {
            let shown = glyphs[state.cursor];
            (shown, model.describe(Item::Emoji(emoji), model::cell_tone(state.cursor)), codepoints(shown))
        }),
        (None, Some(item)) => {
            let shown = model.display(item);
            let tone = match item {
                Item::Emoji(_) => model.default_tone(),
                Item::Extra(_) => None,
            };
            let sub = match (item, model.tab()) {
                (Item::Emoji(_), _) => codepoints(shown),
                (Item::Extra(x), Tab::Kaomoji) => format!("kaomoji · {} chars", x.text.chars().count()),
                (Item::Extra(_), _) => format!("symbol · {}", codepoints(shown)),
            };
            Some((shown, model.describe(item, tone), sub))
        }
        (None, None) => None,
    };

    let big: El<'a, Message, Renderer> = match (&subject, model.selected_item(), picker) {
        (Some((shown, ..)), Some(Item::Extra(_)), None) => {
            text(shown.to_string()).size(look.font_size * 1.15).font(kit::strong()).color(look.text).wrapping(Wrapping::None).into()
        }
        (Some((shown, ..)), _, _) => glyph(shown, look.font_size * 1.85),
        (None, ..) => Space::new().into(),
    };
    let words: El<'a, Message, Renderer> = match subject {
        Some((_, name, sub)) => column![
            text(name).size(look.font_size).font(Font { weight: iced_runtime::core::font::Weight::Medium, ..Font::DEFAULT }).color(look.text).wrapping(Wrapping::None),
            text(sub).size(look.label() * 1.05).font(look.mono).color(look.dim).wrapping(Wrapping::None),
        ]
        .spacing(2)
        .into(),
        None => Space::new().into(),
    };

    let paste = match model.paste_target() {
        Some(target) => format!("paste into {target}"),
        None => "paste".to_string(),
    };
    let hints: Vec<(&str, String)> = match picker.map(|p| p.mode) {
        Some(ToneMode::Paste) => vec![("Enter", "paste this tone".into()), ("Shift Enter", "make default".into())],
        Some(ToneMode::Default) => vec![("Enter", "make default".into()), ("Esc", "cancel".into())],
        None => {
            let mut hints = vec![("Enter", paste)];
            match model.selected_item() {
                Some(Item::Emoji(e)) if e.supports_tones() => hints.push(("hold", "skin tones".into())),
                _ if !model.filter_text().is_empty() => hints.push(("\u{2190} \u{2192}", "move".into())),
                _ => {}
            }
            hints
        }
    };
    let hints = Column::with_children(hints.iter().map(|(key, action)| kit::key_hint(key, action, look))).spacing(5).align_x(Horizontal::Right);

    // The name gives way to the hints rather than running under them: it
    // is clipped in a box that fills only what they leave.
    container(
        row![big, container(words).width(Length::Fill).clip(true), hints].spacing(10).align_y(Vertical::Center),
    )
    .width(Length::Fill)
    .height(Length::Fixed(layout.footer_height as f32))
    .padding(Padding { top: 0.0, right: 12.0, bottom: 0.0, left: 12.0 })
    .align_y(Vertical::Center)
    .style(move |_: &iced_widget::Theme| container::Style {
        background: Some(look_copy.footer.into()),
        ..Default::default()
    })
    .into()
}

/// A glyph's code points — `U+1F44D U+1F3FD` — the footer's second line
/// for an emoji. The design shows a `:shortcode:` there, but the emoji
/// data this picker is built from has no shortcodes, and inventing them
/// would be a claim about names that no other program shares. The code
/// points are true and useful. Variation selectors are left out (they say
/// how to draw it, not what it is), and a long ZWJ sequence is cut short.
fn codepoints(value: &str) -> String {
    let points: Vec<String> = value.chars().filter(|&c| c != '\u{fe0f}').map(|c| format!("U+{:04X}", c as u32)).collect();
    if points.len() > 4 {
        format!("{} \u{2026}", points[..4].join(" "))
    } else {
        points.join(" ")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn code_points_leave_out_the_presentation_selector_and_cut_a_long_sequence() {
        assert_eq!(codepoints("❤️"), "U+2764");
        assert_eq!(codepoints("👍🏽"), "U+1F44D U+1F3FD");
        assert_eq!(codepoints("👨‍👩‍👧‍👦"), "U+1F468 U+200D U+1F469 U+200D \u{2026}");
        assert_eq!(codepoints("→"), "U+2192");
    }

    #[test]
    fn the_corner_radius_follows_the_theme() {
        let square = Theme { rounding: 0, ..Theme::default() };
        assert_eq!(Look::new(&square).radius_for(34.0, 7.0), 0.0);
        let absurd = Theme { rounding: u32::MAX, ..Theme::default() };
        assert!(Look::new(&absurd).radius_for(34.0, 7.0) <= 17.0);
    }
}
