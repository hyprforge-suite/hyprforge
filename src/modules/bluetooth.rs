//! Bluetooth: the adapter, discovery, and the device list — over BlueZ.
//!
//! Same shape as `modules::network`: everything that talks to BlueZ lives
//! in `hyprforge-bluetooth` already, this module is the screen on top of
//! it, and it is generic over [`BluetoothBackend`] so the tests below can
//! drive a [`MockBackend`] while `main.rs` drives the real one.

use hyprforge_bluetooth::backend::{for_display, BluetoothBackend};
use hyprforge_bluetooth::{Address, AdapterState, BluetoothError, Device, Status};
use hyprforge_tray::Prefs as TrayPrefs;
use hyprforge_ui::theme::{self, spacing, FontScale};
use hyprforge_ui::widgets::{divider, meta_text, scaled_text, secondary_button, section};
use iced::widget::{checkbox, column, row};
use iced::{Alignment, Element, Length, Subscription, Task};
use std::sync::Arc;
use std::time::Duration;

use crate::module::SettingsModule;

/// How often the screen re-polls status and devices while it's open, so a
/// device that appeared, connected, or disconnected elsewhere shows up
/// without the user hitting a refresh button. Same interval as the Network
/// screen; independent of [`Message::ScanToggled`], which is the one thing
/// this poll must never touch — see the comment on that variant.
const POLL_INTERVAL: Duration = Duration::from_secs(10);

/// A load-time failure, reduced to what the screen needs: text to show,
/// and whether it's the one kind of failure ([`BluetoothError::Unavailable`])
/// that must never be confused with an empty device list.
///
/// `BluetoothError` itself isn't `Clone` — this is the boundary where a
/// borrowed error becomes an owned value a `Message` can carry, same as
/// `network::LoadError`.
#[derive(Debug, Clone)]
pub struct LoadError {
    message: String,
    unavailable: bool,
}

impl From<BluetoothError> for LoadError {
    fn from(e: BluetoothError) -> Self {
        LoadError {
            unavailable: matches!(e, BluetoothError::Unavailable),
            message: e.to_string(),
        }
    }
}

/// The result of one refresh: two independent calls, two independent
/// outcomes. A failure in `devices` must not discard a successful
/// `status`, and vice versa.
#[derive(Debug, Clone)]
pub struct Loaded {
    status: Result<Status, LoadError>,
    devices: Result<Vec<Device>, LoadError>,
}

async fn load<B: BluetoothBackend + ?Sized>(backend: Arc<B>) -> Loaded {
    Loaded {
        status: backend.status().await.map_err(LoadError::from),
        devices: backend.devices().await.map_err(LoadError::from),
    }
}

/// Whether the adapter row gets a control that can actually do something.
///
/// `AdapterState::HardwareBlocked` means a rfkill switch has to move — no
/// D-Bus call changes that — so this is `false` there and nowhere else.
/// `AdapterState::Changing` also gets no toggle: the state is mid-flight,
/// and a control offered while it's unknown which way it will land just
/// invites a second click that races the first.
///
/// Pulled out as its own function, rather than left as an inline match arm
/// in `adapter_row`, for the same reason `network::radio_offers_toggle`
/// is: the property is something a test can assert directly.
fn adapter_offers_toggle(state: Option<AdapterState>) -> bool {
    matches!(state, Some(AdapterState::On) | Some(AdapterState::Off))
}

#[derive(Debug, Clone)]
pub enum Message {
    Refresh,
    Loaded(Loaded),
    AdapterToggled(bool),
    AdapterSet(Result<(), LoadError>),
    /// Discovery, started or stopped. This is the *only* place a discovery
    /// call is made — never from `Refresh`, and never from entering the
    /// screen (see [`BluetoothModule::new`]). Discovery costs airtime and
    /// battery on both ends, and `hyprforge-bluetooth::BluetoothBackend`
    /// already documents why it's a separate call from listing devices;
    /// starting it implicitly here would quietly turn every ten-second
    /// poll, and every app launch, into a radio-on event the user never
    /// asked for.
    ScanToggled(bool),
    ScanSet(Result<(), LoadError>),
    ConnectPressed(Address),
    Connected(Address, Result<(), LoadError>),
    DisconnectPressed(Address),
    Disconnected(Address, Result<(), LoadError>),
    ForgetPressed(Address),
    Forgotten(Address, Result<(), LoadError>),
    /// Trusting a device is what lets it reconnect on its own — a headset
    /// that comes back when it's switched on rather than needing a click
    /// here every time. See `BluetoothBackend::set_trusted`.
    TrustToggled(Address, bool),
    /// Carries the target `trusted` value along with the result: BlueZ's
    /// reply is just success/failure, and the row needs to know what it
    /// asked for in order to show the outcome without waiting on the next
    /// poll to re-read it.
    Trusted(Address, bool, Result<(), LoadError>),
    /// Whether `hyprforge-trayd` should show the Bluetooth icon. Same
    /// shape as `network::Message::TrayToggled` — only reachable from a
    /// loaded [`TrayPrefs`], guarded again in `update`.
    TrayToggled(bool),
}

