//! `hyprforge-trayd`: two `org.kde.StatusNotifierItem`s, Wi-Fi and
//! Bluetooth, joined to `hyprforge-network` and `hyprforge-bluetooth`.
//!
//! Everything that decides *which icon* a state deserves lives in the
//! pure functions below (`network_item`, `bluetooth_item`) — no D-Bus, no
//! I/O — so the interesting question is unit-tested without a bus, a bar,
//! or a radio, the same split `crate::item` documents. Everything else
//! here is plumbing: polling the two backends, keeping two `TrayIcon`s in
//! sync, and forwarding clicks to `hyprforge-settings`.

use hyprforge_bluetooth::backend::BluetoothBackend;
use hyprforge_bluetooth::{AdapterState, BlueZBackend, Device};
use hyprforge_bluetooth::Status as BtStatus;
use hyprforge_network::backend::NetworkBackend;
use hyprforge_network::{NetworkManagerBackend, RadioState, Ssid};
use hyprforge_network::Status as NetStatus;
use hyprforge_tray::{watcher_present, Category, TrayIcon, TrayItem};
use hyprforge_tray::Status as TrayStatus;
use std::sync::Arc;
use std::time::Duration;

/// How often both backends are re-polled. Ten seconds is what the
/// Settings app's Network and Bluetooth screens already poll at while
/// open — matching it means the tray icon and an open Settings window
/// never visibly disagree for long.
const POLL_INTERVAL: Duration = Duration::from_secs(10);

/// How often a failed `TrayIcon::register` or a not-yet-present watcher
/// is retried. Short enough that a bar starting up is followed quickly,
/// long enough not to spin.
const RETRY_INTERVAL: Duration = Duration::from_secs(3);

/// How often the watcher's presence is polled to notice a bar restart.
///
/// A `NameOwnerChanged` subscription would need matching on
/// `org.kde.StatusNotifierWatcher` specifically and filtering out the
/// "still has an owner, just a different one during a graceful restart"
/// case — `watcher_present` (which `main` already uses to wait at
/// startup) answers the only question that matters here just as well,
/// and a tray host is not a hot path, so polling it is not a real cost.
const WATCHER_POLL_INTERVAL: Duration = Duration::from_secs(5);

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    // Default to `info` rather than whatever `from_default_env` alone
    // gives with RUST_LOG unset, which is nothing — a daemon with no GUI
    // that prints nothing on start is indistinguishable from a hung one.
    // RUST_LOG still overrides this when set.
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")),
        )
        .init();

    // One session-bus connection, kept for the lifetime of the process,
    // used only to ask the watcher whether it's there — `TrayIcon::register`
    // opens its own connection per item, for the reason on `TrayIcon`.
    let probe = zbus::Connection::session().await?;

    if !watcher_present(&probe).await {
        // No bar is an ordinary state (`hl.exec_cmd` starts this before
        // much of the session exists), not a failure — wait rather than
        // exit, or the daemon that starts before the bar is the one that
        // never shows an icon.
        tracing::info!("no tray host yet; waiting for one to start");
        loop {
            tokio::time::sleep(WATCHER_POLL_INTERVAL).await;
            if watcher_present(&probe).await {
                break;
            }
        }
    }
    tracing::info!("tray host found; registering icons");

    let (clicks_tx, clicks_rx) = tokio::sync::mpsc::unbounded_channel();

    // Registered unavailable-looking to start: the first real poll is at
    // most one `POLL_INTERVAL` away, and an icon that starts blank until
    // then would look identical to a daemon that never started.
    let network_icon =
        register_with_retry(network_item(None, None, None, true), 0, clicks_tx.clone()).await;
    let bluetooth_icon =
        register_with_retry(bluetooth_item(None, None, true), 1, clicks_tx.clone()).await;

    let poll_task = tokio::spawn(poll_loop(network_icon.clone(), bluetooth_icon.clone()));
    let click_task = tokio::spawn(handle_clicks(clicks_rx));
    let reannounce_task =
        tokio::spawn(reannounce_loop(probe, network_icon, bluetooth_icon));

    // None of these three loops return under normal operation. If one
    // panics, that is worth knowing about rather than leaving the other
    // two running silently short-handed.
    tokio::select! {
        r = poll_task => log_task_exit("poll", r),
        r = click_task => log_task_exit("clicks", r),
        r = reannounce_task => log_task_exit("reannounce", r),
    }

    Ok(())
}

