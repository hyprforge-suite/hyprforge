//! Drawing an app-supplied popup onto a `wlr_layer` surface.
//!
//! Same shape as `hyprforge-lock::surface`, which `hyprforge-clipmenu`'s
//! original `surface.rs` was built by reading closely, and which this
//! module is that file's Wayland/iced plumbing pulled out unchanged: the
//! same `smithay_client_toolkit` setup, the same `iced_tiny_skia::Renderer`
//! painting into a raw BGRA buffer, the same one-frame-at-a-time
//! `UserInterface`, the same pointer and keyboard handling, the same
//! teardown-then-paste ordering. The one structural difference from the
//! lock screen: the lock screen holds the *session* open with
//! `ext-session-lock-v1`; a [`Popup`] only ever needs one layer-shell
//! surface, positioned once, with no per-output fan-out and no PAM
//! underneath it.
//!
//! # The seam: [`PopupApp`]
//!
//! Everything that is specific to *what* a popup shows — a clipboard
//! history's rows, or a future grid of emoji — is behind [`PopupApp`].
//! [`Popup`] itself never names a `Model`, a `Layout`, or any other
//! consumer type; it only calls the trait.
//!
//! # Rendering at the output's actual scale
//!
//! A layer-shell surface is sized in logical pixels, but the buffer
//! behind it is real pixels, and on a fractionally-scaled output (this
//! suite's own reference machine reports `scale: 1.6`) "real pixels"
//! is not an integer multiple of the logical size. This crate asks the
//! compositor for the *exact* scale over `wp_fractional_scale_v1` and
//! renders the buffer at that many physical pixels per logical pixel,
//! then hands the compositor `wp_viewporter`'s `wp_viewport` to say
//! "this buffer is my logical size" — the same pair of protocols GTK
//! and Qt use for the same reason. `smithay-client-toolkit` 0.20 has no
//! support for either, so both are bound and dispatched by hand here
//! (see the `Dispatch<WpFractionalScaleV1, _>` impl below); neither is a
//! new dependency to the workspace — `wayland-protocols` is already in
//! `Cargo.lock`, brought in by `hyprforge-clipboard`.
//!
//! A compositor missing one or both protocols falls back to the
//! integer scale `CompositorHandler::scale_factor_changed` reports
//! (legacy per-output `wl_surface` scale, always a whole number) and
//! `wl_surface.set_buffer_scale`. That path renders at most as crisply
//! as the compositor's own rounding allows — for a `1.6`-scale output
//! with no fractional-scale support that is `2`, a supersample rather
//! than a lie — and is not something this workspace's own compositor
//! ever exercises, so it is unverified beyond compiling and matching
//! the protocol's own contract; see this module's own tests and the
//! doc on [`Popup::draw`] for what *is* verified.
//!
//! Everything downstream of the buffer stays in logical pixels
//! regardless of which path is taken: [`Placement`]'s margins, the
//! layer surface's own `set_size`, and — critically — the pointer
//! coordinates [`PointerEvent::position`] reports. Wayland delivers
//! pointer input in surface-local logical coordinates by contract,
//! never in buffer pixels, so nothing downstream of an event
//! (each popup's `Layout::hit`, `MenuLayout::row_at`,
//! the scrollbar's own thumb) has to know the scale changed at all —
//! changing what the buffer holds cannot change what coordinate space
//! the compositor hands back for a click. That invariant is why this
//! change touches only [`Popup::draw`] and surface setup, not one line
//! of any consumer's hit-testing.
//!
//! [`PopupApp`] bundles three things a consumer might have expected to
//! find split apart — "how many items fit", "which item is at this
//! point", and "draw yourself" — behind *one* trait implemented once per
//! consumer, rather than three independent traits (or a geometry object
//! and a view closure) a caller could mismatch. That is a deliberate
//! choice, not the only one possible, and the reason is the bug this
//! whole popup shape exists to keep from recurring: CLAUDE.md's "the
//! thing drawn, the thing hit-tested, and the number of things that fit
//! must all be the same." A `Layout` built for one theme's font size
//! passed alongside a `Model` windowed for another is exactly the class
//! of drift that produced the original clicking-does-nothing bug and the
//! auto-scroll bug both — two independently-constructed values that
//! *should* agree and quietly didn't. Requiring one `impl PopupApp` per
//! consumer means `rows_that_fit`, `pointer_click`'s hit-test and `view`
//! all read the same `&self` — the same font size, the same window,
//! the same live model — so there is no seam at which two of them can be
//! handed different state by mistake. A finer-grained split (a
//! `Geometry` trait plus a separate `View` trait or closure) was
//! considered and rejected for exactly that reason: it would let a
//! caller construct a `Geometry` for one popup instance and a `View` for
//! another and hand `Popup` both, which is precisely the mismatch this
//! design forecloses.
//!
//! What is *not* bundled in is the domain-specific meaning of an action
//! ("F2 pins the selected row") — that stays inside each consumer's own
//! `PopupApp` implementation (`hyprforge-clipmenu::surface::ClipApp`,
//! reading a keysym and deciding what it means), because a grid picker's
//! own keybinds are its own business and this crate has no reason to
//! know their names.

use smithay_client_toolkit::compositor::{CompositorHandler, CompositorState, SurfaceData};
use smithay_client_toolkit::output::{OutputHandler, OutputState};
use smithay_client_toolkit::registry::{ProvidesRegistryState, RegistryState};
use smithay_client_toolkit::seat::keyboard::{KeyEvent, KeyboardHandler, Modifiers};
use smithay_client_toolkit::seat::pointer::{
    CursorIcon, PointerData, PointerEvent, PointerEventKind, PointerHandler, ThemeSpec, ThemedPointer, BTN_LEFT,
};
use smithay_client_toolkit::seat::{Capability, SeatHandler, SeatState};
use smithay_client_toolkit::shell::wlr_layer::{
    Anchor, KeyboardInteractivity, Layer, LayerShell, LayerShellHandler, LayerSurface, LayerSurfaceConfigure,
};
use smithay_client_toolkit::shell::WaylandSurface;
use smithay_client_toolkit::shm::slot::SlotPool;
use smithay_client_toolkit::shm::{Shm, ShmHandler};
use smithay_client_toolkit::{
    delegate_compositor, delegate_keyboard, delegate_layer, delegate_output, delegate_pointer, delegate_registry,
    delegate_seat, delegate_shm, registry_handlers,
};
use calloop_wayland_source::WaylandSource;
use wayland_client::globals::registry_queue_init;
use wayland_client::protocol::{wl_keyboard, wl_output, wl_pointer, wl_seat, wl_shm, wl_surface};
use wayland_client::{Connection, Dispatch, QueueHandle};
use wayland_protocols::wp::fractional_scale::v1::client::{
    wp_fractional_scale_manager_v1::WpFractionalScaleManagerV1,
    wp_fractional_scale_v1::{self, WpFractionalScaleV1},
};
use wayland_protocols::wp::viewporter::client::{wp_viewport::WpViewport, wp_viewporter::WpViewporter};

