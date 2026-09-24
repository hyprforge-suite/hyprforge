//! The image viewer's model: what is in the folder, which picture is
//! next, how one is fitted to the window, and what the keyboard means.
//!
//! A library as well as an application, the arrangement `hyprforge-files`
//! and `hyprforge-tray` already use: everything the window is made of
//! lives here, where a test can reach it without opening a window or
//! talking to a compositor. The 81 tests below this line run on a
//! machine with no display and no pictures on it.
//!
//! # What is here, and what is still to come
//!
//! Every module below is the *decision* half of the viewer, and none of
//! it draws:
//!
//! | Question | Module |
//! |---|---|
//! | What is in this folder, and what is next? | [`folder`], [`order`] |
//! | How big is this picture on screen, and where? | [`transform`], [`rotation`] |
//! | Which pictures are decoded, and how many at once? | [`cache`] |
//! | What does this key do? | [`keys`] |
//! | What is remembered between launches? | [`prefs`], [`config`] |
//! | What does the info panel say? | [`info`] |
//! | What does the filmstrip show? | [`filmstrip`] |
//! | What did argv ask for? | [`args`] |
//! | Which program does "Open With…" use? | [`launch`] |
//!
//! The window itself — the `iced` application that renders all of this —
//! is `src/main.rs`, and is deliberately thin: it turns these decisions
//! into widgets and runs the slow work (decoding, trashing, the
//! clipboard, the wallpaper) off the thread that paints.
//!
//! # Why the order comes from somewhere else
//!
//! [`order`] does not decide what "next" means; it asks
//! `hyprforge-listing` for the same order the file manager is showing.
//! Two windows over one folder that disagree about which picture follows
//! this one is the sort of difference nobody can explain and everybody
//! notices — see `hyprforge-paths`'s note on `photos.toml` for the same
//! decision written from the settings end.

pub mod args;
pub mod cache;
pub mod config;
pub mod filmstrip;
pub mod folder;
pub mod info;
pub mod keys;
pub mod launch;
pub mod order;
pub mod prefs;
pub mod rotation;
pub mod transform;
