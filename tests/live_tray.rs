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

/// `GetLayout`'s reply, spelled out: `(u(ia{sv}av))` — a revision, then a
/// node of `(id, properties, children)` whose children are variants
/// wrapping more nodes of the same shape.
type LayoutReply = (
    u32,
    (
        i32,
        std::collections::HashMap<String, zbus::zvariant::OwnedValue>,
        Vec<zbus::zvariant::OwnedValue>,
    ),
);

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

/// The menu, over the wire.
///
/// `GetLayout` returns `(u(ia{sv}av))` — a recursive structure whose
/// children are variants wrapping more of the same. A wrong signature
/// does not fail anywhere: the host reads a menu with no rows and shows
/// an empty popup, with nothing logged at either end. The only way to
/// know is to put it on a real bus and read it back.
#[tokio::test]
#[ignore]
async fn a_menu_survives_the_round_trip_through_d_bus() {
    use hyprforge_tray::menu::{Menu, MenuItem};

    let (clicks, _rx) = tokio::sync::mpsc::unbounded_channel();
    let (menu_clicks, _mrx) = tokio::sync::mpsc::unbounded_channel();
    let menu = Menu::new(vec![
        MenuItem::checkmark("Wi-Fi", true, "radio:toggle"),
        MenuItem::separator(),
        MenuItem::standard("home", "connect:home"),
        MenuItem::standard("Network settings…", "settings"),
    ]);

    let icon = match TrayIcon::register_with_menu(
        item("hyprforge-network"),
        menu,
        93,
        clicks,
        menu_clicks,
    )
    .await
    {
        Ok(icon) => icon,
        Err(TrayError::NoWatcher) => {
            eprintln!("{SKIP_MARKER} no StatusNotifierWatcher is running (no bar with a tray)");
            return;
        }
        Err(e) => panic!("registration failed: {e}"),
    };

    let connection = zbus::Connection::session().await.expect("a session bus");

    // The item must advertise where its menu lives, or no host ever asks.
    let menu_path: zbus::zvariant::OwnedObjectPath = connection
        .call_method(
            Some(icon.bus_name()),
            "/StatusNotifierItem",
            Some("org.freedesktop.DBus.Properties"),
            "Get",
            &("org.kde.StatusNotifierItem", "Menu"),
        )
        .await
        .expect("the Menu property is readable")
        .body()
        .deserialize::<zbus::zvariant::Value>()
        .map(|v| {
            zbus::zvariant::OwnedObjectPath::try_from(v).expect("Menu is an object path")
        })
        .expect("the Menu property deserializes");
    assert_eq!(menu_path.as_str(), "/StatusNotifierItem/Menu");

    // And the layout must actually deserialize into the shape the
    // protocol specifies, with our rows in it.
    let reply = connection
        .call_method(
            Some(icon.bus_name()),
            menu_path.as_str(),
            Some("com.canonical.dbusmenu"),
            "GetLayout",
            &(0i32, -1i32, Vec::<String>::new()),
        )
        .await
        .expect("GetLayout answers");

    let (revision, root): LayoutReply = reply.body().deserialize().expect(
        "the layout deserializes as (u(ia{sv}av)) — if this fails the signature is wrong \
         and every host would show an empty menu",
    );

    assert!(revision >= 1, "a host ignores a layout it has already seen");
    assert_eq!(root.0, 0, "the root is id 0");
    assert_eq!(root.2.len(), 4, "four rows went in; got {}", root.2.len());

    // And the rows carry their labels, which is what the user reads.
    let labels: Vec<String> = root
        .2
        .iter()
        .filter_map(|child| {
            let value = zbus::zvariant::Value::from(child.clone());
            let zbus::zvariant::Value::Structure(s) = value else {
                return None;
            };
            let props: std::collections::HashMap<String, zbus::zvariant::OwnedValue> =
                std::collections::HashMap::try_from(s.fields()[1].try_clone().ok()?).ok()?;
            props
                .get("label")
                .and_then(|v| String::try_from(v.clone()).ok())
        })
        .collect();
    assert!(
        labels.iter().any(|l| l == "home"),
        "the rows lost their labels on the wire; got {labels:?}"
    );
}
