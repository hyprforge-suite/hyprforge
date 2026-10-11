---
name: install-locally
description: Install, update or uninstall Hyprforge components on this machine with ./hyprforge, and run an app from the build without disturbing the user's own copy. Use when asked to install or reinstall something, when a fix needs to reach the running daemons, when an installed binary seems stale, or before running the Settings app or any daemon from target/.
---

# Installing on this machine

`./hyprforge --help` lists everything. The shape:

```
./hyprforge --install --<component>... [--dry-run] [--no-restart] [--yes]
./hyprforge --uninstall --<component>... [--force-cleanup] [--dry-run]
./hyprforge --status
```

Components: greeter, lock, displayd, tray, clipboard, notif, emoji,
polkit, procman, files, media, mime, settings, or `--all`.

- It builds release binaries into `target/release`, installs to
  `/usr/local/bin`, and asks for sudo once (`sudo -v`) — the Bash tool has
  no terminal for that prompt, so **ask the user to run it**:
  `! ./hyprforge --install --tray`. `--dry-run` needs no sudo and you
  may run it.
- It finishes with Settings' Set up (`hyprforge-settings --setup`), which
  offers the wiring, services and binds a component needs.
- **An installed binary is not a restarted daemon.** It restarts a user
  service only if that service was running *and* its binary changed this
  run. `--no-restart` defers that. It never touches greetd — that is the
  login manager, and `crates/hyprforge-greet/INSTALL.md` covers it, from
  a spare VT.
- `--procman` installs a root helper, a D-Bus system service and the
  recorder's system unit; the recorder's sandbox can be proved with
  `tools/hf sandbox` afterwards.
- A daemon answering "unknown variant" or offering something it no
  longer does is version skew across a socket: the install succeeded and
  the old process is still running.

## Running from target/ instead

- **Settings writes real config**, including `hyprland.lua`. Run it with
  an isolated `XDG_CONFIG_HOME`, and a short `XDG_RUNTIME_DIR` of its own
  when Settings is already open (it is single-instance and hands a second
  copy's request to the running one). Keep that runtime path under 108
  bytes, or the Hyprland socket path truncates and every `hyprctl` fails.
- **Never start a second instance of one of the suite's daemons** to
  inspect it — `hyprforge-clipd` would share the live history file. Read
  `--help` or the source.
- Never point the lock screen at the session (`disposable-hyprland`).
