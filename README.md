# hyprforge-clipboard

A Wayland clipboard history: the library, and `hyprforge-clipd`, the
daemon that watches the compositor and records what is worth keeping.

Part of [Hyprforge](https://github.com/apost/hyprforge), a suite of
native Hyprland desktop apps — but it runs alone. Installing this gets
you a clipboard daemon and nothing else: no settings app, no tray, no
Hyprland config machinery.

## The rule it is built around

A clipboard manager writes a history to disk. A password manager puts
your password on the clipboard. Without care, using both means your
vault ends up in a file in your home directory and neither program ever
mentions it.

So sensitivity is checked *before* an offer's bytes are ever requested.
A secret is not read, not hashed, not stored and not logged — the check
happens early enough that the content never enters this process at all.
`Debug` on an entry renders a description rather than the content, for
the same reason.

## What is in here

- **The library** — the read side, the write side, and paste synthesis,
  over `wlr-data-control` and `ext-data-control`. It knows nothing about
  a popup or about any particular consumer of a history.
- **`hyprforge-clipd`** — the daemon, and the only process that ever
  writes the history file.

There is deliberately no IPC: no socket, no D-Bus name, nothing a reader
could ask the daemon to do. A popup reads the index and the image
directory directly. Two writers racing on one index file is a problem
that atomic writes cannot solve on their own — last write wins either
way — so there is only ever one writer.

The popup itself (`hyprforge-clipmenu`) lives in the main Hyprforge
repository, because it draws with the suite's shared theme and that
pulls in more than a standalone clipboard should need.

## Building

```
cargo build --release
```

It depends on two other Hyprforge crates, `hyprforge-paths` and
`hyprforge-secret`, taken as git dependencies on the main repository
rather than from crates.io, which is where they will move once they are
published. Nothing else here is Hyprforge-specific.

## Running

`packaging/hyprforge-clipd.service` is a user unit:

```
systemctl --user enable --now hyprforge-clipd
```

`Restart=always` rather than `on-failure` is deliberate — a clean exit
that stops recording is exactly as bad as a crash, and shows up as no
failed unit at all.

## Licence

MIT. See `LICENSE`.
