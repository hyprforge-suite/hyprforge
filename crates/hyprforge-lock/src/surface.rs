//! Holding the session locked, and drawing on the surfaces the
//! compositor hands back.
//!
//! `ext-session-lock-v1` is unusual among Wayland protocols in that the
//! compositor trusts the client with the session's security, and is
//! built so that trust survives the client failing:
//!
//! - Once `locked` arrives, the session **stays** locked. If this process
//!   crashes, the compositor keeps everything hidden rather than falling
//!   open. That is the property that makes writing your own lock screen
//!   reasonable instead of reckless.
//! - The compositor sends a surface **per output**, and expects every one
//!   of them to be drawn on. An output left blank is a monitor showing
//!   whatever was there before.
//! - `finished` means the compositor has refused or revoked the lock. The
//!   only correct response is to exit immediately without unlocking —
//!   something else is already handling the session.
//!
//! Because a crash keeps the session locked, the failure to avoid is not
//! "crashing" but **hanging**: a surface that stops repainting looks
//! exactly like one that died, except the compositor still thinks
//! everything is fine. Every path here is written to keep drawing.

use smithay_client_toolkit::compositor::{CompositorHandler, CompositorState};
use smithay_client_toolkit::output::{OutputHandler, OutputState};
use smithay_client_toolkit::registry::{ProvidesRegistryState, RegistryState};
use smithay_client_toolkit::seat::keyboard::{KeyEvent, KeyboardHandler, Keysym, Modifiers};
use smithay_client_toolkit::seat::{Capability, SeatHandler, SeatState};
use smithay_client_toolkit::session_lock::{
    SessionLock, SessionLockHandler, SessionLockState, SessionLockSurface,
    SessionLockSurfaceConfigure,
};
use smithay_client_toolkit::shm::slot::SlotPool;
use smithay_client_toolkit::shm::{Shm, ShmHandler};
use smithay_client_toolkit::{
    delegate_compositor, delegate_keyboard, delegate_output, delegate_registry, delegate_seat,
    delegate_session_lock, delegate_shm, registry_handlers,
};
use calloop_wayland_source::WaylandSource;
use wayland_client::globals::registry_queue_init;
use wayland_client::protocol::{wl_keyboard, wl_output, wl_seat, wl_shm, wl_surface};
use wayland_client::{Connection, QueueHandle};

use hyprforge_authui::conversation::{Backend, Conversation, State};
use hyprforge_authui::Theme;

/// How often the surface repaints while the authenticator is busy.
///
/// Only while busy — an idle lock screen still draws nothing at all.
const PULSE: std::time::Duration = std::time::Duration::from_millis(100);

/// Why the lock screen stopped.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Outcome {
    /// Authenticated and unlocked.
    Unlocked,
    /// The compositor never granted the lock, so the session was never
    /// locked at all. Kept separate from `Revoked` because the caller's
    /// situation is completely different: whatever asked for a lock
    /// hasn't got one, and reporting success would leave an unlocked
    /// machine looking like a locked one.
    Refused,
    /// The compositor took the lock away after granting it. Something
    /// else owns the session now; exiting without unlocking is the only
    /// safe move, and the session is still secured.
    Revoked,
    /// The connection died. The compositor keeps the session locked.
    Disconnected,
}

/// One output's surface and the size the compositor asked for.
struct Locked {
    surface: SessionLockSurface,
    width: u32,
    height: u32,
    /// Kept so a monitor plugged in while locked can be matched against
    /// the surfaces that already exist.
    output: wl_output::WlOutput,
}

pub struct LockScreen<B: Backend + 'static> {
    registry: RegistryState,
    outputs: OutputState,
    seats: SeatState,
    shm: Shm,
    compositor: CompositorState,
    pool: SlotPool,

    lock: Option<SessionLock>,
    surfaces: Vec<Locked>,
    keyboard: Option<wl_keyboard::WlKeyboard>,

    /// `None` until the compositor has actually granted the lock.
    ///
    /// Building it starts PAM talking, and that must not happen while
    /// the session is still open: a PAM module that takes its time — or
    /// hangs — would otherwise hold the screen unlocked for exactly as
    /// long as it took. Lock first, ask questions second.
    conversation: Option<Conversation<B>>,
    theme: Theme,
    outcome: Option<Outcome>,
    /// Set whenever something changed that the user should see. Drawing
    /// is driven by this rather than by a timer, so an idle lock screen
    /// costs nothing.
    dirty: bool,
    /// Frames actually committed. Zero after a configure means the
    /// session is locked with nothing on screen — the failure worth
    /// noticing loudly.
    frames: u64,
    /// Whether the compositor ever granted the lock. Recorded rather
    /// than inferred from the surfaces: a grant with no outputs attached
    /// has none either, and mistaking that for "never locked" would
    /// report the session as open when it is held.
    granted: bool,
    /// Advances while the authenticator is busy, so the surface has
    /// something to animate and a user can see it is still alive.
    tick: u64,
    /// Text to type in by itself once something has been drawn, for
    /// testing against a nested compositor. `None` in every real run;
    /// `main` refuses to set it without an explicit `--display`.
    self_test: Option<String>,
    /// Whether this run is a self test, kept after `self_test` is
    /// consumed so the frame count can be reported at the end.
    self_testing: bool,
}