fn log_task_exit(name: &str, result: Result<(), tokio::task::JoinError>) {
    match result {
        Ok(()) => tracing::warn!(task = name, "background task exited unexpectedly"),
        Err(e) => tracing::error!(task = name, error = %e, "background task panicked"),
    }
}

/// Registers one tray icon, retrying indefinitely rather than giving up.
///
/// `main` already waited for a tray host before calling this, but
/// registration can still fail — a host that goes away between the
/// watcher check and `register_status_notifier_item`, for instance — and
/// exiting the whole daemon over that would take *both* icons down for
/// one host hiccup, which defeats the reason there are two independent
/// icons in the first place.
async fn register_with_retry(
    item: TrayItem,
    index: u32,
    clicks: tokio::sync::mpsc::UnboundedSender<String>,
) -> Arc<TrayIcon> {
    loop {
        match TrayIcon::register(item.clone(), index, clicks.clone()).await {
            Ok(icon) => return Arc::new(icon),
            Err(e) => {
                tracing::warn!(error = %e, icon = %item.id, "failed to register tray icon; retrying");
                tokio::time::sleep(RETRY_INTERVAL).await;
            }
        }
    }
}

/// Holds a backend connection once one succeeds, and keeps retrying on
/// every tick until it does.
///
/// Caching a *failed* connection attempt would repeat exactly the
/// mistake documented on `LazyNetworkManagerBackend` in
/// `hyprforge-settings`: a `NetworkError::Unavailable` tells the user how
/// to fix it (`systemctl start NetworkManager`), and a daemon that stops
/// checking after the first failure means following that instruction and
/// watching the tray icon say the same thing forever.
struct Reconnecting<B> {
    backend: Option<Arc<B>>,
}

impl<B> Reconnecting<B> {
    fn new() -> Self {
        Reconnecting { backend: None }
    }

    /// Returns the cached backend, or runs `connect` again. Only
    /// `Ok` is ever cached, which is the whole property this type
    /// exists to guarantee — see the struct doc.
    async fn get_or_connect<F, Fut, E>(&mut self, connect: F) -> Result<Arc<B>, E>
    where
        F: FnOnce() -> Fut,
        Fut: std::future::Future<Output = Result<B, E>>,
    {
        if let Some(backend) = &self.backend {
            return Ok(backend.clone());
        }
        let backend = Arc::new(connect().await?);
        self.backend = Some(backend.clone());
        Ok(backend)
    }
}

async fn network_backend(
    state: &mut Reconnecting<NetworkManagerBackend>,
) -> Option<Arc<NetworkManagerBackend>> {
    match state.get_or_connect(NetworkManagerBackend::connect).await {
        Ok(backend) => Some(backend),
        Err(e) => {
            tracing::warn!(error = %e, "could not connect to NetworkManager; will retry");
            None
        }
    }
}

async fn bluetooth_backend(state: &mut Reconnecting<BlueZBackend>) -> Option<Arc<BlueZBackend>> {
    match state.get_or_connect(BlueZBackend::connect).await {
        Ok(backend) => Some(backend),
        Err(e) => {
            tracing::warn!(error = %e, "could not connect to BlueZ; will retry");
            None
        }
    }
}

/// One tick's worth of the network icon.
///
/// The connection, once opened, is a handle to the *session bus name*
/// `org.freedesktop.NetworkManager` — not a socket to the daemon itself —
/// so it is kept even when a call against it fails; only the call result
/// is reported as unavailable that tick. A dead bus connection (the
/// exceedingly rare case) would fail `network_backend` itself and go
/// through the retry path above rather than this one.
async fn sample_network(state: &mut Reconnecting<NetworkManagerBackend>) -> TrayItem {
    let Some(backend) = network_backend(state).await else {
        return network_item(None, None, None, true);
    };
    match backend.status().await {
        Ok(status) => {
            let strength = match &status.connected_to {
                Some(ssid) => match backend.access_points().await {
                    Ok(points) => points.iter().find(|ap| &ap.ssid == ssid).map(|ap| ap.strength),
                    // The connected network can briefly be missing from a
                    // fresh scan; that is not "unavailable", just "signal
                    // not known this tick".
                    Err(_) => None,
                },
                None => None,
            };
            network_item(Some(&status), status.connected_to.as_ref(), strength, false)
        }
        Err(e) => {
            tracing::warn!(error = %e, "NetworkManager status call failed");
            network_item(None, None, None, true)
        }
    }
}