use hyprforge_look::Theme;
use iced_runtime::core::{mouse, renderer, Element, Rectangle, Size};
use iced_runtime::user_interface::{Cache, UserInterface};
use iced_tiny_skia::graphics::Viewport;
use std::convert::Infallible;

fn iced_color(c: hyprforge_look::Color) -> iced_runtime::core::Color {
    iced_runtime::core::Color::from_rgba8(c.r, c.g, c.b, c.a as f32 / 255.0)
}

/// How long [`Popup::run`] waits for the compositor to confirm it has
/// processed this popup's own layer-surface teardown before giving up
/// and calling [`PopupApp::finish`] anyway.
///
/// `Connection::roundtrip` itself has no bound — see
/// [`finish_after_teardown`]'s doc for why a roundtrip is what this
/// needs — and CLAUDE.md is explicit that nothing here waits on another
/// process (here, the compositor) without one, so the bound is applied
/// from outside on a helper thread. A healthy compositor answers a
/// `wl_display.sync` within microseconds; this is generous well past
/// that while still being short enough that a popup which, for whatever
/// reason, never gets an answer does not sit resident for anything a
/// person would call "stuck".
pub const FOCUS_RELEASE_TIMEOUT: std::time::Duration = std::time::Duration::from_millis(500);

/// Why the popup stopped.
///
/// [`Outcome::Closed`] and [`Outcome::Disconnected`] are the two ways
/// this crate's own event loop can end a popup with no help from the
/// app at all (an output unplugged, the shell deciding to close it, the
/// Wayland connection dying); [`Outcome::App`] is however the consumer's
/// own [`PopupApp`] decided to end it — "chosen" or "cancelled" for
/// `hyprforge-clipmenu`, whatever a future grid picker calls its own
/// outcomes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Outcome<T> {
    /// The app itself ended the popup — see [`PopupApp::key`] and
    /// [`PopupApp::pointer_click`].
    App(T),
    /// The compositor closed the surface out from under this process
    /// (an output was unplugged, or the shell decided to).
    Closed,
    /// The Wayland connection died.
    Disconnected,
}

/// How a popup holds the keyboard — and, as a direct consequence, what
/// happens when the user clicks somewhere that is not this popup.
///
/// The two are the same question, which is why this is one enum and not
/// two independent settings. A layer surface asking for exclusive
/// keyboard interactivity is telling the compositor "route keys here
/// until I am gone"; one asking for on-demand is telling it "focus me
/// while the user is interacting with me, and take it back when they
/// are not". The second is the only one under which a click elsewhere
/// is something this process can observe at all — it arrives as
/// `wl_keyboard.leave`, because the compositor moved focus to whatever
/// was clicked. There is no event for "a click landed outside your
/// surface": a Wayland client is not told about input it did not
/// receive, and the usual workaround (a full-output surface with a
/// transparent input region) swallows that click instead of letting it
/// through to whatever the user was actually aiming at.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Dismissal {
    /// Holds the keyboard exclusively for as long as the popup is open,
    /// and closes only when it decides to. What a popup you *type into*
    /// needs: `hyprforge-clipmenu` and `hyprforge-emojimenu` both filter
    /// on what is typed, and a keystroke going anywhere else would be a
    /// character missing from the search and, worse, a character
    /// delivered to whatever is behind the popup.
    HoldKeyboard,
    /// Takes the keyboard on demand and closes as soon as the compositor
    /// gives focus away. What a *menu* wants: clicking outside it
    /// dismisses it, the way every other menu on the desktop behaves,
    /// and — the reason this exists — that click reaches whatever it
    /// landed on rather than being eaten.
    CloseOnFocusLoss,
}

/// Where to place the popup: an anchor of top-left plus the margins that
/// put its corner at the already-clamped cursor position. Computed by
/// `crate::placement::place`, so this module never touches `hyprctl`
/// output at all.
pub struct Placement {
    pub output_name: String,
    pub margin_top: i32,
    pub margin_left: i32,
    pub width: u32,
    pub height: u32,
}

/// What a popup shows and how it responds to input — the seam between
/// this crate's Wayland/iced plumbing and a consumer's own model. See
/// this module's own doc for why these three responsibilities are one
/// trait rather than several.
pub trait PopupApp {
    /// However this app's own logic decided to end the popup — see
    /// [`Outcome::App`].
    type Outcome: Copy + PartialEq;

    /// This frame's widget tree, at the theme's font size and the
    /// popup's current logical-pixel `width`. `now` is seconds since the
    /// Unix epoch, for anything time-relative the view draws (a "5m ago"
    /// label, say) — read fresh every frame rather than once at startup,
    /// since a popup can sit open for a while.
    ///
    /// `&mut self` because a consumer may need to populate a cache
    /// lazily while building this frame's tree (`hyprforge-clipmenu`'s
    /// thumbnail cache, for image rows) — the returned [`Element`]
    /// itself only ever borrows from `self` immutably.
    fn view<'a>(&'a mut self, theme: &'a Theme, now: u64, width: f64) -> Element<'a, Infallible, iced_widget::Theme, iced_tiny_skia::Renderer>;

    /// How many items (rows, cells, whatever this app draws) fit in a
    /// popup `height` logical pixels tall at this theme's font size —
    /// the inverse of whatever [`Self::pointer_click`] and
    /// [`Self::pointer_move`] hit-test against. Read once, before the
    /// popup opens, to size the consumer's own visible window; see
    /// CLAUDE.md's rule that the thing drawn, the thing hit-tested, and
    /// the number of things that fit must never be three different
    /// numbers.
    fn rows_that_fit(&self, theme: &Theme, height: f64) -> usize;

    /// The pointer moved to (or entered at) `position`, in the
    /// surface-local logical-pixel space [`PointerEvent::position`]
    /// reports — the same space [`Self::view`] draws into. Returns
    /// whether `position` landed on something clickable, so [`Popup`]
    /// can show a hand cursor and knows whether anything actually
    /// changed (and so needs a redraw) — the exact same boolean answers
    /// both questions, since nothing here means "clickable" without also
    /// meaning "the highlighted item changed".
    fn pointer_move(&mut self, theme: &Theme, position: (f64, f64)) -> bool;

    /// The pointer clicked at `position`, with the popup currently
    /// `width` logical pixels wide (needed for anything positioned from
    /// the popup's own right edge). `Some` ends the popup with that
    /// outcome.
    fn pointer_click(&mut self, theme: &Theme, width: f64, position: (f64, f64)) -> Option<Self::Outcome>;

    /// One wheel/touchpad axis event, already turned into whole rows —
    /// see [`crate::scroll::scroll_rows`].
    fn pointer_scroll(&mut self, rows: i32);