impl<B: Backend + 'static> LockScreen<B> {
    /// Locks the session and runs until it unlocks.
    ///
    /// Takes the connection rather than opening one so a caller can point
    /// it at a nested compositor — which is the only safe way to test a
    /// lock screen, since a mistake against the real session is a machine
    /// you have to power-cycle.
    ///
    /// Takes the backend rather than a built `Conversation` so that
    /// nothing asks the user anything until the session is actually
    /// locked — see the `conversation` field.
    ///
    /// `wake` fires when the backend may have an answer ready; `None`
    /// suits a backend that always answers immediately.
    pub fn run(
        connection: Connection,
        backend: B,
        username: impl Into<String>,
        theme: Theme,
        wake: Option<calloop::ping::PingSource>,
        self_test: Option<String>,
    ) -> Result<Outcome, LockError> {
        let username = username.into();
        let mut pending = Some(backend);
        let (globals, mut queue) = registry_queue_init(&connection)?;
        let qh = queue.handle();

        let shm = Shm::bind(&globals, &qh)?;
        // One page is plenty to start: the pool grows when a real size
        // arrives, and guessing the output size here would be wrong on
        // the first configure anyway.
        let pool = SlotPool::new(4096, &shm).map_err(|e| LockError::Buffer(e.to_string()))?;
        let lock_state = SessionLockState::new(&globals, &qh);

        let mut screen = LockScreen {
            registry: RegistryState::new(&globals),
            outputs: OutputState::new(&globals, &qh),
            seats: SeatState::new(&globals, &qh),
            compositor: CompositorState::bind(&globals, &qh)?,
            shm,
            pool,
            lock: None,
            surfaces: Vec::new(),
            keyboard: None,
            conversation: None,
            theme,
            outcome: None,
            dirty: true,
            frames: 0,
            granted: false,
            tick: 0,
            self_testing: self_test.is_some(),
            self_test,
        };

        // The outputs have to be known *before* locking. `locked` can
        // arrive in the same batch as the lock request, and creating
        // surfaces from an output list that hasn't been filled in yet
        // produces a locked session with nothing drawn on it — which
        // looks exactly like the lock screen having crashed, except
        // nothing has crashed and nothing will recover.
        queue.roundtrip(&mut screen).map_err(|_| LockError::Disconnected)?;
        screen.lock = Some(lock_state.lock(&qh)?);

        // From here on the loop waits on calloop rather than on the
        // Wayland queue alone. That is what lets the authenticator
        // answer on its own schedule: the backend's ping is just another
        // source, so a reply wakes the loop exactly like a keystroke
        // does, and nothing has to sit blocked waiting for PAM.
        let mut event_loop: calloop::EventLoop<LockScreen<B>> =
            calloop::EventLoop::try_new().map_err(|e| LockError::EventLoop(e.to_string()))?;
        let handle = event_loop.handle();
        WaylandSource::new(connection.clone(), queue)
            .insert(handle.clone())
            .map_err(|e| LockError::EventLoop(e.to_string()))?;

        if let Some(wake) = wake {
            handle
                .insert_source(wake, |_, _, screen: &mut LockScreen<B>| {
                    if let Some(conversation) = screen.conversation.as_mut() {
                        if conversation.pump() {
                            screen.dirty = true;
                        }
                    }
                })
                .map_err(|e| LockError::EventLoop(e.to_string()))?;
        }

        // A heartbeat while the authenticator is busy, so the surface
        // visibly keeps moving. Without it the screen would be correct
        // and responsive but look frozen, which on a lock screen is the
        // thing a user cannot tell apart from a crash.
        let pulse = calloop::timer::Timer::from_duration(PULSE);
        handle
            .insert_source(pulse, |_, _, screen: &mut LockScreen<B>| {
                if matches!(screen.state(), State::Working) {
                    screen.tick = screen.tick.wrapping_add(1);
                    screen.dirty = true;
                }
                calloop::timer::TimeoutAction::ToDuration(PULSE)
            })
            .map_err(|e| LockError::EventLoop(e.to_string()))?;

        while screen.outcome.is_none() {
            event_loop
                .dispatch(None, &mut screen)
                .map_err(|_| LockError::Disconnected)?;
            // The session is now genuinely locked, so it is safe to let
            // the authenticator start talking.
            if screen.granted && screen.conversation.is_none() {
                if let Some(backend) = pending.take() {
                    screen.conversation = Some(Conversation::new(backend, username.clone()));
                    screen.mark_dirty();
                }
            }
            // Before drawing, not after: typing marks the screen dirty,
            // and doing it afterwards would leave the result unpainted
            // until some unrelated event happened to wake the loop.
            //
            // Waits for two things. Something must have been drawn — the
            // point is to prove the whole path, and typing into a lock
            // screen that never painted would prove half of it. And the
            // conversation must actually be asking: a backend that takes
            // a moment to produce its first prompt would otherwise be
            // typed at while still working, and the conversation would
            // correctly ignore every keystroke.
            if screen.frames > 0 && screen.state().accepts_input() {
                if let Some(text) = screen.self_test.take() {
                    eprintln!("self test: typing {} character(s)", text.chars().count());
                    for character in text.chars() {
                        screen.key(Keysym::NoSymbol, Some(character.to_string()));
                    }
                    screen.key(Keysym::Return, None);
                    eprintln!("self test: submitted, state is {:?}", screen.state());
                }
            }
            if screen.dirty {
                let before = screen.frames;
                screen.draw_all();
                // Only the first frame is worth a line. It is the one
                // that proves there is something on screen; the rest
                // are just a person typing.
                if before == 0 && screen.frames > 0 {
                    eprintln!("first frame drawn");
                }
            }
            // Checked after dispatching rather than inside a handler:
            // unlocking has to be the last thing that happens, and doing
            // it from inside an event callback risks drawing afterwards
            // on a surface that no longer exists.
            //
            // Only when nothing else has already decided how this ends.
            // `finished` and the last keystroke can arrive in the same
            // batch, and without this guard a revoked lock would be
            // overwritten with `Unlocked` — reporting a successful
            // unlock for a session this process never held and never
            // released.
            if let Some(end) = conclude(
                screen.outcome,
                screen.state().is_authenticated(),
                screen.lock.is_some(),
            ) {
                if let Some(lock) = screen.lock.take() {
                    lock.unlock();
                    // The unlock request has to reach the compositor
                    // before this process exits, or the session stays
                    // locked with nothing left to unlock it.
                    connection.roundtrip().ok();
                }
                screen.outcome = Some(end);
            }
        }
        if screen.self_testing {
            // The number that matters when a backend is slow: a screen
            // that drew once and then sat there is the failure this
            // whole arrangement exists to prevent.
            eprintln!("self test: {} frame(s) drawn in total", screen.frames);
        }
        Ok(screen.outcome.unwrap_or(Outcome::Disconnected))
    }

    fn draw_all(&mut self) {
        self.dirty = false;
        for index in 0..self.surfaces.len() {
            self.draw(index);
        }
    }

    /// Paints one surface.
    ///
    /// Deliberately software-rendered and deliberately simple. This is
    /// the surface that stands between a locked machine and its user; it
    /// has no business depending on a GPU being in a good mood.
    fn draw(&mut self, index: usize) {
        let Some(locked) = self.surfaces.get(index) else {
            return;
        };
        // Nothing is drawn before the compositor says how big the surface
        // is. Drawing into a guessed size is what produced the first
        // panic here: a 1px-wide canvas with a panel clamped to a 280px
        // minimum underflowed on the centring subtraction.
        let (width, height) = (locked.width, locked.height);
        if width == 0 || height == 0 {
            return;
        }
        let stride = width as i32 * 4;

        // One dot per typed character, and a colour that says what state
        // the conversation is in. No text: a font renderer is a
        // dependency this surface doesn't need yet, and the shape of the
        // feedback is what the spike is proving.
        let working = matches!(self.state(), State::Working);
        let (indicator, count) = match self.state() {
            State::Asking { entered, .. } => (argb(&self.theme.accent), entered.chars().count()),
            State::Working => (argb(&self.theme.accent), 0),
            State::Failed { .. } => (argb(&self.theme.error), 0),
            State::Telling { error, .. } => {
                (argb(if *error { &self.theme.error } else { &self.theme.foreground }), 0)
            }
            State::Authenticated => (argb(&self.theme.foreground), 0),
        };

        let background = argb(&self.theme.background);
        let surface_colour = argb(&self.theme.surface);

        let Ok((buffer, canvas)) = self.pool.create_buffer(
            width as i32,
            height as i32,
            stride,
            wl_shm::Format::Argb8888,
        ) else {
            // Out of memory for a buffer. Leaving the previous frame up
            // is right: the surface stays as it was rather than going
            // blank, and the next event tries again.
            return;
        };

        for pixel in canvas.chunks_exact_mut(4) {
            pixel.copy_from_slice(&background.to_le_bytes());
        }

        // A panel, centred, sized as a fraction of the output so it
        // looks the same on every screen rather than tiny on a 4K one.
        //
        // Every step saturates. The size the compositor asks for is not
        // this program's to assume, and a panic here is the one failure
        // with no recovery: the session stays locked with nothing left
        // running to unlock it.
        let (panel_w, panel_h) = panel_size(width, height);
        let panel_x = width.saturating_sub(panel_w) / 2;
        let panel_y = height.saturating_sub(panel_h) / 2;
        fill(canvas, width, panel_x, panel_y, panel_w, panel_h, surface_colour);

        let dot = 14u32;
        let gap = 10u32;
        let dots = count.min(16) as u32;
        let row_w = dots * dot + dots.saturating_sub(1) * gap;
        let row_x = panel_x + panel_w.saturating_sub(row_w) / 2;
        let row_y = panel_y + (panel_h / 2).saturating_sub(dot / 2);
        for i in 0..dots {
            fill(canvas, width, row_x + i * (dot + gap), row_y, dot, dot, indicator);
        }
        // A bar under the panel carries the state even with nothing
        // typed — otherwise a failure with an empty field shows nothing
        // at all.
        let bar_y = panel_y + panel_h.saturating_sub(4);
        if working {
            // While the authenticator is busy the bar becomes a segment
            // sliding along a dim track. It is the only thing on screen
            // that says "still thinking" rather than "stopped": PAM can
            // sit for seconds on a wrong password, and a still frame for
            // that long reads as a crash.
            fill(canvas, width, panel_x, bar_y, panel_w, 4, surface_lit(surface_colour));
            let travel = panel_w.saturating_sub(BAR_SEGMENT);
            fill(canvas, width, panel_x + sweep(self.tick, travel), bar_y, BAR_SEGMENT, 4, indicator);
        } else {
            fill(canvas, width, panel_x, bar_y, panel_w, 4, indicator);
        }

        let surface = locked.surface.wl_surface();
        surface.damage_buffer(0, 0, width as i32, height as i32);
        if buffer.attach_to(surface).is_ok() {
            surface.commit();
            self.frames += 1;
        }
    }

    fn mark_dirty(&mut self) {
        self.dirty = true;
    }

    /// The conversation's state, or `Working` before there is one.
    ///
    /// `Working` rather than anything else because it is true: the lock
    /// is being set up, nothing is being asked yet, and it is the one
    /// state that accepts no input and claims no success. In particular
    /// it can never read as authenticated, so the window before the lock
    /// is granted cannot unlock anything.
    fn state(&self) -> &State {
        const STARTING: &State = &State::Working;
        self.conversation.as_ref().map_or(STARTING, |c| c.state())
    }
}