async fn sample_bluetooth(state: &mut Reconnecting<BlueZBackend>) -> TrayItem {
    let Some(backend) = bluetooth_backend(state).await else {
        return bluetooth_item(None, None, true);
    };
    match backend.status().await {
        Ok(status) => {
            let connected = match backend.devices().await {
                Ok(devices) => devices.into_iter().find(|d| d.connected),
                Err(_) => None,
            };
            bluetooth_item(Some(&status), connected.as_ref(), false)
        }
        Err(e) => {
            tracing::warn!(error = %e, "BlueZ status call failed");
            bluetooth_item(None, None, true)
        }
    }
}

async fn poll_loop(network_icon: Arc<TrayIcon>, bluetooth_icon: Arc<TrayIcon>) {
    let mut net_state = Reconnecting::new();
    let mut bt_state = Reconnecting::new();
    // `interval` fires its first tick immediately, which is wanted here:
    // both icons started out saying "unavailable" in `main`, and the
    // first real read should not wait a further `POLL_INTERVAL`.
    let mut ticker = tokio::time::interval(POLL_INTERVAL);
    loop {
        ticker.tick().await;

        let net_item = sample_network(&mut net_state).await;
        if let Err(e) = network_icon.update(net_item).await {
            tracing::warn!(error = %e, "failed to update the network tray icon");
        }

        let bt_item = sample_bluetooth(&mut bt_state).await;
        if let Err(e) = bluetooth_icon.update(bt_item).await {
            tracing::warn!(error = %e, "failed to update the bluetooth tray icon");
        }
    }
}

/// Forwards left-clicks (and the button-agnostic activations `sni.rs`
/// maps onto them) to `hyprforge-settings --screen <name>`.
///
/// Spawned and never waited on. The click arrived over `activate`, whose
/// D-Bus caller — the bar — is blocked on the method returning; waiting
/// here for the child to exit would make the whole bar stutter on every
/// click, and a launcher that fails to start is a warning, not a reason
/// to stop handling further clicks.
async fn handle_clicks(mut clicks: tokio::sync::mpsc::UnboundedReceiver<String>) {
    while let Some(screen) = clicks.recv().await {
        match tokio::process::Command::new("hyprforge-settings")
            .arg("--screen")
            .arg(&screen)
            // The child's output goes nowhere rather than inheriting
            // this daemon's. A GUI started from here prints its own
            // renderer chatter — wgpu alone logs its adapter and surface
            // formats at startup — and inheriting it puts all of that in
            // the tray daemon's log, once per click, for as long as the
            // session lasts. The daemon's log is then unreadable and
            // unbounded, and the one thing it exists to record — a
            // failed registration — is buried. The child keeps its own
            // logging; it does not need ours.
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .spawn()
        {
            Ok(_) => {}
            Err(e) => {
                tracing::warn!(error = %e, screen = %screen, "failed to launch hyprforge-settings")
            }
        }
    }
}

/// Re-announces both icons whenever the tray host reappears.
///
/// A bar restart takes every registration on the old connection with it;
/// `TrayIcon::announce`'s doc comment covers why re-registering is the
/// only way back. This only fires on the *transition* into "present" —
/// re-announcing every poll while a host is already there would ask it to
/// re-add an icon it already has for no reason.
async fn reannounce_loop(
    probe: zbus::Connection,
    network_icon: Arc<TrayIcon>,
    bluetooth_icon: Arc<TrayIcon>,
) {
    // `main` only reaches this point after `watcher_present` returned
    // true, so the host is presumed present until proven otherwise.
    let mut present = true;
    let mut ticker = tokio::time::interval(WATCHER_POLL_INTERVAL);
    loop {
        ticker.tick().await;
        let now_present = watcher_present(&probe).await;
        if now_present && !present {
            tracing::info!("tray host reappeared; re-announcing icons");
            if let Err(e) = network_icon.announce().await {
                tracing::warn!(error = %e, "failed to re-announce the network icon");
            }
            if let Err(e) = bluetooth_icon.announce().await {
                tracing::warn!(error = %e, "failed to re-announce the bluetooth icon");
            }
        }
        present = now_present;
    }
}

