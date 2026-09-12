//! Does a **real tray host** accept what this crate registers?
//!
//! The unit tests check this crate against its own idea of the protocol.
//! Nothing in them can tell whether a bar will actually show the icon —
//! a wrong property type, a missing one, or a bus name in the wrong shape
//! all produce an item that registers happily and never appears.
//!
//! These tests register a real item with whatever `StatusNotifierWatcher`
//! is running and assert the watcher lists it. **The visible side effect
//! is an icon appearing in the user's bar for a fraction of a second**,
//! which is the smallest observable form of "it worked". Nothing else is
//! touched: no configuration is written, nothing is clicked, and the item
//! is dropped at the end of each test.
//!
//! Run with `cargo test -p hyprforge-tray -- --ignored`.

use hyprforge_tray::item::{Category, Status};
use hyprforge_tray::sni::TrayError;
use hyprforge_tray::{TrayIcon, TrayItem};

/// libtest has no skipped state, so a check that could not run says so
/// rather than returning early and printing `ok` like one that passed.
const SKIP_MARKER: &str = "HYPRFORGE-SKIP:";

fn item(id: &str) -> TrayItem {
    TrayItem {
        id: id.to_string(),
        category: Category::Hardware,
        status: Status::Active,
        title: "Hyprforge test item".to_string(),
        icon_name: "network-wireless-signal-good".to_string(),
        tooltip_title: "Hyprforge".to_string(),
        tooltip_body: "live test, disappears immediately".to_string(),
    }
}

/// The claim everything else rests on: a bar accepts this registration.
#[tokio::test]
#[ignore]
async fn a_running_tray_host_accepts_the_item_this_crate_registers() {
    let (clicks, _rx) = tokio::sync::mpsc::unbounded_channel();
    match TrayIcon::register(item("hyprforge-network"), 90, clicks).await {
        Ok(icon) => {
            println!("registered as {}", icon.bus_name());
            assert!(icon.bus_name().starts_with("org.kde.StatusNotifierItem-"));
        }
        Err(TrayError::NoWatcher) => {
            eprintln!("{SKIP_MARKER} no StatusNotifierWatcher is running (no bar with a tray)");
        }
        Err(e) => panic!("a tray host was present but refused the item: {e}"),
    }
}

/// Registering is not the same as being listed. A watcher can accept the
/// call and drop the item — for a malformed bus name, say — so this asks
/// the watcher what it actually holds.
#[tokio::test]
#[ignore]
async fn the_watcher_lists_the_item_after_it_registers() {
    let (clicks, _rx) = tokio::sync::mpsc::unbounded_channel();
    let icon = match TrayIcon::register(item("hyprforge-bluetooth"), 91, clicks).await {
        Ok(icon) => icon,
        Err(TrayError::NoWatcher) => {
            eprintln!("{SKIP_MARKER} no StatusNotifierWatcher is running (no bar with a tray)");
            return;
        }
        Err(e) => panic!("registration failed: {e}"),
    };

    // The watcher records the registration asynchronously; give it a beat
    // rather than racing it.
    tokio::time::sleep(std::time::Duration::from_millis(400)).await;

    let connection = zbus::Connection::session().await.expect("a session bus");
    let listed: Vec<String> = connection
        .call_method(
            Some("org.kde.StatusNotifierWatcher"),
            "/StatusNotifierWatcher",
            Some("org.freedesktop.DBus.Properties"),
            "Get",
            &("org.kde.StatusNotifierWatcher", "RegisteredStatusNotifierItems"),
        )
        .await
        .and_then(|m| m.body().deserialize::<zbus::zvariant::Value>().map(|v| {
            Vec::<String>::try_from(v).unwrap_or_default()
        }))
        .unwrap_or_default();

    assert!(
        listed.iter().any(|n| n.contains(icon.bus_name())),
        "the watcher accepted the registration but does not list {}; it holds {listed:?}",
        icon.bus_name()
    );
}

/// Updating must not require re-registering, and must not error when
/// nothing changed — the daemon calls this on every poll tick.
#[tokio::test]
#[ignore]
async fn an_update_reaches_the_host_and_an_unchanged_one_is_cheap() {
    let (clicks, _rx) = tokio::sync::mpsc::unbounded_channel();
    let icon = match TrayIcon::register(item("hyprforge-network"), 92, clicks).await {
        Ok(icon) => icon,
        Err(TrayError::NoWatcher) => {
            eprintln!("{SKIP_MARKER} no StatusNotifierWatcher is running (no bar with a tray)");
            return;
        }
        Err(e) => panic!("registration failed: {e}"),
    };

    icon.update(item("hyprforge-network"))
        .await
        .expect("an identical update is a no-op, not an error");

    let mut changed = item("hyprforge-network");
    changed.icon_name = "network-wireless-signal-weak".to_string();
    changed.status = Status::NeedsAttention;
    icon.update(changed).await.expect("a changed update emits and succeeds");
}
