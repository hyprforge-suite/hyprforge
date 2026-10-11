# Changelog

Every notable change to the Hyprforge suite, newest first. The format is
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and the suite is
versioned as one: the libraries, the eleven components and the Arch packages
share each version number (see `docs/design/repo-plan.md`).

Each component's repository keeps a `CHANGELOG.md` of its own from the
first release after this file was started. `tools/changelog.py draft`
drafts the next section from git; `tools/release.sh` files it.

## [Unreleased]

## [0.1.13] - 2026-10-10

### Added

- `hyprforge-polkit`, the eleventh component: the administrator prompt,
  drawn with the lock screen's card, with your fingerprint offered
  beside the password. Packaged as `hyprforge-polkit`, and `--polkit`
  in the installer.
- `hyprforge-ui`: `stacked_bar`, one bar split into coloured parts, and
  a downward chevron (`Nav::Down`), so a disclosure can show it is open.
- Set up: "Ctrl+Shift+Escape opens the process manager", and "Record
  system history", off by default — enables the process manager's
  recorder through pkexec; undo stops it and keeps what it recorded.
  Both wait for `hyprforge-procman`, which follows this release.

### Fixed

- polkit: the agent started as a systemd user unit found no login
  session — a user unit belongs to none — and restarted every two
  seconds; it now answers for the session your display is in.

## [0.1.12] - 2026-10-10

### Added

- `hyprforge-popup`: `PopupApp::namespace`, so a popup can name its own
  layer surface and a compositor rule can tell it apart from the rest.
- Set up: "Administrator prompts", which makes `hyprforge-polkit` this
  session's polkit agent — masking hyprpolkitagent for your user, which
  holds even when your config starts it, and unmasking it on undo.
- Set up: "Blur behind administrator prompts", a layer rule for the
  polkit prompt's namespace.

## [0.1.11] - 2026-10-10

### Added

- Files: "Retry as administrator" on a copy or move that was refused for
  permission, run by the administrator helper.
- `hyprforge-popup`: an inbox (`inbox`, `Receives`,
  `Popup::run_receiving`), so something outside a popup can tell it
  things while it is open.
- Set up: "Fingerprint for administrator prompts", off by default — adds
  your fingerprint reader to polkit's PAM stack, with the password ten
  seconds later if you don't touch it.
- `hyprforge-authui`: `card_body` and `glass_panel`, the lock screen's card
  as two public pieces, so another password prompt draws with the same
  code.

## [0.1.10] - 2026-10-09

### Added

- A website, at https://hyprforge-suite.github.io/hyprforge/, with a page
  for each component and screenshots of each; the READMEs show the same
  screenshots.
- `hyprforge-fileops`: `Report::denied` and `OpsError::is_permission` say
  which failures were for want of permission, as data rather than as a
  sentence to recognise.

### Changed

- Tray: a menu is as wide as its widest label, measured, from 220 up to
  360px, and only a label wider than that is cut — the battery line no
  longer ends "charging, fu…".
- Files: the path bar shows as much of the path as the field has room
  for, rather than at most three crumbs at any width.

### Fixed

- Popups: no square corner showed behind the rounded corners of the
  clipboard history, the emoji picker and the notification center.

## [0.1.9] - 2026-10-08

### Added

- Files: phones and cameras in the Devices section, through gvfs's MTP and
  gPhoto2 backends (`hyprforge-volumes`).
- Files: Quick Look plays videos and turns 3D models, through the same
  panes Media's viewer draws — a new library, `hyprforge-viewer`.
- Files: Previous Versions, from snapper's snapshots, and a Set up item
  that offers it (`hyprforge-setup`).
- Files: tags, kept on the file the way Dolphin keeps them, and copies
  that keep them (`hyprforge-fileops`).
- Files: "Open as administrator", through a pkexec helper the package
  ships.
- Files: a rubber-band selection, type-to-jump, free space in the status
  bar, and an undo history.
- Files: one window — folders opened from other applications arrive as
  tabs, on Hyprland.
- Tray: `menu = "auto" | "popup" | "dbusmenu"` in `tray.toml`. The
  daemon serves `com.canonical.dbusmenu` wherever its own popup can't run,
  so a bar on another compositor gets a right-click menu; Settings' Tray
  page has the choice.
- Lock screen: restarts itself when it crashes. A supervisor relaunches
  the lock, switches to a safe mode (default theme, no wallpaper, no
  extras, no power menu) from the second crash, and gives up — the
  session staying locked — after four crashes in a minute.
- Set up: "Restart the lock screen if it crashes" turns on Hyprland's
  `misc:allow_session_lock_restore`, with an undo.
- Greeter: input methods. It asks for one while a question is open and
  starts fcitx5 in its own compositor, with every addon that can launch
  a program disabled; IBus is not started, because its Wayland input
  method only runs inside a panel that launches programs.
- `tools/release.sh`, `tools/versions.py`, `tools/changelog.py` and
  `tools/aur.py`: one suite version, these changelogs, and an AUR
  package rendered from `packaging/arch`, with `check.sh` steps that hold
  all three.

### Changed

- Files: drags inside the window stay in it, folders spring open under a
  resting drag, and the drag itself is drawn as a card.
- Files: job notifications say who did what, from where to where, and in
  how long.
- Media: videos and models are drawn through `hyprforge-viewer`, shared
  with Files' Quick Look.
- `./hyprforge --install --media` removes the Hyprforge Photos binary and
  launcher entry a machine installed before the rename still had.
- The README is now a landing page; the reference moved to `docs/`, and
  the design documents to `docs/design/`.

### Fixed

- Greeter: typing left a stack of card outlines behind, one per
  keystroke. Shadows are now drawn as images, which `iced_tiny_skia`
  0.14.1 clips to the repainted area where it does not clip a quad's own
  shadow (fixed upstream after the 0.14 branch).
- notif: the notification center drew the theme's colours swapped.
- Docs: stale counts (the Appearance catalogue's 155 options, the
  packages the PKGBUILD builds, the Settings app's eighteen Hyprforge
  dependencies).

## [0.1.8] and earlier

Versions 0.1.1 through 0.1.8 were released before this file was started
and are recorded only as tags (`git log v0.1.7..v0.1.8`, and so on).
