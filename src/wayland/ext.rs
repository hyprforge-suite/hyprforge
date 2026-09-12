//! `ext-data-control-v1`: the standardised protocol, and the one this
//! crate prefers.
//!
//! Verified on this machine: Hyprland 0.56 answers `wl-paste
//! --list-types`, which is backed by data-control, and upstream's own
//! `wlr-data-control-unstable-v1.xml` says plainly "this protocol is
//! deprecated... use ext-data-control-v1 [instead]". So this module is
//! tried first; `hyprforge_clipboard::wayland::connect` only falls back
//! to `wlr.rs` when a compositor does not advertise this manager at
//! all. The two protocols are otherwise the same requests, the same
//! events, in the same order — `wlr.rs`'s doc comment says more.
//!
//! Deliberately does not track `primary_selection` — see `wlr.rs`.

use crate::resolve::resolve_offer;
use crate::types::{Content, Entry, EntryId, Mime};
use crate::wayland::pipe;
use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::{SystemTime, UNIX_EPOCH};
use tokio::sync::mpsc::UnboundedSender;
use wayland_client::protocol::{wl_registry, wl_seat};
use wayland_client::{event_created_child, Connection, Dispatch, Proxy, QueueHandle};
use wayland_protocols::ext::data_control::v1::client::{
    ext_data_control_device_v1::{self, ExtDataControlDeviceV1},
    ext_data_control_manager_v1::{self, ExtDataControlManagerV1},
    ext_data_control_offer_v1::{self, ExtDataControlOfferV1},
};

/// See `wlr::lock` and `hyprforge-displayd`'s `backend::wlr::lock` — the
/// same "a mutex here only ever guards data replaced wholesale" reasoning.
fn lock<T>(m: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|e| e.into_inner())
}

struct ExtState {
    manager: Option<ExtDataControlManagerV1>,
    seat: Option<wl_seat::WlSeat>,
    pending: HashMap<ExtDataControlOfferV1, Vec<Mime>>,
    current: Option<ExtDataControlOfferV1>,
    subscribers: Arc<Mutex<Vec<UnboundedSender<Entry>>>>,
}

impl Dispatch<wl_registry::WlRegistry, ()> for ExtState {
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
            if interface == ExtDataControlManagerV1::interface().name {
                let bound = version.min(ExtDataControlManagerV1::interface().version);
                state.manager =
                    Some(registry.bind::<ExtDataControlManagerV1, _, _>(name, bound, qh, ()));
            } else if interface == wl_seat::WlSeat::interface().name && state.seat.is_none() {
                let bound = version.min(wl_seat::WlSeat::interface().version);
                state.seat = Some(registry.bind::<wl_seat::WlSeat, _, _>(name, bound, qh, ()));
            }
        }
    }
}

impl Dispatch<wl_seat::WlSeat, ()> for ExtState {
    fn event(
        _: &mut Self,
        _: &wl_seat::WlSeat,
        _: wl_seat::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        // Only bound to satisfy `get_data_device`'s argument.
    }
}

impl Dispatch<ExtDataControlManagerV1, ()> for ExtState {
    fn event(
        _: &mut Self,
        _: &ExtDataControlManagerV1,
        event: ext_data_control_manager_v1::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        // Request-only interface — no events are ever sent.
        let _ = event;
    }
}

impl Dispatch<ExtDataControlDeviceV1, ()> for ExtState {
    fn event(
        state: &mut Self,
        _proxy: &ExtDataControlDeviceV1,
        event: ext_data_control_device_v1::Event,
        _: &(),
        conn: &Connection,
        _qh: &QueueHandle<Self>,
    ) {
        use ext_data_control_device_v1::Event as E;
        match event {
            E::DataOffer { id } => {
                state.pending.insert(id, Vec::new());
            }
            E::Selection { id } => {
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
                tracing::warn!("compositor closed the ext-data-control-v1 device");
                lock(&state.subscribers).clear();
            }
            E::PrimarySelection { .. } => {}
            _ => {}
        }
    }

    event_created_child!(ExtState, ExtDataControlDeviceV1, [
        0 => (ExtDataControlOfferV1, ()),
    ]);
}

impl Dispatch<ExtDataControlOfferV1, ()> for ExtState {
    fn event(
        state: &mut Self,
        proxy: &ExtDataControlOfferV1,
        event: ext_data_control_offer_v1::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        if let ext_data_control_offer_v1::Event::Offer { mime_type } = event {
            state
                .pending
                .entry(proxy.clone())
                .or_default()
                .push(Mime::new(mime_type));
        }
    }
}

fn resolve(offer: &ExtDataControlOfferV1, mimes: &[Mime], conn: &Connection) -> Option<Entry> {
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

/// Connects, binds `ext-data-control-manager-v1` and a seat, and spawns
/// the dispatch thread. Fails fast, leaving nothing running, when the
/// compositor does not advertise this protocol — the caller's cue to
/// try `wlr::connect` instead.
pub fn connect(subscribers: Arc<Mutex<Vec<UnboundedSender<Entry>>>>) -> anyhow::Result<()> {
    let connection = Connection::connect_to_env()?;
    let display = connection.display();
    let mut event_queue = connection.new_event_queue::<ExtState>();
    let qh = event_queue.handle();

    let mut state = ExtState {
        manager: None,
        seat: None,
        pending: HashMap::new(),
        current: None,
        subscribers,
    };

    let _registry = display.get_registry(&qh, ());
    event_queue.roundtrip(&mut state)?;

    let manager = state.manager.clone().ok_or_else(|| {
        anyhow::anyhow!("compositor does not advertise ext-data-control-manager-v1")
    })?;
    let seat = state.seat.clone().ok_or_else(|| {
        anyhow::anyhow!("compositor advertises ext-data-control-manager-v1 but no wl_seat")
    })?;

    let _device = manager.get_data_device(&seat, &qh, ());
    event_queue.roundtrip(&mut state)?;

    std::thread::Builder::new()
        .name("hyprforge-clip-ext".to_string())
        .spawn(move || {
            loop {
                if let Err(e) = event_queue.blocking_dispatch(&mut state) {
                    tracing::error!(error = %e, "ext-data-control-v1 event queue closed; stopping");
                    break;
                }
            }
            lock(&state.subscribers).clear();
        })?;

    Ok(())
}