pub struct BluetoothModule<B: BluetoothBackend + 'static> {
    backend: Arc<B>,
    /// `true` until the first [`Message::Loaded`] lands, so the screen can
    /// say "loading" instead of "no devices" while the first round trip is
    /// still in flight.
    loading: bool,
    /// Set only from [`BluetoothError::Unavailable`], and rendered as its
    /// own state rather than folded into `devices` being empty — the
    /// distinction `hyprforge-bluetooth` was written to keep.
    unavailable: Option<String>,
    /// Anything else that went wrong: a connect that failed, a forget that
    /// didn't take. Cleared on the next successful action, not on every
    /// refresh, so it doesn't flash away before it's been read.
    error: Option<String>,
    status: Option<Status>,
    /// Already grouped and sorted by [`for_display`] — this module does
    /// not re-implement that.
    devices: Vec<Device>,
    /// The on-disk `tray.toml`, loaded once at construction. Same shape
    /// and same reasoning as `network::NetworkModule::tray_prefs`: kept
    /// as the whole [`TrayPrefs`] so a toggle here can write back
    /// `network` unchanged, and `Err` (a file that exists and would not
    /// parse) renders no checkbox at all rather than guessing a value.
    tray_prefs: Result<TrayPrefs, String>,
}

impl<B: BluetoothBackend + 'static> BluetoothModule<B> {
    /// Builds the module around an already-usable backend and kicks off
    /// the first load.
    ///
    /// The first load is `status` and `devices` only — never
    /// `set_discovery`. Entering the screen must not itself start a scan;
    /// that is `Message::ScanToggled`'s alone to do, from an explicit
    /// click.
    pub fn new(backend: Arc<B>) -> (Self, Task<Message>) {
        // A missing file is first run and loads as defaults; a file that
        // exists and will not parse is reported here, in the same banner
        // every other load failure on this screen uses, rather than
        // silently treated as "both icons shown".
        let (tray_prefs, tray_error) = match hyprforge_tray::prefs::load() {
            Ok(prefs) => (Ok(prefs), None),
            Err(e) => (Err(e.to_string()), Some(e.to_string())),
        };
        let module = BluetoothModule {
            backend,
            loading: true,
            unavailable: None,
            error: tray_error,
            status: None,
            devices: Vec::new(),
            tray_prefs,
        };
        let task = Task::perform(load(Arc::clone(&module.backend)), Message::Loaded);
        (module, task)
    }

    fn refresh_task(&self) -> Task<Message> {
        Task::perform(load(Arc::clone(&self.backend)), Message::Loaded)
    }
}

