# Hyprforge — Project Vision & Component Inventory

**Read this before working on any individual component.** Hyprforge is not
a single app — it's a suite of default desktop applications for Hyprland
users, filling the role that Windows/macOS built-in apps fill, without
requiring GNOME or KDE lock-in or the bloat/theming fights that come with
pulling in either of those stacks piecemeal.

Any given work session will typically target ONE component from the
inventory below. That component's detailed spec lives in its own prompt
file. This document exists so that work on one component stays consistent
with the shape of the whole suite, and so no single session mistakes "the
part I'm building" for "the whole project."

## The pitch, one sentence

Mac-quality "it just works" defaults — auto-remembering, consistent,
calm, reversible — implemented as small, focused, iced-based native apps
with zero GTK/Qt dependency anywhere in the stack.

## Design pillars (apply to every component, not just Settings)

1. **Opinionated defaults, hidden complexity.** Auto-detect and auto-apply
   sane behavior; put raw protocol/system detail behind an "advanced" view,
   not the main one.
2. **One shared visual and interaction grammar across all apps.** Same
   sidebar/search pattern, same widget vocabulary, same spacing — a user
   who learns one Hyprforge app should already half-know the others.
3. **No dead ends.** Errors surface as an actionable message in the app,
   never "check the logs."
4. **Instant, visible, reversible feedback.** Nothing requires faith that a
   change worked. Destructive actions default to reversible (trash not
   delete) or get a confirm/revert window (config changes).
5. **Search-first navigation** once an app has enough surface area to need
   it (Settings, File Manager).