/// How the loop should end this iteration, if it should end at all.
///
/// Pulled out of the loop because it is the one decision in this program
/// that can be wrong in a dangerous direction: reporting a successful
/// unlock for a session that was never unlocked. In the loop it needed a
/// live compositor to exercise; here it can just be checked.
///
/// `decided` is whatever a handler already concluded — `finished` in
/// particular — and it always wins. A revoked lock and the last
/// keystroke can arrive in the same batch, and the revocation is the
/// truth about the session.
fn conclude(decided: Option<Outcome>, authenticated: bool, holds_lock: bool) -> Option<Outcome> {
    if decided.is_some() || !authenticated {
        return None;
    }
    // Authenticated while holding nothing means there is nothing to
    // unlock and nothing was unlocked. Whatever asked for a lock hasn't
    // got one, which is exactly what `Refused` reports.
    Some(if holds_lock { Outcome::Unlocked } else { Outcome::Refused })
}

/// How wide the moving segment is while the authenticator is busy.
const BAR_SEGMENT: u32 = 64;

/// Where the busy segment sits at `tick`, sweeping `travel` pixels and
/// coming back rather than jumping.
///
/// Pulled out to be tested: an off-by-one at either end would push the
/// segment past the panel, and `fill` would quietly clip it into a bar
/// that looks stuck exactly when it is meant to prove liveness.
fn sweep(tick: u64, travel: u32) -> u32 {
    if travel == 0 {
        return 0;
    }
    let span = u64::from(travel);
    // A triangle wave: 0..span, then span..0.
    let phase = tick % (span * 2);
    (if phase < span { phase } else { span * 2 - phase }) as u32
}

