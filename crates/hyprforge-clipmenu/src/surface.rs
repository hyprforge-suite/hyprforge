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

use smithay_client_toolkit::compositor::{CompositorHandler, CompositorState};
use smithay_client_toolkit::output::{OutputHandler, OutputState};
use smithay_client_toolkit::registry::{ProvidesRegistryState, RegistryState};
use smithay_client_toolkit::seat::keyboard::{KeyEvent, KeyboardHandler, Keysym, Modifiers};
use smithay_client_toolkit::seat::{Capability, SeatHandler, SeatState};
use smithay_client_toolkit::shell::wlr_layer::{
    Anchor, KeyboardInteractivity, Layer, LayerShell, LayerShellHandler, LayerSurface,
    LayerSurfaceConfigure,
};
use smithay_client_toolkit::shell::WaylandSurface;
use smithay_client_toolkit::shm::slot::SlotPool;
use smithay_client_toolkit::shm::{Shm, ShmHandler};
use smithay_client_toolkit::{
    delegate_compositor, delegate_keyboard, delegate_layer, delegate_output, delegate_registry,
    delegate_seat, delegate_shm, registry_handlers,
};
use calloop_wayland_source::WaylandSource;
use wayland_client::globals::registry_queue_init;
use wayland_client::protocol::{wl_keyboard, wl_output, wl_seat, wl_shm, wl_surface};
use wayland_client::{Connection, QueueHandle};

use crate::chooser::Chooser;
use crate::model::Model;
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

    placement: Placement,
    model: Model,
    thumbnails: thumbnail::Cache,
    chooser: C,
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
        theme: Theme,
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
            placement,
            model,
            thumbnails: thumbnail::Cache::new(),
            chooser,
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

        let size = Size::new(width as f32, height as f32);
        let mut ui = UserInterface::<Nothing, iced_widget::Theme, iced_tiny_skia::Renderer>::build(
            view::view(&self.model, &self.theme, &mut self.thumbnails),
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

        self.renderer.draw(
            &mut pixels,
            &mut mask,
            &Viewport::with_physical_size(Size::new(width, height), 1.0),
            &[Rectangle::with_size(size)],
            iced_color(self.theme.surfaces.root),
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
        if let Some(outcome) = dispatch_key(&mut self.model, &self.chooser, keysym, utf8) {
            self.outcome = Some(outcome);
        }
        self.mark_dirty();
    }
}

/// The keystroke rules, over a [`Model`] and a [`Chooser`] alone.
///
/// Split out from [`ClipMenu::key`] so every rule about Up/Down/Enter/
/// Escape/typing can be tested without a Wayland connection — the same
/// split `hyprforge-lock::surface::dispatch_key` uses for the same
/// reason.
///
/// Nothing typed here is ever logged, matched on for its value, or
/// otherwise inspected beyond being appended to the filter — a
/// clipboard history search is not a password, but this crate follows
/// the same rule regardless of what the field means.
fn dispatch_key<C: Chooser>(
    model: &mut Model,
    chooser: &C,
    keysym: Keysym,
    utf8: Option<String>,
) -> Option<Outcome> {
    match keysym {
        Keysym::Escape => Some(Outcome::Cancelled),
        Keysym::Return | Keysym::KP_Enter => {
            let entry = model.selected_entry()?;
            match chooser.choose(&entry) {
                Ok(()) => Some(Outcome::Chosen),
                Err(message) => {
                    eprintln!("couldn't paste the chosen entry: {message}");
                    Some(Outcome::Cancelled)
                }
            }
        }
        Keysym::Up => {
            model.move_selection(-1);
            None
        }
        Keysym::Down => {
            model.move_selection(1);
            None
        }
        Keysym::BackSpace => {
            model.backspace();
            None
        }
        _ => {
            if let Some(text) = utf8 {
                for c in text.chars().filter(|c| !c.is_control()) {
                    model.type_char(c);
                }
            }
            None
        }
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
    }

    fn remove_seat(&mut self, _: &Connection, _: &QueueHandle<Self>, _: wl_seat::WlSeat) {}
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
delegate_shm!(@<C: Chooser + 'static> ClipMenu<C>);
delegate_layer!(@<C: Chooser + 'static> ClipMenu<C>);
delegate_registry!(@<C: Chooser + 'static> ClipMenu<C>);

#[cfg(test)]
mod tests {
    use super::*;
    use crate::chooser::mock::MockChooser;
    use crate::model::HistoryState;
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
        let outcome = dispatch_key(&mut model, &chooser, Keysym::Escape, None);
        assert_eq!(outcome, Some(Outcome::Cancelled));
        assert!(chooser.calls.borrow().is_empty());
    }

    #[test]
    fn enter_chooses_the_selected_entry_and_ends_the_popup() {
        let mut model = model_with(&["a", "b"]);
        model.move_selection(1);
        let chooser = MockChooser::succeeding();
        let outcome = dispatch_key(&mut model, &chooser, Keysym::Return, None);
        assert_eq!(outcome, Some(Outcome::Chosen));
        assert_eq!(chooser.calls.borrow().as_slice(), &[EntryId::of(&Content::Text("b".into()))]);
    }

    #[test]
    fn enter_with_an_empty_list_does_nothing() {
        let mut model = Model::new(HistoryState::Loaded(Vec::new()));
        let chooser = MockChooser::succeeding();
        assert_eq!(dispatch_key(&mut model, &chooser, Keysym::Return, None), None);
        assert!(chooser.calls.borrow().is_empty());
    }

    #[test]
    fn a_failing_choose_still_ends_the_popup_rather_than_hanging_open() {
        let mut model = model_with(&["a"]);
        let chooser = MockChooser::failing("no seat");
        let outcome = dispatch_key(&mut model, &chooser, Keysym::Return, None);
        assert_eq!(outcome, Some(Outcome::Cancelled));
    }

    #[test]
    fn up_and_down_move_the_selection_through_the_key_dispatcher() {
        let mut model = model_with(&["a", "b", "c"]);
        let chooser = MockChooser::succeeding();
        dispatch_key(&mut model, &chooser, Keysym::Down, None);
        assert_eq!(model.selected_index(), 1);
        dispatch_key(&mut model, &chooser, Keysym::Up, None);
        assert_eq!(model.selected_index(), 0);
    }

    #[test]
    fn typing_reaches_the_filter() {
        let mut model = model_with(&["alpha", "beta"]);
        let chooser = MockChooser::succeeding();
        dispatch_key(&mut model, &chooser, Keysym::NoSymbol, Some("a".to_string()));
        dispatch_key(&mut model, &chooser, Keysym::NoSymbol, Some("l".to_string()));
        assert_eq!(model.filter_text(), "al");
        assert_eq!(model.filtered().len(), 1);
    }
}
