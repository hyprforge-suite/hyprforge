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
use hyprforge_ui::theme::{self, spacing, FontScale};
use hyprforge_ui::widgets::scaled_text;
use iced::widget::container;
use iced::{Background, Border, Element, Length, Theme as IcedTheme};

/// Looks up a themed icon name for an entry, via the installed icon
/// theme. **Always `None`** — see this module's doc for why the real
/// lookup isn't built yet, and what it would take to fill this in.
pub fn theme_icon_name(_kind: EntryKind, _mime: Option<&str>) -> Option<&'static str> {
    None
}

/// The glyph drawn on a fallback badge for `kind`. Plain, legible at
/// small sizes, and distinct enough from its neighbours to read at a
/// glance even before the colour is noticed.
pub fn badge_glyph(kind: EntryKind) -> &'static str {
    match kind {
        EntryKind::Folder => "\u{1F4C1}",   // 📁
        EntryKind::Image => "\u{1F5BC}",    // 🖼
        EntryKind::Document => "\u{1F4C4}", // 📄
        EntryKind::Archive => "\u{1F5DC}",  // 🗜
        EntryKind::Code => "\u{1F4BB}",     // 💻
        EntryKind::Audio => "\u{1F3B5}",    // 🎵
        EntryKind::Video => "\u{1F3AC}",    // 🎬
        EntryKind::Other => "\u{1F4CE}",    // 📎
    }
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
    container(scaled_text(badge_glyph(kind), size * 0.6, scale))
        .center(Length::Fixed(side))
        .style(move |_theme: &IcedTheme| container::Style {
            background: Some(Background::Color(iced::Color { a: 0.18, ..color })),
            border: Border {
                radius: spacing::XS.into(),
                width: 0.0,
                color,
            },
            ..container::Style::default()
        })
        .into()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn theme_icon_lookup_is_not_implemented_and_says_so_by_always_returning_none() {
        assert_eq!(theme_icon_name(EntryKind::Folder, Some("inode/directory")), None);
    }

    #[test]
    fn every_kind_gets_its_own_glyph() {
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
        let mut glyphs: Vec<&str> = kinds.iter().map(|k| badge_glyph(*k)).collect();
        let before = glyphs.len();
        glyphs.sort_unstable();
        glyphs.dedup();
        assert_eq!(glyphs.len(), before, "every EntryKind must have a distinct glyph");
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