    /// A left-button press landed at `position` — asks whether this is
    /// the start of a scrollbar-thumb drag rather than an ordinary click.
    /// `true` tells [`Popup`] to route every following `Motion` to
    /// [`Self::pointer_drag_move`] instead of [`Self::pointer_move`], and
    /// the eventual `Release` to [`Self::pointer_drag_end`] instead of
    /// [`Self::pointer_click`] — the same "gate the whole mechanism on
    /// one opt-in call" shape [`Self::long_press_duration`] already uses,
    /// for the same reason: an app with no draggable thumb (or no thumb
    /// under this particular press) must see *exactly* the click
    /// behaviour it always has, not a new deferred path it never asked
    /// for. Default `false`, which is what every consumer gets until it
    /// has a scrollbar of its own to drag.
    fn pointer_drag_start(&mut self, _theme: &Theme, _position: (f64, f64)) -> bool {
        false
    }

    /// The pointer moved while dragging the thumb this press started —
    /// called instead of [`Self::pointer_move`] for as long as the drag
    /// lasts. Never called unless [`Self::pointer_drag_start`] returned
    /// `true` for the press that is still down.
    fn pointer_drag_move(&mut self, _theme: &Theme, _position: (f64, f64)) {}

    /// The button that started a thumb drag was released — the drag's
    /// counterpart to an ordinary click landing, except a drag never
    /// itself ends the popup (there is no [`Outcome`] a scrollbar drag
    /// could mean).
    fn pointer_drag_end(&mut self) {}

    /// A key was pressed (or is auto-repeating). `Some` ends the popup
    /// with that outcome.
    ///
    /// `modifiers` is the state as of this press. The compositor sends it
    /// as its own event, ahead of the key it applies to, so [`Popup`]
    /// remembers it and hands it over here — which is what lets an app
    /// tell `Enter` from `Shift+Enter` or recognise `Ctrl+P`. Check it
    /// before `utf8` for anything bound to a chord: `Ctrl+P` arrives with
    /// `utf8` set to the control character `U+0010`, not to `p`.
    fn key(
        &mut self,
        keysym: smithay_client_toolkit::seat::keyboard::Keysym,
        utf8: Option<String>,
        modifiers: Modifiers,
    ) -> Option<Self::Outcome>;

    /// How this popup holds the keyboard, and therefore what a click
    /// somewhere else means. Defaults to [`Dismissal::HoldKeyboard`],
    /// which is what a popup you type into needs.
    fn dismissal(&self) -> Dismissal {
        Dismissal::HoldKeyboard
    }

    /// How long the pointer must stay down before a press becomes a
    /// "long press" rather than an ordinary click — `None` (the default)
    /// disables the whole mechanism, which is what every existing
    /// consumer wants: `hyprforge-clipmenu`'s rows have nothing a
    /// long-press should do, and leaving this at the default means a
    /// click there fires exactly as it always has (immediately on
    /// `Press`, at the pressed position — see [`Popup`]'s own pointer
    /// handling). Only when this returns `Some` does [`Popup`] defer a
    /// click until `Release` at all, which is what makes the two paths
    /// distinguishable in the first place; see [`Self::pointer_long_press`].
    fn long_press_duration(&self) -> Option<std::time::Duration> {
        None
    }

    /// The pointer has been held at `position` for at least
    /// [`Self::long_press_duration`] without releasing. Returns whether
    /// anything changed (and therefore needs a redraw) — an emoji picker
    /// opening a skin-tone strip over a tone-capable cell says `true`;
    /// a press that landed on nothing worth long-pressing says `false`,
    /// which is what lets [`Popup`] still treat the eventual release as
    /// an ordinary click rather than silently swallowing it (see
    /// [`Popup`]'s `pointer_frame` for exactly that distinction). Called
    /// at most once per press, never repeated while the button stays
    /// down.
    fn pointer_long_press(&mut self, _theme: &Theme, _position: (f64, f64)) -> bool {
        false
    }

    /// Whether `outcome` needs [`Self::finish`] called — and therefore
    /// needs this popup's own surface torn down and the compositor's
    /// teardown of it confirmed — before the process using this popup
    /// exits. Most outcomes need nothing further (cancelling just
    /// closes); `hyprforge-clipmenu` says yes only for "chosen", which is
    /// the one outcome that still has a paste left to synthesize.
    fn needs_finish(outcome: Self::Outcome) -> bool;

    /// Runs once [`Self::needs_finish`] said yes, and — critically —
    /// only after this popup's own layer surface is gone and a bounded
    /// roundtrip has proven the compositor has processed that; see
    /// [`finish_after_teardown`]'s own doc for why the proof matters and
    /// not just a flush. By the time this runs, anything synthesized
    /// here structurally cannot be delivered back to this popup, because
    /// this popup no longer has a surface to deliver it to.
    fn finish(&mut self, outcome: Self::Outcome);
}

pub struct Popup<A: PopupApp> {
    registry: RegistryState,
    outputs: OutputState,
    seats: SeatState,
    shm: Shm,
    compositor: CompositorState,
    layer_shell: LayerShell,
    pool: SlotPool,

    /// Bound once, if the compositor advertises it — `None` on a
    /// compositor with no fractional-scale support, which is the
    /// signal [`Self::create_surface`] uses to skip creating a
    /// per-surface [`WpFractionalScaleV1`] at all.
    fractional_scale_manager: Option<WpFractionalScaleManagerV1>,
    /// Bound once, if the compositor advertises it — `None` falls back
    /// to plain `wl_surface.set_buffer_scale` with no viewport at all,
    /// which only ever needs an integer scale.
    viewporter: Option<WpViewporter>,
    /// This popup's own fractional-scale object, created alongside its
    /// surface — see [`Self::create_surface`]. Its `preferred_scale`
    /// event is what keeps [`Self::scale`] exact rather than rounded.
    fractional_scale: Option<WpFractionalScaleV1>,
    /// This popup's own viewport, created alongside its surface. Its
    /// `set_destination` is what lets the buffer be a *different* pixel
    /// size than the surface's own logical size — the mechanism that
    /// makes a non-integer scale renderable at all.
    viewport: Option<WpViewport>,
    /// Physical pixels per logical pixel, kept exact when
    /// `fractional_scale` supplies it and otherwise a whole number from
    /// [`CompositorHandler::scale_factor_changed`]. Starts at `1.0`
    /// (unscaled) for the handful of frames before either can answer —
    /// see this module's own doc for why that gap is not worth blocking
    /// the first frame over.
    scale: f64,

    layer: Option<LayerSurface>,
    keyboard: Option<wl_keyboard::WlKeyboard>,
    /// A themed pointer rather than a bare `WlPointer`: this is what
    /// gives a clickable item a hand cursor. `ThemedPointer` uses
    /// `wp_cursor_shape_v1` under the hood when the compositor advertises
    /// it and falls back to a themed software cursor drawn from the
    /// system's own cursor theme when it does not — see `new_capability`
    /// for where it is created and why nothing here has to probe for the
    /// protocol itself.
    pointer: Option<ThemedPointer<PointerData, SurfaceData>>,
    /// Cloned from the connection [`Popup::run`] was handed, so
    /// `update_cursor` can call `ThemedPointer::set_cursor` (which needs
    /// a `&Connection`) from inside pointer event handling without
    /// threading one through every call site. Cheap — `Connection`'s own
    /// `clone` is the same handle to the same socket, not a new one.
    connection: Connection,