/// Freedesktop icon names below the boundary in [`u8`] percent, matching
/// the row NetworkManager applets themselves use: 80/60/40/20.
fn wifi_signal_icon(strength: u8) -> &'static str {
    match strength {
        80..=u8::MAX => "network-wireless-signal-excellent",
        60..=79 => "network-wireless-signal-good",
        40..=59 => "network-wireless-signal-ok",
        20..=39 => "network-wireless-signal-weak",
        _ => "network-wireless-signal-none",
    }
}

/// What the Wi-Fi icon should say, from plain data — no D-Bus, no I/O.
///
/// `connected_ssid` and `strength` are passed separately from `status`
/// rather than derived from it here, so a test can assert the tooltip and
/// icon for a specific signal reading without constructing a full access
/// point list. `unavailable` is checked before `status` is even looked
/// at, since a `None` status on its own already means the same thing:
/// nothing was read this tick, so say so rather than guessing.
fn network_item(
    status: Option<&NetStatus>,
    connected_ssid: Option<&Ssid>,
    strength: Option<u8>,
    unavailable: bool,
) -> TrayItem {
    let id = "hyprforge-network".to_string();
    let title = "Wi-Fi".to_string();

    let Some(status) = status.filter(|_| !unavailable) else {
        return TrayItem {
            id,
            category: Category::Hardware,
            // Visible, not hidden — an icon that disappears exactly when
            // the daemon behind it can't be reached looks identical to
            // "nothing to report", which is the one thing pillar 3
            // forbids.
            status: TrayStatus::Active,
            title,
            icon_name: "network-wireless-offline".to_string(),
            tooltip_title: "Wi-Fi unavailable".to_string(),
            tooltip_body: "NetworkManager isn't running.".to_string(),
        };
    };

    match status.radio {
        // Off (switched off in software) and HardwareOff (rfkill) both
        // mean the radio is not on — the one case `item.rs` says to hide.
        RadioState::Off | RadioState::HardwareOff => TrayItem {
            id,
            category: Category::Hardware,
            status: TrayStatus::Passive,
            title,
            icon_name: "network-wireless-disconnected".to_string(),
            tooltip_title: "Wi-Fi is off".to_string(),
            tooltip_body: String::new(),
        },
        RadioState::On => match connected_ssid {
            Some(ssid) => TrayItem {
                id,
                category: Category::Hardware,
                status: TrayStatus::Active,
                title,
                icon_name: wifi_signal_icon(strength.unwrap_or(0)).to_string(),
                tooltip_title: ssid.to_display_string(),
                tooltip_body: match strength {
                    Some(s) => format!("Signal {s}%"),
                    None => "Connected".to_string(),
                },
            },
            // On and unconnected is exactly when someone goes looking for
            // this icon, so — unlike the radio-off case — it stays
            // visible rather than hiding.
            None => TrayItem {
                id,
                category: Category::Hardware,
                status: TrayStatus::Active,
                title,
                icon_name: "network-wireless-disconnected".to_string(),
                tooltip_title: "Not connected".to_string(),
                tooltip_body: "Wi-Fi is on".to_string(),
            },
        },
    }
}

