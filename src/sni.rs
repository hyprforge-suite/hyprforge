//! Serving `org.kde.StatusNotifierItem`, and staying registered.

use crate::item::TrayItem;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::Mutex;
use zbus::object_server::SignalEmitter;
use zbus::Connection;

/// Watcher calls. A bar that has stopped answering must not hang the
/// daemon — same rule as every other cross-process call in this suite.
pub const TIMEOUT: Duration = Duration::from_secs(5);

pub const ITEM_PATH: &str = "/StatusNotifierItem";

#[derive(Debug, thiserror::Error)]
pub enum TrayError {
    /// No bar is offering a tray. An ordinary state, not a failure: a
    /// user may run no bar at all, or start one later. Callers wait and
    /// retry rather than exiting.
    #[error("no StatusNotifierWatcher is running, so there is no tray to appear in")]
    NoWatcher,
    #[error("the tray host didn't answer within {0:?}")]
    TimedOut(Duration),
    #[error("{0}")]
    Refused(String),
}

fn classify(e: zbus::Error) -> TrayError {
    if let zbus::Error::MethodError(name, _, _) = &e {
        let name = name.as_str();
        if name.ends_with(".ServiceUnknown") || name.ends_with(".NameHasNoOwner") {
            return TrayError::NoWatcher;
        }
        return TrayError::Refused(e.to_string());
    }
    TrayError::Refused(e.to_string())
}

async fn bounded<T>(
    call: impl std::future::Future<Output = zbus::Result<T>>,
) -> Result<T, TrayError> {
    match tokio::time::timeout(TIMEOUT, call).await {
        Err(_) => Err(TrayError::TimedOut(TIMEOUT)),
        Ok(Ok(v)) => Ok(v),
        Ok(Err(e)) => Err(classify(e)),
    }
}

#[zbus::proxy(
    interface = "org.kde.StatusNotifierWatcher",
    default_service = "org.kde.StatusNotifierWatcher",
    default_path = "/StatusNotifierWatcher"
)]
trait StatusNotifierWatcher {
    fn register_status_notifier_item(&self, service: &str) -> zbus::Result<()>;
    #[zbus(property)]
    fn is_status_notifier_host_registered(&self) -> zbus::Result<bool>;
    #[zbus(property)]
    fn registered_status_notifier_items(&self) -> zbus::Result<Vec<String>>;
}

/// The object a host reads. One per icon.
struct ItemInterface {
    state: Arc<Mutex<TrayItem>>,
    /// Left-clicks, as the Settings screen name to open. Unbounded
    /// because a click must never block the D-Bus handler it arrives on:
    /// the host is waiting for the method to return, and spawning the
    /// process inline would make the bar stutter.
    clicks: tokio::sync::mpsc::UnboundedSender<String>,
}

#[zbus::interface(name = "org.kde.StatusNotifierItem")]
impl ItemInterface {
    #[zbus(property)]
    async fn category(&self) -> String {
        self.state.lock().await.category.as_str().to_string()
    }

    #[zbus(property)]
    async fn id(&self) -> String {
        self.state.lock().await.id.clone()
    }

    #[zbus(property)]
    async fn title(&self) -> String {
        self.state.lock().await.title.clone()
    }

    #[zbus(property)]
    async fn status(&self) -> String {
        self.state.lock().await.status.as_str().to_string()
    }

    #[zbus(property)]
    async fn icon_name(&self) -> String {
        self.state.lock().await.icon_name.clone()
    }

    /// No pixmaps, ever. The icon is a *name*, resolved by the host from
    /// the user's own icon theme — which is the whole reason this crate
    /// needs no toolkit and owns no images.
    #[zbus(property)]
    async fn icon_pixmap(&self) -> Vec<(i32, i32, Vec<u8>)> {
        Vec::new()
    }

