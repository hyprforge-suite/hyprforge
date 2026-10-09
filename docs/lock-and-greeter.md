# Lock screen and greeter

`hyprforge-lock` and `hyprforge-greet` share one look and one
authentication conversation (`hyprforge-authui`), so the screen you log in
on and the screen you unlock look like the same system.

## The lock screen

`hyprforge-lock` locks the session with `ext-session-lock-v1` and
authenticates against PAM. It shares `hyprforge-authui`'s conversation
model with the greeter, because the problem being solved is that a
greeter and a lock screen usually look like two different systems.

### What the screen shows

The layout is the "Hyprlock: glass card" mockup, drawn by
`hyprforge-authui` for both hosts, with every colour taken from the shared
theme at an alpha:

- **Idle**: the clock alone, large, with "Type to unlock" — or "Type, or
  touch the sensor, to unlock" while the fingerprint reader is listening.
  Any key brings the card up; Enter, Escape or Backspace on the idle clock
  only wake it, and are never sent as an empty password (which would spend
  a `pam_faillock` attempt). Twelve quiet seconds with nothing typed puts
  it back, and Escape on an empty card does so at once.
- **The card**: avatar (`~/.face`, then AccountsService, else the initial),
  name, a dot per character with a caret, and under it the keyboard layout
  and Caps Lock. A rejected password turns the field red, keeps *that
  attempt's* dots up while it shakes three times, then clears; "attempt N"
  sits beside PAM's own message.
- **Status**, top right: layout from the keymap the compositor sent, the
  network from NetworkManager, the battery from UPower — each part simply
  absent when its daemon is. A low battery turns the reading orange and
  adds a warning to the card.
- **Extras on the idle clock**: what an MPRIS player is playing, with
  previous / play-pause / next (media keys work too), and notification
  counts per application — the app and the count, never the text.
- **Power menu** from ⏻, or Tab: Suspend, Hibernate, Reboot, Shut down,
  each only when logind answers `yes`, chosen by pointer, arrows and
  Enter, or its letter. While it is open, nothing typed reaches the
  password. Switch user is not offered: greetd has no session switching
  to hand over to.
- **Several monitors**: the card, status and ⏻ are drawn on the output
  that has the keyboard; every other output shows the clock alone.

Notification counts come from watching `Notify` calls on the session bus
as a D-Bus monitor, because the freedesktop protocol has no way to ask a
server how many are unread. That also makes them mean the right thing
for a lock screen: what arrived while you were away. Summary and body are
dropped the moment the application's name has been read.

### The fingerprint goes to fprintd, not PAM

PAM asks its modules in turn, so with `pam_fprintd` in the stack either
the password prompt waits for the reader to give up or the reader waits
for a password to be refused — it cannot offer both at once, which is
what the card does. So, like hyprlock, the lock talks to fprintd on the
system bus beside the password conversation (`src/fingerprint.rs`).

It is not a way round PAM's *policy*: a match is followed by the PAM
account stack (`pam_acct_mgmt`) before anything unlocks, so an account
PAM would bar on a correct password is barred on a correct finger. Only
`VerifyStatus` signals from the process that owns fprintd's bus name,
about the device that was claimed, are believed. Three unrecognised
fingers — `pam_fprintd`'s own default — stop the reader, because fprintd
keeps no count of its own and `pam_faillock` never sees these; a bad
*read* (too short, off-centre) is not counted. `check.sh`'s fprintd tier
asks the running daemon the lock's own questions, read-only; the first
version of this code named the wrong manager path and passed every unit
test.

### It draws at the output's real resolution

The lock used to paint a logical-size buffer at scale 1.0 and let the
compositor stretch it — blurry on a 1.6 output, the mistake CLAUDE.md
records for the popups. It now uses `wp_fractional_scale_v1` with
`wp_viewporter`, per output.

