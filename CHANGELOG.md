# Changelog

Every notable change to the Hyprforge suite, newest first. The format is
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and the suite is
versioned as one: the libraries, the ten components and the Arch packages
share each version number (see `docs/design/repo-plan.md`).

Each component's repository keeps a `CHANGELOG.md` of its own from the
first release after this file was started. `tools/changelog.py draft`
drafts the next section from git; `tools/release.sh` files it.

## [Unreleased]

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

- notif: the notification center drew the theme's colours swapped.
- Docs: stale counts (the Appearance catalogue's 155 options, the
  packages the PKGBUILD builds, the Settings app's eighteen Hyprforge
  dependencies).

## [0.1.8] and earlier

Versions 0.1.1 through 0.1.8 were released before this file was started
and are recorded only as tags (`git log v0.1.7..v0.1.8`, and so on).