impl<B: BluetoothBackend + 'static> SettingsModule for BluetoothModule<B> {
    type Message = Message;

    fn icon(&self) -> &'static str {
        // There is no standard Unicode Bluetooth glyph — the logo is a
        // registered trademark symbol, not a codepoint — so this picks a
        // color distinct from every other sidebar icon rather than reusing
        // Network's 📶, which would read as the same screen twice.
        "\u{1F535}" // 🔵
    }

    fn update(&mut self, message: Message) -> Task<Message> {
        match message {
            Message::Refresh => {
                self.loading = self.status.is_none() && self.unavailable.is_none();
                self.refresh_task()
            }
            Message::Loaded(result) => {
                self.loading = false;
                // Either call can be the one that notices bluetoothd is
                // gone — the mock fails both at once, the real backend
                // fails whichever it was mid-call on. Whichever it is, it
                // wins: an empty device list must never stand in for this.
                let unavailable_msg = [result.status.as_ref().err(), result.devices.as_ref().err()]
                    .into_iter()
                    .flatten()
                    .find(|e| e.unavailable)
                    .map(|e| e.message.clone());

                if let Some(msg) = unavailable_msg {
                    self.unavailable = Some(msg);
                    self.status = None;
                    self.devices.clear();
                    return Task::none();
                }
                self.unavailable = None;
                match result.status {
                    Ok(status) => self.status = Some(status),
                    Err(e) => self.error = Some(e.message),
                }
                match result.devices {
                    Ok(devices) => self.devices = for_display(devices),
                    Err(e) => self.error = Some(e.message),
                }
                Task::none()
            }
            Message::AdapterToggled(on) => {
                // A blocked or already-changing adapter never reaches this
                // arm because the view renders no toggle for either state
                // — see `adapter_row` — but the check stays here too, so a
                // stray message can never dispatch a call that can only
                // fail or race.
                if !adapter_offers_toggle(self.status.as_ref().map(|s| s.state)) {
                    return Task::none();
                }
                let backend = Arc::clone(&self.backend);
                Task::perform(
                    async move { backend.set_powered(on).await.map_err(LoadError::from) },
                    Message::AdapterSet,
                )
            }
            Message::AdapterSet(Ok(())) => self.refresh_task(),
            Message::AdapterSet(Err(e)) => {
                self.error = Some(e.message);
                Task::none()
            }
            Message::ScanToggled(on) => {
                let backend = Arc::clone(&self.backend);
                Task::perform(
                    async move { backend.set_discovery(on).await.map_err(LoadError::from) },
                    Message::ScanSet,
                )
            }
            Message::ScanSet(Ok(())) => self.refresh_task(),
            Message::ScanSet(Err(e)) => {
                self.error = Some(e.message);
                Task::none()
            }
            Message::ConnectPressed(address) => {
                // An unpaired device is listed but its row has no Connect
                // button wired to it — see `device_row` — so a stray
                // message here still must not start a connection that can
                // only fail. Same guard `network.rs` keeps for enterprise
                // Wi-Fi.
                let Some(device) = self.devices.iter().find(|d| d.address == address) else {
                    return Task::none();
                };
                if device.unsupported_reason().is_some() {
                    return Task::none();
                }
                self.error = None;
                let backend = Arc::clone(&self.backend);
                let for_result = address.clone();
                Task::perform(
                    async move { backend.connect(&address).await.map_err(LoadError::from) },
                    move |result| Message::Connected(for_result.clone(), result),
                )
            }
            Message::Connected(address, Ok(())) => {
                self.error = None;
                // Reflected immediately rather than waiting on the refresh
                // that follows: the round trip that just succeeded is
                // itself proof of the new state, and ten seconds is a long
                // time for a row to keep reading "disconnected" about a
                // connection the user watched succeed.
                if let Some(d) = self.devices.iter_mut().find(|d| d.address == address) {
                    d.connected = true;
                }
                self.refresh_task()
            }
            Message::Connected(_, Err(e)) => {
                self.error = Some(e.message);
                Task::none()
            }
            Message::DisconnectPressed(address) => {
                self.error = None;
                let backend = Arc::clone(&self.backend);
                let for_result = address.clone();
                Task::perform(
                    async move { backend.disconnect(&address).await.map_err(LoadError::from) },
                    move |result| Message::Disconnected(for_result.clone(), result),
                )
            }
            Message::Disconnected(address, Ok(())) => {
                if let Some(d) = self.devices.iter_mut().find(|d| d.address == address) {
                    d.connected = false;
                }
                self.refresh_task()
            }
            Message::Disconnected(_, Err(e)) => {
                self.error = Some(e.message);
                Task::none()
            }
            Message::ForgetPressed(address) => {
                self.error = None;
                let backend = Arc::clone(&self.backend);
                let for_result = address.clone();
                Task::perform(
                    async move { backend.forget(&address).await.map_err(LoadError::from) },
                    move |result| Message::Forgotten(for_result.clone(), result),
                )
            }
            Message::Forgotten(address, Ok(())) => {
                self.devices.retain(|d| d.address != address);
                Task::none()
            }
            Message::Forgotten(_, Err(e)) => {
                self.error = Some(e.message);
                Task::none()
            }
            Message::TrustToggled(address, trusted) => {
                self.error = None;
                let backend = Arc::clone(&self.backend);
                let for_result = address.clone();
                Task::perform(
                    async move {
                        backend.set_trusted(&address, trusted).await.map_err(LoadError::from)
                    },
                    move |result| Message::Trusted(for_result.clone(), trusted, result),
                )
            }
            Message::Trusted(address, trusted, Ok(())) => {
                self.error = None;
                if let Some(d) = self.devices.iter_mut().find(|d| d.address == address) {
                    d.trusted = trusted;
                }
                self.refresh_task()
            }
            Message::Trusted(_, _, Err(e)) => {
                self.error = Some(e.message);
                Task::none()
            }
            Message::TrayToggled(shown) => {
                let Ok(prefs) = &self.tray_prefs else {
                    // `tray_row` renders no checkbox while `tray_prefs` is
                    // `Err`, so a stray message here still must not turn
                    // an unreadable file into a freshly-written default.
                    return Task::none();
                };
                let mut updated = *prefs;
                updated.bluetooth = shown;
                match hyprforge_tray::prefs::save(&updated) {
                    Ok(()) => self.tray_prefs = Ok(updated),
                    Err(e) => self.error = Some(e.to_string()),
                }
                Task::none()
            }
        }
    }

    fn view(&self, scale: FontScale) -> Element<'_, Message> {
        let mut content = column![row![
            scaled_text("Bluetooth", 22.0, scale).width(Length::Fill),
            secondary_button("Refresh").on_press(Message::Refresh),
        ]
        .spacing(spacing::SM)
        .align_y(Alignment::Center)]
        .spacing(spacing::LG);

        if let Some(msg) = &self.error {
            content = content.push(scaled_text(msg.clone(), 13.0, scale).color(theme::warning()));
        }

        // Unavailable is a dead end, not a section among sections: there is
        // nothing else useful to show underneath "Bluetooth isn't
        // running", so the rest of the screen doesn't render at all —
        // same call `network.rs` makes for NetworkManager being gone.
        if let Some(msg) = &self.unavailable {
            // The tray toggle stays reachable here. It is the one control
            // on this screen that does not depend on the daemon being up
            // — and it is wanted most when the daemon is down, because
            // that is exactly when the tray icon is sitting there showing
            // an error nobody can currently do anything about. Hiding the
            // switch that turns it off is its own small dead end.
            content = content.push(section(
                "Bluetooth",
                scale,
                column![scaled_text(msg.clone(), 14.0, scale), self.tray_row(scale)]
                    .spacing(spacing::SM),
            ));
            return content.into();
        }

        if self.loading {
            content = content.push(meta_text("Loading…", 14.0, scale));
            return content.into();
        }

        content = content.push(self.adapter_row(scale));

        let adapter_on = matches!(self.status.as_ref().map(|s| s.state), Some(AdapterState::On));
        if adapter_on {
            content = content.push(self.scan_row(scale));
            content = content.push(self.devices_section(scale));
        }

        content.into()
    }

    fn subscription(&self) -> Subscription<Message> {
        if self.unavailable.is_some() {
            return Subscription::none();
        }
        iced::time::every(POLL_INTERVAL).map(|_| Message::Refresh)
    }
}