At a real resolution, resampling a 4K wallpaper onto the output every
frame was more than half of each frame: 45ms with it, 20ms without, at
1440×900 physical. So each output's wallpaper is prepared once on a
worker thread (`src/backdrop.rs`) — decoded through `hyprforge-image`'s
budget, cropped to cover, dim baked in — and drawn one-to-one: 32ms. The
first frames still draw from the path, since the session is not locked
until they exist.

### It needs its own PAM file

```
sudo install -m 644 crates/hyprforge-lock/pam/hyprforge-lock /etc/pam.d/hyprforge-lock
```

Without it, `service_name()` falls back to `hyprlock` — and that
**rejects correct passwords**. `/etc/pam.d/hyprlock` declares only
`auth include login`, so the account stack for that service is empty and
falls through to `/etc/pam.d/other`, which is `account required
pam_deny.so`. hyprlock itself never notices because it only calls
`pam_authenticate`; this checks `pam_acct_mgmt` too, since an account can
have the right password and still be expired, locked, or barred from the
host. The failure message names the file to fix.

### Testing it without locking yourself out

**Never run it against the session you are using.** `--fake-password`
refuses to start without an explicit `--display` for that reason.

```
./crates/hyprforge-lock/testing/nested.sh        # nested Hyprland on wayland-2
./target/debug/hyprforge-lock --display wayland-2 \
    --fake-password hunter2 --type-in hunter2
```

