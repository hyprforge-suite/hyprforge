//! Text cut to a number of lines, with an ellipsis where it was cut.
//!
//! iced 0.14 has no ellipsis. A name too long for its box is either
//! wrapped past the box and clipped — the bottom line sliced through its
//! letters, or simply missing with nothing to say more was there — or,
//! unwrapped, drawn straight across whatever is beside it. Both read as
//! a bug rather than as "this goes on". Files' grid names at Extra large
//! were the case that made this a widget: a long name lost its third
//! line mid-glyph.
//!
//! So this measures. In `layout` — the one place the real width is known
//! — the whole text is shaped by the renderer's own `Paragraph`; if it
//! takes more than the allowed lines (or, unwrapped, more than the
//! width), the longest prefix that fits *with* "…" after it is found by
//! binary search over grapheme boundaries, so the cut never splits an
//! accented letter or an emoji in two. What is drawn is that prefix; what
//! is measured is what is drawn, so the box it reports is exact.
//!
//! Shaping is the cost, and it is paid once per text, width and size:
//! the answer is kept in the widget's state and reused by every later
//! layout that asks the same question — a grid of six hundred names
//! re-laid out by a hover costs six hundred comparisons, not six hundred
//! searches.

use iced::advanced::layout;
use iced::advanced::mouse;
use iced::advanced::renderer;
use iced::advanced::text::{self, paragraph, Paragraph, Text};
use iced::advanced::widget::{tree, Tree};
use iced::advanced::{Layout, Widget};
use iced::alignment;
use iced::widget::text::{LineHeight, Shaping, Wrapping};
use iced::{Color, Element, Length, Pixels, Rectangle, Size};
use std::borrow::Cow;
use unicode_segmentation::UnicodeSegmentation;

/// What is put where the text was cut.
pub const ELLIPSIS: &str = "\u{2026}";

/// `content`, at most `lines` lines tall, cut with "…" when it would be
/// taller. `usize::MAX` lines is the whole text — the same widget for a
/// name shown whole when selected, so selecting it does not change which
/// widget draws it.
pub fn clamped_text<'a>(content: impl Into<Cow<'a, str>>, lines: usize) -> ClampedText<'a> {
    ClampedText {
        content: content.into(),
        lines: lines.max(1),
        size: None,
        line_height: LineHeight::default(),
        font: None,
        width: Length::Shrink,
        align_x: text::Alignment::Default,
        wrapping: Wrapping::WordOrGlyph,
        shaping: Shaping::default(),
        color: None,
        _theme: std::marker::PhantomData,
    }
}

/// See [`clamped_text`].
pub struct ClampedText<'a, Theme = iced::Theme, Renderer = iced::Renderer>
where
    Renderer: text::Renderer,
{
    content: Cow<'a, str>,
    lines: usize,
    size: Option<Pixels>,
    line_height: LineHeight,
    font: Option<Renderer::Font>,
    width: Length,
    align_x: text::Alignment,
    wrapping: Wrapping,
    shaping: Shaping,
    color: Option<Color>,
    _theme: std::marker::PhantomData<Theme>,
}

impl<'a, Theme, Renderer> ClampedText<'a, Theme, Renderer>
where
    Renderer: text::Renderer,
{
    pub fn size(mut self, size: impl Into<Pixels>) -> Self {
        self.size = Some(size.into());
        self
    }

    pub fn line_height(mut self, line_height: impl Into<LineHeight>) -> Self {
        self.line_height = line_height.into();
        self
    }

    pub fn font(mut self, font: impl Into<Renderer::Font>) -> Self {
        self.font = Some(font.into());
        self
    }

    pub fn width(mut self, width: impl Into<Length>) -> Self {
        self.width = width.into();
        self
    }

    pub fn align_x(mut self, align: impl Into<text::Alignment>) -> Self {
        self.align_x = align.into();
        self
    }

    /// How lines break. `WordOrGlyph` by default, so a name with no
    /// spaces still breaks rather than running out of its box;
    /// `Wrapping::None` makes it a single line cut at the width — a list
    /// row's name.
    pub fn wrapping(mut self, wrapping: Wrapping) -> Self {
        self.wrapping = wrapping;
        self
    }

    pub fn shaping(mut self, shaping: Shaping) -> Self {
        self.shaping = shaping;
        self
    }

    /// The text's colour; the renderer's default text colour otherwise.
    pub fn color(mut self, color: impl Into<Color>) -> Self {
        self.color = Some(color.into());
        self
    }
}