    placement: Placement,
    app: A,
    theme: Theme,

    outcome: Option<Outcome<A::Outcome>>,
    dirty: bool,
    /// Configured size, `(0, 0)` until the first `configure`. The
    /// compositor is free to send back something other than what was
    /// requested; nothing is drawn before it says how big the surface
    /// actually is, for the same reason the lock screen waits.
    size: (u32, u32),
    renderer: iced_tiny_skia::Renderer,
    cache: Cache,

    /// The position and start time of a left-button press this popup has
    /// not yet resolved into a click, kept only for an app that opted
    /// into long-press detection at all (see [`PopupApp::long_press_duration`]).
    /// An app that never opts in never has this set, and a click there
    /// still fires the instant `Press` arrives — see `pointer_frame`'s own
    /// comment for why this is gated on `long_press_duration` rather than
    /// deferring every click unconditionally.
    pending_press: Option<((f64, f64), std::time::Instant)>,
    /// Whether [`Self::pending_press`]'s elapsed time has already been
    /// checked against [`PopupApp::long_press_duration`] — set the first
    /// time it crosses the threshold so [`PopupApp::pointer_long_press`]
    /// is called exactly once per press, not on every tick the button
    /// stays down.
    long_press_checked: bool,
    /// Whether [`PopupApp::pointer_long_press`] actually changed
    /// something for the current press — `Release` uses this, not
    /// `long_press_checked`, to decide whether the eventual release is
    /// still an ordinary click (see `pointer_frame`).
    long_press_fired: bool,
    /// Whether the button currently held down started a scrollbar-thumb
    /// drag — set by [`PopupApp::pointer_drag_start`] answering `true` on
    /// `Press`, cleared on `Release`. While this is `true`, `Motion`
    /// routes to [`PopupApp::pointer_drag_move`] instead of
    /// [`PopupApp::pointer_move`], and the eventual `Release` calls
    /// [`PopupApp::pointer_drag_end`] instead of resolving a click —
    /// see [`PopupApp::pointer_drag_start`]'s own doc for why this is
    /// gated the same way long-press detection is.
    dragging: bool,
    /// The modifiers as the compositor last reported them.
    /// `wl_keyboard.modifiers` is its own event, sent ahead of the key it
    /// applies to, so it is remembered here and handed to
    /// [`PopupApp::key`] with each press.
    modifiers: Modifiers,
}