/// A slightly brighter version of the panel, for the track the busy
/// segment slides along. Derived rather than themed: it is a shade of
/// the surface, not a colour anyone should have to configure.
fn surface_lit(surface: u32) -> u32 {
    let lift = |shift: u32| {
        let channel = (surface >> shift) & 0xff;
        (channel + (0xff - channel) / 6) << shift
    };
    (surface & 0xff00_0000) | lift(16) | lift(8) | lift(0)
}

/// The panel's size for an output of `width` by `height`.
///
/// Never larger than the surface. A minimum that exceeds the output is
/// how the first version panicked, and a tiny surface is not
/// hypothetical — a compositor may configure one before the real size
/// is known.
fn panel_size(width: u32, height: u32) -> (u32, u32) {
    let panel_w = (width / 3).clamp(280, 640).min(width);
    let panel_h = (height / 6).clamp(120, 260).min(height);
    (panel_w, panel_h)
}

/// Fills a rectangle, clipped to the canvas.
///
/// Clipped rather than asserted: a configure can arrive with a size that
/// doesn't match the buffer in flight, and panicking on a lock screen is
/// the one outcome with no recovery.
fn fill(canvas: &mut [u8], canvas_w: u32, x: u32, y: u32, w: u32, h: u32, colour: u32) {
    let bytes = colour.to_le_bytes();
    let canvas_h = (canvas.len() / 4) as u32 / canvas_w.max(1);
    // Saturating, not wrapping: these are clamped to the canvas on the
    // very next line anyway, so the only thing an overflow could add is
    // a panic on the one surface that must never panic.
    for row in y..y.saturating_add(h).min(canvas_h) {
        for column in x..x.saturating_add(w).min(canvas_w) {
            let offset = ((row * canvas_w + column) * 4) as usize;
            if let Some(pixel) = canvas.get_mut(offset..offset + 4) {
                pixel.copy_from_slice(&bytes);
            }
        }
    }
}

