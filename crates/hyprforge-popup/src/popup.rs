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
//! [`Popup`] itself never names a `Model`, a `RowLayout`, or any other
//! consumer type; it only calls the trait.
//!
//! [`PopupApp`] bundles three things a consumer might have expected to
//! find split apart — "how many items fit", "which item is at this
//! point", and "draw yourself" — behind *one* trait implemented once per
//! consumer, rather than three independent traits (or a geometry object
//! and a view closure) a caller could mismatch. That is a deliberate
//! choice, not the only one possible, and the reason is the bug this
//! whole popup shape exists to keep from recurring: CLAUDE.md's "the
//! thing drawn, the thing hit-tested, and the number of things that fit
//! must all be the same." A `RowLayout` used for one theme's font size
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
use wayland_client::{Connection, QueueHandle};

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

    /// A key was pressed (or is auto-repeating). `Some` ends the popup
    /// with that outcome.
    fn key(&mut self, keysym: smithay_client_toolkit::seat::keyboard::Keysym, utf8: Option<String>) -> Option<Self::Outcome>;

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

        let mut popup = Popup {
            registry: RegistryState::new(&globals),
            outputs: OutputState::new(&globals, &qh),
            seats: SeatState::new(&globals, &qh),
            compositor: CompositorState::bind(&globals, &qh)?,
            shm,
            pool,
            layer_shell,
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
        let layer =
            self.layer_shell.create_layer_surface(qh, surface, Layer::Overlay, Some("hyprforge-popup"), Some(&output));
        layer.set_anchor(Anchor::TOP | Anchor::LEFT);
        layer.set_margin(self.placement.margin_top, 0, 0, self.placement.margin_left);
        layer.set_size(self.placement.width, self.placement.height);
        // Exclusive: typing has to reach this popup, not whatever had
        // focus before it opened — the same reasoning `KeyboardInteractivity`'s
        // own doc gives for a lock screen or a password prompt.
        layer.set_keyboard_interactivity(KeyboardInteractivity::Exclusive);
        layer.wl_surface().commit();
        self.layer = Some(layer);
    }

    fn draw(&mut self) {
        self.dirty = false;
        let (width, height) = self.size;
        if width == 0 || height == 0 {
            return;
        }
        let Some(layer) = &self.layer else { return };

        let Ok((buffer, canvas)) =
            self.pool.create_buffer(width as i32, height as i32, width as i32 * 4, wl_shm::Format::Argb8888)
        else {
            // Out of memory for a buffer. Leave the previous frame up —
            // same reasoning as the lock screen: the alternative is a
            // blank surface, which is worse than a stale one.
            return;
        };

        // See `hyprforge-lock::surface`'s identical comment and its
        // pinned test: `iced_tiny_skia` writes B, G, R, A on purpose,
        // which is exactly what `Argb8888` wants byte-for-byte. No
        // swizzle here either.
        let Some(mut pixels) = tiny_skia::PixmapMut::from_bytes(canvas, width, height) else {
            return;
        };
        let Some(mut mask) = tiny_skia::Mask::new(width, height) else {
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
            &Viewport::with_physical_size(Size::new(width, height), 1.0),
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
        wl_surface.damage_buffer(0, 0, width as i32, height as i32);
        if buffer.attach_to(wl_surface).is_ok() {
            wl_surface.commit();
        }
    }

    fn mark_dirty(&mut self) {
        self.dirty = true;
    }

    fn key(&mut self, keysym: smithay_client_toolkit::seat::keyboard::Keysym, utf8: Option<String>) {
        if let Some(outcome) = self.app.key(keysym, utf8) {
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

    fn leave(&mut self, _: &Connection, _: &QueueHandle<Self>, _: &wl_keyboard::WlKeyboard, _: &wl_surface::WlSurface, _: u32) {}

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
        _: Modifiers,
        _: smithay_client_toolkit::seat::keyboard::RawModifiers,
        _: u32,
    ) {
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
                PointerEventKind::Enter { .. } | PointerEventKind::Motion { .. } => {
                    self.pointer_move(event.position);
                }
                PointerEventKind::Leave { .. } => {
                    // Nothing to un-highlight: the selection is the same
                    // state the keyboard drives, and losing the pointer
                    // is not a reason to lose the keyboard's selection
                    // too.
                }
                PointerEventKind::Press { button, .. } => {
                    if *button == BTN_LEFT {
                        // Deferring the click to `Release` unconditionally
                        // would change every existing popup's timing (click
                        // on mouse-up instead of mouse-down) for no benefit
                        // to it — long-press detection is the only reason
                        // to wait at all, so this only defers when the app
                        // actually opted in (see `PopupApp::long_press_duration`'s
                        // own doc). `hyprforge-clipmenu` never does, so its
                        // click still fires right here, exactly as before.
                        if self.app.long_press_duration().is_some() {
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
                        if let Some((press_position, _)) = self.pending_press.take() {
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
    fn scale_factor_changed(&mut self, _: &Connection, _: &QueueHandle<Self>, _: &wl_surface::WlSurface, _: i32) {}

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

delegate_compositor!(@<A: PopupApp + 'static> Popup<A>);
delegate_output!(@<A: PopupApp + 'static> Popup<A>);
delegate_seat!(@<A: PopupApp + 'static> Popup<A>);
delegate_keyboard!(@<A: PopupApp + 'static> Popup<A>);
delegate_pointer!(@<A: PopupApp + 'static> Popup<A>);
delegate_shm!(@<A: PopupApp + 'static> Popup<A>);
delegate_layer!(@<A: PopupApp + 'static> Popup<A>);
delegate_registry!(@<A: PopupApp + 'static> Popup<A>);
