---
name: dbus-module
description: Build or extend a Hyprforge module that talks to a system or session service over D-Bus (NetworkManager, BlueZ, logind, UPower, power-profiles-daemon, UDisks2, polkit, a tray host, systemd), or any other daemon client. Use when starting a new service-backed crate or page, adding a call to an existing client, or deciding where a decision belongs relative to the bus.
---

# The shape a D-Bus-backed module takes

Moved here from CLAUDE.md, which keeps a pointer. The order below is the
procedure; each paragraph after it is why.

1. The backend trait and its mock, first.
2. The model as plain data, and every decision as a pure function over
   it, unit-tested against the mock.
3. Only then the client: marshalling, behind the trait.
4. Live tests in the `live` tier, read-only, behind a gate on this one
   service — the `add-a-live-test` skill.
5. The two failure states named in the UI: the service not running, and
   a connection that failed and is retried rather than cached.

Five of these now exist — Network over NetworkManager, Bluetooth over
BlueZ, power over systemd-logind, UPower and power-profiles-daemon, the
tray's item over StatusNotifierItem (its menu is drawn by its own popup on
Hyprland and served over dbusmenu everywhere else), and Files' drives over UDisks2 —
and they were each better for being built in this order. It is
written down so a sixth does not re-derive it.

**The backend trait and its mock come before the D-Bus client.** Not
after. The machine running tier 1 has no guaranteed NetworkManager, no
adapter, no access point and no bar, so everything above the trait is
testable only if the seam exists first. `hyprforge-displayd`'s
`backend/wlr.rs` is the counter-example: 595 lines with no tests, because
the mock arrived late and the protocol code never got a seam of its own.

**The model is plain data, and the decisions are pure functions over it.**
"Which icon does this state deserve", "which rows does this menu need",
"is this network WPA3" — all of it belongs above the bus, where a test
can reach it. What is left in the client is marshalling, and marshalling
is what the live tier is for.

**Every service gets its own gate in `tools/hflib/tiers.py`, on the thing it
actually asks.** Not on the compositor. See the rule above about a check
that silently never runs.

**The live tier is read-only.** It runs on a machine somebody is using,
quite possibly over the connection or the bar being inspected. A test
that can drop your Wi-Fi, unpair your mouse or start a discovery session
is not worth the coverage. The tray is the one exception worth naming:
registering really does put an icon in the user's bar for a fraction of a
second, and that is the smallest observable form of "a host accepted it".

**Ask what only the real service can answer, and assert that.** Not that
a call returned. `Strength` is a percentage; `NM_DEVICE_TYPE_WIFI` is
still 2; the watcher *lists* the item rather than merely accepting the
registration; the item declares no `Menu` property at all.
Every one of those is a claim this code makes about somebody else's, and
the compiler cannot see any of them.

**Two failures that always need naming, in both directions.** The daemon
not running is not an empty list — it is its own state with its own
message and a way out of it. And a connection that failed is never
cached, because the message telling the user to start the service keeps
being shown after they do. That one has been got wrong three times.