/// `rgba(rrggbbaa)` to the packed ARGB a Wayland shm buffer wants.
///
/// Falls back to opaque black rather than failing: a lock screen with an
/// unparseable colour should be plain, not absent.
pub fn argb(colour: &str) -> u32 {
    let body = colour
        .strip_prefix("rgba(")
        .or_else(|| colour.strip_prefix("rgb("))
        .and_then(|rest| rest.strip_suffix(')'))
        .unwrap_or("");
    let byte = |i: usize| u32::from_str_radix(body.get(i..i + 2).unwrap_or("00"), 16).unwrap_or(0);
    match body.len() {
        6 => 0xff00_0000 | (byte(0) << 16) | (byte(2) << 8) | byte(4),
        8 => (byte(6) << 24) | (byte(0) << 16) | (byte(2) << 8) | byte(4),
        _ => 0xff00_0000,
    }
}

#[derive(Debug, thiserror::Error)]
pub enum LockError {
    #[error("couldn't talk to the compositor: {0}")]
    Connect(#[from] wayland_client::globals::GlobalError),
    /// Nearly always: this compositor doesn't implement
    /// `ext-session-lock-v1`, so there is no way to lock at all.
    #[error("this compositor can't lock the session (needs ext-session-lock-v1): {0}")]
    CannotLock(#[from] smithay_client_toolkit::error::GlobalError),
    /// Most often: this compositor doesn't implement
    /// `ext-session-lock-v1`, so there is nothing to lock with.
    #[error("this compositor is missing something the lock screen needs: {0}")]
    Missing(#[from] smithay_client_toolkit::reexports::client::globals::BindError),
    #[error("couldn't set up a drawing buffer: {0}")]
    Buffer(String),
    #[error("couldn't set up the event loop: {0}")]
    EventLoop(String),
    #[error("the connection to the compositor was lost")]
    Disconnected,
}

impl<B: Backend + 'static> SessionLockHandler for LockScreen<B> {
    fn locked(&mut self, _conn: &Connection, qh: &QueueHandle<Self>, lock: SessionLock) {
        // From here the session is locked whatever happens to this
        // process. A surface per output, because an output without one
        // keeps showing what was on it.
        for output in self.outputs.outputs() {
            let surface = self.compositor.create_surface(qh);
            let locked = lock.create_lock_surface(surface, &output, qh);
            self.surfaces.push(Locked { surface: locked, width: 0, height: 0, output });
        }
        eprintln!("locked: {} surface(s) created", self.surfaces.len());
        if self.surfaces.is_empty() {
            // Nothing to draw on means a locked session showing nothing,
            // which is indistinguishable from a crash and just as
            // unrecoverable. Say so loudly rather than sit there.
            eprintln!("no outputs to draw on — the session is locked with nothing visible");
        }
        self.granted = true;
        self.lock = Some(lock);
        self.mark_dirty();
    }

    fn finished(&mut self, _conn: &Connection, _qh: &QueueHandle<Self>, _lock: SessionLock) {
        // The compositor refused or revoked the lock. Never unlock here:
        // something else owns the session now, and unlocking would be
        // taking a decision that isn't ours.
        //
        // Which of the two it was matters, and only this handler can
        // tell them apart: arriving before `locked` means the request
        // was refused outright — almost always because another lock
        // client already holds the session. Saying so is the difference
        // between a one-line diagnosis and an afternoon of guessing.
        self.outcome = Some(if !self.granted {
            eprintln!(
                "the compositor refused the lock — another lock client probably \
                 already holds this session"
            );
            Outcome::Refused
        } else {
            eprintln!("the compositor revoked the lock");
            Outcome::Revoked
        });
        self.lock = None;
    }

    fn configure(
        &mut self,
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
        surface: SessionLockSurface,
        configure: SessionLockSurfaceConfigure,
        _serial: u32,
    ) {
        let (width, height) = configure.new_size;
        if let Some(locked) = self
            .surfaces
            .iter_mut()
            .find(|l| l.surface.wl_surface() == surface.wl_surface())
        {
            locked.width = width;
            locked.height = height;
            eprintln!("configure: {width}x{height}");
        }
        self.mark_dirty();
    }
}

impl<B: Backend + 'static> KeyboardHandler for LockScreen<B> {
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
        eprintln!("keyboard focus entered a lock surface");
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

    /// Held keys repeat, so a held backspace clears a password the way
    /// it does in every other text field.
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

impl<B: Backend + 'static> LockScreen<B> {
    /// One key, as the conversation sees it.
    ///
    /// Split out from the Wayland handler so a key can be delivered
    /// without a compositor sending it. That is what makes the unlock
    /// path testable at all: everything from the keystroke to the
    /// compositor releasing the session runs here, and only the
    /// `wl_keyboard` delivery is left out.
    fn key(&mut self, keysym: Keysym, utf8: Option<String>) {
        // Nothing about a keystroke is logged here, ever. A keysym name
        // *is* the character — `XK_a` for `a`, `XK_comma` for `,` — so
        // logging "just the keysym" to debug input writes the password
        // to disk in a barely-encoded form. This comment exists because
        // that mistake was made here once already.
        // Nothing is being asked until the lock is granted, so a key
        // pressed in that window has nowhere to go.
        let Some(conversation) = self.conversation.as_mut() else {
            return;
        };
        match keysym {
            Keysym::Return | Keysym::KP_Enter => match conversation.state() {
                State::Telling { .. } => conversation.acknowledge(),
                State::Failed { .. } => conversation.retry(),
                _ => conversation.submit(),
            },
            Keysym::Escape => conversation.clear(),
            Keysym::BackSpace => {
                let mut entered = conversation.entered().to_string();
                entered.pop();
                conversation.type_into(entered);
            }
            _ => {
                // Any key at all leaves the failed state, so the user can
                // simply start typing again rather than having to work
                // out which key dismisses the error.
                if matches!(conversation.state(), State::Failed { .. }) {
                    conversation.retry();
                }
                if let Some(text) = utf8 {
                    // Control characters would otherwise count as typed
                    // characters and show a dot for nothing.
                    if !text.is_empty() && !text.chars().any(char::is_control) {
                        let mut entered = conversation.entered().to_string();
                        entered.push_str(&text);
                        conversation.type_into(entered);
                    }
                }
            }
        }
        self.mark_dirty();
    }
}

impl<B: Backend + 'static> SeatHandler for LockScreen<B> {
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
            eprintln!("keyboard bound: {}", self.keyboard.is_some());
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

impl<B: Backend + 'static> CompositorHandler for LockScreen<B> {
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

impl<B: Backend + 'static> OutputHandler for LockScreen<B> {
    fn output_state(&mut self) -> &mut OutputState {
        &mut self.outputs
    }

