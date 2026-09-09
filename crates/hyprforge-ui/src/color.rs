//! Converting the shared colour type into iced's.
//!
//! A free function rather than a `From` impl: `hyprforge-look` must not
//! know that iced exists — the lock screen and the greeter paint into a
//! raw Wayland buffer and would otherwise carry a GUI toolkit for
//! nothing — and the orphan rule puts the conversion on this side
//! anyway.

/// `hyprforge-look`'s colour as iced wants it.
pub fn to_iced(c: hyprforge_look::Color) -> iced::Color {
    iced::Color::from_rgba8(c.r, c.g, c.b, c.a as f32 / 255.0)
}
