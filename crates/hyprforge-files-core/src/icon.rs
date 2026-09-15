//! What an entry's row draws for its icon.
//!
//! The brief for this crate asks for theme icon first, our own drawn
//! badge as fallback. Only the fallback half is built here:
//! [`theme_icon_name`] always returns `None`. A real lookup needs the
//! `freedesktop-icons` crate — resolving a MIME type to an icon name,
//! then walking an icon theme's own inheritance chain (`Inherits=`) to
//! find a file that actually exists — and CLAUDE.md already has a
//! measured fact about why that chain matters here: this machine's
//! configured theme, Dracula, declares eight parent themes of which six
//! are not installed and ships no `mimetypes/` of its own, so almost
//! every lookup falls through several levels before landing on
//! breeze-dark or hicolor. That is exactly the kind of resolution this
//! crate should not half-build — a lookup that skips the inheritance
//! walk would silently miss icons on this very machine and look "done"
//! on a tidier one. Rather than add the dependency without being able to
//! verify that walk end to end in this pass, [`theme_icon_name`] is left
//! as a named seam a later pass can fill in behind, and every caller
//! already goes through [`entry_icon`]'s fallback path — a working
//! fallback beats a half-done lookup.
//!
//! The fallback is a small coloured badge: a glyph for the [`EntryKind`],
//! a colour read from [`hyprforge_look::Theme`] — never a hardcoded
//! colour, which CLAUDE.md forbids for any app in this suite. Eight
//! kinds share the five colours the theme actually exposes for this
//! purpose (accent, success, warning, error, and the dim text colour),
//! so no two *adjacent-in-meaning* kinds share both a glyph and a
//! colour, but the glyph is what actually carries the distinction —
//! the colour only adds a second cue.

use crate::types::EntryKind;
use hyprforge_look::Color;
use hyprforge_ui::theme::{self, FontScale};
use iced::widget::container;
use iced::{Background, Border, Element, Length, Theme as IcedTheme};

/// Looks up a themed icon name for an entry, via the installed icon
/// theme. **Always `None`** — see this module's doc for why the real
/// lookup isn't built yet, and what it would take to fill this in.
pub fn theme_icon_name(_kind: EntryKind, _mime: Option<&str>) -> Option<&'static str> {
    None
}

/// The badge colour for `kind`, read from the active [`hyprforge_look::Theme`]
/// rather than hardcoded — see this module's doc.
pub fn badge_color(kind: EntryKind) -> Color {
    let t = theme::active();
    match kind {
        EntryKind::Folder => t.accent,
        EntryKind::Image => t.success,
        EntryKind::Document => t.surfaces.text_dim,
        EntryKind::Archive => t.warning,
        EntryKind::Code => t.accent,
        EntryKind::Audio => t.success,
        EntryKind::Video => t.error,
        EntryKind::Other => t.surfaces.text_dim,
    }
}

/// A small square badge for `kind`: [`theme_icon_name`]'s fallback,
/// always used for now. `size` is the badge's side length in logical
/// pixels before `scale` is applied.
pub fn entry_icon<'a, Message: 'a>(kind: EntryKind, size: f32, scale: FontScale) -> Element<'a, Message> {
    let side = scale.apply(size);
    let color = hyprforge_ui::color::to_iced(badge_color(kind));

    // A drawn square, not a glyph.
    //
    // This used to put an emoji — 📁, 🖼, 📄 — inside the badge, and the
    // badge's own `color` was computed and then thrown away, because a
    // colour emoji renders in the emoji font with *its* colours. The
    // folder came out the font's yellow on every theme, which breaks
    // the suite's one real styling rule in spirit while never writing a
    // hex code anywhere a grep would find it.
    //
    // It also depended on an emoji font being installed. The picker
    // needs one and says so; a file manager listing a directory should
    // not, and without one every row would have drawn an identical
    // empty box.
    //
    // So the mark is the shape itself: a filled rounded square in the
    // kind's colour, inside a lighter field of the same hue. Two
    // elevations of one colour, which is the design's own idiom, and
    // every pixel of it comes from the theme.
    let mark_side = side * MARK_FRACTION;
    let mark = container(iced::widget::Space::new())
        .width(Length::Fixed(mark_side))
        .height(Length::Fixed(mark_side))
        .style(move |_theme: &IcedTheme| container::Style {
            background: Some(Background::Color(color)),
            border: Border { radius: (mark_side * 0.28).into(), width: 0.0, color },
            ..container::Style::default()
        });

    container(mark)
        .center(Length::Fixed(side))
        .style(move |_theme: &IcedTheme| container::Style {
            background: Some(Background::Color(iced::Color { a: 0.18, ..color })),
            border: {
                // Scaled with the badge rather than fixed, so it stays a
                // rounded square at 200% instead of a square with a
                // decorative nick in each corner.
                Border { radius: (side * 0.28).into(), width: 0.0, color }
            },
            ..container::Style::default()
        })
        .into()
}

