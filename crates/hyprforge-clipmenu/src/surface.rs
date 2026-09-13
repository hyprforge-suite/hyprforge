//! Drawing the popup onto a `wlr_layer` surface.
//!
//! Same shape as `hyprforge-lock::surface`, which this was built by
//! reading closely: `smithay_client_toolkit` for the protocol,
//! `iced_tiny_skia::Renderer` painting into a raw BGRA buffer,
//! `iced_widget` for the tree, `UserInterface::build` driving it one
//! frame at a time. The one structural difference is the shell: the
//! lock screen holds the *session* open with `ext-session-lock-v1`; this
//! only needs one layer-shell surface, positioned once, with no per-output
//! fan-out and no PAM underneath it.

use smithay_client_toolkit::compositor::{CompositorHandler, CompositorState, SurfaceData};
use smithay_client_toolkit::output::{OutputHandler, OutputState};
use smithay_client_toolkit::registry::{ProvidesRegistryState, RegistryState};
use smithay_client_toolkit::seat::keyboard::{KeyEvent, KeyboardHandler, Keysym, Modifiers};
use smithay_client_toolkit::seat::pointer::{
    CursorIcon, PointerData, PointerEvent, PointerEventKind, PointerHandler, ThemeSpec,
    ThemedPointer, BTN_LEFT,
};
use smithay_client_toolkit::seat::{Capability, SeatHandler, SeatState};
use smithay_client_toolkit::shell::wlr_layer::{
    Anchor, KeyboardInteractivity, Layer, LayerShell, LayerShellHandler, LayerSurface,
    LayerSurfaceConfigure,
};
use smithay_client_toolkit::shell::WaylandSurface;
use smithay_client_toolkit::shm::slot::SlotPool;
use smithay_client_toolkit::shm::{Shm, ShmHandler};
use smithay_client_toolkit::{
    delegate_compositor, delegate_keyboard, delegate_layer, delegate_output, delegate_pointer,
    delegate_registry, delegate_seat, delegate_shm, registry_handlers,
};
use calloop_wayland_source::WaylandSource;
use wayland_client::globals::registry_queue_init;
use wayland_client::protocol::{wl_keyboard, wl_output, wl_pointer, wl_seat, wl_shm, wl_surface};
use wayland_client::{Connection, QueueHandle};

use crate::chooser::Chooser;
use crate::geometry::{Hit, RowLayout};
use crate::model::Model;
use crate::pinner::Pinner;
use crate::thumbnail;
use crate::view;
use hyprforge_look::Theme;
use iced_runtime::core::{mouse, renderer, Rectangle, Size};
use iced_runtime::user_interface::{Cache, UserInterface};
use iced_tiny_skia::graphics::Viewport;

/// No messages travel through iced's own update loop — input reaches
/// the model through `key`, exactly as the lock screen reads keystrokes
/// directly rather than through iced, because the compositor hands them
/// to this process before iced ever sees them.
#[derive(Debug, Clone)]
enum Nothing {}

fn iced_color(c: hyprforge_look::Color) -> iced_runtime::core::Color {
    iced_runtime::core::Color::from_rgba8(c.r, c.g, c.b, c.a as f32 / 255.0)
}

/// How long [`finish_choice`] waits for the compositor to confirm it has
/// processed this popup's own layer-surface teardown before giving up
/// and synthesizing the paste anyway.
///
/// `Connection::roundtrip` itself has no bound — see [`finish_choice`]'s
/// doc for why a roundtrip is what this needs — and CLAUDE.md is
/// explicit that nothing here waits on another process (here, the
/// compositor) without one, so the bound is applied from outside on a
/// helper thread. A healthy compositor answers a `wl_display.sync`
/// within microseconds; this is generous well past that while still
/// being short enough that a popup which, for whatever reason, never
/// gets an answer does not sit resident (holding a keypress it has not
/// yet sent) for anything a person would call "stuck".
const FOCUS_RELEASE_TIMEOUT: std::time::Duration = std::time::Duration::from_millis(500);

/// Why the popup stopped.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Outcome {
    /// An entry was chosen and handed to the [`Chooser`] successfully.
    Chosen,
    /// Escape was pressed, or the [`Chooser`] failed and there was
    /// nothing more useful to do than close.
    Cancelled,
    /// The compositor closed the surface out from under this process
    /// (an output was unplugged, or the shell decided to).
    Closed,
    /// The Wayland connection died.
    Disconnected,
}

/// Where to place the popup: an anchor of top-left plus the margins that
/// put its corner at the already-clamped cursor position. Computed by
/// `main.rs` from `geometry::clamp_popup`, so this module never touches
/// `hyprctl` output at all.
pub struct Placement {
    pub output_name: String,
    pub margin_top: i32,
    pub margin_left: i32,
    pub width: u32,
    pub height: u32,
}

/// The paste shortcut this popup was already committed to before it ever
/// drew a frame — decided by `main.rs` (`target::paste_shortcut`) from
/// whatever window had focus before this popup's own surface stole it.
/// Threaded through as plain data so `finish_choice` can hand it to
/// `Chooser::finish_paste` without this module knowing anything about
/// window classes or `hyprctl` itself.
pub use hyprforge_clipboard::Shortcut;

pub struct ClipMenu<C: Chooser> {
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
    /// gives rows and the pin toggle a hand cursor. `ThemedPointer` uses
    /// `wp_cursor_shape_v1` under the hood when the compositor advertises
    /// it and falls back to a themed software cursor drawn from the
    /// system's own cursor theme when it does not — see `new_capability`
    /// for where it is created and why nothing here has to probe for the
    /// protocol itself.
    pointer: Option<ThemedPointer<PointerData, SurfaceData>>,
    /// Cloned from the connection `ClipMenu::run` was handed, so
    /// `update_cursor` can call `ThemedPointer::set_cursor` (which needs
    /// a `&Connection`) from inside pointer event handling without
    /// threading one through every call site. Cheap — `Connection`'s own
    /// `clone` is the same handle to the same socket, not a new one.
    connection: Connection,

