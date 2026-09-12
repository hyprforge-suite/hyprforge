//! `zwlr-data-control-v1`: the older wlroots protocol.
//!
//! Kept as a fallback, not the default: upstream itself now says "this
//! protocol is deprecated and not intended for production use... use
//! ext-data-control-v1" (see the copy of the XML in
//! `wayland-protocols-wlr`). `hyprforge_clipboard::wayland::connect`
//! only reaches this module when a compositor does not advertise
//! `ext-data-control-v1` at all — see `ext.rs`, which is otherwise
//! structurally identical to this file because the two protocols are:
//! same requests, same events, same argument shapes, right down to the
//! opcode order the `event_created_child!` mapping below depends on.
//!
//! Deliberately does not track `primary_selection` (the wlroots/X11
//! notion of "whatever text is currently highlighted"): recording every
//! mouse selection as if it were a copy would flood the history with
//! things the user never asked to keep.

use crate::resolve::resolve_offer;
use crate::types::{Content, Entry, EntryId, Mime};
use crate::wayland::pipe;
use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::{SystemTime, UNIX_EPOCH};
use tokio::sync::mpsc::UnboundedSender;
use wayland_client::protocol::{wl_registry, wl_seat};
use wayland_client::{event_created_child, Connection, Dispatch, Proxy, QueueHandle};
use wayland_protocols_wlr::data_control::v1::client::{
    zwlr_data_control_device_v1::{self, ZwlrDataControlDeviceV1},
    zwlr_data_control_manager_v1::{self, ZwlrDataControlManagerV1},
    zwlr_data_control_offer_v1::{self, ZwlrDataControlOfferV1},
};

/// Locks past poisoning, the same reasoning as `hyprforge-displayd`'s
/// `backend::wlr::lock`: every mutex here guards a subscriber list that
/// is replaced/retained wholesale, so one unrelated panic elsewhere is
/// not a reason to also lose the ability to publish clipboard entries.
fn lock<T>(m: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|e| e.into_inner())
}

struct WlrState {
    manager: Option<ZwlrDataControlManagerV1>,
    seat: Option<wl_seat::WlSeat>,
    /// Offers seen via `data_offer` that have not yet been named by a
    /// `selection` event (or have, and are simply not yet cleaned up).
    pending: HashMap<ZwlrDataControlOfferV1, Vec<Mime>>,
    /// The offer currently backing the clipboard, kept only so it can be
    /// destroyed when superseded — required by the protocol ("the client
    /// must destroy the previous selection offer... upon receiving this
    /// event").
    current: Option<ZwlrDataControlOfferV1>,
    subscribers: Arc<Mutex<Vec<UnboundedSender<Entry>>>>,
}

impl Dispatch<wl_registry::WlRegistry, ()> for WlrState {
    fn event(
        state: &mut Self,
        registry: &wl_registry::WlRegistry,
        event: wl_registry::Event,
        _: &(),
        _conn: &Connection,
        qh: &QueueHandle<Self>,
    ) {
        if let wl_registry::Event::Global {
            name,
            interface,
            version,
        } = event
        {
            if interface == ZwlrDataControlManagerV1::interface().name {
                let bound = version.min(ZwlrDataControlManagerV1::interface().version);
                state.manager =
                    Some(registry.bind::<ZwlrDataControlManagerV1, _, _>(name, bound, qh, ()));
            } else if interface == wl_seat::WlSeat::interface().name && state.seat.is_none() {
                // One seat is all a single-seat desktop needs, and this
                // crate has no use for input capabilities beyond it —
                // only `get_data_device` cares which seat.
                let bound = version.min(wl_seat::WlSeat::interface().version);
                state.seat = Some(registry.bind::<wl_seat::WlSeat, _, _>(name, bound, qh, ()));
            }
        }
    }
}

impl Dispatch<wl_seat::WlSeat, ()> for WlrState {
    fn event(
        _: &mut Self,
        _: &wl_seat::WlSeat,
        _: wl_seat::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        // Only bound to satisfy `get_data_device`'s argument; this crate
        // has no interest in keyboards, pointers or seat names.
    }
}

impl Dispatch<ZwlrDataControlManagerV1, ()> for WlrState {
    fn event(
        _: &mut Self,
        _: &ZwlrDataControlManagerV1,
        event: zwlr_data_control_manager_v1::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        // The manager interface defines no events at all (request-only);
        // this impl exists only because `Dispatch` is required for
        // anything bound from the registry.
        let _ = event;
    }
}