/// How much of the badge the inner mark fills.
///
/// Small enough that the ring of lighter colour around it reads as a
/// deliberate field rather than as a border that failed to render.
const MARK_FRACTION: f32 = 0.5;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn theme_icon_lookup_is_not_implemented_and_says_so_by_always_returning_none() {
        assert_eq!(theme_icon_name(EntryKind::Folder, Some("inode/directory")), None);
    }

    #[test]
    fn every_kind_gets_a_colour_that_depends_on_the_kind() {
        let kinds = [
            EntryKind::Folder,
            EntryKind::Image,
            EntryKind::Document,
            EntryKind::Archive,
            EntryKind::Code,
            EntryKind::Audio,
            EntryKind::Video,
            EntryKind::Other,
        ];
        let mut colors: Vec<[u8; 4]> = kinds
            .iter()
            .map(|k| {
                let c = badge_color(*k);
                [c.r, c.g, c.b, c.a]
            })
            .collect();
        colors.sort_unstable();
        colors.dedup();
        // Eight kinds, five colours the theme exposes for this: some
        // sharing is unavoidable and fine. What must not happen is all
        // of them collapsing to one, which would mean `badge_color` had
        // stopped reading the kind at all.
        assert!(colors.len() >= 4, "the badge colour must actually vary with the kind, got {colors:?}");
    }

    /// The badges are drawn, not typed.
    ///
    /// They used to be emoji, and a colour emoji renders in the emoji
    /// font with *its* colours — so the badge's carefully theme-derived
    /// colour was computed and then thrown away. Every folder came out
    /// the font's yellow whatever the theme said, which breaks this
    /// suite's one real styling rule in spirit while never writing a hex
    /// code anywhere a grep would find it. It also meant a machine with
    /// no emoji font drew an identical empty box on every row.
    ///
    /// The property that keeps it fixed is structural rather than
    /// textual: this module exposes **no function returning a string for
    /// an icon**. An earlier version of this test scanned the source for
    /// the emoji themselves and kept matching its own assertion text —
    /// the check has to live somewhere the thing it forbids cannot also
    /// appear. If a `badge_glyph`-shaped function comes back, it will
    /// have to be called from `entry_icon`, and the only way to feed a
    /// string to a container is a `text` widget: the compiler is the
    /// check, and this test is the note saying why nobody should add one.
    #[test]
    fn an_icon_carries_no_text_so_no_font_can_override_the_theme() {
        // `entry_icon` builds an `Element` with no text fragment
        // anywhere in it. What can be asserted here without a renderer
        // is that the colour it draws with is the theme's, which the
        // test below does, and that nothing in this module's public API
        // offers a glyph to draw. Both `badge_color` and `entry_icon`
        // are the entire surface:
        let _: fn(EntryKind) -> Color = badge_color;
        let _: fn(EntryKind, Option<&str>) -> Option<&'static str> = theme_icon_name;
        // `theme_icon_name` returns an icon *name* for a future
        // freedesktop lookup, not a glyph to render — see its own doc.
        assert_eq!(theme_icon_name(EntryKind::Folder, None), None);
    }

    #[test]
    fn badge_color_never_hardcodes_a_colour_it_reads_the_active_theme() {
        // Try to install a theme with a distinctive accent. `theme::init`
        // is backed by a `OnceLock` that accepts only the *first* call in
        // the whole test binary — CLAUDE.md's own note on this — and
        // this crate now has more than one test that can reach
        // `theme::active()` (the browser's selection-colour tests do,
        // through `entry_row_style`), so this call is not guaranteed to
        // win the race. Asserting against `theme::active().accent`
        // instead of a literal `0x010203` keeps the test proving the
        // property that actually matters — "badge_color reads whatever
        // the active theme says, not a constant" — regardless of which
        // test happened to set that theme first.
        let theme = hyprforge_look::Theme {
            accent: Color::rgba(0x01, 0x02, 0x03, 0xff),
            ..hyprforge_look::Theme::default()
        };
        theme::init(theme);
        assert_eq!(badge_color(EntryKind::Folder), theme::active().accent);
    }
}