    placement: Placement,
    model: Model,
    thumbnails: thumbnail::Cache,
    chooser: C,
    /// Decided once, before this popup's own surface existed to steal
    /// focus — see [`Shortcut`]'s own doc. Read only by
    /// [`finish_choice`], after the choice is made.
    shortcut: Shortcut,
    /// Boxed rather than a second generic parameter alongside `C`: this
    /// popup has exactly one production `Pinner` (`pinner::Wired`) and
    /// one mock, both zero-sized or nearly so, called at most once per
    /// invocation — the dynamic dispatch costs nothing worth avoiding,
    /// and it keeps every `impl<C: Chooser + 'static> ClipMenu<C>` block
    /// and `delegate_*!` invocation below from having to grow a second
    /// type parameter it would otherwise need to thread through all of
    /// them just for this.
    pinner: Box<dyn Pinner>,
    theme: Theme,

    outcome: Option<Outcome>,
    dirty: bool,
    /// Configured size, `(0, 0)` until the first `configure`. The
    /// compositor is free to send back something other than what was
    /// requested; nothing is drawn before it says how big the surface
    /// actually is, for the same reason the lock screen waits.
    size: (u32, u32),
    renderer: iced_tiny_skia::Renderer,
    cache: Cache,
}

impl<C: Chooser + 'static> ClipMenu<C> {
    /// Shows the popup and runs until an entry is chosen or it is
    /// dismissed.
    ///
    /// Takes the connection rather than opening one so a caller can
    /// point it at a nested compositor for testing — see
    /// `crates/hyprforge-lock/testing/nested.sh`; this popup takes
    /// exclusive keyboard focus, which is exactly the kind of thing
    /// CLAUDE.md says never to point at the session you're using.
    pub fn run(
        connection: Connection,
        placement: Placement,
        model: Model,
        chooser: C,
        pinner: impl Pinner + 'static,
        theme: Theme,
        shortcut: Shortcut,
    ) -> Result<Outcome, ClipMenuError> {
        let (globals, mut queue) = registry_queue_init(&connection)?;
        let qh = queue.handle();

        let shm = Shm::bind(&globals, &qh)?;
        let pool = SlotPool::new(4096, &shm).map_err(|e| ClipMenuError::Buffer(e.to_string()))?;
        let layer_shell = LayerShell::bind(&globals, &qh)?;

        let mut menu = ClipMenu {
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
            model,
            thumbnails: thumbnail::Cache::new(),
            chooser,
            shortcut,
            pinner: Box::new(pinner),
            theme: theme.clone(),
            outcome: None,
            dirty: true,
            size: (0, 0),
            renderer: iced_tiny_skia::Renderer::new(
                iced_runtime::core::Font::DEFAULT,
                iced_runtime::core::Pixels(theme.font_size),
            ),
            cache: Cache::default(),
        };

        // Outputs have to be known before a surface can be pinned to
        // one by name — the same ordering reason the lock screen
        // roundtrips before locking.
        queue.roundtrip(&mut menu).map_err(|_| ClipMenuError::Disconnected)?;
        menu.create_surface(&qh);

        let mut event_loop: calloop::EventLoop<ClipMenu<C>> =
            calloop::EventLoop::try_new().map_err(|e| ClipMenuError::EventLoop(e.to_string()))?;
        let handle = event_loop.handle();
        WaylandSource::new(connection.clone(), queue)
            .insert(handle)
            .map_err(|e| ClipMenuError::EventLoop(e.to_string()))?;

        while menu.outcome.is_none() {
            event_loop
                .dispatch(None, &mut menu)
                .map_err(|_| ClipMenuError::Disconnected)?;
            if menu.dirty {
                menu.draw();
            }
        }

        // `Action::Choose` (see `dispatch_action`) only put the entry on
        // the clipboard — it deliberately stops short of synthesizing
        // the paste. That happens here, now that the event loop this
        // popup was driving has stopped: see `finish_choice`'s own doc
        // for why the ordering below is the fix, not an optimisation.
        if menu.outcome == Some(Outcome::Chosen) {
            let shortcut = menu.shortcut;
            finish_choice(&connection, &mut menu, shortcut);
        }

        Ok(menu.outcome.unwrap_or(Outcome::Disconnected))
    }

    /// Finds the output the popup was placed on and creates the one
    /// surface it needs.
    ///
    /// A named output that has disappeared between `main.rs` reading
    /// `hyprctl monitors -j` and this running (unplugged in the gap) has
    /// nowhere to attach to; falling back to *no* output rather than
    /// picking a different one, since a popup that silently landed on
    /// the wrong monitor at the wrong position is worse than one that
    /// visibly failed to appear at all.
    fn create_surface(&mut self, qh: &QueueHandle<Self>) {
        let target = self
            .outputs
            .outputs()
            .find(|o| self.outputs.info(o).and_then(|i| i.name) == Some(self.placement.output_name.clone()));
        let Some(output) = target else {
            eprintln!(
                "output {:?} is no longer present — not showing the popup",
                self.placement.output_name
            );
            self.outcome = Some(Outcome::Closed);
            return;
        };

        let surface = self.compositor.create_surface(qh);
        let layer = self.layer_shell.create_layer_surface(
            qh,
            surface,
            Layer::Overlay,
            Some("hyprforge-clipmenu"),
            Some(&output),
        );
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

        // Seconds since the epoch, the same unit `Entry::copied_at` is
        // stored in — computed fresh every frame rather than once at
        // startup, since a popup can sit open for a while and "now"
        // read once would make its own rows visibly stop aging. A clock
        // that cannot be read at all (no realistic way on this machine,
        // but `SystemTime::now` returning before the epoch is possible
        // in principle) falls back to `0`, which `view::relative_age`
        // treats the same as a clock that jumped backward: "now",
        // never a panic or a nonsense multi-decade age.
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0);

        let size = Size::new(width as f32, height as f32);
        let mut ui = UserInterface::<Nothing, iced_widget::Theme, iced_tiny_skia::Renderer>::build(
            view::view(&self.model, &self.theme, &mut self.thumbnails, now, width as f64),
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
            // Transparent, not `surfaces.root`. The root container in
            // `view` paints that colour itself, rounded to the theme's
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

    fn key(&mut self, keysym: Keysym, utf8: Option<String>) {
        if let Some(outcome) =
            dispatch_key(&mut self.model, &self.chooser, self.pinner.as_ref(), keysym, utf8)
        {
            self.outcome = Some(outcome);
        }
        self.mark_dirty();
    }

    /// The pointer moved (or entered) at `position`, in the surface-local
    /// logical-pixel space [`PointerEvent::position`] reports — the same
    /// space `view.rs` draws into, which is what makes
    /// `geometry::RowLayout::row_at` the right thing to hit-test against
    /// without any translation first.
    ///
    /// Landing outside every row (the header, the padding, a gap between
    /// rows) does nothing rather than clearing the selection — the same
    /// reasoning `move_selection` uses at the ends of the list: leaving
    /// via the header does not mean "select nothing". Either way, this is
    /// also where the cursor shape is kept in sync — see
    /// [`Self::update_cursor`] — since every `Enter` and `Motion` event
    /// routes through here (see `PointerHandler::pointer_frame` below).
    fn pointer_move(&mut self, position: (f64, f64)) {
        let layout = RowLayout::for_font_size(self.theme.font_size);
        let range = self.model.visible_range();
        let row = layout.row_at(position.1, range.len());
        if let Some(row) = row {
            dispatch_action(
                &mut self.model,
                &self.chooser,
                self.pinner.as_ref(),
                Action::Select(range.start + row),
            );
            self.mark_dirty();
        }
        self.update_cursor(row.is_some());
    }

    /// Sets the pointer to a hand cursor whenever it is over a row —
    /// whether that row's preview (which chooses) or its pin toggle
    /// (which pins) — and back to the default arrow everywhere else in
    /// the popup (the header, the padding between rows and the popup's
    /// own edge). Both targets get the same cursor: both are things a
    /// click on this row does something to, and CLAUDE.md's ask was that
    /// a row read as clickable at all, not that each kind of click look
    /// different from the other.
    ///
    /// Errors from [`ThemedPointer::set_cursor`] (most commonly a missing
    /// enter serial, in the gap before this popup's first pointer event
    /// has actually arrived) are not worth reporting — a wrong cursor for
    /// one frame is not a failure the user needs told about, and nothing
    /// here depends on the request having landed.
    fn update_cursor(&self, over_row: bool) {
        let Some(pointer) = &self.pointer else { return };
        let icon = if over_row { CursorIcon::Pointer } else { CursorIcon::Default };
        let _ = pointer.set_cursor(&self.connection, icon);
    }

    /// The pointer clicked at `position`. Re-hit-tests at the click
    /// position first (rather than trusting the last hover) — the same
    /// reasoning the previous version of this method gave for calling
    /// `pointer_move` before acting: a click that lands between a hover
    /// event and a redraw must still be honest about which row, and now
    /// which *part* of that row, it is actually over.
    ///
    /// [`geometry::RowLayout::hit_test`] answers that "which part": a hit
    /// on the pin toggle selects the row and toggles its pin — the exact
    /// same [`Action::TogglePin`] F2 already sends, never a second route
    /// to pinning — and a hit anywhere else on the row selects and
    /// chooses it, same as before this row grew a second click target.
    fn pointer_click(&mut self, position: (f64, f64)) {
        let layout = RowLayout::for_font_size(self.theme.font_size);
        let range = self.model.visible_range();
        let Some(hit) = layout.hit_test(self.size.0 as f64, position, range.len()) else { return };
        dispatch_action(
            &mut self.model,
            &self.chooser,
            self.pinner.as_ref(),
            Action::Select(range.start + hit.row()),
        );
        let action = match hit {
            Hit::Pin(_) => Action::TogglePin,
            Hit::Row(_) => Action::Choose,
        };
        if let Some(outcome) = dispatch_action(&mut self.model, &self.chooser, self.pinner.as_ref(), action) {
            self.outcome = Some(outcome);
        }
        self.mark_dirty();
    }

    /// One wheel/touchpad axis event — moved through
    /// [`Action::Move`], the same action Up/Down produce, so scrolling
    /// cannot desync from what the arrow keys do.
    fn pointer_scroll(&mut self, rows: i32) {
        if rows == 0 {
            return;
        }
        dispatch_action(&mut self.model, &self.chooser, self.pinner.as_ref(), Action::Move(rows));
        self.mark_dirty();
    }
}