/// The question a cut answers, so a later layout asking the same one
/// reuses the answer.
#[derive(Debug, Clone, PartialEq)]
struct Asked {
    content: String,
    width: f32,
    size: f32,
    lines: usize,
}

struct State<P: Paragraph> {
    paragraph: paragraph::Plain<P>,
    asked: Option<Asked>,
    shown: String,
}

impl<Message, Theme, Renderer> Widget<Message, Theme, Renderer> for ClampedText<'_, Theme, Renderer>
where
    Renderer: text::Renderer,
{
    fn tag(&self) -> tree::Tag {
        tree::Tag::of::<State<Renderer::Paragraph>>()
    }

    fn state(&self) -> tree::State {
        tree::State::new(State::<Renderer::Paragraph> {
            paragraph: paragraph::Plain::default(),
            asked: None,
            shown: String::new(),
        })
    }

    fn size(&self) -> Size<Length> {
        Size { width: self.width, height: Length::Shrink }
    }

    fn layout(&mut self, tree: &mut Tree, renderer: &Renderer, limits: &layout::Limits) -> layout::Node {
        let state = tree.state.downcast_mut::<State<Renderer::Paragraph>>();
        let limits = limits.width(self.width);
        let width = limits.max().width;
        let size = self.size.unwrap_or_else(|| renderer.default_size());
        let font = self.font.unwrap_or_else(|| renderer.default_font());
        let template = Text {
            content: "",
            bounds: Size::new(width, f32::INFINITY),
            size,
            line_height: self.line_height,
            font,
            align_x: self.align_x,
            align_y: alignment::Vertical::Top,
            shaping: self.shaping,
            wrapping: self.wrapping,
        };
        let asked = Asked { content: self.content.to_string(), width, size: size.0, lines: self.lines };
        if state.asked.as_ref() != Some(&asked) {
            state.shown = clamp::<Renderer::Paragraph>(Text { content: &self.content, ..template }, self.lines).into_owned();
            state.asked = Some(asked);
        }
        let _ = state.paragraph.update(Text { content: &state.shown, ..template });
        let measured = state.paragraph.min_bounds();
        layout::Node::new(limits.resolve(self.width, Length::Shrink, measured))
    }

    /// Says what it reads as, as iced's own text does: the whole of it,
    /// not the "…"-cut part drawn — a search or a screen reader wants the
    /// name, and a test finds a row by it. Without this nothing drawn
    /// with it could be found at all, so Files' tabs and names were
    /// invisible to `iced_test` and to anything else walking the tree.
    fn operate(&mut self, _tree: &mut Tree, layout: Layout<'_>, _renderer: &Renderer, operation: &mut dyn iced::advanced::widget::Operation) {
        operation.text(None, layout.bounds(), &self.content);
    }

    fn draw(
        &self,
        tree: &Tree,
        renderer: &mut Renderer,
        _theme: &Theme,
        style: &renderer::Style,
        layout: Layout<'_>,
        _cursor: mouse::Cursor,
        viewport: &Rectangle,
    ) {
        let state = tree.state.downcast_ref::<State<Renderer::Paragraph>>();
        let paragraph = state.paragraph.raw();
        let bounds = layout.bounds();
        let anchor = bounds.anchor(paragraph.min_bounds(), paragraph.align_x(), paragraph.align_y());
        // Clipped at its own sides, never above or below. A name is cut
        // with "…" to its width, so nothing should cross either side — and
        // the clip makes sure of it for a frame laid out before the cut
        // caught up. Callers used to get that by wrapping this in a
        // `container(..).clip(true)`, which clips all four sides at the
        // line box, and a font's descenders hang below its line box:
        // with Noto Sans every g, p and y in Files' names was cut through
        // (2026-10-18). A line's height of room above and below is more
        // than any font's overhang.
        let room = paragraph.min_bounds().height.max(1.0);
        let clip = Rectangle { x: bounds.x, y: bounds.y - room, width: bounds.width, height: bounds.height + room * 2.0 };
        // Through the paragraph's own clip bounds — iced clips text by
        // that argument, not by a layer; a layer around it was measured to
        // cut nothing.
        let Some(clip) = viewport.intersection(&clip) else { return };
        renderer.fill_paragraph(paragraph, anchor, self.color.unwrap_or(style.text_color), clip);
    }
}

