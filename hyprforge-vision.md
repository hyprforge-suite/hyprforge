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
- Shared look/theme/widget code lives in `hyprforge-core` and every app
  depends on it rather than reinventing its own styling.

## Component inventory

| Component | Crate(s) | Replaces | Daemon? | Status |
|---|---|---|---|---|
| **Settings app** (shell) | `hyprforge-settings` | GNOME/KDE Settings | no (hosts modules that may talk to daemons) | scaffolding started |
| — Displays module | `hyprforge-displayd` (daemon) | manual `wlr-randr`/GUI fiddling | **yes**, systemd user service, D-Bus API | in progress |
| — Window Rules module | `hyprforge-windowrules` (lib) | hand-written Lua rules | no | in progress |
| — Network module | *(planned)* | nm-applet/nmtui | probably talks to NetworkManager D-Bus directly, no new daemon | not started |
| — Bluetooth module | *(planned)* | blueman | talks to BlueZ D-Bus directly | not started |
| — Audio module | *(planned)* | pavucontrol | talks to PipeWire | not started |
| — Power module | *(planned)* | — | talks to UPower/power-profiles-daemon | not started |
| — Desktop module (wallpaper, night light, idle) | `hyprforge-ecosystem` (lib) | hyprpaper/hyprsunset/hypridle config by hand | no (drives the existing daemons) | in progress |
| — Appearance module | `hyprforge-appearance` (lib) | GNOME/KDE appearance settings | no (writes gsettings via CLI) | in progress |
| — Input/keyboard module | `hyprforge-input` (lib) | GNOME/KDE keyboard & touchpad settings | no | in progress |
| — Session module (autostart, environment, gestures, permissions) | `hyprforge-session` (lib) | hand-edited hyprland.lua | no | in progress |
| — System module (behaviour, shortcuts behaviour, X11, rendering) | `hyprforge-system` (lib) | hand-edited hyprland.lua | no | in progress |
| — Users/time module | *(planned)* | — | accountsservice/timedated | not started |
| **File Manager** | `hyprforge-files-core` (shared logic), `hyprforge-files` (standalone), `hyprforge-files-portal` (xdg-desktop-portal FileChooser backend) | Nautilus/Dolphin/Thunar | portal backend runs as a D-Bus service | not started |
| **Photo Viewer** | `hyprforge-photos` | eog/gwenview | no | not started |
| **Video Viewer** | `hyprforge-videos` | — (likely thin mpv wrapper; revisit build-vs-wrap) | no | not started |
| **Process Manager** | `hyprforge-procman` | Windows Task Manager / GNOME System Monitor (Mission Center already covers this well — revisit whether to build vs. skip before starting) | no (reads /proc, polls) | not started |
| **Service Manager** | `hyprforge-servicemgr` | `systemctl` CLI | talks to systemd D-Bus API directly | not started |
| **Disk Utility** | `hyprforge-disks` | GNOME Disks/GParted | talks to udisks2 D-Bus | not started |
| **Notepad** | `hyprforge-notes` | Notepad/TextEdit | no | not started |
| **Calculator** | `hyprforge-calc` | Calculator apps | no | not started |
| **Calendar** | `hyprforge-calendar` | Calendar apps | possibly syncs via CalDAV — daemon TBD if background sync is wanted | not started |

This list is not necessarily final or exhaustive — treat it as the current
shared understanding of scope, updated as decisions are made, not as a
locked spec.

## Workspace structure

Single Cargo workspace at the repo root. Every component above gets its own
crate(s) under `crates/`, even before it's built — stub/empty crates are
fine as placeholders so the workspace shape reflects the intended full
suite, not just whatever's been built so far.

```
hyprforge/
  Cargo.toml
  crates/
    hyprforge-core/            # shared trait(s), theme, widgets, D-Bus helpers
    hyprforge-settings/        # the Settings app shell + its modules
    hyprforge-displayd/
    hyprforge-windowrules/
    hyprforge-files-core/
    hyprforge-files/
    hyprforge-files-portal/
    hyprforge-photos/
    hyprforge-videos/
    hyprforge-procman/
    hyprforge-servicemgr/
    hyprforge-disks/
    hyprforge-notes/
    hyprforge-calc/
    hyprforge-calendar/
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
NOT the whole suite. The Displays + Window Rules work (see
`hyprforge-v1-prompt.md`) is one slice of the Settings app, not the
entirety of Hyprforge. Do not expand scope to other components without
it being explicitly requested for that session.