/// Finishes a choice already recorded via `Action::Choose`: tears down
/// this popup's own layer surface, proves — with a bound — that the
/// compositor has actually processed that, and only then synthesizes
/// the paste through [`Chooser::finish_paste`].
///
/// # Why the proof, not just a flush
///
/// This popup's layer is `KeyboardInteractivity::Exclusive` (see the
/// comment where [`create_surface`] sets it) for as long as it is on
/// screen, precisely so its own keystrokes reach it rather than
/// whatever had focus before it opened. That is exactly backwards for a
/// *synthesized* paste: sending Ctrl+V while this popup still holds
/// that focus delivers it back to the popup, not to the window the user
/// meant to paste into — which was the whole bug this function exists
/// to close. A `Connection::flush` only proves the destroy request
/// *left this process*; it says nothing about whether the compositor
/// has acted on it and handed focus to whatever is next.
///
/// # Why a roundtrip proves it
///
/// Wayland serializes a single connection's requests in the order the
/// server receives them, and answers them in that same order. A
/// `wl_display.sync` sent *after* the surface's destroy requests
/// (queued when `menu.layer` is dropped below) cannot be answered until
/// the destroy has already been processed — so `Connection::roundtrip`
/// returning is the proof this needs, deterministically, with no sleep
/// and no guessing at a delay. `roundtrip` itself has no bound, though,
/// so [`FOCUS_RELEASE_TIMEOUT`] is applied from outside on a helper
/// thread: a compositor that cannot answer a sync within it is not
/// something this popup — which is still holding the clipboard's
/// backing thread open — should stay resident waiting on. Pasting
/// anyway, with a warning, is the lesser failure; see
/// `chooser::Wired::finish_paste` for what happens if the paste itself
/// then lands nowhere.
fn finish_choice<C: Chooser + 'static>(connection: &Connection, menu: &mut ClipMenu<C>, shortcut: Shortcut) {
    // Dropping the `LayerSurface` queues its role object's destroy
    // request (see `smithay_client_toolkit`'s `Drop for
    // LayerSurfaceInner`) — nothing is sent to the compositor yet,
    // only queued locally, which is exactly why a flush alone would
    // not be proof of anything.
    menu.layer = None;

    let (tx, rx) = std::sync::mpsc::channel();
    let roundtrip_connection = connection.clone();
    std::thread::spawn(move || {
        // The result is only a signal that the sync happened at all;
        // a `roundtrip` failure (a dead connection) leaves nothing
        // more useful to do than proceed to `finish_paste` anyway, so
        // it is not inspected.
        let _ = tx.send(roundtrip_connection.roundtrip());
    });
    if rx.recv_timeout(FOCUS_RELEASE_TIMEOUT).is_err() {
        eprintln!(
            "the compositor didn't confirm the popup's own surface was torn down within \
             {FOCUS_RELEASE_TIMEOUT:?} — pasting anyway"
        );
    }

    menu.chooser.finish_paste(shortcut);
}

