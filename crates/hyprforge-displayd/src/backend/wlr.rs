//! Real `wlr-output-management-v1` backend.
//!
//! A dedicated OS thread owns the `Connection`/`EventQueue` and runs
//! `blocking_dispatch` in a loop, updating a shared, mutex-protected
//! snapshot on every `done` event. Requests (enabling/disabling/configuring
//! heads) are sent from whichever thread calls [`OutputBackend::apply_configuration`]
//! by cloning proxy handles out of that shared state — wayland-client's
//! `Connection`/proxies support sending requests concurrently with a
//! separate thread dispatching events, so no command channel back into the
//! dispatch thread is needed.

use super::OutputBackend;
use crate::types::{Head, Identity, LayoutPlan, Mode, ModeSpec, Transform, TopologyEvent};
use std::collections::HashMap;
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::mpsc::SyncSender;
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tokio::sync::mpsc::{self, UnboundedReceiver, UnboundedSender};
use wayland_client::protocol::{wl_output, wl_registry};
use wayland_client::{event_created_child, Connection, Dispatch, Proxy, QueueHandle, WEnum};
use wayland_protocols_wlr::output_management::v1::client::{
    zwlr_output_configuration_head_v1::{self, ZwlrOutputConfigurationHeadV1},
    zwlr_output_configuration_v1::{self, ZwlrOutputConfigurationV1},
    zwlr_output_head_v1::{self, ZwlrOutputHeadV1},
    zwlr_output_manager_v1::{self, ZwlrOutputManagerV1},
    zwlr_output_mode_v1::{self, ZwlrOutputModeV1},
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ConfigOutcome {
    Succeeded,
    Failed,
    Cancelled,
}

#[derive(Debug, Clone)]
struct PendingHead {
    connector: String,
    description: String,
    make: String,
    model: String,
    serial: String,
    modes: Vec<ZwlrOutputModeV1>,
    current_mode: Option<ZwlrOutputModeV1>,
    enabled: bool,
    position: (i32, i32),
    transform: wl_output::Transform,
    scale: f64,
    finished: bool,
}

impl Default for PendingHead {
    fn default() -> Self {
        PendingHead {
            connector: String::new(),
            description: String::new(),
            make: String::new(),
            model: String::new(),
            serial: String::new(),
            modes: Vec::new(),
            current_mode: None,
            enabled: false,
            position: (0, 0),
            transform: wl_output::Transform::Normal,
            scale: 1.0,
            finished: false,
        }
    }
}

#[derive(Debug, Default, Clone, Copy)]
struct PendingMode {
    width: i32,
    height: i32,
    refresh: i32,
    preferred: bool,
}

struct WlrState {
    manager: Option<ZwlrOutputManagerV1>,
    heads: HashMap<ZwlrOutputHeadV1, PendingHead>,
    modes: HashMap<ZwlrOutputModeV1, PendingMode>,
    snapshot: Arc<Mutex<Vec<Head>>>,
    connector_proxies: Arc<Mutex<HashMap<String, ZwlrOutputHeadV1>>>,
    subscribers: Arc<Mutex<Vec<UnboundedSender<TopologyEvent>>>>,
    latest_serial: Arc<AtomicU32>,
}

impl WlrState {
    fn build_snapshot(&self) -> Vec<Head> {
        self.heads
            .iter()
            .filter(|(_, h)| !h.finished && !h.connector.is_empty())
            .map(|(_, h)| {
                let modes: Vec<Mode> = h
                    .modes
                    .iter()
                    .filter_map(|m| self.modes.get(m))
                    .map(|pm| Mode {
                        width: pm.width,
                        height: pm.height,
                        refresh_mhz: pm.refresh,
                        preferred: pm.preferred,
                    })
                    .collect();
                let current_mode = h
                    .current_mode
                    .as_ref()
                    .and_then(|cm| self.modes.get(cm))
                    .map(|pm| Mode {
                        width: pm.width,
                        height: pm.height,
                        refresh_mhz: pm.refresh,
                        preferred: pm.preferred,
                    });
                Head {
                    connector: h.connector.clone(),
                    identity: Identity {
                        make: h.make.clone(),
                        model: h.model.clone(),
                        serial: h.serial.clone(),
                    },
                    description: h.description.clone(),
                    modes,
                    current_mode,
                    position: h.position,
                    transform: from_wl_transform(h.transform),
                    scale: h.scale,
                    enabled: h.enabled,
                }
            })
            .collect()
    }
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
            if interface == ZwlrOutputManagerV1::interface().name {
                let bound_version = version.min(ZwlrOutputManagerV1::interface().version);
                let manager = registry.bind::<ZwlrOutputManagerV1, _, _>(name, bound_version, qh, ());
                state.manager = Some(manager);
            }
        }
    }
}