    /// `false` means a left-click should call [`Self::activate`] rather
    /// than open a menu. Since this suite serves no `com.canonical.dbusmenu`
    /// yet, saying `true` here would make every click do nothing at all.
    #[zbus(property)]
    async fn item_is_menu(&self) -> bool {
        false
    }

    /// `(icon_name, icon_pixmap, title, description)`.
    #[zbus(property)]
    async fn tool_tip(&self) -> (String, Vec<(i32, i32, Vec<u8>)>, String, String) {
        let state = self.state.lock().await;
        (
            state.icon_name.clone(),
            Vec::new(),
            state.tooltip_title.clone(),
            state.tooltip_body.clone(),
        )
    }

    /// Zero: these items have no window of their own. A host uses this
    /// only to associate an item with an X11 window, which is meaningless
    /// for a daemon that has none.
    #[zbus(property)]
    async fn window_id(&self) -> i32 {
        0
    }

    async fn activate(&self, _x: i32, _y: i32) {
        let screen = self.state.lock().await.activate_screen();
        if let Some(screen) = screen {
            // Send, never spawn, for the reason on `clicks`.
            let _ = self.clicks.send(screen.to_string());
        }
    }

    /// Middle-click. Same as a left-click here rather than nothing: an
    /// item that responds to one button and silently ignores another
    /// reads as broken.
    async fn secondary_activate(&self, x: i32, y: i32) {
        self.activate(x, y).await;
    }

    /// Right-click. Also opens the page, because there is no menu to show
    /// yet; doing nothing would be indistinguishable from a hung daemon.
    async fn context_menu(&self, x: i32, y: i32) {
        self.activate(x, y).await;
    }

    async fn scroll(&self, _delta: i32, _orientation: String) {}