/// What the Bluetooth icon should say, from plain data — no D-Bus, no
/// I/O. Same shape and same reasoning as [`network_item`].
fn bluetooth_item(
    status: Option<&BtStatus>,
    connected_device: Option<&Device>,
    unavailable: bool,
) -> TrayItem {
    let id = "hyprforge-bluetooth".to_string();
    let title = "Bluetooth".to_string();

    let Some(status) = status.filter(|_| !unavailable) else {
        return TrayItem {
            id,
            category: Category::Hardware,
            status: TrayStatus::Active,
            title,
            icon_name: "bluetooth-offline".to_string(),
            tooltip_title: "Bluetooth unavailable".to_string(),
            tooltip_body: "bluetoothd isn't running.".to_string(),
        };
    };

    match status.state {
        // HardwareBlocked is rfkill, not "off" in software, but both
        // mean the radio is not on — the case worth hiding for.
        AdapterState::Off | AdapterState::HardwareBlocked => TrayItem {
            id,
            category: Category::Hardware,
            status: TrayStatus::Passive,
            title,
            icon_name: "bluetooth-disabled".to_string(),
            tooltip_title: "Bluetooth is off".to_string(),
            tooltip_body: String::new(),
        },
        AdapterState::On | AdapterState::Changing => match connected_device {
            Some(device) => TrayItem {
                id,
                category: Category::Hardware,
                status: TrayStatus::Active,
                title,
                icon_name: "bluetooth-active".to_string(),
                // `alias` always exists (falls back to `Name`, then the
                // address) — never the raw address alone, and never
                // anything derived from pairing secrets, of which this
                // crate exposes none anyway.
                tooltip_title: device.alias.clone(),
                tooltip_body: device.kind.label().to_string(),
            },
            None => TrayItem {
                id,
                category: Category::Hardware,
                status: TrayStatus::Active,
                title,
                icon_name: "bluetooth-active".to_string(),
                tooltip_title: "Not connected".to_string(),
                tooltip_body: "Bluetooth is on".to_string(),
            },
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use hyprforge_bluetooth::DeviceKind;

    // The exact freedesktop names this daemon is allowed to hand to a
    // tray host. Not every name the doc comment lists has to appear here
    // — only every name the functions below can actually *return* has to
    // be a member — which is what catches an invented name before it
    // ships: an unresolvable icon name renders as a blank gap, with
    // nothing logged anywhere to say why.
    const NETWORK_ICON_NAMES: &[&str] = &[
        "network-wireless-signal-excellent",
        "network-wireless-signal-good",
        "network-wireless-signal-ok",
        "network-wireless-signal-weak",
        "network-wireless-signal-none",
        "network-wireless-disconnected",
        "network-wireless-offline",
    ];
    const BLUETOOTH_ICON_NAMES: &[&str] = &["bluetooth-active", "bluetooth-disabled", "bluetooth-offline"];

    fn net_status(radio: RadioState, connected: Option<&str>) -> NetStatus {
        NetStatus { radio, connected_to: connected.map(Ssid::new) }
    }

    fn bt_status(state: AdapterState) -> BtStatus {
        BtStatus { state, discovering: false, alias: "mock-adapter".to_string() }
    }

    fn device(alias: &str, connected: bool) -> Device {
        Device {
            address: hyprforge_bluetooth::Address::new("AA:BB:CC:DD:EE:FF"),
            alias: alias.to_string(),
            name: Some(alias.to_string()),
            kind: DeviceKind::Headset,
            paired: true,
            trusted: true,
            connected,
            rssi: Some(-50),
        }
    }

    // --- radio off vs on-but-unconnected -----------------------------

    #[test]
    fn a_switched_off_wifi_radio_is_passive_but_on_and_unconnected_is_active() {
        let off = network_item(Some(&net_status(RadioState::Off, None)), None, None, false);
        assert_eq!(off.status, TrayStatus::Passive);

        let hw_off =
            network_item(Some(&net_status(RadioState::HardwareOff, None)), None, None, false);
        assert_eq!(hw_off.status, TrayStatus::Passive, "rfkill also means the radio is not on");

        let on_unconnected = network_item(Some(&net_status(RadioState::On, None)), None, None, false);
        assert_eq!(
            on_unconnected.status,
            TrayStatus::Active,
            "on and unconnected is exactly when someone looks for the icon"
        );
    }

    #[test]
    fn a_switched_off_bluetooth_adapter_is_passive_but_on_and_unconnected_is_active() {
        let off = bluetooth_item(Some(&bt_status(AdapterState::Off)), None, false);
        assert_eq!(off.status, TrayStatus::Passive);

        let blocked = bluetooth_item(Some(&bt_status(AdapterState::HardwareBlocked)), None, false);
        assert_eq!(blocked.status, TrayStatus::Passive);

        let on_unconnected = bluetooth_item(Some(&bt_status(AdapterState::On)), None, false);
        assert_eq!(on_unconnected.status, TrayStatus::Active);
    }

    // --- signal-strength boundaries ------------------------------------

    #[test]
    fn signal_strength_maps_to_the_right_icon_at_its_boundaries() {
        let cases: &[(u8, &str)] = &[
            (100, "network-wireless-signal-excellent"),
            (80, "network-wireless-signal-excellent"),
            (79, "network-wireless-signal-good"),
            (60, "network-wireless-signal-good"),
            (59, "network-wireless-signal-ok"),
            (40, "network-wireless-signal-ok"),
            (39, "network-wireless-signal-weak"),
            (20, "network-wireless-signal-weak"),
            (19, "network-wireless-signal-none"),
            (0, "network-wireless-signal-none"),
        ];
        for &(strength, expected) in cases {
            assert_eq!(
                wifi_signal_icon(strength),
                expected,
                "strength {strength} should map to {expected}"
            );
        }
    }

    // --- unavailable daemons say so, distinctly from "nothing connected" --

    #[test]
    fn an_unavailable_network_daemon_produces_an_icon_and_tooltip_that_say_so() {
        let item = network_item(None, None, None, true);
        assert_eq!(item.status, TrayStatus::Active, "must stay visible to be seen at all");
        assert_eq!(item.icon_name, "network-wireless-offline");
        assert!(item.tooltip_title.to_lowercase().contains("unavailable"));

        let nothing_connected = network_item(Some(&net_status(RadioState::On, None)), None, None, false);
        assert_ne!(
            item.tooltip_title, nothing_connected.tooltip_title,
            "unavailable and merely unconnected must not read the same"
        );
        assert_ne!(item.icon_name, nothing_connected.icon_name);
    }

    #[test]
    fn a_missing_status_reads_the_same_as_an_explicit_unavailable_flag() {
        // Defensive: a caller that forgets to set `unavailable` but has
        // no status at all (nothing was ever polled) must not fall
        // through to a default that looks like "off" or "connected".
        let item = network_item(None, None, None, false);
        assert!(item.tooltip_title.to_lowercase().contains("unavailable"));
    }

    #[test]
    fn an_unavailable_bluetooth_daemon_produces_an_icon_and_tooltip_that_say_so() {
        let item = bluetooth_item(None, None, true);
        assert_eq!(item.status, TrayStatus::Active);
        assert_eq!(item.icon_name, "bluetooth-offline");
        assert!(item.tooltip_title.to_lowercase().contains("unavailable"));

        let nothing_connected = bluetooth_item(Some(&bt_status(AdapterState::On)), None, false);
        assert_ne!(item.tooltip_title, nothing_connected.tooltip_title);
        assert_ne!(item.icon_name, nothing_connected.icon_name);
    }

    // --- a connected network's tooltip names it ------------------------

    #[test]
    fn a_connected_networks_tooltip_names_it() {
        let ssid = Ssid::new("Coffee Shop");
        let item = network_item(
            Some(&net_status(RadioState::On, Some("Coffee Shop"))),
            Some(&ssid),
            Some(72),
            false,
        );
        assert_eq!(item.tooltip_title, "Coffee Shop");
        assert!(item.tooltip_body.contains("72"));
        assert_eq!(item.icon_name, "network-wireless-signal-good");
    }

    #[test]
    fn a_connected_bluetooth_devices_tooltip_names_it() {
        let d = device("WH-1000XM4", true);
        let item = bluetooth_item(Some(&bt_status(AdapterState::On)), Some(&d), false);
        assert_eq!(item.tooltip_title, "WH-1000XM4");
        assert_eq!(item.tooltip_body, "Headset");
    }

    // --- no invented icon names ------------------------------------------

    #[test]
    fn every_network_icon_the_function_can_return_is_a_documented_freedesktop_name() {
        let ssid = Ssid::new("home");
        let items = [
            network_item(None, None, None, true),
            network_item(Some(&net_status(RadioState::Off, None)), None, None, false),
            network_item(Some(&net_status(RadioState::HardwareOff, None)), None, None, false),
            network_item(Some(&net_status(RadioState::On, None)), None, None, false),
            network_item(Some(&net_status(RadioState::On, Some("home"))), Some(&ssid), Some(0), false),
            network_item(Some(&net_status(RadioState::On, Some("home"))), Some(&ssid), Some(19), false),
            network_item(Some(&net_status(RadioState::On, Some("home"))), Some(&ssid), Some(20), false),
            network_item(Some(&net_status(RadioState::On, Some("home"))), Some(&ssid), Some(39), false),
            network_item(Some(&net_status(RadioState::On, Some("home"))), Some(&ssid), Some(40), false),
            network_item(Some(&net_status(RadioState::On, Some("home"))), Some(&ssid), Some(59), false),
            network_item(Some(&net_status(RadioState::On, Some("home"))), Some(&ssid), Some(60), false),
            network_item(Some(&net_status(RadioState::On, Some("home"))), Some(&ssid), Some(79), false),
            network_item(Some(&net_status(RadioState::On, Some("home"))), Some(&ssid), Some(80), false),
            network_item(Some(&net_status(RadioState::On, Some("home"))), Some(&ssid), Some(100), false),
            network_item(Some(&net_status(RadioState::On, Some("home"))), Some(&ssid), None, false),
        ];
        for item in items {
            assert!(
                NETWORK_ICON_NAMES.contains(&item.icon_name.as_str()),
                "invented icon name: {}",
                item.icon_name
            );
        }
    }

    #[test]
    fn every_bluetooth_icon_the_function_can_return_is_a_documented_freedesktop_name() {
        let connected = device("Headset", true);
        let items = [
            bluetooth_item(None, None, true),
            bluetooth_item(Some(&bt_status(AdapterState::Off)), None, false),
            bluetooth_item(Some(&bt_status(AdapterState::HardwareBlocked)), None, false),
            bluetooth_item(Some(&bt_status(AdapterState::On)), None, false),
            bluetooth_item(Some(&bt_status(AdapterState::Changing)), None, false),
            bluetooth_item(Some(&bt_status(AdapterState::On)), Some(&connected), false),
        ];
        for item in items {
            assert!(
                BLUETOOTH_ICON_NAMES.contains(&item.icon_name.as_str()),
                "invented icon name: {}",
                item.icon_name
            );
        }
    }

    // --- click-to-screen mapping, with no process spawned ----------------

    #[test]
    fn a_click_on_either_icon_maps_to_its_own_settings_screen() {
        let network = network_item(Some(&net_status(RadioState::On, None)), None, None, false);
        assert_eq!(network.activate_screen(), Some("network"));

        let bluetooth = bluetooth_item(Some(&bt_status(AdapterState::On)), None, false);
        assert_eq!(bluetooth.activate_screen(), Some("bluetooth"));
    }

    // --- Reconnecting never caches a failure -----------------------------

    #[tokio::test]
    async fn a_failed_connect_is_retried_on_the_next_tick_rather_than_cached() {
        use std::sync::atomic::{AtomicUsize, Ordering};

        let attempts = AtomicUsize::new(0);
        let mut state: Reconnecting<u32> = Reconnecting::new();

        // First tick: the backend isn't there yet.
        let first = state
            .get_or_connect(|| async {
                attempts.fetch_add(1, Ordering::SeqCst);
                Err::<u32, &'static str>("not running")
            })
            .await;
        assert!(first.is_err());
        assert!(
            state.backend.is_none(),
            "a failed connect must leave nothing cached, or a later tick would never try again"
        );

        // Second tick: it comes up. This must actually run the connect
        // closure again — a cached-failure design (`Option<Result<..>>`)
        // would skip straight to returning the same error without ever
        // calling it.
        let second = state
            .get_or_connect(|| async {
                attempts.fetch_add(1, Ordering::SeqCst);
                Ok::<u32, &'static str>(42)
            })
            .await;
        assert_eq!(*second.unwrap(), 42);
        assert_eq!(attempts.load(Ordering::SeqCst), 2, "both ticks must have called connect");

        // Third tick: a successful connect *is* cached, so this must not
        // call connect again.
        let calls_before_third = attempts.load(Ordering::SeqCst);
        let third = state
            .get_or_connect(|| async {
                attempts.fetch_add(1, Ordering::SeqCst);
                Ok::<u32, &'static str>(99)
            })
            .await;
        assert_eq!(*third.unwrap(), 42, "a cached success is reused, not replaced");
        assert_eq!(
            attempts.load(Ordering::SeqCst),
            calls_before_third,
            "a cached success must not reconnect"
        );
    }
}