impl<B: BluetoothBackend + 'static> BluetoothModule<B> {
    /// The Bluetooth on/off row.
    ///
    /// `AdapterState::HardwareBlocked` gets no toggle at all: a rfkill
    /// switch is what has to move, and a control that dispatches a D-Bus
    /// call which can't possibly change anything is worse than no control.
    /// `AdapterState::Changing` gets no toggle either, shown as busy
    /// instead — flicking a checkbox while the real state is still
    /// mid-transition is how it snaps back under the pointer.
    fn adapter_row(&self, scale: FontScale) -> Element<'_, Message> {
        let state = self.status.as_ref().map(|s| s.state);
        debug_assert_eq!(
            matches!(state, Some(AdapterState::HardwareBlocked) | Some(AdapterState::Changing)),
            !adapter_offers_toggle(state),
            "the toggle arm and the no-toggle arms below must stay in sync with this helper",
        );
        let body: Element<'_, Message> = match state {
            Some(AdapterState::HardwareBlocked) => column![
                scaled_text("Bluetooth is off", 15.0, scale),
                meta_text(
                    "A physical switch or Fn key is blocking the radio. \
                     Hyprforge can't turn it back on from here.",
                    13.0,
                    scale,
                ),
            ]
            .spacing(spacing::XS)
            .into(),
            Some(AdapterState::Changing) => meta_text("Bluetooth is changing…", 14.0, scale).into(),
            Some(state) => {
                let on = state == AdapterState::On;
                row![
                    checkbox(on).on_toggle(Message::AdapterToggled),
                    scaled_text("Bluetooth", 15.0, scale),
                ]
                .spacing(spacing::SM)
                .align_y(Alignment::Center)
                .into()
            }
            None => meta_text("Bluetooth status unknown.", 14.0, scale).into(),
        };
        section("Bluetooth", scale, column![body, self.tray_row(scale)].spacing(spacing::SM))
    }

    /// The "show in tray" row, appended to the Bluetooth section. Same
    /// shape and same reasoning as `network::NetworkModule::tray_row`.
    fn tray_row(&self, scale: FontScale) -> Element<'_, Message> {
        match &self.tray_prefs {
            Ok(prefs) => row![
                checkbox(prefs.bluetooth).on_toggle(Message::TrayToggled),
                scaled_text("Show in tray", 15.0, scale),
            ]
            .spacing(spacing::SM)
            .align_y(Alignment::Center)
            .into(),
            Err(_) => meta_text(
                "Tray setting unavailable — see the error above.",
                13.0,
                scale,
            )
            .into(),
        }
    }

    /// Discovery: an explicit toggle, never implied by a refresh or by
    /// opening the screen. See the comment on [`Message::ScanToggled`].
    fn scan_row(&self, scale: FontScale) -> Element<'_, Message> {
        let discovering = self.status.as_ref().is_some_and(|s| s.discovering);
        section(
            "Scan",
            scale,
            row![
                checkbox(discovering).on_toggle(Message::ScanToggled),
                scaled_text(
                    if discovering { "Scanning for devices…" } else { "Scan for devices" },
                    15.0,
                    scale,
                ),
            ]
            .spacing(spacing::SM)
            .align_y(Alignment::Center),
        )
    }

    fn devices_section(&self, scale: FontScale) -> Element<'_, Message> {
        if self.devices.is_empty() {
            return section(
                "Devices",
                scale,
                meta_text("No devices yet. Turn on Scan to look for nearby ones.", 14.0, scale),
            );
        }
        let mut list = column![].spacing(spacing::SM);
        for (i, device) in self.devices.iter().enumerate() {
            if i > 0 {
                list = list.push(divider());
            }
            list = list.push(self.device_row(device, scale));
        }
        section("Devices", scale, list)
    }

    fn device_row<'a>(&'a self, device: &'a Device, scale: FontScale) -> Element<'a, Message> {
        // A device that has never told us its name shows its address, and
        // presenting that as though it were a name is worse than saying
        // plainly that no name is known yet — same reasoning `Device` docs
        // give for keeping `alias` and `name` separate.
        let name: Element<'_, Message> = if device.is_unnamed() {
            meta_text(format!("Unnamed device ({})", device.address), 14.0, scale).into()
        } else {
            scaled_text(device.alias.clone(), 14.0, scale).into()
        };

        let connection = if device.connected { " \u{b7} Connected" } else { "" };
        let mut line = column![
            name,
            meta_text(format!("{}{}", device.kind.label(), connection), 12.0, scale),
        ]
        .spacing(2.0);

        // Unpaired devices are listed but never get a Connect button — the
        // reason is the row's own second line instead, exactly how
        // `network.rs` handles enterprise Wi-Fi, so a user scanning the
        // list sees why without clicking first and being told.
        if let Some(reason) = device.unsupported_reason() {
            line = line.push(meta_text(reason, 12.0, scale));
            return row![line.width(Length::Fill)]
                .spacing(spacing::SM)
                .align_y(Alignment::Center)
                .into();
        }

        let connect_action: Element<'_, Message> = if device.connected {
            secondary_button("Disconnect")
                .on_press(Message::DisconnectPressed(device.address.clone()))
                .into()
        } else {
            secondary_button("Connect")
                .on_press(Message::ConnectPressed(device.address.clone()))
                .into()
        };

        let actions = column![
            row![connect_action, secondary_button("Forget").on_press(Message::ForgetPressed(device.address.clone()))]
                .spacing(spacing::SM),
            row![
                checkbox(device.trusted)
                    .on_toggle({
                        let address = device.address.clone();
                        move |v| Message::TrustToggled(address.clone(), v)
                    }),
                meta_text("Trust (reconnect automatically)", 12.0, scale),
            ]
            .spacing(spacing::SM)
            .align_y(Alignment::Center),
        ]
        .spacing(spacing::XS);

        row![line.width(Length::Fill), actions]
            .spacing(spacing::SM)
            .align_y(Alignment::Center)
            .into()
    }
}

