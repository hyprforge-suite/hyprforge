//! Shared machinery for a `wlr_layer`-shell popup: the surface and its
//! event loop, pointer and keyboard handling, placement, the
//! single-instance lock, and the teardown-then-paste ordering CLAUDE.md
//! is explicit about.
//!
//! Extracted from `hyprforge-clipmenu`, which built and tested every
//! piece of this against a real clipboard-history popup first — see
//! `crates/hyprforge-clipmenu/src/surface.rs`'s own doc for where this
//! diverged from `hyprforge-lock`'s layer-shell code, and `popup`'s
//! module doc for the design of [`popup::PopupApp`], the seam a
//! grid-based picker is meant to implement against instead of copying
//! ~1800 lines the way a second popup otherwise would have to.
//!
//! # What is, and is not, in here
//!
//! In: the Wayland/iced plumbing ([`popup`]), placement
//! ([`geometry`], [`placement`]), the single-instance lock
//! ([`singleton`]), and the pure scroll-wheel arithmetic ([`scroll`]).
//!
//! Not in: what a popup's content *means* — a clipboard entry, a pin, an
//! emoji, a search filter's own keybinds. That is `PopupApp`'s job, one
//! implementation per consumer, living in that consumer's own crate.
//!
//! # Why this crate does not depend on `hyprforge-clipboard`
//!
//! `hyprforge-clipmenu`'s "teardown, then paste" ordering is the poster
//! child for what belongs here (see [`popup::finish_after_teardown`]),
//! and it would be tempting to also move `hyprforge-clipboard`'s
//! `Chooser`/paste-synthesis seam in alongside it. That was considered
//! and rejected: `hyprforge-clipboard` pulls in `tokio` for its own
//! daemon and IPC client, and this crate has to stay a leaf with no
//! async runtime and no D-Bus, the same way `hyprforge-look` and
//! `hyprforge-process` do — CLAUDE.md's "What the layering is for" is
//! explicit that this is what keeps a new popup shareable without
//! forcing an async runtime onto whoever links it. So the *mechanism*
//! (tear the surface down, prove the compositor processed that, only
//! then let the app act) lives here as
//! [`popup::PopupApp::finish`]/[`popup::PopupApp::needs_finish`], called
//! generically with no idea what "finish" does; the *policy* (put the
//! entry on the clipboard, synthesize Ctrl+V) stays in
//! `hyprforge-clipmenu::chooser`, which is exactly where the
//! `hyprforge-clipboard` dependency already was.
//!
//! # Why `hyprctl` lives here
//!
//! `hyprctl` usage is Hyprland-specific knowledge, but "where is the
//! cursor, and which monitor is it over" is knowledge every layer-shell
//! popup this crate serves needs to answer identically — it is not a
//! clipboard concept, it is a popup-placement concept. Keeping it here
//! (see [`placement`]) means a future picker asks this crate where to
//! open, rather than re-deriving the same `hyprctl monitors -j` /
//! `cursorpos -j` parsing a second time.

pub mod geometry;
pub mod placement;
pub mod popup;
pub mod scroll;
pub mod singleton;

pub use placement::{cursor_position, monitors, place};
pub use popup::{Outcome, Placement, Popup, PopupApp, PopupError, FOCUS_RELEASE_TIMEOUT};
pub use scroll::scroll_rows;
/// Re-exported so a `PopupApp` implementation never needs its own direct
/// `smithay-client-toolkit` dependency just to name the type
/// [`PopupApp::key`] hands it.
pub use smithay_client_toolkit::seat::keyboard::Keysym;
pub use wayland_client::Connection;