/// `text`'s content, or the longest prefix of it that fits in `lines`
/// lines with [`ELLIPSIS`] after it. Generic over the paragraph so a test
/// can measure with a real one and no window.
///
/// "Fits" is both directions: no more than `lines` lines tall, and no
/// wider than the bounds — which only an unwrapped text can be.
pub fn clamp<'t, P: Paragraph>(text: Text<&'t str, P::Font>, lines: usize) -> Cow<'t, str> {
    if lines == usize::MAX {
        return Cow::Borrowed(text.content);
    }
    let line = text.line_height.to_absolute(text.size).0;
    // Half a pixel either way: shaping reports fractions, and a text
    // exactly two lines tall must not read as a hair over.
    let tallest = line * lines as f32 + 0.5;
    let widest = text.bounds.width + 0.5;
    let fits = |content: &str| {
        let bounds = P::with_text(Text { content, ..text }).min_bounds();
        bounds.height <= tallest && bounds.width <= widest
    };
    if fits(text.content) {
        return Cow::Borrowed(text.content);
    }
    let cuts: Vec<usize> = text.content.grapheme_indices(true).map(|(at, _)| at).collect();
    let with_ellipsis = |at: usize| format!("{}{ELLIPSIS}", text.content[..at].trim_end());
    // The largest number of graphemes kept that still fits. Zero always
    // "fits" for the search's purposes: a box too small for even "…"
    // shows "…", which still says something is there.
    let (mut lo, mut hi) = (0usize, cuts.len());
    while lo + 1 < hi {
        let mid = (lo + hi) / 2;
        if fits(&with_ellipsis(cuts[mid])) {
            lo = mid;
        } else {
            hi = mid;
        }
    }
    Cow::Owned(with_ellipsis(cuts.get(lo).copied().unwrap_or(0)))
}

impl<'a, Message, Theme, Renderer> From<ClampedText<'a, Theme, Renderer>> for Element<'a, Message, Theme, Renderer>
where
    Theme: 'a,
    Renderer: text::Renderer + 'a,
{
    fn from(text: ClampedText<'a, Theme, Renderer>) -> Self {
        Element::new(text)
    }
}

#[cfg(test)]
mod tests {

    /// Draws `element` at `w`×`h` logical pixels, at this machine's 1.6
    /// output scale, on black, and returns which *physical* pixels got
    /// any ink — through iced's own renderer with tiny-skia inside.
    const SCALE: f32 = 1.6;