/// What the user asked the popup to do, independent of whether a key or a
/// pointer produced it. The seam that keeps mouse and keyboard handling
/// from growing two different sets of rules for the same outcome:
/// `dispatch_key` below turns a keysym into one of these, `ClipMenu`'s
/// pointer handling turns a hit-tested row or a click into another, and
/// [`dispatch_action`] is the one place either ends up.
#[derive(Debug, Clone, Copy, PartialEq)]
enum Action {
    /// Move the selection by this many rows (negative is up) — Up/Down,
    /// or the wheel.
    Move(i32),
    /// Select exactly this row of the filtered list — the pointer
    /// entering or moving over it.
    Select(usize),
    /// Choose whatever is currently selected — Enter, or a click.
    Choose,
    /// Close without choosing anything — Escape.
    Cancel,
    Backspace,
    Type(char),
    /// Pin the selected entry if it is not pinned, or unpin it if it
    /// is — F2, the one keybind this popup has beyond filtering and
    /// choosing.
    TogglePin,
}

/// The one place any [`Action`] takes effect on a [`Model`] and a
/// [`Chooser`]. Split out so every rule about what a row selection, a
/// click or a keystroke *means* can be tested without a Wayland
/// connection — the same split `hyprforge-lock::surface::dispatch_key`
/// uses for the same reason, generalised here to cover the pointer too.
fn dispatch_action<C: Chooser>(
    model: &mut Model,
    chooser: &C,
    pinner: &dyn Pinner,
    action: Action,
) -> Option<Outcome> {
    // Any action at all supersedes a stale pin notice from an earlier
    // attempt — see `Model::pin_notice`'s own doc for why this is
    // cleared this aggressively rather than left to linger until the
    // next successful pin. `Action::TogglePin` below still gets the
    // final say: if this new attempt also fails, it sets its own notice
    // right back.
    model.set_pin_notice(None);
    match action {
        Action::Cancel => Some(Outcome::Cancelled),
        Action::Choose => {
            // Only puts the entry on the clipboard. Synthesizing the
            // paste is `Chooser::finish_paste`'s job, called from
            // `finish_choice` — never from here — once this popup's own
            // surface has been torn down; see both docs for why the
            // split exists. `Outcome::Chosen` ends the event loop this
            // returns into (see `ClipMenu::run`), which is what makes
            // that ordering possible in the first place.
            let entry = model.selected_entry()?;
            match chooser.set_clipboard(&entry) {
                Ok(()) => Some(Outcome::Chosen),
                Err(message) => {
                    eprintln!("couldn't put the chosen entry on the clipboard: {message}");
                    Some(Outcome::Cancelled)
                }
            }
        }
        Action::Move(delta) => {
            model.move_selection(delta);
            None
        }
        Action::Select(index) => {
            model.select(index);
            None
        }
        Action::Backspace => {
            model.backspace();
            None
        }
        Action::Type(c) => {
            model.type_char(c);
            None
        }
        Action::TogglePin => {
            // Nothing selected (an empty filtered list) is nothing to
            // pin — the same "no-op, not an error" `Action::Choose`
            // above uses for the same situation.
            let entry = model.selected_entry()?;
            let new_pinned = !entry.pinned;
            match pinner.set_pinned(entry.id.as_str(), new_pinned) {
                // The daemon is still the only writer of the history
                // file (see `hyprforge_clipboard::ipc`'s module doc) —
                // this only updates the popup's own in-memory copy,
                // once the daemon has already agreed to the change.
                Ok(()) => model.set_entry_pinned(&entry.id, new_pinned),
                // Never applied locally on failure: doing so would make
                // the row look pinned when it is not, on disk or in the
                // daemon's own memory — exactly the "must not look like
                // it worked" failure this popup has to avoid.
                Err(message) => model.set_pin_notice(Some(message)),
            }
            None
        }
    }
}