impl Dispatch<ZwlrDataControlDeviceV1, ()> for WlrState {
    fn event(
        state: &mut Self,
        _proxy: &ZwlrDataControlDeviceV1,
        event: zwlr_data_control_device_v1::Event,
        _: &(),
        conn: &Connection,
        _qh: &QueueHandle<Self>,
    ) {
        use zwlr_data_control_device_v1::Event as E;
        match event {
            E::DataOffer { id } => {
                state.pending.insert(id, Vec::new());
            }
            E::Selection { id } => {
                // Required by the protocol: destroy whatever the
                // clipboard used to be before adopting what it is now.
                if let Some(previous) = state.current.take() {
                    previous.destroy();
                }
                match id {
                    Some(offer) => {
                        let mimes = state.pending.remove(&offer).unwrap_or_default();
                        if let Some(entry) = resolve(&offer, &mimes, conn) {
                            let mut subs = lock(&state.subscribers);
                            subs.retain(|tx| tx.send(entry.clone()).is_ok());
                        }
                        state.current = Some(offer);
                    }
                    None => state.current = None,
                }
            }
            E::Finished => {
                // The compositor is done talking to us over this
                // device — nothing further will ever arrive. Clearing
                // subscribers is what turns "silently stopped watching"
                // into an observable end of the stream, the same
                // reasoning `hyprforge-displayd`'s `WlrBackend` uses.
                tracing::warn!("compositor closed the zwlr-data-control-v1 device");
                lock(&state.subscribers).clear();
            }
            // Deliberately not tracked — see the module doc comment.
            E::PrimarySelection { .. } => {}
            _ => {}
        }
    }

    event_created_child!(WlrState, ZwlrDataControlDeviceV1, [
        0 => (ZwlrDataControlOfferV1, ()),
    ]);
}

impl Dispatch<ZwlrDataControlOfferV1, ()> for WlrState {
    fn event(
        state: &mut Self,
        proxy: &ZwlrDataControlOfferV1,
        event: zwlr_data_control_offer_v1::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        if let zwlr_data_control_offer_v1::Event::Offer { mime_type } = event {
            state
                .pending
                .entry(proxy.clone())
                .or_default()
                .push(Mime::new(mime_type));
        }
    }
}

/// Consults `resolve_offer` for one now-selected offer, wiring its
/// `receive_mime` closure to this specific offer proxy and a bounded
/// pipe read. `Sensitivity` is checked inside `resolve_offer` before any
/// of these closures is ever called.
fn resolve(offer: &ZwlrDataControlOfferV1, mimes: &[Mime], conn: &Connection) -> Option<Entry> {
    let content = resolve_offer(mimes, |mime| {
        let mime_type = mime.as_str().to_string();
        pipe::receive(conn, |fd| offer.receive(mime_type, fd))
    })?;
    Some(entry_of(content))
}

fn entry_of(content: Content) -> Entry {
    let copied_at = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    Entry {
        id: EntryId::of(&content),
        content,
        copied_at,
        pinned: false,
    }
}

/// Connects, binds `zwlr-data-control-manager-v1` and a seat, and spawns
/// the dispatch thread that turns future selections into entries pushed
/// to `subscribers`.
///
/// Fails fast (without leaving anything running) if the compositor
/// offers neither the manager nor a seat — the caller's cue to report
/// that no clipboard protocol at all is available.
pub fn connect(subscribers: Arc<Mutex<Vec<UnboundedSender<Entry>>>>) -> anyhow::Result<()> {
    let connection = Connection::connect_to_env()?;
    let display = connection.display();
    let mut event_queue = connection.new_event_queue::<WlrState>();
    let qh = event_queue.handle();

    let mut state = WlrState {
        manager: None,
        seat: None,
        pending: HashMap::new(),
        current: None,
        subscribers,
    };

    let _registry = display.get_registry(&qh, ());
    event_queue.roundtrip(&mut state)?;

    let manager = state.manager.clone().ok_or_else(|| {
        anyhow::anyhow!("compositor does not advertise zwlr-data-control-manager-v1")
    })?;
    let seat = state.seat.clone().ok_or_else(|| {
        anyhow::anyhow!("compositor advertises zwlr-data-control-manager-v1 but no wl_seat")
    })?;

    let _device = manager.get_data_device(&seat, &qh, ());
    // The first `selection` event is sent as soon as the device is
    // bound — this second roundtrip is what picks up whatever is
    // already on the clipboard, not just future copies.
    event_queue.roundtrip(&mut state)?;

    std::thread::Builder::new().name("hyprforge-clip-wlr".to_string()).spawn(move || {
        loop {
            if let Err(e) = event_queue.blocking_dispatch(&mut state) {
                tracing::error!(error = %e, "zwlr-data-control-v1 event queue closed; stopping");
                break;
            }
        }
        // Same reasoning as `WlrBackend`'s dispatch thread: dropping
        // `state` here does not drop the `Arc`'d subscriber list the
        // caller may still hold, so clearing it explicitly is what turns
        // a dead dispatch thread into something a caller can notice
        // (every future `subscribe()` still works; it just never fires).
        lock(&state.subscribers).clear();
    })?;

    Ok(())
}