    #[zbus(signal)]
    async fn new_icon(emitter: &SignalEmitter<'_>) -> zbus::Result<()>;
    #[zbus(signal)]
    async fn new_status(emitter: &SignalEmitter<'_>, status: &str) -> zbus::Result<()>;
    #[zbus(signal)]
    async fn new_tool_tip(emitter: &SignalEmitter<'_>) -> zbus::Result<()>;
    #[zbus(signal)]
    async fn new_title(emitter: &SignalEmitter<'_>) -> zbus::Result<()>;
}

/// A live tray icon: its own bus name, its own connection, its own
/// registration.
///
/// One connection per item rather than one shared: the spec identifies an
/// item *by its bus name*, so two items on one connection would be one
/// item to every host.
pub struct TrayIcon {
    connection: Connection,
    bus_name: String,
    state: Arc<Mutex<TrayItem>>,
    last: Mutex<TrayItem>,
}

impl TrayIcon {
    /// Claims a bus name, serves the item on it, and registers with the
    /// watcher.
    ///
    /// `index` distinguishes several items from one process; the spec's
    /// convention is `org.kde.StatusNotifierItem-<pid>-<n>`.
    pub async fn register(
        item: TrayItem,
        index: u32,
        clicks: tokio::sync::mpsc::UnboundedSender<String>,
    ) -> Result<Self, TrayError> {
        let bus_name = format!("org.kde.StatusNotifierItem-{}-{}", std::process::id(), index);
        let state = Arc::new(Mutex::new(item.clone()));

        let connection = zbus::connection::Builder::session()
            .map_err(classify)?
            .name(bus_name.as_str())
            .map_err(classify)?
            .serve_at(
                ITEM_PATH,
                ItemInterface {
                    state: state.clone(),
                    clicks,
                },
            )
            .map_err(classify)?
            .build()
            .await
            .map_err(classify)?;

        let icon = TrayIcon {
            connection,
            bus_name,
            state,
            last: Mutex::new(item),
        };
        icon.announce().await?;
        Ok(icon)
    }

    /// Tells the watcher this item exists.
    ///
    /// Called again whenever the watcher reappears — a bar restart takes
    /// every registration with it, and an item that does not re-announce
    /// is simply gone until the daemon is restarted too. That is the
    /// failure mode every hand-rolled tray implementation has.
    pub async fn announce(&self) -> Result<(), TrayError> {
        let watcher = bounded(StatusNotifierWatcherProxy::new(&self.connection)).await?;
        bounded(watcher.register_status_notifier_item(&self.bus_name)).await
    }

    /// Replaces the visible state, emitting only the signals that
    /// correspond to something that actually changed.
    ///
    /// Emitting all four every tick would make a bar redraw every icon
    /// every few seconds for no reason, which is visible as flicker on
    /// some hosts.
    pub async fn update(&self, next: TrayItem) -> Result<(), TrayError> {
        let mut last = self.last.lock().await;
        if *last == next {
            return Ok(());
        }
        let emitter = SignalEmitter::new(&self.connection, ITEM_PATH).map_err(classify)?;

        let icon_changed = last.icon_name != next.icon_name;
        let status_changed = last.status != next.status;
        let title_changed = last.title != next.title;
        let tooltip_changed =
            last.tooltip_title != next.tooltip_title || last.tooltip_body != next.tooltip_body;

        *self.state.lock().await = next.clone();
        *last = next.clone();
        drop(last);

        if icon_changed {
            ItemInterface::new_icon(&emitter).await.map_err(classify)?;
        }
        if status_changed {
            ItemInterface::new_status(&emitter, next.status.as_str())
                .await
                .map_err(classify)?;
        }
        if title_changed {
            ItemInterface::new_title(&emitter).await.map_err(classify)?;
        }
        if tooltip_changed {
            ItemInterface::new_tool_tip(&emitter).await.map_err(classify)?;
        }
        Ok(())
    }

    pub fn bus_name(&self) -> &str {
        &self.bus_name
    }
}

/// Whether a tray host is currently available to register with.
///
/// Separate from registering so a daemon can wait for a bar to start
/// rather than failing at boot — `hl.exec_cmd` starts things before much
/// of the session exists, and a tray daemon losing that race would
/// otherwise be permanently invisible.
pub async fn watcher_present(connection: &Connection) -> bool {
    match bounded(StatusNotifierWatcherProxy::new(connection)).await {
        Ok(watcher) => bounded(watcher.is_status_notifier_host_registered())
            .await
            .unwrap_or(false),
        Err(_) => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::item::{Category, Status};

    fn item() -> TrayItem {
        TrayItem {
            id: "hyprforge-network".to_string(),
            category: Category::Hardware,
            status: Status::Active,
            title: "Network".to_string(),
            icon_name: "network-wireless-signal-good".to_string(),
            tooltip_title: "Connected".to_string(),
            tooltip_body: "home".to_string(),
        }
    }

    /// The spec identifies an item by its bus name, so the shape of that
    /// name is not cosmetic — a host parses the pid out of it.
    #[test]
    fn the_bus_name_follows_the_convention_hosts_expect() {
        let name = format!("org.kde.StatusNotifierItem-{}-{}", std::process::id(), 1);
        assert!(name.starts_with("org.kde.StatusNotifierItem-"));
        assert_eq!(name.matches('-').count(), 2);
    }

    /// No bar running is an ordinary state — plenty of people run none —
    /// so it has to be distinguishable from the bar being broken, or a
    /// daemon cannot decide whether to wait or to complain.
    #[test]
    fn no_bar_running_is_its_own_state_rather_than_a_failure() {
        let message = TrayError::NoWatcher.to_string();
        assert!(message.contains("no StatusNotifierWatcher"));
        assert!(!matches!(TrayError::NoWatcher, TrayError::Refused(_)));
    }

    /// An item whose state has not changed emits nothing. A bar that is
    /// told its icon changed redraws it, and being told so every tick is
    /// flicker.
    #[test]
    fn an_unchanged_item_is_equal_to_itself_so_update_can_skip_it() {
        assert_eq!(item(), item());
        let mut changed = item();
        changed.icon_name = "network-wireless-signal-weak".to_string();
        assert_ne!(item(), changed);
    }
}