impl<A: PopupApp + 'static> Popup<A> {
    /// Shows the popup and runs until the app ends it or it is
    /// dismissed.
    ///
    /// Takes the connection rather than opening one so a caller can
    /// point it at a nested compositor for testing — see
    /// `crates/hyprforge-lock/testing/nested.sh`; a popup like this takes
    /// exclusive keyboard focus, which is exactly the kind of thing
    /// CLAUDE.md says never to point at the session you're using.
    pub fn run(connection: Connection, placement: Placement, app: A, theme: Theme) -> Result<Outcome<A::Outcome>, PopupError> {
        let (globals, mut queue) = registry_queue_init(&connection)?;
        let qh = queue.handle();

        let shm = Shm::bind(&globals, &qh)?;
        let pool = SlotPool::new(4096, &shm).map_err(|e| PopupError::Buffer(e.to_string()))?;
        let layer_shell = LayerShell::bind(&globals, &qh)?;
        // Both optional: a compositor that lacks either just gets the
        // integer-scale fallback this module's own doc describes.
        // `bind`'s only failure mode here is the global being absent, so
        // nothing beyond `.ok()` is worth reporting.
        let viewporter = globals.bind::<WpViewporter, Self, ()>(&qh, 1..=1, ()).ok();
        let fractional_scale_manager =
            globals.bind::<WpFractionalScaleManagerV1, Self, ()>(&qh, 1..=1, ()).ok();

        let mut popup = Popup {
            registry: RegistryState::new(&globals),
            outputs: OutputState::new(&globals, &qh),
            seats: SeatState::new(&globals, &qh),
            compositor: CompositorState::bind(&globals, &qh)?,
            shm,
            pool,
            layer_shell,
            fractional_scale_manager,
            viewporter,
            fractional_scale: None,
            viewport: None,
            scale: 1.0,
            layer: None,
            keyboard: None,
            pointer: None,
            connection: connection.clone(),
            placement,
            app,
            theme: theme.clone(),
            outcome: None,
            dirty: true,
            size: (0, 0),
            renderer: iced_tiny_skia::Renderer::new(
                iced_runtime::core::Font::DEFAULT,
                iced_runtime::core::Pixels(theme.font_size),
            ),
            cache: Cache::default(),
            pending_press: None,
            long_press_checked: false,
            long_press_fired: false,
            dragging: false,
            modifiers: Modifiers::default(),
        };

        // Outputs have to be known before a surface can be pinned to
        // one by name — the same ordering reason the lock screen
        // roundtrips before locking.
        queue.roundtrip(&mut popup).map_err(|_| PopupError::Disconnected)?;
        popup.create_surface(&qh);

        let mut event_loop: calloop::EventLoop<Popup<A>> =
            calloop::EventLoop::try_new().map_err(|e| PopupError::EventLoop(e.to_string()))?;
        let handle = event_loop.handle();
        WaylandSource::new(connection.clone(), queue)
            .insert(handle)
            .map_err(|e| PopupError::EventLoop(e.to_string()))?;

        while popup.outcome.is_none() {
            // No app this crate serves today opts into long-press
            // detection (`PopupApp::long_press_duration` defaults to
            // `None`), so `dispatch(None, ..)` — block until the next
            // Wayland event, burning no CPU while idle — is still what
            // every existing popup gets. Only while a press is actually
            // pending *and* the app wants long-press at all does this
            // wake up on its own, so elapsed time can be checked below
            // even with no new pointer event to trigger it.
            let timeout = popup
                .pending_press
                .is_some()
                .then(|| popup.app.long_press_duration())
                .flatten()
                .map(|_| std::time::Duration::from_millis(16));
            event_loop.dispatch(timeout, &mut popup).map_err(|_| PopupError::Disconnected)?;

            if let Some((position, started)) = popup.pending_press {
                if !popup.long_press_checked {
                    if let Some(duration) = popup.app.long_press_duration() {
                        if started.elapsed() >= duration {
                            popup.long_press_checked = true;
                            if popup.app.pointer_long_press(&popup.theme, position) {
                                popup.long_press_fired = true;
                                popup.mark_dirty();
                            }
                        }
                    }
                }
            }

            if popup.dirty {
                popup.draw();
            }
        }

        // Only an app outcome that says so gets `PopupApp::finish` at
        // all, and only after the teardown-then-roundtrip below —
        // see `finish_after_teardown`'s own doc for why the ordering is
        // the fix, not an optimisation.
        if let Some(Outcome::App(outcome)) = popup.outcome {
            if A::needs_finish(outcome) {
                finish_after_teardown(&connection, &mut popup.layer, FOCUS_RELEASE_TIMEOUT);
                popup.app.finish(outcome);
            }
        }

        Ok(popup.outcome.unwrap_or(Outcome::Disconnected))
    }

    /// Finds the output the popup was placed on and creates the one
    /// surface it needs.
    ///
    /// A named output that has disappeared between `hyprctl monitors -j`
    /// being read and this running (unplugged in the gap) has nowhere to
    /// attach to; falling back to *no* output rather than picking a
    /// different one, since a popup that silently landed on the wrong
    /// monitor at the wrong position is worse than one that visibly
    /// failed to appear at all.
    fn create_surface(&mut self, qh: &QueueHandle<Self>) {
        let target = self
            .outputs
            .outputs()
            .find(|o| self.outputs.info(o).and_then(|i| i.name) == Some(self.placement.output_name.clone()));
        let Some(output) = target else {
            eprintln!("output {:?} is no longer present — not showing the popup", self.placement.output_name);
            self.outcome = Some(Outcome::Closed);
            return;
        };

        let surface = self.compositor.create_surface(qh);
        // Both created from `&surface` before it is moved into the layer
        // surface below — a `wp_viewport`/`wp_fractional_scale_v1`
        // attaches to the `wl_surface` itself, not to whatever role gets
        // assigned to it afterward, so the order here does not matter to
        // the protocol; it matters to the borrow checker.
        self.viewport = self.viewporter.as_ref().map(|v| v.get_viewport(&surface, qh, ()));
        self.fractional_scale =
            self.fractional_scale_manager.as_ref().map(|m| m.get_fractional_scale(&surface, qh, ()));
        let layer =
            self.layer_shell.create_layer_surface(qh, surface, Layer::Overlay, Some("hyprforge-popup"), Some(&output));
        layer.set_anchor(Anchor::TOP | Anchor::LEFT);
        // `-1`, not the default `0`. A layer surface whose exclusive zone
        // is `0` is positioned inside whatever space is *left over* after
        // every exclusive-zone surface has taken its cut, so a margin is
        // measured from the bar's bottom edge rather than from the
        // output's. Every placement this crate computes is in full-output
        // logical coordinates — a cursor position from `hyprctl`, or a
        // monitor's own `reserved` top plus the user's offset — so with
        // the default, the bar's height got counted twice: measured live,
        // a menu that should have opened at y=50 (a bar occupying 20..50)
        // opened at y=100. `-1` says "do not move me out of anyone's
        // way", which makes a margin mean the same thing the placement
        // code already meant by it.
        layer.set_exclusive_zone(-1);
        layer.set_margin(self.placement.margin_top, 0, 0, self.placement.margin_left);
        layer.set_size(self.placement.width, self.placement.height);
        // Exclusive for a popup you type into — typing has to reach this
        // popup, not whatever had focus before it opened, the same
        // reasoning `KeyboardInteractivity`'s own doc gives for a lock
        // screen or a password prompt. On-demand for a menu, which wants
        // the opposite: see [`Dismissal`] for why those are one choice
        // rather than two.
        layer.set_keyboard_interactivity(match self.app.dismissal() {
            Dismissal::HoldKeyboard => KeyboardInteractivity::Exclusive,
            Dismissal::CloseOnFocusLoss => KeyboardInteractivity::OnDemand,
        });
        layer.wl_surface().commit();
        self.layer = Some(layer);
    }

    /// Renders this frame and hands the compositor a buffer sized for
    /// the output's actual scale — see this module's own doc for the
    /// mechanism. `width`/`height` (from [`Self::size`], set by
    /// [`LayerShellHandler::configure`]) stay logical throughout: they
    /// are what the widget tree is built and hit-tested against, and
    /// what [`Self::viewport`] tells the compositor this surface's own
    /// size is. Only the buffer itself — its pixel dimensions, and the
    /// `Viewport` handed to `iced_tiny_skia` — is scaled up from them.
    fn draw(&mut self) {
        self.dirty = false;
        let (width, height) = self.size;
        if width == 0 || height == 0 {
            return;
        }
        let Some(layer) = &self.layer else { return };

        // A scale of `0` or less cannot come from either source that
        // sets `self.scale` (a `preferred_scale` of `0` is nonsensical
        // and `scale_factor_changed`'s `factor` is always positive per
        // its own protocol), but guarding it here rather than trusting
        // that costs nothing and turns "divide by zero" into "render at
        // 1x" if it ever were violated.
        let scale = if self.scale > 0.0 { self.scale } else { 1.0 };
        let buffer_width = ((width as f64) * scale).round().max(1.0) as u32;
        let buffer_height = ((height as f64) * scale).round().max(1.0) as u32;

        let Ok((buffer, canvas)) = self.pool.create_buffer(
            buffer_width as i32,
            buffer_height as i32,
            buffer_width as i32 * 4,
            wl_shm::Format::Argb8888,
        ) else {
            // Out of memory for a buffer. Leave the previous frame up —
            // same reasoning as the lock screen: the alternative is a
            // blank surface, which is worse than a stale one.
            return;
        };

        // See `hyprforge-lock::surface`'s identical comment and its
        // pinned test: `iced_tiny_skia` writes B, G, R, A on purpose,
        // which is exactly what `Argb8888` wants byte-for-byte. No
        // swizzle here either — scaling the buffer up changes nothing
        // about the channel order it is filled in.
        let Some(mut pixels) = tiny_skia::PixmapMut::from_bytes(canvas, buffer_width, buffer_height) else {
            return;
        };
        let Some(mut mask) = tiny_skia::Mask::new(buffer_width, buffer_height) else {
            return;
        };

        // Seconds since the epoch — computed fresh every frame rather
        // than once at startup, since a popup can sit open for a while
        // and "now" read once would make its own time-relative labels
        // visibly stop aging. A clock that cannot be read at all (no
        // realistic way on this machine, but `SystemTime::now`
        // returning before the epoch is possible in principle) falls
        // back to `0`.
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0);

        // Logical: what the widget tree is built and clipped against.
        // `Viewport::with_physical_size` below derives its own logical
        // size back out of `(buffer_width, buffer_height, scale)` — see
        // this function's own doc for why those two must describe the
        // same rectangle.
        let size = Size::new(width as f32, height as f32);
        let mut ui = UserInterface::<Infallible, iced_widget::Theme, iced_tiny_skia::Renderer>::build(
            self.app.view(&self.theme, now, width as f64),
            size,
            std::mem::take(&mut self.cache),
            &mut self.renderer,
        );
        ui.draw(
            &mut self.renderer,
            &iced_widget::Theme::Dark,
            &renderer::Style { text_color: iced_color(self.theme.surfaces.text) },
            mouse::Cursor::Unavailable,
        );
        self.cache = ui.into_cache();

        // Cleared outright rather than left to the background colour
        // below to cover it: this buffer comes from a pool and can still
        // hold the previous frame, and a *transparent* fill blended over
        // stale pixels changes nothing at all.
        pixels.fill(tiny_skia::Color::TRANSPARENT);

        self.renderer.draw(
            &mut pixels,
            &mut mask,
            // Physical buffer size plus the exact scale that produced
            // it — `iced_tiny_skia` multiplies every logical coordinate
            // in the widget tree by this scale before it reaches a
            // pixel, which is the entire mechanism: the tree above was
            // built and clipped in logical units (`size`), and this is
            // what turns that into `buffer_width`x`buffer_height`
            // physical ones.
            &Viewport::with_physical_size(Size::new(buffer_width, buffer_height), scale as f32),
            &[Rectangle::with_size(size)],
            // Transparent, not a background colour: the app's own root
            // container paints that itself, rounded to the theme's
            // corner radius — and if the surface underneath it were
            // cleared to the same opaque colour, the rounding would be
            // root-over-root and invisible. The corners are only corners
            // because what is outside them is nothing. `Argb8888` (see
            // the buffer above) is what makes that expressible.
            iced_runtime::core::Color::TRANSPARENT,
        );

        let wl_surface = layer.wl_surface();
        if let Some(viewport) = &self.viewport {
            // Tells the compositor this surface is still `width`x`height`
            // logical pixels regardless of the buffer's own (larger,
            // scaled-up) pixel size — the half of fractional scaling
            // that is not "render more pixels", and the reason a
            // consumer's hit-testing never has to hear about any of
            // this: the surface's logical size, and therefore every
            // `PointerEvent::position` the compositor reports against
            // it, is unchanged by what scale this renders at.
            viewport.set_destination(width as i32, height as i32);
        } else if self.fractional_scale.is_none() {
            // No viewport at all: the only way to tell a compositor a
            // buffer is scaled is the legacy integer `wl_surface`
            // scale, which is exactly what produced `self.scale` on
            // this path (see `scale_factor_changed`) — so it is already
            // the right number to hand back.
            wl_surface.set_buffer_scale(scale.round().max(1.0) as i32);
        }
        // Buffer-local coordinates (the spec's own distinction from
        // plain `damage`, which is surface-local): this buffer is
        // `buffer_width`x`buffer_height` pixels regardless of what
        // logical size the viewport above declares it maps to.
        wl_surface.damage_buffer(0, 0, buffer_width as i32, buffer_height as i32);
        if buffer.attach_to(wl_surface).is_ok() {
            wl_surface.commit();
        }
    }

    fn mark_dirty(&mut self) {
        self.dirty = true;
    }

    fn key(&mut self, keysym: smithay_client_toolkit::seat::keyboard::Keysym, utf8: Option<String>) {
        if let Some(outcome) = self.app.key(keysym, utf8, self.modifiers) {
            self.outcome = Some(Outcome::App(outcome));
        }
        self.mark_dirty();
    }

    /// The pointer moved (or entered) at `position` — forwarded straight
    /// to [`PopupApp::pointer_move`], whose return value is both "was
    /// this clickable" (for the cursor icon) and "did anything change"
    /// (for whether to redraw): nothing here means the first without
    /// meaning the second too.
    fn pointer_move(&mut self, position: (f64, f64)) {
        let hit = self.app.pointer_move(&self.theme, position);
        if hit {
            self.mark_dirty();
        }
        self.update_cursor(hit);
    }

    /// Sets the pointer to a hand cursor whenever it is over something
    /// clickable, and back to the default arrow otherwise.
    ///
    /// Errors from [`ThemedPointer::set_cursor`] (most commonly a missing
    /// enter serial, in the gap before this popup's first pointer event
    /// has actually arrived) are not worth reporting — a wrong cursor for
    /// one frame is not a failure the user needs told about, and nothing
    /// here depends on the request having landed.
    fn update_cursor(&self, over_something: bool) {
        let Some(pointer) = &self.pointer else { return };
        let icon = if over_something { CursorIcon::Pointer } else { CursorIcon::Default };
        let _ = pointer.set_cursor(&self.connection, icon);
    }

    fn pointer_click(&mut self, position: (f64, f64)) {
        if let Some(outcome) = self.app.pointer_click(&self.theme, self.size.0 as f64, position) {
            self.outcome = Some(Outcome::App(outcome));
        }
        self.mark_dirty();
    }

    fn pointer_scroll(&mut self, rows: i32) {
        if rows == 0 {
            return;
        }
        self.app.pointer_scroll(rows);
        self.mark_dirty();
    }
}