impl Dispatch<ZwlrOutputManagerV1, ()> for WlrState {
    fn event(
        state: &mut Self,
        _proxy: &ZwlrOutputManagerV1,
        event: zwlr_output_manager_v1::Event,
        _: &(),
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
    ) {
        match event {
            zwlr_output_manager_v1::Event::Head { head } => {
                state.heads.insert(head, PendingHead::default());
            }
            zwlr_output_manager_v1::Event::Done { serial } => {
                state.latest_serial.store(serial, Ordering::SeqCst);

                let snapshot = state.build_snapshot();
                let mut connector_proxies = HashMap::new();
                for (proxy, h) in &state.heads {
                    if !h.finished && !h.connector.is_empty() {
                        connector_proxies.insert(h.connector.clone(), proxy.clone());
                    }
                }
                *state.snapshot.lock().unwrap() = snapshot.clone();
                *state.connector_proxies.lock().unwrap() = connector_proxies;

                let mut subs = state.subscribers.lock().unwrap();
                subs.retain(|tx| tx.send(TopologyEvent::Snapshot(snapshot.clone())).is_ok());
                drop(subs);

                // Release resources for anything that's gone.
                let finished_modes: Vec<ZwlrOutputModeV1> = state
                    .modes
                    .keys()
                    .filter(|m| {
                        !state
                            .heads
                            .values()
                            .any(|h| !h.finished && h.modes.contains(m))
                    })
                    .cloned()
                    .collect();
                for mode in finished_modes {
                    if mode.version() >= 3 {
                        mode.release();
                    }
                    state.modes.remove(&mode);
                }
                let finished_heads: Vec<ZwlrOutputHeadV1> = state
                    .heads
                    .iter()
                    .filter(|(_, h)| h.finished)
                    .map(|(p, _)| p.clone())
                    .collect();
                for head in finished_heads {
                    if head.version() >= 3 {
                        head.release();
                    }
                    state.heads.remove(&head);
                }
            }
            zwlr_output_manager_v1::Event::Finished => {
                tracing::warn!("compositor closed the wlr-output-management-v1 manager");
            }
            _ => {}
        }
    }

    event_created_child!(WlrState, ZwlrOutputManagerV1, [
        0 => (ZwlrOutputHeadV1, ()),
    ]);
}

impl Dispatch<ZwlrOutputHeadV1, ()> for WlrState {
    fn event(
        state: &mut Self,
        proxy: &ZwlrOutputHeadV1,
        event: zwlr_output_head_v1::Event,
        _: &(),
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
    ) {
        use zwlr_output_head_v1::Event as E;
        let entry = state.heads.entry(proxy.clone()).or_default();
        match event {
            E::Name { name } => entry.connector = name,
            E::Description { description } => entry.description = description,
            E::PhysicalSize { .. } => {}
            E::Mode { mode } => entry.modes.push(mode),
            E::Enabled { enabled } => entry.enabled = enabled != 0,
            E::CurrentMode { mode } => entry.current_mode = Some(mode),
            E::Position { x, y } => entry.position = (x, y),
            E::Transform {
                transform: WEnum::Value(t),
            } => entry.transform = t,
            E::Transform { .. } => {}
            E::Scale { scale } => entry.scale = scale,
            E::Finished => entry.finished = true,
            E::Make { make } => entry.make = make,
            E::Model { model } => entry.model = model,
            E::SerialNumber { serial_number } => entry.serial = serial_number,
            E::AdaptiveSync { .. } => {}
            _ => {}
        }
    }

    event_created_child!(WlrState, ZwlrOutputHeadV1, [
        3 => (ZwlrOutputModeV1, ()),
    ]);
}

impl Dispatch<ZwlrOutputModeV1, ()> for WlrState {
    fn event(
        state: &mut Self,
        proxy: &ZwlrOutputModeV1,
        event: zwlr_output_mode_v1::Event,
        _: &(),
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
    ) {
        use zwlr_output_mode_v1::Event as E;
        match event {
            E::Size { width, height } => {
                let entry = state.modes.entry(proxy.clone()).or_default();
                entry.width = width;
                entry.height = height;
            }
            E::Refresh { refresh } => {
                state.modes.entry(proxy.clone()).or_default().refresh = refresh;
            }
            E::Preferred => {
                state.modes.entry(proxy.clone()).or_default().preferred = true;
            }
            E::Finished => {
                // Actual removal + release happens in the manager's `Done`
                // handler, once we know no live head still references it.
            }
            _ => {}
        }
    }
}

impl Dispatch<ZwlrOutputConfigurationV1, SyncSender<ConfigOutcome>> for WlrState {
    fn event(
        _state: &mut Self,
        _proxy: &ZwlrOutputConfigurationV1,
        event: zwlr_output_configuration_v1::Event,
        data: &SyncSender<ConfigOutcome>,
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
    ) {
        use zwlr_output_configuration_v1::Event as E;
        let outcome = match event {
            E::Succeeded => ConfigOutcome::Succeeded,
            E::Failed => ConfigOutcome::Failed,
            E::Cancelled => ConfigOutcome::Cancelled,
            _ => return,
        };
        let _ = data.send(outcome);
    }
}