/// The keystroke rules: which [`Action`] each key produces.
///
/// Nothing typed here is ever logged, matched on for its value, or
/// otherwise inspected beyond being appended to the filter — a
/// clipboard history search is not a password, but this crate follows
/// the same rule regardless of what the field means.
fn dispatch_key<C: Chooser>(
    model: &mut Model,
    chooser: &C,
    pinner: &dyn Pinner,
    keysym: Keysym,
    utf8: Option<String>,
) -> Option<Outcome> {
    match keysym {
        Keysym::Escape => dispatch_action(model, chooser, pinner, Action::Cancel),
        Keysym::Return | Keysym::KP_Enter => dispatch_action(model, chooser, pinner, Action::Choose),
        Keysym::Up => dispatch_action(model, chooser, pinner, Action::Move(-1)),
        Keysym::Down => dispatch_action(model, chooser, pinner, Action::Move(1)),
        Keysym::BackSpace => dispatch_action(model, chooser, pinner, Action::Backspace),
        // F2 rather than a printable character or a modifier combo:
        // every printable keystroke already means "append to the
        // filter" (see the fallback arm below), and modifiers are not
        // even tracked here (`update_modifiers` is a no-op — see this
        // file's `KeyboardHandler` impl), so a Ctrl+P-style binding
        // cannot be recognised without wiring that up. F2 is free,
        // conventional for "rename/toggle a property of the selected
        // row" elsewhere, and cannot collide with typing a search term.
        Keysym::F2 => dispatch_action(model, chooser, pinner, Action::TogglePin),
        _ => {
            if let Some(text) = utf8 {
                for c in text.chars().filter(|c| !c.is_control()) {
                    dispatch_action(model, chooser, pinner, Action::Type(c));
                }
            }
            None
        }
    }
}

/// Turns one scroll-wheel axis event into whole rows to move the
/// selection by.
///
/// Tries three ways to answer "how many notches", in order of how
/// reliable each is on a current compositor: `value120` is what a modern
/// wl_pointer sends (120 per logical step — see the field's own doc),
/// `discrete` is the older "one integer per click" event some
/// compositors still send instead, and the sign of `absolute` (raw
/// scrolled pixels) is the fallback for a touchpad's continuous scroll,
/// which reports neither — moving exactly one row per axis frame there,
/// since there is no such thing as half a row to select.
fn scroll_rows(discrete: i32, value120: i32, absolute: f64) -> i32 {
    if value120 != 0 {
        match value120 / 120 {
            0 if value120 > 0 => 1,
            0 => -1,
            steps => steps,
        }
    } else if discrete != 0 {
        discrete
    } else if absolute > 0.0 {
        1
    } else if absolute < 0.0 {
        -1
    } else {
        0
    }
}

#[derive(Debug, thiserror::Error)]
pub enum ClipMenuError {
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

impl<C: Chooser + 'static> LayerShellHandler for ClipMenu<C> {
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

impl<C: Chooser + 'static> KeyboardHandler for ClipMenu<C> {
    fn enter(
        &mut self,
        _: &Connection,
        _: &QueueHandle<Self>,
        _: &wl_keyboard::WlKeyboard,
        _: &wl_surface::WlSurface,
        _: u32,
        _: &[u32],
        _: &[Keysym],
    ) {
    }

    fn leave(
        &mut self,
        _: &Connection,
        _: &QueueHandle<Self>,
        _: &wl_keyboard::WlKeyboard,
        _: &wl_surface::WlSurface,
        _: u32,
    ) {
    }

    fn press_key(
        &mut self,
        _: &Connection,
        _: &QueueHandle<Self>,
        _: &wl_keyboard::WlKeyboard,
        _: u32,
        event: KeyEvent,
    ) {
        self.key(event.keysym, event.utf8);
    }

    fn release_key(
        &mut self,
        _: &Connection,
        _: &QueueHandle<Self>,
        _: &wl_keyboard::WlKeyboard,
        _: u32,
        _: KeyEvent,
    ) {
    }

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

impl<C: Chooser + 'static> SeatHandler for ClipMenu<C> {
    fn seat_state(&mut self) -> &mut SeatState {
        &mut self.seats
    }

    fn new_seat(&mut self, _: &Connection, _: &QueueHandle<Self>, _: wl_seat::WlSeat) {}

    fn new_capability(
        &mut self,
        _: &Connection,
        qh: &QueueHandle<Self>,
        seat: wl_seat::WlSeat,
        capability: Capability,
    ) {
        if capability == Capability::Keyboard && self.keyboard.is_none() {
            self.keyboard = self.seats.get_keyboard(qh, &seat, None).ok();
        }
        if capability == Capability::Pointer && self.pointer.is_none() {
            // `get_pointer_with_theme` is what makes this a hand cursor
            // over a row rather than a plain arrow everywhere: it uses
            // `wp_cursor_shape_v1` when the compositor advertises it and
            // falls back to a themed software cursor (drawn from the
            // system's own cursor theme, via `shm`, into its own small
            // surface here) when the protocol is absent — this crate
            // never has to check for it itself, and a compositor with
            // neither just makes `ThemedPointer::set_cursor` return an
            // error `update_cursor` already ignores, so the popup still
            // opens and still works with the plain system cursor.
            let cursor_surface = self.compositor.create_surface(qh);
            match self.seats.get_pointer_with_theme(
                qh,
                &seat,
                self.shm.wl_shm(),
                cursor_surface,
                ThemeSpec::default(),
            ) {
                Ok(pointer) => self.pointer = Some(pointer),
                Err(e) => eprintln!(
                    "couldn't set up a themed pointer cursor ({e}) — \
                     clicking still works, just without a hand cursor over a row"
                ),
            }
        }
    }