/// Tears down this popup's own layer surface, proves — with a bound —
/// that the compositor has actually processed that, and returns so the
/// caller can then run whatever needed the surface gone (synthesizing a
/// paste, say).
///
/// # Why the proof, not just a flush
///
/// This popup's layer is `KeyboardInteractivity::Exclusive` for as long
/// as it is on screen, precisely so its own keystrokes reach it rather
/// than whatever had focus before it opened. That is exactly backwards
/// for anything a [`PopupApp::finish`] synthesizes as input: sending it
/// while this popup still holds that focus delivers it back to the
/// popup, not to whatever the user meant to reach — which was the whole
/// bug this function exists to close, for `hyprforge-clipmenu`'s
/// synthesized paste. A `Connection::flush` only proves the destroy
/// request *left this process*; it says nothing about whether the
/// compositor has acted on it and handed focus to whatever is next.
///
/// # Why a roundtrip proves it
///
/// Wayland serializes a single connection's requests in the order the
/// server receives them, and answers them in that same order. A
/// `wl_display.sync` sent *after* the surface's destroy requests
/// (queued when `layer` is dropped below) cannot be answered until the
/// destroy has already been processed — so `Connection::roundtrip`
/// returning is the proof this needs, deterministically, with no sleep
/// and no guessing at a delay. `roundtrip` itself has no bound, though,
/// so `timeout` is applied from outside on a helper thread: a
/// compositor that cannot answer a sync within it is not something a
/// popup should stay resident waiting on. Proceeding anyway, with a
/// warning, is the lesser failure.
fn finish_after_teardown(connection: &Connection, layer: &mut Option<LayerSurface>, timeout: std::time::Duration) {
    // Dropping the `LayerSurface` queues its role object's destroy
    // request (see `smithay_client_toolkit`'s `Drop for
    // LayerSurfaceInner`) — nothing is sent to the compositor yet,
    // only queued locally, which is exactly why a flush alone would
    // not be proof of anything.
    *layer = None;

    let (tx, rx) = std::sync::mpsc::channel();
    let roundtrip_connection = connection.clone();
    std::thread::spawn(move || {
        // The result is only a signal that the sync happened at all;
        // a `roundtrip` failure (a dead connection) leaves nothing
        // more useful to do than proceed anyway, so it is not
        // inspected.
        let _ = tx.send(roundtrip_connection.roundtrip());
    });
    if rx.recv_timeout(timeout).is_err() {
        eprintln!(
            "the compositor didn't confirm this popup's own surface was torn down within \
             {timeout:?} — proceeding anyway"
        );
    }
}

