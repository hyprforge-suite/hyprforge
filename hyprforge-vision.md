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
  and the runtime `Theme`; no iced, because config machinery and the
  theme publisher use it without a window) and `hyprforge-ui` (the iced widgets and
  palette built on top of it), and every app depends on those rather than
  reinventing its own styling. `hyprforge-core` is Hyprland config
  machinery — hlconfig, hyprlang, Lua codegen — not the look; see "The
  shared look" below.

## Component inventory

| Component | Crate(s) | Replaces | Daemon? | Status |
|---|---|---|---|---|
| **Settings app** (shell) | `hyprforge-settings` | GNOME/KDE Settings | no (hosts modules that may talk to daemons) | eighteen pages over thirteen modules (Monitors, Window Rules, Shortcuts, Input, Network, Bluetooth, Power, Tray, Appearance, Desktop, Default Applications, Session, System — Appearance's and Desktop's tabs became pages of their own; see "Where things stand"), every page restyled to the Settings mockup, ~26.5k lines. Planned: a Keyboard screen for the suite's own app bindings (Files' `files-config.toml`, the viewer's `media-config.toml`), editing the same `hyprforge-keys` grammar both apps already read — asked for once the shared grammar landed |
| — Displays module | `hyprforge-displayd` (daemon) | manual `wlr-randr`/GUI fiddling | **yes**, systemd user service, D-Bus API | in progress |
| — Window Rules module | `hyprforge-windowrules` (lib) | hand-written Lua rules | no | in progress |
| — Shortcuts module | `hyprforge-shortcuts` (lib) | hand-edited keybinds | no | in progress; TOML storage, Lua codegen, live conflict detection against `hyprctl binds` |
| — Network module | `hyprforge-network` (lib) | nm-applet/nmtui | no new daemon, talks to NetworkManager D-Bus directly | in progress; model, backend seam, NetworkManager client and the Settings screen (Wi-Fi list, join, forget, radio toggle) are done. Wired: each Ethernet interface's state (connected, connecting, cable unplugged, or cable in with nothing active), connection name and link speed, and connecting or disconnecting one — on both the tray's network icon and the Network screen, which also treats a machine with no Wi-Fi adapter as a state rather than a warning. Deferred: VPN, 802.1X enterprise, hotspot |
| — Bluetooth module | `hyprforge-bluetooth` (lib) | blueman | no new daemon, talks to BlueZ D-Bus directly | in progress; adapter power, discovery, device list, connect/disconnect/forget/trust, and a Settings screen. Pairing is now built: `agent::register` implements `org.bluez.Agent1`, `pairing::PairingPrompt` models Confirm/Display/Authorize, and the Settings screen's `subscription()` wires a live `pairing_stream()` that forwards BlueZ's prompts to the screen and sends the answer back |
| — Audio module | *(planned)* | pavucontrol | talks to PipeWire | not started |
| — Power module (keep awake, battery, power profiles) | `hyprforge-power` (lib) | — | no new daemon, talks to systemd-logind/UPower/power-profiles-daemon directly | in progress; three independent backend traits (`InhibitBackend`, `BatteryBackend`, `PowerProfilesBackend`), each with its own mock and its own live `check.sh` tier, and now a Settings **Power** screen on top — keep-awake toggle plus the other holders of an inhibit, battery percentage/state, and profile switching, each section collapsing on its own if that one daemon is down rather than taking the whole screen with it. the tray's keep-awake icon now opens it (both its left click and its menu row were repointed off the Desktop screen's old `idle` tab, which a trayd test keeps in sync). The tray has an item for battery and profile too (see "Tray: battery and power profile") |
| — Desktop module (wallpaper, night light, idle) | `hyprforge-ecosystem` (lib) | hyprpaper/hyprsunset/hypridle config by hand | no (drives the existing daemons) | in progress |
| — Appearance module | `hyprforge-appearance` (lib) | GNOME/KDE appearance settings | no (writes gsettings via CLI) | in progress |
| — Input/keyboard module | `hyprforge-input` (lib) | GNOME/KDE keyboard & touchpad settings | no | in progress |
| — Session module (autostart, environment, gestures, permissions) | `hyprforge-session` (lib) | hand-edited hyprland.lua | no | in progress |
| — System module (behaviour, shortcuts behaviour, X11, rendering) | `hyprforge-system` (lib) | hand-edited hyprland.lua | no | in progress |
| — Users/time module | *(planned)* | — | accountsservice/timedated | not started |
| — Lua import (hand-written config) | `hyprforge-lua-import` (lib) | — | no (sandboxed `mlua` evaluator, the only crate depending on mlua) | in progress; imports hand-written `hl.bind()`/`hl.window_rule()`/`hl.monitor()` calls |
| **Tray icons** | `hyprforge-tray` (lib + two binaries: `hyprforge-trayd` and `hyprforge-traymenu`, its menu popup) | nm-applet, blueman-tray | **yes**, a small user daemon | in progress; six `org.kde.StatusNotifierItem`s (network covering both Wi-Fi and Ethernet, Bluetooth, keep-awake, night light, battery and power profile, display layouts), scanning on menu open, each icon's visibility gated by `hyprforge_tray::prefs` and re-read every poll tick. Visibility there means the user's own choice and nothing else: no item is ever published `Status::Passive`, so a radio switched off keeps its icon (drawn switched-off, since the icon name is the whole vocabulary StatusNotifierItem offers for that) rather than vanishing at the moment its own menu could switch it back on. A Settings **Tray** screen now owns all of it — the six toggles, the menu's Y offset, and whether clicking away dismisses the menu. No longer serves `com.canonical.dbusmenu` at all: `menu()` now answers no `Menu` property, and a right click spawns `hyprforge-traymenu`, a sibling `PopupApp` built on `hyprforge-popup` that draws the menu itself, themed and anchored like the rest of the suite. The cost, stated plainly: before this, any spec-compliant tray host could draw these menus itself, styled and positioned however it chose; now only `hyprforge-traymenu` can, and a bar with no Hyprforge installed sees no menu at all, just the icon's primary action. Deferred: submenus |
| — Tray: keep awake | `hyprforge-tray` item, `hyprforge-power`'s `InhibitBackend` | a hand-run `systemd-inhibit … sleep 8h` | no (logind `Inhibit`) | built; `check.sh` has a live tier, "Live tests against systemd-logind". Now has a home in Settings — the Power screen's keep-awake toggle and its list of other holders — and the tray icon opens it, having been repointed off the Desktop screen's old `idle` tab stand-in |
| — Tray: night light | `hyprforge-tray` item, `hyprforge-ecosystem::sunset_control` | redshift/gammastep applets | no (drives hyprsunset via `hyprctl`) | built; `check.sh` has a live tier, "Live tests against hyprsunset". Its Settings home is the Night light page, and both the icon and its menu's settings row open it directly (`--screen night-light`) |
| — Tray: battery and power profile | `hyprforge-tray` item, `hyprforge-power`'s `BatteryBackend` and `PowerProfilesBackend` | battery applets, `powerprofilesctl` | no (UPower and `power-profiles-daemon`) | built, off by default; the battery level when there is a battery and the active profile when there is not, and a menu to switch profiles. Each daemon failing is shown on its own, so UPower being down neither hides the profile switch nor reads as "no battery". Opens the Power screen |
| — Tray: displays | `hyprforge-tray` item, `hyprforge_core::displayd_proxy` | — | no (talks to `hyprforge-displayd` over D-Bus) | built, off by default; lists saved layouts to switch between, most recently used first, and switching is always the reversible apply. While a layout waits to be kept — however it was applied, since the icon follows displayd's `RevertPending`/`RevertResolved` signals — the icon asks for attention under a name of its own and the menu is only Keep and Revert. Always visible: this row used to plan it as the first real use of `Status::Passive`, and that was decided against for the reason in CLAUDE.md about an icon a host is asked to hide |
| **Bar** | *(planned)* | waybar | no | not started; would also be the `StatusNotifierHost`, so the tray items already target it |
| **Clipboard** | `hyprforge-clipboard` (lib + two binaries: the `hyprforge-clipd` daemon and the `hyprforge-clipmenu` popup) | copyq | **yes**, `hyprforge-clipd`, a user daemon over `wlr-data-control`/`ext-data-control` | in progress; history recording, paste synthesis, and a popup that appears at the pointer, drawn to the menus mockup, are built — search, filter tabs (text, images, links, files, read from the content since nothing else is recorded), Pinned and Recent sections, a preview pane, and pin, delete and a two-click "clear history" that never touches pinned entries. Built around one rule: `Sensitivity` is checked before an offer's bytes are ever requested, so a secret is never read, hashed, stored or logged, and `Debug` on an entry renders a description rather than content. `hyprforge-clipboard` is deliberately standalone — no settings app, no tray, no Hyprland config machinery — and is one of the nine components already split into its own repository (see "Where things stand"). `check.sh` has a live tier, "Live tests against the Wayland clipboard"; the CI in the split repository is tier-1 only, since a compositor-backed test needs a real Wayland session no runner has |
| **Emoji picker** | `hyprforge-emoji` (data + pure search), `hyprforge-emojimenu` (popup) | the emoji picker GNOME/KDE ship and Hyprland users otherwise go without | no (short-lived, per-invocation process on a keybind, same shape as `hyprforge-clipmenu`) | built, drawn to the menus mockup; a grid of all 1914 fully-qualified Unicode 17.0 emoji at the pointer, type-to-filter search, a "Frequently used" section from remembered pick counts, skin tones with a long-press (or Shift+Enter) for the five variants and a persisted default tone, and Kaomoji and Symbols tabs, bound to a key the user sets. `hyprforge-emoji` is pure data and a pure `search` function, no filesystem/network/runtime access at all; `hyprforge-emojimenu` is the popup built on `hyprforge-popup` |
| **Notifications** | the nested `notif/` workspace: `notif-types`, `notif-config`, `notif-core`, `notif-dbus`, `notif-render`, `notif-wl`, `notif-ipc`, plus the `notifd`/`notifctl` binaries | dunst/mako | **yes**, `notifd`, a user daemon (`org.freedesktop.Notifications` over D-Bus) | built and usable standalone: full D-Bus notification server, layer-shell toasts, a notification-center panel, history, do-not-disturb, hot-reloading config, and `notifctl` for CLI control. Kept as its own nested Cargo workspace on purpose, excluded from the root workspace's `members` — zbus picks its async runtime by feature, the suite needs `zbus/tokio` and notif needs `zbus/async-io`, and unifying the two would give notif's D-Bus code a tokio code path with no tokio runtime under it. `notif-render`'s golden-image tests read the shared `hyprforge_look::Theme` (the same `lock.toml` the lock screen and Settings app read) so notification colours match the rest of the suite without being configured twice. `check.sh`'s "notif workspace" step is what runs its tests, since `cargo test --workspace` at the root does not reach it |
| **Lock screen** | `hyprforge-lock` (binary), `hyprforge-authui` (conversation model) | hyprlock | no (a client holding `ext-session-lock-v1`) | locks, draws, authenticates against PAM (or a finger through fprintd, then PAM's account stack) and unlocks, drawn to the glass-card mockup at each output's real resolution — idle clock, card, shake on failure, status line, media and notification counts, power menu, the card on the keyboard's monitor only — with a look shared with the greeter via `hyprforge-look`; see "The lock screen" in the README for the genuine remaining gaps (no input-method support, no password attempt limiting of its own — deliberately, since that belongs in `/etc/pam.d`; the fingerprint path limits itself to three misses because PAM never sees it). The typed password is now erased on drop by `hyprforge-secret`; what PAM and greetd keep once the answer is handed over is outside this code |
| *(shared foundation)* | `hyprforge-paths` (xdg + atomic writes), `hyprforge-look` (colour type + runtime Theme), `hyprforge-ui` (iced widgets and palette), `hyprforge-popup` (layer-shell popup shell: surface, pointer/keyboard, flip-then-clamp placement, singleton lock, scrollbar, fractional-scale rendering, and the `Dismissal` choice that decides whether a click elsewhere closes the popup or is swallowed by it) | — | no | in place; every future app builds on these. `hyprforge-popup` was extracted from `hyprforge-clipmenu` once a second and third popup (`hyprforge-emojimenu`, `hyprforge-traymenu`) needed the same Wayland/iced plumbing rather than copying it; it is listed here rather than as its own inventory component because, like `hyprforge-look`/`hyprforge-ui`, nothing user-facing depends on it alone — it only ever appears through a popup that builds on it |
| **Greeter / display manager** | `hyprforge-greet`, on `hyprforge-authui` | greetd greeters (gtkgreet/tuigreet) | runs under greetd | ~1.1k lines, drawn to the same glass-card mockup as the lock screen; has an installer (`./hyprforge --install --greeter`) and its own `INSTALL.md` |
| **File Manager** | `hyprforge-files-core` (the model, the `FsBackend` seam, and the browsing view both hosts render), `hyprforge-fileops` (trash, copy, move), `hyprforge-files` (the app), `hyprforge-files-portal` (xdg-desktop-portal FileChooser backend) *(planned)* | Nautilus/Dolphin/Thunar | portal backend runs as a D-Bus service | in progress; the model, sorting/filtering, remembered prefs, the freedesktop trash spec (including the cross-filesystem `$topdir/.Trash-$uid` case, which is reachable on an ordinary btrfs laptop because subvolumes report different device numbers), copy/move/rename with per-chunk cancellation, and the browsing view — sidebar, path bar, list and grid, search, multi-select, history. The open/save dialog renders that *same* view rather than a reduced copy of it, which is enforced structurally: `view()` takes a private `ViewModel` with no `mode` field, so it cannot branch on which host it is in. Archives are built (`hyprforge-archive`): zip, tar over gzip/bzip2/xz/zstd, and 7z, all pure Rust so a machine with no p7zip still opens a `.7z`. An archive is a *place* — `~/Downloads/x.tar.xz/bin` is a path this browser navigates to, routed by the same `RoutingBackend` that gives the Trash its own listing — and it is read *and* written: extract, compress, and add/rename/delete in place, each rewrite going to a temporary file beside the original and renamed over it so an interrupted edit never leaves a truncated archive. Encrypted archives ask for a password, kept in memory for the life of the process and never written anywhere. A file opened out of an archive is a scratch copy, watched, with the window offering to put an edit back rather than doing it silently. Mockup `1j` called the archive view read-only; that was reconsidered in favour of editing. A preview pane beside the listing shows the selected entry with its kind, size and date, or a total for a multiple selection: a picture (decoded by `hyprforge-image`, the same decode the image viewer uses, now including BMP, TIFF and ICO) or an SVG, the first lines of a text file, a folder's or archive's first names, a PDF's first page and page count (poppler), a video's frame, length and size, or a song's cover and tags (ffmpeg) — each external tool bounded by the usual timeout, and a missing one named in the pane as what to install; Alt+P, the header's right-hand button or the status bar toggles it, remembered, and it narrows on a smaller window rather than disappearing. Every entry draws the configured icon theme's icon for its type (`hyprforge-icons`, through the theme's `Inherits=` chain), with the drawn badge as the fallback; the sidebar's places and the Trash take the theme's own `folder-*`/`user-*` icons, which follow those folders into a listing, and any place or folder path can be given another theme icon or an image of the user's own in `files-config.toml`'s `[sidebar.icons]`. The grid shows real thumbnails — pictures, SVGs, a PDF's first page and a video's frame — and the list shows pictures and SVGs at its row icon's size, decoded one at a time by the host and streamed in display order, capped at 600 a folder — and from the freedesktop thumbnail cache (`hyprforge-thumbnails`) while the file is unchanged, so a video's frame is made once per version of the file, shared with every other program, and stored at a few kilobytes. Files and folders drag out of the window into any other application, as a copy offered as `text/uri-list` over the window's own `wl_data_device` — winit has no Wayland drag source, so `drag_out.rs` joins its connection to start one; members of an archive do not drag yet (Copy still works for them). Deferred: the portal backend itself, a spacebar Quick Look overlay, network shares, and dropping *onto* Files, including dragging between its own folders |
| **Photo Viewer** | `hyprforge-media`, on `hyprforge-image` (bounded decode, EXIF orientation), `hyprforge-video` (libmpv playback, first-real-frame thumbnails), `hyprforge-mesh` (STL/3MF/OBJ models) and `hyprforge-listing` (the folder order Files shows) | eog/gwenview, mpv, fstl | no | built; PNG, JPEG, WebP, GIF (first frame), BMP, TIFF and ICO, decoded to fit the window rather than at full size, EXIF orientation applied once. Drawn in the file manager's shell (the `Hyprview Photo Viewer` mockups, `2a`–`2c`): header, Places sidebar with the folders beside this one, status bar with key hints read from the keymap, and Photo, Grid and Library as modes of one window with back/forward between folders. A picture opened on its own gets a compact viewer instead — no sidebar, no inspector — that maps already floating on Hyprland, sized to the picture and centred — the size worked out before the window exists and the window mapped at a fixed size, which Hyprland floats at once, then made resizable (`float.rs`; floating it after mapping made it appear tiled and jump); Grid or Library brings back the whole shell at its usual size. Photo pages through the folder in Files' own order, with zoom about the pointer, drag to pan, rotate, fullscreen and a filmstrip; Grid groups the folder by day, decoding only the tiles in view; Library is the directory tree as folder cards — no catalogue. A docked inspector shows the file and the camera's EXIF (body, lens, exposure, date taken, GPS as coordinates); a slideshow (`1e`) with interval, shuffle and loop (a video plays through before it moves on). Videos play in the pane: libmpv, opened at run time so a machine without mpv still runs the viewer, draws each frame into memory at the pane's size and the video's shape, and a `shader` primitive writes it into one GPU texture — not an image handle per frame, which iced uploads on a worker for anything over 2MB and so never drew. Space plays and pauses (and still pages on a picture), Shift+←/→ seek, Shift+M mutes, and a bar carries play, the clock, a seek bar and mute. Tiles show a video's first real frame (ffmpeg, past the black lead-in; shared with Files, which now uses the same function) and a model's CPU-rendered drawing, both through the shared freedesktop cache at the `large` size. 3D models take over what the separate view3d app did: STL, 3MF (with components, units and colour groups) and OBJ/MTL page with the pictures in the same folder order, drawn by view3d's wgpu renderer inside an iced `shader` widget on the theme's backdrop — drag to turn, right-drag to pan, scroll to zoom about the pointer, five draw modes, fstl's viewpoints on 0–6 and 9, perspective or orthographic, axes, and autoreload when the file changes on disk; the inspector shows triangles, vertices, size and colours. Trash with undo, Copy, Open With (never itself), Show in Files, and Set as Wallpaper through the same settings Settings' Wallpaper page edits. Key bindings come from `media-config.toml` on the shared `hyprforge-keys` grammar. Not yet: the editor (`1d` — crop, straighten, exposure; it writes to originals, which is its own decision), the inspector's map and tags, view3d's `--screenshot` render-to-PNG, 3D models and video playback on a machine where iced falls back to software rendering (the pane says so for each), animated GIFs past their first frame, and re-decoding a sharper crop when zoomed past the decoded size |
| **Video Viewer** | — (folded into the Photo Viewer) | mpv | no | built as part of `hyprforge-media`, on `hyprforge-video` (libmpv): a video pages with the pictures in its folder rather than opening a second app. See the Photo Viewer row |
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