    fn remove_capability(
        &mut self,
        _: &Connection,
        _: &QueueHandle<Self>,
        _: wl_seat::WlSeat,
        capability: Capability,
    ) {
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

impl<C: Chooser + 'static> PointerHandler for ClipMenu<C> {
    fn pointer_frame(
        &mut self,
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
        _pointer: &wl_pointer::WlPointer,
        events: &[PointerEvent],
    ) {
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
                        self.pointer_click(event.position);
                    }
                }
                PointerEventKind::Release { .. } => {}
                PointerEventKind::Axis { vertical, .. } => {
                    self.pointer_scroll(scroll_rows(
                        vertical.discrete,
                        vertical.value120,
                        vertical.absolute,
                    ));
                }
            }
        }
    }
}

impl<C: Chooser + 'static> CompositorHandler for ClipMenu<C> {
    fn scale_factor_changed(
        &mut self,
        _: &Connection,
        _: &QueueHandle<Self>,
        _: &wl_surface::WlSurface,
        _: i32,
    ) {
    }

    fn transform_changed(
        &mut self,
        _: &Connection,
        _: &QueueHandle<Self>,
        _: &wl_surface::WlSurface,
        _: wl_output::Transform,
    ) {
    }

    fn frame(&mut self, _: &Connection, _: &QueueHandle<Self>, _: &wl_surface::WlSurface, _: u32) {
        self.mark_dirty();
    }

    fn surface_enter(
        &mut self,
        _: &Connection,
        _: &QueueHandle<Self>,
        _: &wl_surface::WlSurface,
        _: &wl_output::WlOutput,
    ) {
    }

    fn surface_leave(
        &mut self,
        _: &Connection,
        _: &QueueHandle<Self>,
        _: &wl_surface::WlSurface,
        _: &wl_output::WlOutput,
    ) {
    }
}

impl<C: Chooser + 'static> OutputHandler for ClipMenu<C> {
    fn output_state(&mut self) -> &mut OutputState {
        &mut self.outputs
    }

    fn new_output(&mut self, _: &Connection, _: &QueueHandle<Self>, _: wl_output::WlOutput) {}

    fn update_output(&mut self, _: &Connection, _: &QueueHandle<Self>, _: wl_output::WlOutput) {}

    fn output_destroyed(&mut self, _: &Connection, _: &QueueHandle<Self>, _: wl_output::WlOutput) {}
}

impl<C: Chooser + 'static> ShmHandler for ClipMenu<C> {
    fn shm_state(&mut self) -> &mut Shm {
        &mut self.shm
    }
}

impl<C: Chooser + 'static> ProvidesRegistryState for ClipMenu<C> {
    fn registry(&mut self) -> &mut RegistryState {
        &mut self.registry
    }

    registry_handlers![OutputState, SeatState];
}