#[derive(Debug, thiserror::Error)]
pub enum PopupError {
    #[error("couldn't talk to the compositor: {0}")]
    Connect(#[from] wayland_client::globals::GlobalError),
    #[error("this compositor is missing something the popup needs: {0}")]
    Missing(#[from] smithay_client_toolkit::reexports::client::globals::BindError),
    #[error("couldn't set up a drawing buffer: {0}")]
    Buffer(String),
    #[error("couldn't set up the event loop: {0}")]
    EventLoop(String),
    #[error("the connection to the compositor was lost")]
    Disconnected,
}

impl<A: PopupApp + 'static> LayerShellHandler for Popup<A> {
    fn closed(&mut self, _conn: &Connection, _qh: &QueueHandle<Self>, _layer: &LayerSurface) {
        if self.outcome.is_none() {
            self.outcome = Some(Outcome::Closed);
        }
    }

    fn configure(
        &mut self,
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
        _layer: &LayerSurface,
        configure: LayerSurfaceConfigure,
        _serial: u32,
    ) {
        let (mut width, mut height) = configure.new_size;
        // A `0` in either axis means "you choose" (see the field's own
        // doc) — the requested size is what this popup already asked
        // for, so that is what fills in.
        if width == 0 {
            width = self.placement.width;
        }
        if height == 0 {
            height = self.placement.height;
        }
        self.size = (width, height);
        self.mark_dirty();
    }
}

impl<A: PopupApp + 'static> KeyboardHandler for Popup<A> {
    fn enter(
        &mut self,
        _: &Connection,
        _: &QueueHandle<Self>,
        _: &wl_keyboard::WlKeyboard,
        _: &wl_surface::WlSurface,
        _: u32,
        _: &[u32],
        _: &[smithay_client_toolkit::seat::keyboard::Keysym],
    ) {
    }

    /// The compositor took the keyboard away, which for an on-demand
    /// popup is the only notice it gets that the user clicked something
    /// else. A popup that holds the keyboard ignores this: it can still
    /// receive a `leave` for reasons that are not a dismissal (an output
    /// going away, a session switch), and closing on those would make it
    /// vanish mid-use.
    fn leave(&mut self, _: &Connection, _: &QueueHandle<Self>, _: &wl_keyboard::WlKeyboard, _: &wl_surface::WlSurface, _: u32) {
        if self.app.dismissal() == Dismissal::CloseOnFocusLoss && self.outcome.is_none() {
            self.outcome = Some(Outcome::Closed);
        }
    }

    fn press_key(&mut self, _: &Connection, _: &QueueHandle<Self>, _: &wl_keyboard::WlKeyboard, _: u32, event: KeyEvent) {
        self.key(event.keysym, event.utf8);
    }

    fn release_key(&mut self, _: &Connection, _: &QueueHandle<Self>, _: &wl_keyboard::WlKeyboard, _: u32, _: KeyEvent) {}

    fn update_modifiers(
        &mut self,
        _: &Connection,
        _: &QueueHandle<Self>,
        _: &wl_keyboard::WlKeyboard,
        _: u32,
        modifiers: Modifiers,
        _: smithay_client_toolkit::seat::keyboard::RawModifiers,
        _: u32,
    ) {
        self.modifiers = modifiers;
    }

    fn repeat_key(
        &mut self,
        conn: &Connection,
        qh: &QueueHandle<Self>,
        keyboard: &wl_keyboard::WlKeyboard,
        serial: u32,
        event: KeyEvent,
    ) {
        self.press_key(conn, qh, keyboard, serial, event);
    }
}

impl<A: PopupApp + 'static> SeatHandler for Popup<A> {
    fn seat_state(&mut self) -> &mut SeatState {
        &mut self.seats
    }

    fn new_seat(&mut self, _: &Connection, _: &QueueHandle<Self>, _: wl_seat::WlSeat) {}

    fn new_capability(&mut self, _: &Connection, qh: &QueueHandle<Self>, seat: wl_seat::WlSeat, capability: Capability) {
        if capability == Capability::Keyboard && self.keyboard.is_none() {
            self.keyboard = self.seats.get_keyboard(qh, &seat, None).ok();
        }
        if capability == Capability::Pointer && self.pointer.is_none() {
            // `get_pointer_with_theme` is what makes this a hand cursor
            // over something clickable rather than a plain arrow
            // everywhere: it uses `wp_cursor_shape_v1` when the
            // compositor advertises it and falls back to a themed
            // software cursor (drawn from the system's own cursor
            // theme, via `shm`, into its own small surface here) when
            // the protocol is absent — this crate never has to check
            // for it itself, and a compositor with neither just makes
            // `ThemedPointer::set_cursor` return an error `update_cursor`
            // already ignores, so the popup still opens and still works
            // with the plain system cursor.
            let cursor_surface = self.compositor.create_surface(qh);
            match self.seats.get_pointer_with_theme(qh, &seat, self.shm.wl_shm(), cursor_surface, ThemeSpec::default()) {
                Ok(pointer) => self.pointer = Some(pointer),
                Err(e) => eprintln!(
                    "couldn't set up a themed pointer cursor ({e}) — \
                     clicking still works, just without a hand cursor"
                ),
            }
        }
    }

    fn remove_capability(&mut self, _: &Connection, _: &QueueHandle<Self>, _: wl_seat::WlSeat, capability: Capability) {
        if capability == Capability::Keyboard {
            if let Some(keyboard) = self.keyboard.take() {
                keyboard.release();
            }
        }
        if capability == Capability::Pointer {
            // `ThemedPointer`'s own `Drop` releases the underlying
            // `wl_pointer` and destroys the cursor surface (and any
            // cursor-shape device) — nothing left to release by hand.
            self.pointer = None;
        }
    }

    fn remove_seat(&mut self, _: &Connection, _: &QueueHandle<Self>, _: wl_seat::WlSeat) {}
}