6. **Learns by behavior, not tutorials.** Auto-learn / auto-remember beats
   onboarding wizards (see: Displays module's fingerprint-based memory).
7. **Accessibility (font scaling, contrast, keyboard nav) built into the
   shared component layer from the start, not retrofitted per-app.**
8. **A shared Quick Look-style preview component** — spacebar-to-preview a
   file — is a cross-cutting piece multiple apps call into (File Manager,
   Photo Viewer), not something owned by one app.
9. **One keyboard shortcut grammar, enforced identically across every app**
   (exact bindings TBD, but pick once, document it, never diverge per-app).

## Toolkit & language (applies to every component)

- **iced**, exclusively, for every GUI in the suite. No GTK, no Qt,
  anywhere, even transitively where avoidable.
- **Rust**, throughout, including daemons.
- Shared look/theme/widget code lives in `hyprforge-look` (the `Color` type
  and the runtime `Theme`; no iced, because the lock screen and greeter
  paint into a raw Wayland buffer) and `hyprforge-ui` (the iced widgets and
  palette built on top of it), and every app depends on those rather than
  reinventing its own styling. `hyprforge-core` is Hyprland config
  machinery — hlconfig, hyprlang, Lua codegen — not the look; see "The
  shared look" below.

## Component inventory

| Component | Crate(s) | Replaces | Daemon? | Status |
|---|---|---|---|---|
| **Settings app** (shell) | `hyprforge-settings` | GNOME/KDE Settings | no (hosts modules that may talk to daemons) | eight working screens (Monitors, Window Rules, Shortcuts, Input, Appearance, Desktop, Session, System), ~17.5k lines |
| — Displays module | `hyprforge-displayd` (daemon) | manual `wlr-randr`/GUI fiddling | **yes**, systemd user service, D-Bus API | in progress |
| — Window Rules module | `hyprforge-windowrules` (lib) | hand-written Lua rules | no | in progress |
| — Shortcuts module | `hyprforge-shortcuts` (lib) | hand-edited keybinds | no | in progress; TOML storage, Lua codegen, live conflict detection against `hyprctl binds` |
| — Network module | `hyprforge-network` (lib) | nm-applet/nmtui | no new daemon, talks to NetworkManager D-Bus directly | in progress; model, backend seam, NetworkManager client and the Settings screen (Wi-Fi list, join, forget, radio toggle) are done. Deferred: VPN, 802.1X enterprise, hotspot |
| — Bluetooth module | `hyprforge-bluetooth` (lib) | blueman | no new daemon, talks to BlueZ D-Bus directly | in progress; adapter power, discovery, device list, connect/disconnect/forget/trust, and a Settings screen. Pairing deferred — needs an `org.bluez.Agent1` passkey flow |
| — Audio module | *(planned)* | pavucontrol | talks to PipeWire | not started |
| — Power module | *(planned)* | — | talks to UPower/power-profiles-daemon | not started |
| — Desktop module (wallpaper, night light, idle) | `hyprforge-ecosystem` (lib) | hyprpaper/hyprsunset/hypridle config by hand | no (drives the existing daemons) | in progress |
| — Appearance module | `hyprforge-appearance` (lib) | GNOME/KDE appearance settings | no (writes gsettings via CLI) | in progress |
| — Input/keyboard module | `hyprforge-input` (lib) | GNOME/KDE keyboard & touchpad settings | no | in progress |
| — Session module (autostart, environment, gestures, permissions) | `hyprforge-session` (lib) | hand-edited hyprland.lua | no | in progress |
| — System module (behaviour, shortcuts behaviour, X11, rendering) | `hyprforge-system` (lib) | hand-edited hyprland.lua | no | in progress |
| — Users/time module | *(planned)* | — | accountsservice/timedated | not started |
| — Lua import (hand-written config) | `hyprforge-lua-import` (lib) | — | no (sandboxed `mlua` evaluator, the only crate depending on mlua) | in progress; imports hand-written `hl.bind()`/`hl.window_rule()`/`hl.monitor()` calls |
| **Tray icons** | `hyprforge-tray` (lib + `hyprforge-trayd`) | nm-applet, blueman-tray | **yes**, a small user daemon | in progress; Wi-Fi and Bluetooth icons over `org.kde.StatusNotifierItem` with `com.canonical.dbusmenu` menus, scanning on menu open. Deferred: submenus, and items for Audio/Power until those backends exist |
| — Tray: keep awake | `hyprforge-tray` item *(planned)* | a hand-run `systemd-inhibit … sleep 8h` | no (logind `Inhibit`) | not started; the smallest of these and the one already being done by hand |
| — Tray: night light | `hyprforge-tray` item *(planned)* | redshift/gammastep applets | no (drives hyprsunset) | not started; `ecosystem::sunset` models the profiles but has no runtime toggle yet |
| — Tray: displays | `hyprforge-tray` item *(planned)* | — | no (talks to `hyprforge-displayd` over D-Bus) | not started; switch profile on dock, and surface the revert countdown. The first item with a real use for `Status::Passive` — invisible until it has something to say |
| **Bar** | *(planned)* | waybar | no | not started; would also be the `StatusNotifierHost`, so the tray items already target it |
| **Lock screen** | `hyprforge-lock` (binary), `hyprforge-authui` (conversation model) | hyprlock | no (a client holding `ext-session-lock-v1`) | locks, draws, authenticates against PAM and unlocks, with a look shared with the greeter via `hyprforge-look`; see "The lock screen" in the README for the genuine remaining gaps (no input-method support, password not zeroized, no attempt limiting of its own) |
| *(shared foundation)* | `hyprforge-paths` (xdg + atomic writes), `hyprforge-look` (colour type + runtime Theme), `hyprforge-ui` (iced widgets and palette) | — | no | in place; every future app builds on these |
| **Greeter / display manager** | `hyprforge-greet`, on `hyprforge-authui` | greetd greeters (gtkgreet/tuigreet) | runs under greetd | ~885 lines; has an installer (`./hyprforge --install --greeter`) and its own `INSTALL.md` |
| **File Manager** | `hyprforge-files-core` (shared logic), `hyprforge-files` (standalone), `hyprforge-files-portal` (xdg-desktop-portal FileChooser backend) | Nautilus/Dolphin/Thunar | portal backend runs as a D-Bus service | not started |
| **Photo Viewer** | `hyprforge-photos` | eog/gwenview | no | not started |
| **Video Viewer** | `hyprforge-videos` | — (likely thin mpv wrapper; revisit build-vs-wrap) | no | not started |
| **Process Manager** | `hyprforge-procman` | Windows Task Manager / GNOME System Monitor (Mission Center already covers this well — revisit whether to build vs. skip before starting) | no (reads /proc, polls) | not started |
| **Service Manager** | `hyprforge-servicemgr` | `systemctl` CLI | talks to systemd D-Bus API directly | not started |
| **Disk Utility** | `hyprforge-disks` | GNOME Disks/GParted | talks to udisks2 D-Bus | not started |
| **Notepad** | `hyprforge-notes` | Notepad/TextEdit | no | not started |
| **Calculator** | `hyprforge-calc` | Calculator apps | no | not started |
| **Calendar** | `hyprforge-calendar` | Calendar apps | possibly syncs via CalDAV — daemon TBD if background sync is wanted | not started |

Five or six tray icons is not obviously too many *here*, because a bar of
this suite's own is planned and the two decisions are related: an item
that is spec-compliant `StatusNotifierItem` works in waybar today and in
that bar later, unchanged. Until then `tray.toml` carries a per-icon
switch, and new items default to off.

This list is not necessarily final or exhaustive — treat it as the current
shared understanding of scope, updated as decisions are made, not as a
locked spec.

## Workspace structure

Single Cargo workspace at the repo root. The original intent was for every
component above to get its own crate(s) under `crates/`, even before it's
built — stub/empty crates as placeholders so the workspace shape reflects
the intended full suite. That convention was not followed in practice:
`Cargo.toml`'s `members` lists only crates that actually have code in them,
and planned components with no crate yet (Bluetooth, Audio, Power,
Users/time, File Manager, Photo Viewer, Video Viewer, Process Manager,
Service Manager, Disk Utility, Notepad, Calculator, Calendar) simply have
none. Treat the table above, not the workspace listing, as the source of
truth for what's planned.

```
hyprforge/
  Cargo.toml
  crates/
    hyprforge-paths/            # xdg + atomic writes, no dependencies
    hyprforge-look/             # Color type + runtime Theme, no iced
    hyprforge-ui/                # the iced layer: widgets, palette, spacing
    hyprforge-core/              # Hyprland config machinery: hlconfig, hyprlang,
                                  # Lua codegen, D-Bus proxy for displayd
    hyprforge-displayd/
    hyprforge-windowrules/
    hyprforge-shortcuts/
    hyprforge-input/
    hyprforge-appearance/
    hyprforge-ecosystem/
    hyprforge-session/
    hyprforge-system/
    hyprforge-lua-import/
    hyprforge-network/
    hyprforge-settings/          # the Settings app shell + its modules
    hyprforge-authui/            # shared auth conversation model
    hyprforge-lock/
    hyprforge-greet/
```

## Shared architectural patterns to reuse across components

- **"Own a separate file, never round-trip the user's own config"** — the
  pattern used for window rules (owning a `require()`'d Lua file rather
  than editing the user's `hyprland.lua` directly) generalizes: wherever a
  Hyprforge component needs to persist something into a config space the
  user might also hand-edit, prefer owning a clearly-labeled separate file
  over parsing and rewriting theirs.
- **TOML as canonical source of truth**, with any generated non-TOML
  artifact (like `window-rules.lua`) treated as fully regenerable and never
  parsed back.
- **D-Bus for daemon-backed modules**, consistent with what Network,
  Bluetooth, Displays, systemd (Service Manager), and udisks2 (Disk
  Utility) all natively expose anyway — don't invent a bespoke IPC format
  per component.
- **A module/app trait modeled on iced's own `update`/`view` shape** for
  anything that plugs into the Settings app shell specifically (Displays,
  Window Rules, Network, etc. are *modules within Settings*; File Manager,
  Photo Viewer etc. are *standalone apps*, not modules — don't conflate
  the two).

## Current session scope

Unless told otherwise, assume the active work session is scoped to
whatever is described in the accompanying session-specific prompt, and
NOT the whole suite. The Displays + Window Rules work is one slice of the
Settings app, not the entirety of Hyprforge. Do not expand scope to other components without
it being explicitly requested for that session.

### The shared look

Every app reads one `hyprforge_look::Theme`, resolved from settings the user
already controls: the accent is Hyprland's `general:col:active_border`, the
fonts and text scale come from gsettings. There is deliberately no Hyprforge
theme file — that would be a second place to configure colours that already
exist, and one the user would have to find.

This was not the original arrangement. The Settings app and the lock screen
each had their own palette and had already drifted apart, which is precisely
the "looks like two different systems" problem this suite exists to solve,
occurring inside the suite. A new app should depend on `hyprforge-ui` and get
the look for free; if it ever needs to define a colour of its own, that is a
sign something belongs in `hyprforge-look` instead.

### Where things stand (2026-09-12)

The Settings modules above marked *in progress* are functional; their look
and feel is deliberately unfinished, functionality first. The Settings app
itself is eight working screens (Monitors, Window Rules, Shortcuts, Input,
Appearance, Desktop, Session, System) at roughly 17.5k lines.

`hyprforge-lock` locks, draws, authenticates against PAM and unlocks, and
now has the look: it shares `hyprforge-look`'s runtime `Theme` with the rest
of the suite, resolved from the user's own Hyprland/gsettings config rather
than an invented palette. See "The lock screen" in the README for what it
still genuinely lacks (no input-method support, the password isn't
zeroized, no attempt limiting of its own).

`hyprforge-greet` exists: ~885 lines, on the same `hyprforge-authui`
conversation model as the lock screen, with an installer
(`./hyprforge --install --greeter`) and its own `INSTALL.md`. That match
between greeter and lock screen is the whole reason `hyprforge-authui`
exists: the problem being solved is that a greeter and a lock screen
normally look like two different systems.

`hyprforge-network` (Wi-Fi over NetworkManager) is new since the last time
this section was written, and is the first module built on a backend trait
from the start rather than after the fact — `NetworkBackend` has a mock, so
everything above it is testable on a machine with no NetworkManager, no
adapter and no access point. What can only be answered by the service gets
a read-only live tier of its own in `check.sh`.

It lists networks, joins them, forgets them and toggles the radio.
Deliberately not done yet: VPN, 802.1X enterprise (listed, but says why it
cannot be joined rather than offering a passphrase box that cannot work)
and hotspot.