    fn ink(element: Element<'_, ()>, w: u32, h: u32) -> Vec<bool> {
        use iced_runtime::user_interface::{Cache, UserInterface};
        let mut renderer = iced::Renderer::Secondary(iced_tiny_skia::Renderer::new(iced::Font::DEFAULT, Pixels(16.0)));
        let size = Size::new(w as f32, h as f32);
        let mut ui = UserInterface::build(element, size, Cache::default(), &mut renderer);
        ui.draw(&mut renderer, &iced::Theme::Dark, &iced::advanced::renderer::Style { text_color: Color::WHITE }, iced::mouse::Cursor::Unavailable);
        let iced::Renderer::Secondary(mut skia) = renderer else { unreachable!() };
        let (pw, ph) = ((w as f32 * SCALE) as u32, (h as f32 * SCALE) as u32);
        let mut pixmap = tiny_skia::Pixmap::new(pw, ph).unwrap();
        let mut mask = tiny_skia::Mask::new(pw, ph).unwrap();
        skia.draw(&mut pixmap.as_mut(), &mut mask, &iced_tiny_skia::graphics::Viewport::with_physical_size(Size::new(pw, ph), SCALE), &[Rectangle::with_size(size)], Color::BLACK);
        // Drawn text borrows from the tree: only now may it go.
        drop(ui);
        pixmap.data().chunks_exact(4).map(|px| px[0] > 60).collect()
    }

    /// A descender hangs below a font's line box: at a line height of 1.0
    /// a g's tail is below the box, and the widget must still draw it.
    /// Files wrapped this in clipping containers and cut every g, p and y
    /// with Noto Sans (2026-10-18); this pins that the widget's own clip,
    /// which replaced theirs, keeps the tail. Verified to fail when the
    /// clip cuts into the glyph. (A clip exactly at the box did not cut
    /// here, by how tiny-skia clips a paragraph — so the test asks for
    /// what is drawn, not for where the clip is.)
    #[test]
    fn a_descender_below_the_line_box_is_drawn() {
        let (w, h) = (120u32, 90u32);
        let text: Element<'_, ()> =
            iced::widget::Column::new().push(clamped_text("ggg", 1).size(40.0).line_height(LineHeight::Relative(1.0)).width(Length::Fill)).into();
        let lit = ink(text, w, h);
        assert!(lit.iter().any(|on| *on), "the rig drew nothing at all, so it would prove nothing");
        let (pw, ph) = ((w as f32 * SCALE) as u32, (h as f32 * SCALE) as u32);
        let floor = (40.0 * SCALE).ceil() as u32 + 1;
        let below = (floor..ph).any(|y| (0..pw).any(|x| lit[(y * pw + x) as usize]));
        assert!(below, "nothing drawn below the 40px line box: the descenders were clipped");
    }

    /// And it still clips its sides: an unbreakable word wider than the
    /// widget never draws past its right edge.
    #[test]
    fn nothing_is_drawn_past_its_right_edge() {
        let (w, h) = (300u32, 40u32);
        let text: Element<'_, ()> =
            iced::widget::Row::new().push(container_of(clamped_text("WWWWWWWWWWWWWWWWWWWW", 1).size(20.0).wrapping(Wrapping::None).width(Length::Fill), 100.0)).into();
        let lit = ink(text, w, h);
        assert!(lit.iter().any(|on| *on), "the rig drew nothing at all, so it would prove nothing");
        let (pw, ph) = ((w as f32 * SCALE) as u32, (h as f32 * SCALE) as u32);
        let edge = (100.0 * SCALE).ceil() as u32 + 2;
        let past = (0..ph).any(|y| (edge..pw).any(|x| lit[(y * pw + x) as usize]));
        assert!(!past, "ink past the widget's right edge");
    }