/// The real backend, connected lazily.
///
/// `BlueZBackend::connect()` is async and fallible (no system bus, no
/// `bluetoothd`), but `App::new` — like every module's — builds its
/// screens synchronously and hands back a `Task` for anything that has to
/// wait. Wrapping the connection behind [`BluetoothBackend`] itself means
/// `BluetoothModule` never needs an `Option` for "not connected yet":
/// connecting is just what the first call does.
pub struct LazyBlueZBackend {
    /// Only a *successful* connection is cached.
    ///
    /// This is the one rule `network::LazyNetworkManagerBackend`'s doc
    /// comment exists to keep, copied here rather than re-derived: caching
    /// a *failed* connect means `BluetoothError::Unavailable` — which
    /// tells the user to run `systemctl start bluetooth` — keeps being
    /// shown after they do exactly that, because nothing ever retries the
    /// connection that failed once. The ten-second refresh runs the whole
    /// time and cannot help, because it's calling through the same cached
    /// failure. A dead end whose exit is printed on it is worse than one
    /// without.
    inner: tokio::sync::Mutex<Option<Arc<hyprforge_bluetooth::BlueZBackend>>>,
}

impl LazyBlueZBackend {
    pub fn new() -> Self {
        LazyBlueZBackend {
            inner: tokio::sync::Mutex::new(None),
        }
    }