    /// A monitor plugged in while the session is locked needs a surface
    /// of its own, or it shows whatever was on it before the lock — on a
    /// laptop being docked, that is the desktop of a locked machine.
    fn new_output(&mut self, _: &Connection, qh: &QueueHandle<Self>, output: wl_output::WlOutput) {
        let Some(lock) = &self.lock else {
            return;
        };
        if self.surfaces.iter().any(|l| l.output == output) {
            return;
        }
        let surface = self.compositor.create_surface(qh);
        let locked = lock.create_lock_surface(surface, &output, qh);
        self.surfaces.push(Locked { surface: locked, width: 0, height: 0, output });
        self.mark_dirty();
    }

    fn update_output(&mut self, _: &Connection, _: &QueueHandle<Self>, _: wl_output::WlOutput) {}

    fn output_destroyed(
        &mut self,
        _: &Connection,
        _: &QueueHandle<Self>,
        output: wl_output::WlOutput,
    ) {
        self.surfaces.retain(|l| l.output != output);
    }
}

impl<B: Backend + 'static> ShmHandler for LockScreen<B> {
    fn shm_state(&mut self) -> &mut Shm {
        &mut self.shm
    }
}

impl<B: Backend + 'static> ProvidesRegistryState for LockScreen<B> {
    fn registry(&mut self) -> &mut RegistryState {
        &mut self.registry
    }

    registry_handlers![OutputState, SeatState];
}