    fn container_of(text: ClampedText<'_>, width: f32) -> Element<'_, ()> {
        iced::widget::container(text).width(Length::Fixed(width)).into()
    }

    use super::*;
    use iced::advanced::graphics::text::Paragraph as Shaped;
    use iced::Font;

    fn text(content: &str, width: f32, wrapping: Wrapping) -> Text<&str, Font> {
        Text {
            content,
            bounds: Size::new(width, f32::INFINITY),
            size: Pixels(14.0),
            line_height: LineHeight::default(),
            font: Font::default(),
            align_x: text::Alignment::Center,
            align_y: alignment::Vertical::Top,
            shaping: Shaping::Advanced,
            wrapping,
        }
    }

    fn lines_of(content: &str, width: f32) -> f32 {
        let t = text(content, width, Wrapping::WordOrGlyph);
        Shaped::with_text(t).min_bounds().height / t.line_height.to_absolute(t.size).0
    }

    /// A short name is drawn as it is — no ellipsis, no copy.
    #[test]
    fn text_that_fits_is_left_whole() {
        let t = text("notes.txt", 200.0, Wrapping::WordOrGlyph);
        assert!(matches!(clamp::<Shaped>(t, 2), Cow::Borrowed("notes.txt")));
    }

    /// The case Files' grid had: a long name in a narrow box. It comes
    /// back ending in "…", within the lines allowed, and as long as it
    /// can be — one more grapheme would not fit.
    #[test]
    fn a_long_text_is_cut_at_the_line_limit_with_an_ellipsis() {
        let long = "Quarterly report for the regional office, final revision three (approved by everyone).pdf";
        for lines in 1..=3 {
            let shown = clamp::<Shaped>(text(long, 120.0, Wrapping::WordOrGlyph), lines).into_owned();
            assert!(shown.ends_with(ELLIPSIS), "{shown}");
            assert!(lines_of(&shown, 120.0) <= lines as f32 + 0.01, "{shown} is more than {lines} lines");
            let kept = shown.trim_end_matches(ELLIPSIS);
            // The next letter after the cut, past any space the cut
            // trimmed: keeping it too must not have fitted.
            let next = long[kept.len()..].char_indices().find(|(_, c)| !c.is_whitespace());
            let one_more = &long[..next.map_or(long.len(), |(at, c)| kept.len() + at + c.len_utf8())];
            let longer = format!("{}{ELLIPSIS}", one_more.trim_end());
            assert!(lines_of(&longer, 120.0) > lines as f32 + 0.01, "{longer} would have fitted in {lines}");
        }
    }

    /// The cut is between graphemes: an accented letter written as a
    /// base and a combining mark is never split from its accent.
    #[test]
    fn a_cut_never_splits_a_grapheme() {
        let decomposed = "e\u{301}".repeat(80);
        let shown = clamp::<Shaped>(text(&decomposed, 60.0, Wrapping::WordOrGlyph), 1).into_owned();
        let kept = shown.trim_end_matches(ELLIPSIS);
        assert!(!kept.is_empty() && kept.len().is_multiple_of(3), "{kept:?} ends inside a grapheme");
    }

    /// Unwrapped, the limit is the width: a list row's name is one line
    /// cut at its column's edge.
    #[test]
    fn an_unwrapped_text_is_cut_at_the_width() {
        let t = text("a-very-long-file-name-without-any-spaces-at-all.tar.gz", 100.0, Wrapping::None);
        let shown = clamp::<Shaped>(t, 1).into_owned();
        assert!(shown.ends_with(ELLIPSIS));
        let width = Shaped::with_text(text(&shown, 100.0, Wrapping::None)).min_bounds().width;
        assert!(width <= 100.5, "{shown} is {width} wide");
    }

    /// No limit is no limit: the selected cell's whole name.
    #[test]
    fn an_unlimited_clamp_is_the_whole_text() {
        let long = "word ".repeat(100);
        assert_eq!(clamp::<Shaped>(text(&long, 50.0, Wrapping::WordOrGlyph), usize::MAX), long);
    }

    /// A box too narrow for anything still says something is there.
    #[test]
    fn a_box_too_small_for_one_letter_shows_the_ellipsis() {
        let shown = clamp::<Shaped>(text("WWWWWWWW", 2.0, Wrapping::None), 1);
        assert_eq!(shown, ELLIPSIS);
    }
}
