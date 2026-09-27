//! The one screen that asks who you are.
//!
//! A greeter and a lock screen are the same screen. Both show a
//! background, a clock and a name, both ask a question, both accept or
//! refuse. What differs is only *how* the question travels — a lock talks
//! to PAM directly, a greeter talks to greetd over a socket — and *where*
//! it is drawn: a lock gets a surface handed to it by the compositor, a
//! greeter is an ordinary window.
//!
//! So this crate is that screen, and nothing else. Two thin hosts
//! ([`hyprforge-lock`] and `hyprforge-greet`) supply a surface and a
//! [`conversation::Backend`]. Neither knows the other exists, and there
//! is no "matching them up" to do, because there is only one of them.
//!
//! The other half of feeling like one system is the [`Theme`] — and that
//! has a hard constraint behind it. A greeter runs as its own user, and a
//! home directory is `drwx------`, so it cannot read your wallpaper or
//! your settings *at all*. Continuity therefore isn't a styling exercise;
//! it's an export. See [`Theme::export`], and `hyprforge_look::theme`,
//! where it lives.

pub mod conversation;
pub mod scene;
pub mod screen;

pub use conversation::{Backend, Conversation, Prompt, Response, State};
// Re-exported rather than owned: the theme is shared with every other
// Hyprforge app, not just the two auth hosts, so it lives in
// `hyprforge-look`. Both hosts still say `hyprforge_authui::Theme`.
pub use hyprforge_look::{Color, Theme, ThemeError};
