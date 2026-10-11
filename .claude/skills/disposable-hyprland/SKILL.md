---
name: disposable-hyprland
description: Work against a throwaway nested Hyprland instead of the user's session. Use for anything that changes a compositor — the disposable test tier, lock-screen testing, the greeter, Files' cross-application clipboard, window rules or binds tried for real, screenshots of an app in a clean compositor — and whenever you are tempted to run hyprctl eval, reload, dispatch or a lock client against the live session.
---

# The disposable Hyprland

Never change the user's own session to find something out. On
2026-10-10 live tests did, and the desktop froze until a re-login.
`tools/hf` manages one nested instance for this; `tools/hflib/hyprland.py`
documents every choice in it.

## Starting it — the user does

There is no headless-only Hyprland (it reaches for the DRM device the
session holds), so the instance is a window in the session. It refuses
to start without a terminal, because the 2026-10-08 hard freeze happened
while an unattended agent cycled nested compositors. Ask:

> Please run `! tools/hf hyprland up` — it opens a small nested Hyprland
> window that ends itself within fifteen minutes.

or, for the test tier, `! tools/hf test disposable` (starts, runs, stops).
Never pass `--unattended-ok`, never from a background agent, and keep
restarts few.

Once it is up, read `target/hf/hyprland.json`: `signature`, `wl_socket`
(e.g. `wayland-2`), `pid`, and the daemons it started. `tools/hf hyprland
status` prints the same. `tools/hf hyprland down` stops it — that you may
run.

## Talking to it

- `hyprctl --instance <signature> …` — always name it. Never pick the
  newest directory under `/run/user/*/hypr/`: the session writes its log
  continuously and wins that race.
- `hyprctl dispatch` takes Lua here like everywhere else:
  `hyprctl --instance <sig> dispatch 'hl.dsp.focus({ monitor = "HF-HEADLESS" })'`.
- It has a headless output, `HF-HEADLESS`. A nested window on a
  workspace the user is not looking at gets no frames, and anything
  waiting on one (grim) waits forever — move the window to
  `HF-HEADLESS` and capture with `timeout 10 grim -o HF-HEADLESS out.png`.
- Keys: `hl.dsp.send_shortcut({ mods = "", key = "g", window = "class:…" })`;
  with no `window`, it goes to whatever holds focus (a popup's layer).
- Its hyprpaper and hyprsunset run under its signature; their sockets
  are in its own runtime directory, so they never touch the session's.

## The lock screen

Debug build only, fake backend only — never the installed binary, never
real PAM (`pam_faillock` locks the *account* after three failures):

```
cargo build -p hyprforge-lock
./target/debug/hyprforge-lock --display <wl_socket> --fake-password hunter2 --type-in hunter2
```

Exit 0 means it locked, drew and unlocked. `--type-in wrong` is the
failure path. Do not also export `WAYLAND_DISPLAY`: `--fake-password`
refuses a display the process would have used anyway, which is what
stops it locking the real session with a known password.

A lock client killed while holding the lock leaves the instance locked
by design. Recover with `hyprctl --instance <sig> eval
'hl.clear_crashed_lockscreen()'`, or ask the user for `tools/hf hyprland
down` + `up`. `crates/hyprforge-lock/testing/nested.sh` is the older way
and still works; it kills anything whose command line says `nested.lua`,
never `hf-disposable.lua`.

## The greeter

Never against greetd. `crates/hyprforge-greet/testing/fake_greetd.py`
speaks greetd's real wire format with scripted replies; its README's
"Testing it safely" has the commands. Run the debug build on the nested
display with `WAYLAND_DISPLAY=<wl_socket>`, `GREETD_SOCK` pointing at the
fake's socket (a short relative path — AF_UNIX caps it near 108 bytes),
and `--type-in` to drive the whole handshake. Its compositor config
(`crates/hyprforge-greet/config/hyprland-greeter.lua`) must keep **no
binds**: every bind there belongs to whoever is at the keyboard,
unauthenticated.

## Files' clipboard between applications

Part of the disposable tier (`binary(nested_clipboard)`); hf exports
`HYPRFORGE_TEST_NESTED_DISPLAY` for it. To run just those:
`! tools/hf test disposable -- -E 'binary(nested_clipboard)'`.

## When something looks wrong

- No window, no error: the compositor's log is in `target/hf/logs/`.
- A config error banner: `hyprctl --instance <sig> configerrors`.
- `pkill -f`/`pgrep -f` match the shell running them; `pgrep -x` cannot
  see names past 15 characters. Use the pid from `hyprland.json`.