    /// The lock is held across the connect so that a burst of calls — the
    /// refresh tick fires status and devices together — opens one bus
    /// connection rather than two.
    async fn get(&self) -> Result<Arc<hyprforge_bluetooth::BlueZBackend>, BluetoothError> {
        let mut slot = self.inner.lock().await;
        if let Some(backend) = slot.as_ref() {
            return Ok(backend.clone());
        }
        let backend = Arc::new(hyprforge_bluetooth::BlueZBackend::connect().await?);
        *slot = Some(backend.clone());
        Ok(backend)
    }
}

impl Default for LazyBlueZBackend {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait::async_trait]
impl BluetoothBackend for LazyBlueZBackend {
    async fn status(&self) -> Result<Status, BluetoothError> {
        self.get().await?.status().await
    }

    async fn devices(&self) -> Result<Vec<Device>, BluetoothError> {
        self.get().await?.devices().await
    }

    async fn set_discovery(&self, on: bool) -> Result<(), BluetoothError> {
        self.get().await?.set_discovery(on).await
    }

    async fn set_powered(&self, on: bool) -> Result<(), BluetoothError> {
        self.get().await?.set_powered(on).await
    }

    async fn connect(&self, address: &Address) -> Result<(), BluetoothError> {
        self.get().await?.connect(address).await
    }

    async fn disconnect(&self, address: &Address) -> Result<(), BluetoothError> {
        self.get().await?.disconnect(address).await
    }

    async fn set_trusted(&self, address: &Address, trusted: bool) -> Result<(), BluetoothError> {
        self.get().await?.set_trusted(address, trusted).await
    }