impl Dispatch<ZwlrOutputConfigurationHeadV1, ()> for WlrState {
    fn event(
        _state: &mut Self,
        _proxy: &ZwlrOutputConfigurationHeadV1,
        event: zwlr_output_configuration_head_v1::Event,
        _: &(),
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
    ) {
        let _ = event;
    }
}

fn from_wl_transform(t: wl_output::Transform) -> Transform {
    match t {
        wl_output::Transform::Normal => Transform::Normal,
        wl_output::Transform::_90 => Transform::Rotate90,
        wl_output::Transform::_180 => Transform::Rotate180,
        wl_output::Transform::_270 => Transform::Rotate270,
        wl_output::Transform::Flipped => Transform::Flipped,
        wl_output::Transform::Flipped90 => Transform::Flipped90,
        wl_output::Transform::Flipped180 => Transform::Flipped180,
        wl_output::Transform::Flipped270 => Transform::Flipped270,
        _ => Transform::Normal,
    }
}

fn to_wl_transform(t: Transform) -> wl_output::Transform {
    match t {
        Transform::Normal => wl_output::Transform::Normal,
        Transform::Rotate90 => wl_output::Transform::_90,
        Transform::Rotate180 => wl_output::Transform::_180,
        Transform::Rotate270 => wl_output::Transform::_270,
        Transform::Flipped => wl_output::Transform::Flipped,
        Transform::Flipped90 => wl_output::Transform::Flipped90,
        Transform::Flipped180 => wl_output::Transform::Flipped180,
        Transform::Flipped270 => wl_output::Transform::Flipped270,
    }
}

pub struct WlrBackend {
    connection: Connection,
    qh: QueueHandle<WlrState>,
    manager: ZwlrOutputManagerV1,
    latest_serial: Arc<AtomicU32>,
    snapshot: Arc<Mutex<Vec<Head>>>,
    connector_proxies: Arc<Mutex<HashMap<String, ZwlrOutputHeadV1>>>,
    subscribers: Arc<Mutex<Vec<UnboundedSender<TopologyEvent>>>>,
}

impl WlrBackend {
    /// Connects to the compositor on `$WAYLAND_DISPLAY`, binds
    /// `wlr-output-management-v1`, and spawns the dedicated dispatch
    /// thread. Fails fast if the compositor doesn't support the protocol —
    /// callers should surface this as a clear, actionable startup error,
    /// not retry silently.
    pub fn connect() -> anyhow::Result<Self> {
        let connection = Connection::connect_to_env()?;
        let display = connection.display();
        let mut event_queue = connection.new_event_queue::<WlrState>();
        let qh = event_queue.handle();

        let snapshot = Arc::new(Mutex::new(Vec::new()));
        let connector_proxies = Arc::new(Mutex::new(HashMap::new()));
        let subscribers: Arc<Mutex<Vec<UnboundedSender<TopologyEvent>>>> =
            Arc::new(Mutex::new(Vec::new()));
        let latest_serial = Arc::new(AtomicU32::new(0));

        let mut state = WlrState {
            manager: None,
            heads: HashMap::new(),
            modes: HashMap::new(),
            snapshot: snapshot.clone(),
            connector_proxies: connector_proxies.clone(),
            subscribers: subscribers.clone(),
            latest_serial: latest_serial.clone(),
        };

        let _registry = display.get_registry(&qh, ());
        event_queue.roundtrip(&mut state)?;

        let manager = state.manager.clone().ok_or_else(|| {
            anyhow::anyhow!(
                "compositor does not advertise wlr-output-management-v1; \
                 hyprforge-displayd requires a wlroots-based (or Hyprland) compositor"
            )
        })?;

        // Second roundtrip: receive the initial head/mode burst and the
        // `done` event that follows it.
        event_queue.roundtrip(&mut state)?;

        std::thread::Builder::new()
            .name("hyprforge-wlr-output".to_string())
            .spawn(move || loop {
                if let Err(e) = event_queue.blocking_dispatch(&mut state) {
                    tracing::error!(error = %e, "wlr-output-management-v1 event queue closed; stopping dispatch thread");
                    break;
                }
            })?;

        Ok(WlrBackend {
            connection,
            qh,
            manager,
            latest_serial,
            snapshot,
            connector_proxies,
            subscribers,
        })
    }