Single Cargo workspace at the repo root, 40 crates under `crates/` plus the
`notif/` nested workspace (see "Notifications" above for why that one is
kept separate). The original intent was for every component above to get
its own crate(s) under `crates/`, even before it's built — stub/empty
crates as placeholders so the workspace shape reflects the intended full
suite. That convention was not followed in practice: `Cargo.toml`'s
`members` lists only crates that actually have code in them, and planned
components with no crate yet (Audio, Users/time, Process Manager, Service Manager, Disk Utility, Notepad,
Calculator, Calendar) simply have none. Power is no longer an
exception to that: `hyprforge-power` now covers keep-awake, battery and
power profiles, matching the module this table lists. Treat the table
above, not the workspace listing, as the source of truth for what's
planned.

```
hyprforge/
  Cargo.toml
  crates/
    hyprforge-paths/            # xdg + atomic writes, no dependencies
    hyprforge-process/           # a bounded subprocess wait, no dependencies
    hyprforge-secret/            # Secret<T>, whose Debug renders a length only
    hyprforge-look/             # Color type + runtime Theme, no iced
    hyprforge-ui/                # the iced layer: widgets, palette, spacing
    hyprforge-popup/             # shared layer-shell popup shell, no daemon
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
    hyprforge-bluetooth/
    hyprforge-power/             # logind Inhibit, UPower battery, power-profiles-daemon
    hyprforge-emoji/             # emoji data + pure search
    hyprforge-emojimenu/         # the emoji picker popup
    hyprforge-keys/              # the keyboard grammar every app binds keys through
    hyprforge-mime/              # the freedesktop shared MIME database
    hyprforge-icons/             # the freedesktop icon theme lookup
    hyprforge-thumbnails/        # the freedesktop thumbnail cache
    hyprforge-archive/           # zip, tar and 7z: list, extract, rewrite
    hyprforge-listing/           # a directory listing and its order
    hyprforge-image/             # bounded, orientation-correct decoding, camera EXIF
    hyprforge-mesh/              # STL/3MF/OBJ loading, camera, CPU thumbnail
    hyprforge-video/             # libmpv playback (dlopen'd), ffmpeg first frame
    hyprforge-media/            # the photo viewer window
    hyprforge-fileops/           # trash (freedesktop spec), copy, move, rename
    hyprforge-files-core/        # the file browser's model and its shared view
    hyprforge-files/             # the file manager window
    hyprforge-clipboard/         # clipboard history lib + hyprforge-clipd daemon
                                  # + hyprforge-clipmenu popup
    hyprforge-tray/              # lib + hyprforge-trayd + hyprforge-traymenu popup
    hyprforge-settings/          # the Settings app shell + its modules
    hyprforge-authui/            # shared auth conversation model
    hyprforge-lock/
    hyprforge-greet/
  notif/                        # separate nested Cargo workspace, excluded
                                  # from the root's members (see "Notifications")
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

### Where things stand (as of 2026-09-27)

The Settings modules above marked *in progress* are functional, and their
look now follows the Settings mockup: the shared widget kit, the
search-first shell with its search palette, the mockup's sidebar, and every
page (see `crates/hyprforge-settings/DESIGN.md` for how, and for what waits
on a backend). The
app is eighteen pages in four groups — System (Displays, Power & battery,
Keyboard & mouse, Default apps), Connectivity (Network, Bluetooth),
Hyprland (Windows & workspaces, Keybinds, Animations, Window rules, Idle &
lock, Session, Advanced) and Personal (Appearance, Wallpaper, Night light,
Screen sharing, Tray) — over thirteen modules, since Appearance's and
Desktop's tabs became pages of their own. It was roughly 26.5k lines as of
2026-09-27.

`hyprforge-lock` locks, draws, authenticates against PAM or a fingerprint and unlocks, is drawn to the glass-card mockup (as of 2026-09-26), and
now has the look: it shares `hyprforge-look`'s runtime `Theme` with the rest
of the suite, resolved from the user's own Hyprland/gsettings config rather
than an invented palette. See "The lock screen" in the README for what it
still genuinely lacks (no input-method support, and no attempt limiting
of its own — the typed password is erased on drop now, and only PAM's own
copy is beyond reach).

`hyprforge-greet` exists: ~1.1k lines, on the same `hyprforge-authui`
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

`hyprforge-bluetooth` (over BlueZ) followed the same backend-trait shape,
and now has a working pairing flow on top: `agent::register` implements
`org.bluez.Agent1`, `pairing::PairingPrompt` models the Confirm/Display/
Authorize shapes BlueZ can ask for, and the Bluetooth Settings screen's
`subscription()` registers the real agent and turns its prompts into
messages. Adapter power, discovery, device list, connect/disconnect/
forget/trust and pairing are all in place.

Two Tray items shipped before this section was last revised: keep-awake
(`hyprforge-power`'s logind `Inhibit` client) and night light (driving
`hyprsunset` through `hyprforge-ecosystem::sunset_control`). Both are
live-tested in `check.sh` against the real service (`systemd-logind`,
`hyprsunset`). Night light's Settings home was then a Desktop tab with no
deep link from the tray icon; it is a page of its own now, and the icon
opens it. Keep-awake's situation changed underneath it — see below.

`hyprforge-power` grew considerably: it now wraps three independent
daemons (`systemd-logind`, UPower, `power-profiles-daemon`) behind three
backend traits, each with its own mock and its own live `check.sh` tier,
and the Settings app gained a **Power** screen on top of all three —
keep-awake plus the other holders of an inhibit, battery percentage and
state, and profile switching, with one daemon being down only collapsing
its own section. The tray's keep-awake icon has since been repointed at
it, in both the places that needed it — the left click
(`TrayItem::activate_screen`) and the menu's own settings row — which a
trayd test exists to keep in step with each other.

The tray itself gained a Settings screen of its own at the same time
(`--screen tray`): each icon's visibility, previously reachable only
as a "Show in tray" checkbox on whichever screen owned that icon, and
`menu_y_offset`, previously reachable only by hand-editing `tray.toml`.
That gave one file three writers inside one process, so every writer now
goes through `hyprforge_tray::prefs::update`, which reloads immediately
before setting its one field rather than saving back a copy loaded at
construction.

The clipboard (`hyprforge-clipboard` + `hyprforge-clipd`, with the
`hyprforge-clipmenu` popup, replacing copyq) went through a long
debugging session that ended with paste actually working end to end: a
missing `event_created_child!` specialization was panicking on the
compositor's own `data_offer` event the moment a selection changed
(including the popup's own write), the clipboard was found to die with
the popup that set it (fixed by outliving the popup rather than the
selection dying with it), and the synthesized Ctrl+V paste was found to
be flushed but not delivered — `flush()` puts the key events on the
socket without waiting for the compositor to read them, measured against
a window that captured raw bytes (0 bytes exiting immediately after flush,
109 bytes after waiting). Both the clipboard and emoji popups also gained
a real pixel-offset scrollbar with a draggable thumb, replacing
item-at-a-time scrolling with nothing to show position.

Notifications (the nested `notif/` workspace, replacing dunst/mako) are
unchanged and described in the inventory table above.

Two components not previously in this document exist and work: the emoji
picker (`hyprforge-emoji` + `hyprforge-emojimenu`, replacing the emoji
picker Hyprland users otherwise go without) and the tray's own menu
popup (`hyprforge-traymenu`, replacing the `com.canonical.dbusmenu` menu
the tray used to serve — see the Tray icons row above for the cost of
that). Building both surfaced enough shared Wayland/iced plumbing that it
was pulled out of `hyprforge-clipmenu` into `hyprforge-popup`, a shared
layer-shell popup shell (surface, pointer/keyboard, placement, singleton
lock, scrollbar, fractional-scale rendering) all three popups now build
on rather than each carrying their own copy of it.

The file manager and the photo viewer are both built and usable;
their inventory rows above say what each does and what is
deferred. Since this section was last revised: the photo viewer moved
into the file manager's shell (Photo, Grid and Library modes, a compact
floating viewer for a picture opened on its own), took over the
separate view3d app's STL/3MF/OBJ models (`hyprforge-mesh`) and plays
videos (`hyprforge-video`, libmpv opened at run time), so there is no
separate Video Viewer; Files gained previews, thumbnails from the shared
freedesktop cache (`hyprforge-thumbnails`), theme icons for its entries
and places, and dragging files out into other applications.
`media-plan.md` records how the viewer got there.

The suite also gained tooling to extract a component into its own
repository while keeping it a workspace member here: `split.sh` (via
`git subtree ... --rejoin`) and `sync.sh`, which checks whether a
split-out repository has drifted from what this monorepo would produce.
Nine components have been split and pushed with their own green CI
(as of 2026-09-27): clipboard, lock, greet, tray, settings, displayd,
emojimenu, files and media — all public, under the `hyprforge-suite`
organisation since 2026-09-27. `repo-plan.md` is the
authority on this work — the reasoning behind it, the dependency-layer
order, and what was learned doing it (a dangling `LICENSE` symlink, a
missing README, and the two-spellings-of-one-git-URL trap once `settings`
depends on an already-split `tray`) — read it rather than this summary.
Every library crate is published to crates.io (as of 2026-09-28, all at
0.1.0), and the split repositories depend on those versions; inside this
workspace a `[patch.crates-io]` section points each name back at
`crates/`.