Run `nested.sh` before *every* attempt. A lock client killed while
holding the lock leaves the session locked with nothing left to unlock
it — that is the design, not a bug, and it is what makes writing your own
lock screen reasonable rather than reckless. Restarting the nested
compositor is the clean way back; the other is
`hyprctl --instance <N> eval 'hl.clear_crashed_lockscreen()'`. Killing
only the child starts another one (see "When it crashes, it starts
again" below); to abandon a test lock, kill the supervisor first.

| Flag | For |
|---|---|
| `--type-in TEXT` | types TEXT and presses Enter once a frame is drawn, driving the real path from keystroke to compositor release without a keyboard |
| `--fake-delay MS` | makes the fake backend take MS to answer, standing in for `pam_unix`'s ~2s pause after a wrong password |

Both require `--fake-password`, which **does not exist in a release
build** — the fake backend is compiled out, so a lock screen that opens
to a known string is not one flag away in the binary people install. In a
debug build it additionally refuses when the display it is given is the
one the process would have connected to anyway; requiring `--display`
alone proved only that a display was named, not that it was a different
one.

Passing something other than the fake password exercises the failure path
against the fake backend, so no real account collects a failed attempt —
which matters where `pam_faillock` is active.

A test lock still talks to the real system bus, so under
`--fake-password` the power menu only *rehearses* — "would Shut down now"
on stderr — rather than asking logind to act on the machine the test is
running on. The fingerprint reader stays on for a hand-driven run (a
real finger unlocks the nested session) and off under `--type-in`, so a
self test's result never depends on who touched the sensor.

Hyprland does not deliver synthetic keys or clicks to a lock surface —
`send_shortcut` targets windows, and a lock is not one — so the nested
compositor cannot be typed at from a script. `--type-in` is the way in;
the card, the shake and the settled failure can be screenshotted with
`grim -o <output>` after `--type-in wrong`. Screenshot a headless output
(`hyprctl output create headless`), not the nested window, whose frames
go stale while the host is not showing it.

### Surfaces are created when the lock is *requested*, not when it is granted

The protocol says: *"The locked event must not be sent until a new 'locked'
frame has been presented on all outputs."* The compositor is waiting for the
surfaces. Creating them in the `locked` handler is therefore a standoff, and the
only thing that breaks it is Hyprland giving up after five seconds and painting
its "lockscreen app died" recovery screen in front of the real one.

That is what the delay was, and the tell was that it measured 5002ms every
single run and never varied with load, build profile or wallpaper size. A
suspiciously round number is a timeout somewhere else, not slowness here. Ours
finished in 1ms. Correct order now: request, cover every output, draw, commit —
then `locked` arrives, around 80ms.

`locked` no longer creates anything. It means the session is genuinely secured,
which is the right thing to gate the authenticator on and nothing else.

### The wallpaper is scaled once, on save

An 8001x4501 photograph is 36 megapixels: 144MB decoded, near 300MB once the
renderer holds its own premultiplied copy. Measured on the lock surface, that is
RSS 170MB and a 296MB peak, against 54MB/72MB for the same image capped at 4K.

This is a correctness fix rather than a tidy-up. `renderable` guards the
wallpaper by reading its *header*, but an allocation failure happens during full
decode — and that is exactly the case that caches as "no entry" in
`iced_tiny_skia` and panics on the next frame. A header check cannot catch it;
not decoding 36 megapixels can.

So the Settings app scales it when settings are saved, capped at 4K's long edge,
reusing a current copy rather than re-encoding every time. Doing it per lock
would pay the same cost repeatedly in the process where failing is
unrecoverable.

### Nothing blocks the drawing

`pam_unix` deliberately sleeps for about two seconds after a wrong
password. If answering meant waiting, the surface would stop repainting
for exactly that long, and a surface that stops repainting is
indistinguishable from one that crashed — on a lock screen, the user's
only other option is a hard reboot.

So `Backend` never blocks: `start`/`answer`/`proceed` post a request and
return, `poll` collects answers, and a calloop ping wakes the event loop
when one arrives. `--fake-delay 3000` draws ~60 frames where a blocking
version drew 2.

Authentication also starts only *after* the compositor grants the lock.
Building the conversation is what starts PAM talking, so doing it any
earlier held the screen unlocked for as long as a slow module took.

### When it crashes, it starts again

A lock client that dies leaves the session locked — Hyprland's choice,
and the right one — behind Hyprland's own "lockscreen app died" screen.
So the binary you run is a supervisor: it starts a second copy of itself
with `--child`, which does all the work, and waits. The parent never
connects to Wayland and never renders, so nothing in iced or tiny-skia
can take it down.

| The child | The supervisor |
|---|---|
| unlocked (exit 0) | exits 0 |
| crashed (a panic, or killed by a signal) | starts it again, and Hyprland hands the dead lock to the new copy |
| crashed a second time | starts it in safe mode: the default theme, no wallpaper, no avatar, no status line, no fingerprint reader, and no power menu or ⏻ button (nothing would be asking logind, so it could do none of what it offered) |
| crashed a fourth time within a minute | stops (exit 4); the session stays locked behind Hyprland's screen |
| was refused the lock after a relaunch | stops (exit 3), naming `misc:allow_session_lock_restore` |

The takeover needs `misc:allow_session_lock_restore`, which Hyprland
leaves off: with it off, the relaunched copy's request is refused, and
the session stays exactly as locked as the crash left it. Set up's
"Restart the lock screen if it crashes" turns it on, through the System
page's own `system.toml`. While a lock is dead, any client of yours can
then take it over — but a process running as you can already clear a
crashed lock with `hyprctl`, so no trust boundary moves, and someone at
the keyboard who crashes the lock has no shell to start a client from.

Measured on a nested compositor (debug build, `kill -ABRT` on the child
mid-unlock): the supervisor noticed 0.46–0.65s after the signal — most
of it the core dump — and the new copy held the lock 1.8s after that, the
time its first frame takes with the wallpaper; in safe mode, 0.24s. The
unlock then finished as normal. Hyprland paints its own error screen
after `misc:lockdead_screen_delay` (1s by default), so a slow relaunch
shows it briefly.

Safe mode is sticky, and none of it is prevention: `screen::renderable`
and the panic sweep below are what stop a crash; this is only what
happens after one gets through. Nothing the supervisor does unlocks —
its only success is a child that unlocked itself.

`kill -SEGV` sent from outside does not crash it, which matters when
testing this: Rust's standard library installs a SIGSEGV handler to
detect stack overflows, and for a signal that is not a guard-page fault
it resets the handler and returns, so the process carries on. Use
`kill -ABRT` (or `-KILL`) on the child — found as the child of the
supervisor, `pgrep -P <pid>` — to stand in for a crash.

### Never log a keystroke

Not the character, and **not the keysym either** — `XK_a` is `a`,
`XK_comma` is `,`, so "just the keysym" writes the password to disk in a
barely-encoded form. This has already happened here once. `Debug for
State` is hand-written to render what was typed as `<N chars>`; use that.

### Where it deliberately differs from hyprlock and swaylock

Two choices are unusual and worth knowing about.

**It renders with a full GUI toolkit.** swaylock draws with cairo;
hyprlock uses the GPU. This drives iced through `iced_tiny_skia` into the
buffer the compositor hands over. gtklock does something similar with
GTK, so it is not unprecedented, but the trade is real: far more code
behind the screen, and that code is not written defensively —
`iced_tiny_skia` and `cosmic-text` are full of `expect`s on geometry.
`screen::renderable` exists because of that, and it is why a panic sweep
runs on every change.

**It checks the account as well as the password.** `pam_acct_mgmt` on top
of `pam_authenticate`, which neither hyprlock nor swaylock does. It is
more correct — an expired or barred account should not unlock a session —
and it is why this ships its own PAM file rather than borrowing one.

### What it does not do yet

Four gaps, none of them a security hole, the first three things an
established lock screen has:

- **No input-method support in the lock screen.** A password typed through
  an IME cannot be entered there. swaylock is the same; it still means some
  users cannot log in. The greeter asks for an input method while a
  question is open and, when fcitx5 is installed, starts one in its own
  compositor — with every addon off but an allow-list that starts no other
  program. A password still goes past it as plain keys, deliberately, so
  it is never composed in a candidate window in clear; see
  `crates/hyprforge-greet/README.md`.
- **The password is erased on this side, and PAM keeps its own copy.**
  `Secret<T>` zeroes its value when it goes out of scope, which matters
  more than it looks: a typed password is not appended to in place — the
  host hands the whole string over on every keystroke and the old one is
  dropped — so eight characters allocate eight strings, each holding a
  prefix, and every one of them is now cleared as it is displaced. The
  clone `submit` makes for the backend is wrapped too. What remains is
  outside this code: PAM and greetd copy the answer once it is handed
  over, and a core dump or a swapped page taken while they hold it can
  still contain it.
- **No attempt limiting of its own for passwords**, on purpose: rate
  limiting belongs in `/etc/pam.d`, where an administrator can see and
  change it, rather than hidden in a settings app. The fingerprint path is
  the exception, because nothing in PAM sees it — see above.
- **No blur behind the card.** The mockup's frosted glass is a
  `backdrop-filter`; a software renderer that draws each widget once has
  no equivalent, and the translucent tint does most of the work of
  keeping text legible over a photograph.

Caps Lock *is* shown, which is not decoration: without it a stuck key
looks exactly like a forgotten password, and where `pam_faillock` is
configured that spends attempts against the account rather than the
screen.

## The greeter

`hyprforge-greet` is a greetd greeter built on the same conversation model
as the lock screen. It runs as its own user, which cannot read your home
directory, so it draws from a copy of the look that Settings exports to
`/var/lib/hyprforge/greet` — see "Why the look is one crate" in
`docs/architecture.md`.

It runs inside a Hyprland instance of its own, configured by
`crates/hyprforge-greet/config/hyprland-greeter.lua`. That config has no
key binds at all, deliberately: every bind in the greeter's compositor
belongs to whoever is at the keyboard, logged in or not.

`./hyprforge --install --greeter` installs it, but does not write
`/etc/greetd/config.toml` or enable greetd — that file decides which VT
the machine logs in on. `crates/hyprforge-greet/INSTALL.md` covers that
step, and its README covers the rest.