impl<A: PopupApp + 'static> PointerHandler for Popup<A> {
    fn pointer_frame(&mut self, _conn: &Connection, _qh: &QueueHandle<Self>, _pointer: &wl_pointer::WlPointer, events: &[PointerEvent]) {
        // A frame can bundle several events (see the trait's own doc
        // comment) — a drag that leaves mid-click sends Release and
        // Leave together, for instance — so every one of them is
        // applied, not just the last.
        for event in events {
            match &event.kind {
                PointerEventKind::Enter { .. } => {
                    self.pointer_move(event.position);
                }
                PointerEventKind::Motion { .. } => {
                    if self.dragging {
                        self.app.pointer_drag_move(&self.theme, event.position);
                        self.mark_dirty();
                    } else {
                        self.pointer_move(event.position);
                    }
                }
                PointerEventKind::Leave { .. } => {
                    // Nothing to un-highlight: the selection is the same
                    // state the keyboard drives, and losing the pointer
                    // is not a reason to lose the keyboard's selection
                    // too.
                }
                PointerEventKind::Press { button, .. } => {
                    if *button == BTN_LEFT {
                        // A thumb drag is checked for first and gates
                        // everything else the same way long-press
                        // detection does (see `PopupApp::pointer_drag_start`'s
                        // own doc): an app with no draggable thumb under
                        // this press always answers `false`, so its click
                        // still fires exactly as before.
                        if self.app.pointer_drag_start(&self.theme, event.position) {
                            self.dragging = true;
                            self.mark_dirty();
                        } else if self.app.long_press_duration().is_some() {
                            // Deferring the click to `Release`
                            // unconditionally would change every existing
                            // popup's timing (click on mouse-up instead of
                            // mouse-down) for no benefit to it —
                            // long-press detection is the only reason to
                            // wait at all, so this only defers when the
                            // app actually opted in (see
                            // `PopupApp::long_press_duration`'s own doc).
                            // `hyprforge-clipmenu` never does, so its
                            // click still fires right here, exactly as
                            // before.
                            self.pending_press = Some((event.position, std::time::Instant::now()));
                            self.long_press_checked = false;
                            self.long_press_fired = false;
                        } else {
                            self.pointer_click(event.position);
                        }
                    }
                }
                PointerEventKind::Release { button, .. } => {
                    if *button == BTN_LEFT {
                        if self.dragging {
                            self.dragging = false;
                            self.app.pointer_drag_end();
                            self.mark_dirty();
                        } else if let Some((press_position, _)) = self.pending_press.take() {
                            if self.long_press_fired {
                                // The long press already changed
                                // something (opened a tone strip, say);
                                // the release that ends the hold is not
                                // also a click on whatever was underneath
                                // it — that would both open the overlay
                                // *and* choose the cell it opened on.
                            } else {
                                // Either long-press detection is off, or
                                // the button came up before the threshold,
                                // or it stayed down past the threshold
                                // over nothing worth long-pressing — all
                                // three are an ordinary click, at the
                                // position the press itself landed on
                                // (not wherever the pointer drifted to
                                // before releasing).
                                self.pointer_click(press_position);
                            }
                        }
                        self.long_press_checked = false;
                        self.long_press_fired = false;
                    }
                }
                PointerEventKind::Axis { vertical, .. } => {
                    self.pointer_scroll(crate::scroll::scroll_rows(vertical.discrete, vertical.value120, vertical.absolute));
                }
            }
        }
    }
}

impl<A: PopupApp + 'static> CompositorHandler for Popup<A> {
    /// The legacy per-output integer scale — SCTK surfaces this whenever
    /// the compositor reports it (via `wl_surface.preferred_buffer_scale`
    /// on newer compositors, or computed from the outputs this surface
    /// has entered on older ones), regardless of whether
    /// `wp_fractional_scale_v1` also exists.
    ///
    /// Only acted on when `fractional_scale` is absent: that protocol's
    /// own `preferred_scale` is the more precise number for the same
    /// question (exact rather than rounded to a whole number), and a
    /// compositor sending both is not a reason to let whichever fires
    /// last win — this is the fallback for a compositor that sends only
    /// this one.
    fn scale_factor_changed(&mut self, _: &Connection, _: &QueueHandle<Self>, _: &wl_surface::WlSurface, factor: i32) {
        if self.fractional_scale.is_none() {
            let factor = (factor.max(1)) as f64;
            if self.scale != factor {
                self.scale = factor;
                self.mark_dirty();
            }
        }
    }

    fn transform_changed(&mut self, _: &Connection, _: &QueueHandle<Self>, _: &wl_surface::WlSurface, _: wl_output::Transform) {}

    fn frame(&mut self, _: &Connection, _: &QueueHandle<Self>, _: &wl_surface::WlSurface, _: u32) {
        self.mark_dirty();
    }

    fn surface_enter(&mut self, _: &Connection, _: &QueueHandle<Self>, _: &wl_surface::WlSurface, _: &wl_output::WlOutput) {}

    fn surface_leave(&mut self, _: &Connection, _: &QueueHandle<Self>, _: &wl_surface::WlSurface, _: &wl_output::WlOutput) {}
}

impl<A: PopupApp + 'static> OutputHandler for Popup<A> {
    fn output_state(&mut self) -> &mut OutputState {
        &mut self.outputs
    }

    fn new_output(&mut self, _: &Connection, _: &QueueHandle<Self>, _: wl_output::WlOutput) {}

    fn update_output(&mut self, _: &Connection, _: &QueueHandle<Self>, _: wl_output::WlOutput) {}

    fn output_destroyed(&mut self, _: &Connection, _: &QueueHandle<Self>, _: wl_output::WlOutput) {}
}

impl<A: PopupApp + 'static> ShmHandler for Popup<A> {
    fn shm_state(&mut self) -> &mut Shm {
        &mut self.shm
    }
}

impl<A: PopupApp + 'static> ProvidesRegistryState for Popup<A> {
    fn registry(&mut self) -> &mut RegistryState {
        &mut self.registry
    }

    registry_handlers![OutputState, SeatState];
}

/// The one event this protocol has — see this module's own doc for why
/// this crate hand-writes it rather than pulling in a helper crate:
/// `smithay-client-toolkit` 0.20 has no delegate for it.
impl<A: PopupApp + 'static> Dispatch<WpFractionalScaleV1, ()> for Popup<A> {
    fn event(
        state: &mut Self,
        _proxy: &WpFractionalScaleV1,
        event: wp_fractional_scale_v1::Event,
        _data: &(),
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
    ) {
        // "the numerator of a fraction with a denominator of 120" — the
        // protocol's own wording, and the reason this is exact where
        // `scale_factor_changed`'s integer `factor` is not: `1.6` arrives
        // as `192`, not rounded to `2`.
        if let wp_fractional_scale_v1::Event::PreferredScale { scale } = event {
            let scale = scale as f64 / 120.0;
            if scale > 0.0 && state.scale != scale {
                state.scale = scale;
                state.mark_dirty();
            }
        }
    }
}

// Neither of these has any event of its own — `delegate_noop!`'s `ignore`
// form is exactly the "there is nothing here to react to" this crate's
// own `SeatHandler`/`OutputHandler` no-op bodies already express for
// events SCTK itself delivers.
wayland_client::delegate_noop!(@<A: PopupApp + 'static> Popup<A>: ignore WpViewporter);
wayland_client::delegate_noop!(@<A: PopupApp + 'static> Popup<A>: ignore WpViewport);
wayland_client::delegate_noop!(@<A: PopupApp + 'static> Popup<A>: ignore WpFractionalScaleManagerV1);

delegate_compositor!(@<A: PopupApp + 'static> Popup<A>);
delegate_output!(@<A: PopupApp + 'static> Popup<A>);
delegate_seat!(@<A: PopupApp + 'static> Popup<A>);
delegate_keyboard!(@<A: PopupApp + 'static> Popup<A>);
delegate_pointer!(@<A: PopupApp + 'static> Popup<A>);
delegate_shm!(@<A: PopupApp + 'static> Popup<A>);
delegate_layer!(@<A: PopupApp + 'static> Popup<A>);
delegate_registry!(@<A: PopupApp + 'static> Popup<A>);