    fn resolve_mode(&self, connector: &str, spec: ModeSpec) -> Option<(i32, i32, i32)> {
        match spec {
            ModeSpec::Exact {
                width,
                height,
                refresh_mhz,
            } => Some((width, height, refresh_mhz)),
            ModeSpec::Preferred => {
                let snapshot = self.snapshot.lock().unwrap();
                snapshot
                    .iter()
                    .find(|h| h.connector == connector)
                    .and_then(|h| h.preferred_mode())
                    .map(|m| (m.width, m.height, m.refresh_mhz))
            }
        }
    }

    fn build_configuration(
        &self,
        serial: u32,
        plan: &LayoutPlan,
        tx: SyncSender<ConfigOutcome>,
    ) -> anyhow::Result<ZwlrOutputConfigurationV1> {
        let config = self.manager.create_configuration(serial, &self.qh, tx);
        let proxies = self.connector_proxies.lock().unwrap();
        for head_plan in &plan.heads {
            let head_proxy = proxies.get(&head_plan.connector).ok_or_else(|| {
                anyhow::anyhow!("unknown connector in layout plan: {}", head_plan.connector)
            })?;
            if head_plan.enabled {
                let config_head = config.enable_head(head_proxy, &self.qh, ());
                if let Some(spec) = head_plan.mode {
                    if let Some((w, h, r)) = self.resolve_mode(&head_plan.connector, spec) {
                        config_head.set_custom_mode(w, h, r);
                    }
                }
                config_head.set_position(head_plan.position.0, head_plan.position.1);
                config_head.set_transform(to_wl_transform(head_plan.transform));
                config_head.set_scale(head_plan.scale);
            } else {
                config.disable_head(head_proxy);
            }
        }
        Ok(config)
    }

    /// Runs one test-then-apply round for `plan` against `serial`, waiting
    /// up to 3s for each compositor reply. Returns `Ok(true)` on success,
    /// `Ok(false)` if the compositor cancelled (caller should retry with a
    /// fresh serial), or `Err` on an outright rejection.
    fn try_apply_once(&self, plan: &LayoutPlan) -> anyhow::Result<bool> {
        let serial = self.latest_serial.load(Ordering::SeqCst);
        let (tx, rx) = std::sync::mpsc::sync_channel(1);
        let test_config = self.build_configuration(serial, plan, tx)?;
        test_config.test();
        self.connection.flush()?;
        let outcome = rx
            .recv_timeout(Duration::from_secs(3))
            .map_err(|_| anyhow::anyhow!("timed out waiting for compositor to test configuration"))?;
        test_config.destroy();
        match outcome {
            ConfigOutcome::Failed => {
                anyhow::bail!("compositor rejected the configuration (test failed)")
            }
            ConfigOutcome::Cancelled => return Ok(false),
            ConfigOutcome::Succeeded => {}
        }

        let serial = self.latest_serial.load(Ordering::SeqCst);
        let (tx, rx) = std::sync::mpsc::sync_channel(1);
        let apply_config = self.build_configuration(serial, plan, tx)?;
        apply_config.apply();
        self.connection.flush()?;
        let outcome = rx
            .recv_timeout(Duration::from_secs(3))
            .map_err(|_| anyhow::anyhow!("timed out waiting for compositor to apply configuration"))?;
        apply_config.destroy();
        match outcome {
            ConfigOutcome::Succeeded => Ok(true),
            ConfigOutcome::Failed => {
                anyhow::bail!("compositor rejected the configuration (apply failed)")
            }
            ConfigOutcome::Cancelled => Ok(false),
        }
    }
}

impl OutputBackend for WlrBackend {
    fn list_outputs(&self) -> anyhow::Result<Vec<Head>> {
        Ok(self.snapshot.lock().unwrap().clone())
    }

    fn apply_configuration(&self, plan: &LayoutPlan) -> anyhow::Result<()> {
        plan.validate()?;
        const MAX_ATTEMPTS: u32 = 3;
        for attempt in 1..=MAX_ATTEMPTS {
            // A `cancelled` response means another client raced us (or the
            // topology itself changed) between our snapshot and this
            // request — normal contention, not a terminal failure. Re-read
            // current state (the dispatch thread keeps `latest_serial` and
            // `connector_proxies` current) and retry with a fresh serial.
            match self.try_apply_once(plan) {
                Ok(true) => return Ok(()),
                Ok(false) => {
                    tracing::warn!(attempt, "configuration cancelled by compositor; retrying");
                    continue;
                }
                Err(e) => return Err(e),
            }
        }
        anyhow::bail!(
            "gave up applying display configuration after {MAX_ATTEMPTS} attempts \
             (compositor kept cancelling — topology may be changing rapidly)"
        )
    }

    fn subscribe(&self) -> UnboundedReceiver<TopologyEvent> {
        let (tx, rx) = mpsc::unbounded_channel();
        let _ = tx.send(TopologyEvent::Snapshot(self.snapshot.lock().unwrap().clone()));
        self.subscribers.lock().unwrap().push(tx);
        rx
    }
}