    async fn forget(&self, address: &Address) -> Result<(), BluetoothError> {
        self.get().await?.forget(address).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use hyprforge_bluetooth::backend::mock::MockBackend;
    use hyprforge_bluetooth::DeviceKind;

    fn device(alias: &str, addr: &str, paired: bool, connected: bool) -> Device {
        Device {
            address: Address::new(addr),
            alias: alias.to_string(),
            name: Some(alias.to_string()),
            kind: DeviceKind::Headset,
            paired,
            trusted: false,
            connected,
            rssi: Some(-55),
        }
    }

    fn status(state: AdapterState, discovering: bool) -> Status {
        Status { state, discovering, alias: "mock-adapter".to_string() }
    }

    fn loaded(status: Status, devices: Vec<Device>) -> Loaded {
        Loaded { status: Ok(status), devices: Ok(devices) }
    }

    /// Builds a module with a fresh `MockBackend`, returning both — the
    /// backend is kept so a test can inspect what the module actually
    /// called it with, which a synchronous `update()` call can't show by
    /// itself.
    fn module() -> (BluetoothModule<MockBackend>, Arc<MockBackend>) {
        let backend = Arc::new(MockBackend::new());
        let (module, _task) = BluetoothModule::new(Arc::clone(&backend));
        (module, backend)
    }

    /// Runs `f` with `$XDG_CONFIG_HOME` repointed at a throwaway
    /// directory, holding `CONFIG_ENV_LOCK` for the duration — same
    /// reasoning as `network::tests::with_temp_config`, whose sibling
    /// this is: the variable is process-global, so the two must not race.
    fn with_temp_config<T>(f: impl FnOnce(&std::path::Path) -> T) -> T {
        let _lock = crate::modules::CONFIG_ENV_LOCK
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        let dir = tempfile::tempdir().unwrap();
        let previous = std::env::var_os("XDG_CONFIG_HOME");
        unsafe {
            std::env::set_var("XDG_CONFIG_HOME", dir.path());
        }
        let out = f(dir.path());
        match previous {
            Some(p) => unsafe { std::env::set_var("XDG_CONFIG_HOME", p) },
            None => unsafe { std::env::remove_var("XDG_CONFIG_HOME") },
        }
        out
    }

    // --- tray preferences -------------------------------------------------

    /// A missing `tray.toml` is first run: both icons read as shown, and
    /// nothing about it is reported as an error.
    #[test]
    fn a_missing_tray_toml_is_first_run_and_reads_as_shown() {
        with_temp_config(|_dir| {
            let (m, _backend) = module();
            let prefs = m.tray_prefs.as_ref().expect("a missing file is defaults, not an error");
            assert!(prefs.network);
            assert!(prefs.bluetooth);
            assert!(m.error.is_none(), "a first run is not a load failure");
        });
    }

    /// A `tray.toml` that exists and will not parse must be reported in
    /// this screen's error banner too, and a toggle afterwards must not
    /// overwrite it — the same property `network.rs` pins for its own
    /// screen.
    #[test]
    fn an_unreadable_tray_toml_is_reported_and_not_overwritten_by_a_toggle() {
        with_temp_config(|dir| {
            let tray_toml = dir.join("hyprforge").join("tray.toml");
            std::fs::create_dir_all(tray_toml.parent().unwrap()).unwrap();
            std::fs::write(&tray_toml, "bluetooth = yes please\n").unwrap();

            let (mut m, _backend) = module();
            assert!(m.tray_prefs.is_err(), "a malformed file must not be treated as defaults");
            assert!(
                m.error.as_ref().is_some_and(|e| e.contains("tray.toml")),
                "the failure must reach the screen's own error banner, got {:?}",
                m.error
            );

            let before = std::fs::read_to_string(&tray_toml).unwrap();
            let _ = m.update(Message::TrayToggled(false));
            let after = std::fs::read_to_string(&tray_toml).unwrap();
            assert_eq!(before, after, "a toggle must never overwrite a file it could not read");
        });
    }

    /// The distinction `hyprforge-bluetooth` exists to keep, one layer up:
    /// a daemon that isn't running must not render as an empty device list
    /// on this screen either.
    #[test]
    fn an_unavailable_bluez_shows_its_message_rather_than_an_empty_device_list() {
        let (mut m, _backend) = module();
        let _ = m.update(Message::Loaded(Loaded {
            status: Err(LoadError::from(BluetoothError::Unavailable)),
            devices: Err(LoadError::from(BluetoothError::Unavailable)),
        }));
        assert!(m.unavailable.as_ref().is_some_and(|msg| msg.contains("isn't running")));
        assert!(m.devices.is_empty(), "no devices to show while the daemon is gone");
        let _ = m.view(FontScale::default());
    }

    /// The property `adapter_row` is built around: a rfkill switch is what
    /// has to move, and no control here can do that.
    #[test]
    fn a_hardware_blocked_adapter_does_not_offer_a_toggle_that_cannot_work() {
        assert!(!adapter_offers_toggle(Some(AdapterState::HardwareBlocked)));
        assert!(adapter_offers_toggle(Some(AdapterState::On)));
        assert!(adapter_offers_toggle(Some(AdapterState::Off)));

        let (mut m, _backend) = module();
        let _ = m.update(Message::Loaded(loaded(
            status(AdapterState::HardwareBlocked, false),
            Vec::new(),
        )));
        // Toggling it must be a no-op even if some stray message reaches
        // update — the view not rendering a control is not the only guard.
        let task = m.update(Message::AdapterToggled(true));
        assert_eq!(task.units(), 0, "a hardware-blocked adapter must dispatch nothing");
        let _ = m.view(FontScale::default());
    }

    /// Unpaired devices are listed — a missing device is a bug report —
    /// but this screen can't connect to them without a pairing agent, and
    /// has to say why rather than pretend the row is like any other.
    #[test]
    fn an_unpaired_device_is_listed_but_offers_no_connect_action_and_says_why() {
        let (mut m, _backend) = module();
        let stranger = device("Stranger", "AA:BB:CC:DD:EE:01", false, false);
        assert!(stranger.unsupported_reason().is_some());
        let _ = m.update(Message::Loaded(loaded(status(AdapterState::On, false), vec![stranger.clone()])));
        assert_eq!(m.devices.len(), 1, "still listed");

        let task = m.update(Message::ConnectPressed(stranger.address.clone()));
        assert_eq!(task.units(), 0, "connecting an unpaired device must not reach the backend");
        let _ = m.view(FontScale::default());
    }

    /// Entering the screen builds the module and kicks off `status` and
    /// `devices` only — never a discovery session. Discovery costs battery
    /// and airtime on both ends and must be a deliberate click.
    #[test]
    fn entering_the_screen_does_not_start_discovery() {
        let (_m, backend) = module();
        assert!(
            backend.discovery_calls.lock().unwrap().is_empty(),
            "the module's own constructor must never call set_discovery"
        );
    }

    /// Powering the adapter off also ends discovery on the backend's side
    /// (see `MockBackend::set_powered`) — the screen must reflect that
    /// truthfully rather than keep showing "Scanning…" over a radio that
    /// is now off.
    #[test]
    fn powering_the_adapter_off_does_not_leave_the_screen_claiming_to_be_scanning() {
        let (mut m, _backend) = module();
        let _ = m.update(Message::Loaded(loaded(status(AdapterState::On, true), Vec::new())));
        assert!(m.status.as_ref().unwrap().discovering);

        let _ = m.update(Message::AdapterSet(Ok(())));
        // AdapterSet(Ok(..)) triggers a refresh; simulate what that refresh
        // would see now that the backend has turned the radio off.
        let _ = m.update(Message::Loaded(loaded(status(AdapterState::Off, false), Vec::new())));
        assert!(!m.status.as_ref().unwrap().discovering);
        let _ = m.view(FontScale::default());
    }

    /// Connecting a paired device has to be reflected immediately — waiting
    /// for the next poll would leave a device the user just told the app
    /// to connect showing as disconnected for up to ten seconds. `update`
    /// runs synchronously and the D-Bus call itself only happens once the
    /// `Task` it returns is polled by iced's executor, which a unit test
    /// never drives — so this delivers the `Connected` outcome directly,
    /// the same way `network.rs`'s join-failure tests do for `Connected`.
    #[test]
    fn connecting_a_paired_device_updates_the_row() {
        let (mut m, _backend) = module();
        let paired = device("Headset", "AA:BB:CC:DD:EE:02", true, false);
        let _ = m.update(Message::Loaded(loaded(status(AdapterState::On, false), vec![paired.clone()])));
        assert!(!m.devices[0].connected);

        let _ = m.update(Message::ConnectPressed(paired.address.clone()));
        let _ = m.update(Message::Connected(paired.address.clone(), Ok(())));
        assert!(m.devices[0].connected, "the row must show connected without waiting on the next poll");
    }

    /// Forgetting has to be reflected immediately, not after the next poll.
    #[test]
    fn forgetting_a_device_removes_it_from_the_list() {
        let (mut m, _backend) = module();
        let paired = device("Headset", "AA:BB:CC:DD:EE:03", true, false);
        let _ = m.update(Message::Loaded(loaded(status(AdapterState::On, false), vec![paired.clone()])));
        assert_eq!(m.devices.len(), 1);

        let _ = m.update(Message::Forgotten(paired.address.clone(), Ok(())));
        assert!(m.devices.is_empty());
    }

    /// A refused action — the mock's stand-in for BlueZ saying no — must
    /// report why and leave the device exactly where it was, not vanish it
    /// or silently pretend the action worked.
    #[test]
    fn a_refused_action_reports_why_and_leaves_the_device_in_place() {
        let (mut m, backend) = module();
        let paired = device("Headset", "AA:BB:CC:DD:EE:04", true, false);
        let _ = m.update(Message::Loaded(loaded(status(AdapterState::On, false), vec![paired.clone()])));

        *backend.refuse.lock().unwrap() = Some("device is off".to_string());
        let _ = m.update(Message::Connected(
            paired.address.clone(),
            Err(LoadError::from(BluetoothError::Refused("device is off".to_string()))),
        ));

        assert!(m.error.as_ref().is_some_and(|e| e.contains("device is off")));
        assert_eq!(m.devices.len(), 1, "still there");
        assert!(!m.devices[0].connected, "a refused connect must not be shown as connected");
    }

    /// Every state the screen can be in has to build without panicking:
    /// loading, unavailable, hardware-blocked, powered-off, and a
    /// populated list mixing paired, connected, and unpaired devices.
    #[test]
    fn the_screen_builds_in_every_state() {
        let (mut m, _backend) = module();
        let scale = FontScale::default();
        let _ = m.view(scale); // loading

        let _ = m.update(Message::Loaded(Loaded {
            status: Err(LoadError::from(BluetoothError::Unavailable)),
            devices: Err(LoadError::from(BluetoothError::Unavailable)),
        }));
        let _ = m.view(scale); // unavailable

        let (mut m, _backend) = module();
        let _ = m.update(Message::Loaded(loaded(status(AdapterState::HardwareBlocked, false), Vec::new())));
        let _ = m.view(scale); // hardware-blocked adapter

        let (mut m, _backend) = module();
        let _ = m.update(Message::Loaded(loaded(status(AdapterState::Off, false), Vec::new())));
        let _ = m.view(scale); // powered off

        let (mut m, _backend) = module();
        let mut unnamed = device("Unnamed", "AA:BB:CC:DD:EE:05", true, false);
        unnamed.name = None;
        let devices = vec![
            device("Connected headset", "AA:BB:CC:DD:EE:06", true, true),
            device("Paired mouse", "AA:BB:CC:DD:EE:07", true, false),
            device("Stranger", "AA:BB:CC:DD:EE:08", false, false),
            unnamed,
        ];
        let _ = m.update(Message::Loaded(loaded(status(AdapterState::On, true), devices)));
        let _ = m.view(scale); // populated: connected, paired, unpaired, unnamed

        m.error = Some("a connect failed".to_string());
        let _ = m.view(scale); // error banner over a populated list
    }
}
