//! The image viewer's model: what is in the folder, which picture is
//! next, how one is fitted to the window, and what the keyboard means.
//!
//! A library as well as an application, the arrangement `hyprforge-files`
//! and `hyprforge-tray` already use: everything the window is made of
//! lives here, where a test can reach it without opening a window or
//! talking to a compositor. Every test below this line runs on a
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
//! | What does the inspector say, and the status bar? | [`info`] |
//! | What does the filmstrip show? | [`filmstrip`] |
//! | Which day is a tile filed under, and where does Down go? | [`grid`] |
//! | What does a folder card in the library say? | [`library`] |
//! | Where do Back and Forward go? | [`history`] |
//! | What comes next in a slideshow? | [`slideshow`] |
//! | What did argv ask for? | [`args`] |
//! | Which program does "Open With…" use? | [`launch`] |
//!
//! The window itself — the `iced` application that renders all of this —
//! is `src/main.rs` (state and the slow work) and `src/view.rs` (the
//! widgets), and is deliberately thin: it turns these decisions into
//! widgets and runs the slow work (decoding, reading EXIF, listing
//! folders, trashing, the clipboard, the wallpaper) off the thread that
//! paints.
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
pub mod float;
pub mod folder;
pub mod grid;
pub mod history;
pub mod info;
pub mod keys;
pub mod launch;
pub mod library;
pub mod order;
pub mod prefs;
pub mod rotation;
pub mod slideshow;
pub mod transform;