delegate_compositor!(@<B: Backend + 'static> LockScreen<B>);
delegate_output!(@<B: Backend + 'static> LockScreen<B>);
delegate_seat!(@<B: Backend + 'static> LockScreen<B>);
delegate_keyboard!(@<B: Backend + 'static> LockScreen<B>);
delegate_shm!(@<B: Backend + 'static> LockScreen<B>);
delegate_session_lock!(@<B: Backend + 'static> LockScreen<B>);
delegate_registry!(@<B: Backend + 'static> LockScreen<B>);

#[cfg(test)]
mod tests {
    use super::*;

    /// The colours come from the shared theme as `rgba()` strings and
    /// have to reach the framebuffer as ARGB.
    #[test]
    fn theme_colours_convert_to_argb() {
        assert_eq!(argb("rgba(bd93f9ff)"), 0xffbd93f9);
        assert_eq!(argb("rgb(282a36)"), 0xff282a36);
        assert_eq!(argb("rgba(00000080)"), 0x8000_0000);
    }

    /// A lock screen with an unparseable colour should be plain, not
    /// absent — so anything unrecognised is opaque black.
    #[test]
    fn an_unparseable_colour_falls_back_to_opaque_black() {
        for bad in ["", "nonsense", "rgba(", "rgba(zz)", "0xffbd93f9"] {
            assert_eq!(argb(bad), 0xff00_0000, "{bad}");
        }
    }

    /// The busy segment must stay inside the panel. Overshooting would
    /// get clipped by `fill` into a segment that sits still at the edge
    /// — looking stuck at the exact moment it is there to show the
    /// screen is alive.
    #[test]
    fn the_busy_segment_never_leaves_its_track() {
        for travel in [0, 1, 2, 63, 64, 500, u32::MAX] {
            for tick in 0..300u64 {
                assert!(sweep(tick, travel) <= travel, "travel {travel} tick {tick}");
            }
        }
    }

    /// It has to actually move, and it has to come back rather than
    /// jump — a segment that teleported to the start would read as a
    /// repaint glitch.
    #[test]
    fn the_busy_segment_sweeps_out_and_back() {
        let travel = 10;
        let path: Vec<u32> = (0..=20).map(|tick| sweep(tick, travel)).collect();
        assert_eq!(path[0], 0);
        assert_eq!(path[10], 10, "reaches the far end");
        assert_eq!(path[20], 0, "and returns");
        // Never more than one step at a time, in either direction.
        for pair in path.windows(2) {
            assert_eq!(pair[0].abs_diff(pair[1]), 1, "{path:?}");
        }
    }

    /// A zero-width track is what a tiny surface produces, and dividing
    /// into it must not panic on the surface that cannot recover.
    #[test]
    fn a_track_with_no_room_stays_put() {
        assert_eq!(sweep(0, 0), 0);
        assert_eq!(sweep(12345, 0), 0);
    }

    /// Reporting a successful unlock for a session that was never
    /// unlocked is the worst thing this program could do: something
    /// upstream would treat an open machine as a locked one.
    #[test]
    fn a_revoked_lock_is_never_reported_as_an_unlock() {
        // The batch that cost this a bug: `finished` and the last
        // keystroke arriving together.
        assert_eq!(conclude(Some(Outcome::Revoked), true, false), None);
        assert_eq!(conclude(Some(Outcome::Refused), true, false), None);
        assert_eq!(conclude(Some(Outcome::Disconnected), true, true), None);
    }

    /// Authenticating without a lock in hand unlocked nothing, so it
    /// must not claim to have.
    #[test]
    fn authenticating_without_the_lock_does_not_claim_success() {
        assert_eq!(conclude(None, true, false), Some(Outcome::Refused));
    }

    #[test]
    fn the_ordinary_unlock_still_concludes() {
        assert_eq!(conclude(None, true, true), Some(Outcome::Unlocked));
    }

    /// Not authenticated is not an ending. A lock screen that concluded
    /// anything here would let go of a session nobody proved they own.
    #[test]
    fn nothing_concludes_while_the_user_has_not_authenticated() {
        assert_eq!(conclude(None, false, true), None);
        assert_eq!(conclude(None, false, false), None);
    }

    /// A configure can arrive with a size that doesn't match the buffer
    /// in flight. Panicking on a lock screen is the one outcome with no
    /// recovery, so the fill clips instead.
    #[test]
    fn filling_outside_the_canvas_clips_rather_than_panicking() {
        let mut canvas = vec![0u8; 4 * 4 * 4]; // 4x4 pixels
        fill(&mut canvas, 4, 2, 2, 100, 100, 0xffff_ffff);
        fill(&mut canvas, 4, 10, 10, 4, 4, 0xffff_ffff);
        fill(&mut canvas, 4, 0, 0, 0, 0, 0xffff_ffff);
        // Coordinates whose sum overflows u32. Clamped on the next line
        // either way, so the only thing an overflow could add is a panic.
        fill(&mut canvas, 4, u32::MAX - 1, u32::MAX - 1, 8, 8, 0xffff_ffff);
        fill(&mut canvas, 4, 1, 1, u32::MAX, u32::MAX, 0xffff_ffff);

        // The bottom-right pixel was inside the first rectangle.
        assert_eq!(&canvas[(3 * 4 + 3) * 4..][..4], &0xffff_ffffu32.to_le_bytes());
        // The top-left was outside all of them.
        assert_eq!(&canvas[0..4], &[0, 0, 0, 0]);
    }

    /// The first version of this panicked on a 1x1 surface, which is
    /// exactly what a compositor hands over before the real size is
    /// known. A panic on a lock screen leaves the session locked with
    /// nothing running to unlock it.
    #[test]
    fn the_panel_never_exceeds_the_surface_at_any_size() {
        for (width, height) in [
            (1, 1),
            (2, 2),
            (100, 50),
            (279, 119),
            (280, 120),
            (1280, 800),
            (3840, 2160),
            (u32::MAX, u32::MAX),
        ] {
            let (panel_w, panel_h) = panel_size(width, height);
            assert!(panel_w <= width, "{width}x{height} gave a panel {panel_w} wide");
            assert!(panel_h <= height, "{width}x{height} gave a panel {panel_h} tall");
            // The centring arithmetic that actually overflowed.
            let _ = width.saturating_sub(panel_w) / 2;
            let _ = height.saturating_sub(panel_h) / 2;
        }
    }

    #[test]
    fn filling_writes_the_colour_in_little_endian_argb() {
        let mut canvas = vec![0u8; 4 * 4];
        fill(&mut canvas, 4, 1, 0, 2, 1, 0xffbd93f9);
        assert_eq!(&canvas[4..8], &0xffbd93f9u32.to_le_bytes());
        assert_eq!(&canvas[8..12], &0xffbd93f9u32.to_le_bytes());
        assert_eq!(&canvas[0..4], &[0, 0, 0, 0], "outside the rectangle is untouched");
    }
}