delegate_compositor!(@<C: Chooser + 'static> ClipMenu<C>);
delegate_output!(@<C: Chooser + 'static> ClipMenu<C>);
delegate_seat!(@<C: Chooser + 'static> ClipMenu<C>);
delegate_keyboard!(@<C: Chooser + 'static> ClipMenu<C>);
delegate_pointer!(@<C: Chooser + 'static> ClipMenu<C>);
delegate_shm!(@<C: Chooser + 'static> ClipMenu<C>);
delegate_layer!(@<C: Chooser + 'static> ClipMenu<C>);
delegate_registry!(@<C: Chooser + 'static> ClipMenu<C>);

#[cfg(test)]
mod tests {
    use super::*;
    use crate::chooser::mock::MockChooser;
    use crate::model::HistoryState;
    use crate::pinner::mock::MockPinner;
    use hyprforge_clipboard::{Content, Entry, EntryId};

    fn entry(text: &str) -> Entry {
        let content = Content::Text(text.to_string());
        Entry { id: EntryId::of(&content), content, copied_at: 0, pinned: false }
    }

    fn model_with(texts: &[&str]) -> Model {
        Model::new(HistoryState::Loaded(texts.iter().map(|s| entry(s)).collect()))
    }

    #[test]
    fn escape_cancels_without_choosing_anything() {
        let mut model = model_with(&["a"]);
        let chooser = MockChooser::succeeding();
        let pinner = MockPinner::succeeding();
        let outcome = dispatch_key(&mut model, &chooser, &pinner, Keysym::Escape, None);
        assert_eq!(outcome, Some(Outcome::Cancelled));
        assert!(chooser.calls.borrow().is_empty());
    }

    #[test]
    fn enter_chooses_the_selected_entry_and_ends_the_popup() {
        let mut model = model_with(&["a", "b"]);
        model.move_selection(1);
        let chooser = MockChooser::succeeding();
        let pinner = MockPinner::succeeding();
        let outcome = dispatch_key(&mut model, &chooser, &pinner, Keysym::Return, None);
        assert_eq!(outcome, Some(Outcome::Chosen));
        assert_eq!(chooser.calls.borrow().as_slice(), &[EntryId::of(&Content::Text("b".into()))]);
    }

    /// Regression test for the bug this fix closes. `Action::Choose`
    /// must only put the entry on the clipboard — synthesizing the
    /// paste is a separate step (`Chooser::finish_paste`, via
    /// `finish_choice`) that only runs after this popup's own layer
    /// surface has been torn down (see `ClipMenu::run`). If this test
    /// ever sees `"finish_paste"` in the log, `dispatch_key`/
    /// `dispatch_action` started calling it directly again — which
    /// welds the two calls back together while this popup still holds
    /// `KeyboardInteractivity::Exclusive`, delivering the synthesized
    /// Ctrl+V back to the popup instead of the window the user meant to
    /// paste into.
    #[test]
    fn choosing_sets_the_clipboard_without_synthesizing_the_paste_yet() {
        let mut model = model_with(&["a"]);
        let chooser = MockChooser::succeeding();
        let pinner = MockPinner::succeeding();
        let outcome = dispatch_key(&mut model, &chooser, &pinner, Keysym::Return, None);
        assert_eq!(outcome, Some(Outcome::Chosen));
        assert_eq!(chooser.log.borrow().as_slice(), &["set_clipboard"]);
    }

    #[test]
    fn enter_with_an_empty_list_does_nothing() {
        let mut model = Model::new(HistoryState::Loaded(Vec::new()));
        let chooser = MockChooser::succeeding();
        let pinner = MockPinner::succeeding();
        assert_eq!(dispatch_key(&mut model, &chooser, &pinner, Keysym::Return, None), None);
        assert!(chooser.calls.borrow().is_empty());
    }

    #[test]
    fn a_failing_choose_still_ends_the_popup_rather_than_hanging_open() {
        let mut model = model_with(&["a"]);
        let chooser = MockChooser::failing("no seat");
        let pinner = MockPinner::succeeding();
        let outcome = dispatch_key(&mut model, &chooser, &pinner, Keysym::Return, None);
        assert_eq!(outcome, Some(Outcome::Cancelled));
    }

    #[test]
    fn up_and_down_move_the_selection_through_the_key_dispatcher() {
        let mut model = model_with(&["a", "b", "c"]);
        let chooser = MockChooser::succeeding();
        let pinner = MockPinner::succeeding();
        dispatch_key(&mut model, &chooser, &pinner, Keysym::Down, None);
        assert_eq!(model.selected_index(), 1);
        dispatch_key(&mut model, &chooser, &pinner, Keysym::Up, None);
        assert_eq!(model.selected_index(), 0);
    }

    #[test]
    fn typing_reaches_the_filter() {
        let mut model = model_with(&["alpha", "beta"]);
        let chooser = MockChooser::succeeding();
        let pinner = MockPinner::succeeding();
        dispatch_key(&mut model, &chooser, &pinner, Keysym::NoSymbol, Some("a".to_string()));
        dispatch_key(&mut model, &chooser, &pinner, Keysym::NoSymbol, Some("l".to_string()));
        assert_eq!(model.filter_text(), "al");
        assert_eq!(model.filtered().len(), 1);
    }

    // --- Pointer input, routed through the same `Action`/`dispatch_action`
    // seam the keyboard uses (see `dispatch_key` above and `ClipMenu`'s
    // `pointer_move`/`pointer_click`/`pointer_scroll`, which are thin
    // wrappers over exactly this). The hit-test that turns a pointer
    // position into a row index is `geometry::RowLayout::row_at`, pinned
    // by its own tests in `geometry.rs`; what belongs here is what an
    // already-resolved row or scroll *does*.

    #[test]
    fn hovering_a_row_selects_it_exactly_like_landing_on_it_with_the_keyboard() {
        let mut model = model_with(&["a", "b", "c"]);
        let chooser = MockChooser::succeeding();
        let pinner = MockPinner::succeeding();
        assert_eq!(dispatch_action(&mut model, &chooser, &pinner, Action::Select(2)), None);
        assert_eq!(model.selected_index(), 2);
        assert!(chooser.calls.borrow().is_empty(), "hovering must never choose anything");
    }

    /// Mirrors `ClipMenu::pointer_click`'s `Hit::Pin` branch: select, then
    /// `TogglePin` rather than `Choose` — the same one-action, two-ways-
    /// to-trigger-it property `hovering_a_row_selects_it_exactly_like_landing_on_it_with_the_keyboard`
    /// pins for selection, applied to the pin toggle instead of choosing.
    #[test]
    fn clicking_the_pin_toggle_pins_the_row_instead_of_choosing_it() {
        let mut model = model_with(&["a", "b"]);
        let chooser = MockChooser::succeeding();
        let pinner = MockPinner::succeeding();
        dispatch_action(&mut model, &chooser, &pinner, Action::Select(1));
        let outcome = dispatch_action(&mut model, &chooser, &pinner, Action::TogglePin);
        assert_eq!(outcome, None, "pinning must never end the popup the way choosing does");
        assert!(chooser.calls.borrow().is_empty(), "a pin click must never choose the row");
        assert_eq!(
            pinner.calls.borrow().as_slice(),
            &[(EntryId::of(&Content::Text("b".into())).as_str().to_string(), true)]
        );
    }

    #[test]
    fn a_click_chooses_whatever_row_the_hit_test_resolved_exactly_as_enter_does() {
        let mut model = model_with(&["a", "b", "c"]);
        let chooser = MockChooser::succeeding();
        let pinner = MockPinner::succeeding();
        // The pointer's own path always selects the hit-tested row first
        // (see `ClipMenu::pointer_click`), then chooses — mirrored here
        // as two actions so this test does not need a live surface.
        dispatch_action(&mut model, &chooser, &pinner, Action::Select(1));
        let outcome = dispatch_action(&mut model, &chooser, &pinner, Action::Choose);
        assert_eq!(outcome, Some(Outcome::Chosen));
        assert_eq!(chooser.calls.borrow().as_slice(), &[EntryId::of(&Content::Text("b".into()))]);
    }

    #[test]
    fn scrolling_moves_the_selection_through_the_same_action_up_and_down_use() {
        let mut model = model_with(&["a", "b", "c"]);
        let chooser = MockChooser::succeeding();
        let pinner = MockPinner::succeeding();
        dispatch_action(&mut model, &chooser, &pinner, Action::Move(1));
        assert_eq!(model.selected_index(), 1);
        dispatch_action(&mut model, &chooser, &pinner, Action::Move(-1));
        assert_eq!(model.selected_index(), 0);
    }

    #[test]
    fn a_high_resolution_wheel_step_of_exactly_120_moves_one_row() {
        assert_eq!(scroll_rows(0, 120, 0.0), 1);
        assert_eq!(scroll_rows(0, -120, 0.0), -1);
    }

    #[test]
    fn a_fractional_high_resolution_step_still_moves_at_least_one_row() {
        // A touchpad or a fine-grained wheel can report a `value120`
        // smaller than one logical step; it must still count as a
        // scroll, not be truncated away to nothing.
        assert_eq!(scroll_rows(0, 40, 0.0), 1);
        assert_eq!(scroll_rows(0, -40, 0.0), -1);
    }

    #[test]
    fn a_legacy_discrete_step_is_used_when_no_value120_is_reported() {
        assert_eq!(scroll_rows(2, 0, 0.0), 2);
        assert_eq!(scroll_rows(-3, 0, 0.0), -3);
    }

    #[test]
    fn a_touchpads_continuous_scroll_falls_back_to_one_row_per_frame() {
        assert_eq!(scroll_rows(0, 0, 5.0), 1);
        assert_eq!(scroll_rows(0, 0, -5.0), -1);
        assert_eq!(scroll_rows(0, 0, 0.0), 0, "no motion at all must move nothing");
    }

    #[test]
    fn value120_wins_over_discrete_when_a_compositor_reports_both() {
        assert_eq!(scroll_rows(1, 240, 0.0), 2);
    }

    // --- Pinning, via the F2 key and `Action::TogglePin`. `Pinner` is the
    // same kind of seam `Chooser` is, so these are tested the same way:
    // no socket, no daemon, just `MockPinner` recording what it was
    // asked and handing back whatever result the test configured.

    #[test]
    fn f2_pins_the_selected_unpinned_entry() {
        let mut model = model_with(&["a", "b"]);
        model.move_selection(1); // "b"
        let chooser = MockChooser::succeeding();
        let pinner = MockPinner::succeeding();
        let outcome = dispatch_key(&mut model, &chooser, &pinner, Keysym::F2, None);
        assert_eq!(outcome, None, "pinning does not end the popup");
        assert_eq!(
            pinner.calls.borrow().as_slice(),
            &[(EntryId::of(&Content::Text("b".into())).as_str().to_string(), true)]
        );
        assert!(
            model.filtered().iter().find(|e| e.content == Content::Text("b".into())).unwrap().pinned,
            "the popup's own copy must reflect the pin once the daemon confirmed it"
        );
    }

    #[test]
    fn f2_unpins_an_already_pinned_entry() {
        let mut model = model_with(&["a"]);
        let id = model.filtered()[0].id.clone();
        model.set_entry_pinned(&id, true);
        let chooser = MockChooser::succeeding();
        let pinner = MockPinner::succeeding();
        dispatch_key(&mut model, &chooser, &pinner, Keysym::F2, None);
        assert_eq!(pinner.calls.borrow().as_slice(), &[(id.as_str().to_string(), false)]);
        assert!(!model.filtered()[0].pinned);
    }

    #[test]
    fn f2_with_nothing_selected_does_not_call_the_pinner_at_all() {
        let mut model = Model::new(HistoryState::Loaded(Vec::new()));
        let chooser = MockChooser::succeeding();
        let pinner = MockPinner::succeeding();
        assert_eq!(dispatch_key(&mut model, &chooser, &pinner, Keysym::F2, None), None);
        assert!(pinner.calls.borrow().is_empty());
    }

    /// The failure case that matters most: no daemon to ask. The popup
    /// must not crash, must not end, and — the property this test
    /// exists to pin — must not apply the pin locally, which would make
    /// the row look pinned when nothing on disk or in the daemon agrees.
    #[test]
    fn a_failed_pin_leaves_the_entry_unpinned_and_sets_a_notice() {
        let mut model = model_with(&["a"]);
        let chooser = MockChooser::succeeding();
        let pinner = MockPinner::no_daemon();
        let outcome = dispatch_key(&mut model, &chooser, &pinner, Keysym::F2, None);
        assert_eq!(outcome, None);
        assert!(
            !model.filtered()[0].pinned,
            "a failed pin must not look like it worked"
        );
        assert_eq!(
            model.pin_notice(),
            Some("hyprforge-clipd isn't running, so pinning isn't available right now")
        );
    }

    /// Any other action clears a stale notice from an earlier failed
    /// pin — see `Model::pin_notice`'s doc for why it must not linger.
    #[test]
    fn a_pin_notice_does_not_survive_the_next_unrelated_action() {
        let mut model = model_with(&["a", "b"]);
        let chooser = MockChooser::succeeding();
        let failing_pinner = MockPinner::no_daemon();
        dispatch_key(&mut model, &chooser, &failing_pinner, Keysym::F2, None);
        assert!(model.pin_notice().is_some());

        let succeeding_pinner = MockPinner::succeeding();
        dispatch_key(&mut model, &chooser, &succeeding_pinner, Keysym::Down, None);
        assert_eq!(model.pin_notice(), None, "an unrelated action must clear the notice");
    }

    /// The daemon's own refusal (an id it does not recognise, or
    /// `can_save == false`) is shown verbatim, distinct from the generic
    /// "isn't running" wording `MockPinner::no_daemon` produces —
    /// proving the two are not collapsed into the same message at this
    /// layer either.
    #[test]
    fn a_refusal_from_the_daemon_shows_its_own_message_not_a_generic_one() {
        let mut model = model_with(&["a"]);
        let chooser = MockChooser::succeeding();
        let pinner = MockPinner::failing("no clipboard entry with id abc123");
        dispatch_key(&mut model, &chooser, &pinner, Keysym::F2, None);
        assert_eq!(model.pin_notice(), Some("no clipboard entry with id abc123"));
    }

    /// A successful pin clears any notice left over from a previous
    /// failed attempt at the same entry.
    #[test]
    fn a_successful_pin_clears_a_previous_notice() {
        let mut model = model_with(&["a"]);
        let chooser = MockChooser::succeeding();
        let failing_pinner = MockPinner::no_daemon();
        dispatch_key(&mut model, &chooser, &failing_pinner, Keysym::F2, None);
        assert!(model.pin_notice().is_some());

        let succeeding_pinner = MockPinner::succeeding();
        dispatch_key(&mut model, &chooser, &succeeding_pinner, Keysym::F2, None);
        assert_eq!(model.pin_notice(), None);
        assert!(model.filtered()[0].pinned);
    }
}
