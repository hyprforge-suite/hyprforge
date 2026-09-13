# Working on Hyprforge

A suite of native Hyprland desktop apps. One cargo workspace, layered so that an
app which has never heard of Hyprland can still use the bottom of it. Start with
the "Workspace layout" section of `README.md`; `hyprforge-vision.md` has the
whole-suite scope.

## The rules that came from being wrong

Each of these cost real debugging. They are here because the mistake was not
obvious in advance.

**Never write anything derived from a keystroke to a log, a panic message or a
debug dump — keysyms included.** A keysym name *is* the character: `XK_a` is
`a`, `XK_comma` is `,`. "Just the keysym" writes the password to disk in a
barely-encoded form, and it reaches the session transcript through tool output.
`Debug for State` is hand-written to render what was typed as `<N chars>`; use
that. To debug input, log the character *count* or whether `utf8` was `Some` —
never the value.

**Measure before optimising anything that talks to another process.** A
suspiciously round delay is someone else's timeout, not slowness here. A
five-second gap before the lock screen appeared was `ext-session-lock-v1`
waiting for surfaces this code was withholding until `locked` arrived; our side
finished in 1ms. Nothing in the test suite caught it, because every test asked
*what* happened and none asked *how long*.

**Test the resource, not just the result.** A 36-megapixel wallpaper peaked at
296MB in the lock screen, and an allocation failure there is unrecoverable —
it caches as "no entry" in `iced_tiny_skia` and panics on the next frame. Ask
what a change allocates, not only whether it works.

**Never collapse "this file could not be read" into "there is nothing
configured".** A missing file is first-run; a file that exists and will not
parse is a problem the user has to hear about. Collapsing the two once cost a
user 37 hand-written binds. `hyprforge-core::hlconfig::storage` documents the
distinction; `look::resolve` follows it by warning and carrying on.

**Never wait on another process without a bound.** `Command::output()` waits
forever, and every external program here — `hyprctl`, `gsettings`, `fc-list` —
can stop answering. Use `hyprforge_core::command::output(.., command::TIMEOUT)`,
or `tokio::time::timeout` in async code. A timeout arrives as
`io::ErrorKind::TimedOut`, so callers that already handle a failed spawn need no
new branch: not answering and not starting are the same problem from their side.

**A shared `Theme` does not give you a shared look — the renderer can
change the colour underneath it.** iced 0.14 enables `web-colors` by
default, which skips the sRGB->linear conversion on the `iced_wgpu`
path. `iced_tiny_skia` has no such feature. So the greeter and the
Settings app drew `rgba(26263aff)` as (107, 107, 130) while the lock
screen drew it as (38, 38, 58) — one extra sRGB encode, and a login
screen visibly washed out beside a lock screen reading the same file.
Nothing in the type system or the tests can see this: both hosts report
the same `Color` and only the pixels differ. `check.sh` asserts the
feature stays off; comparing a screenshot's centre pixel against the
theme value is how it was found, and is the way to settle any
"these two should look the same" question.

**In a last-one-wins format, flagging the duplicate that *wins* deletes the
value that applies.** Every `invalid()` here doubles as a filter: `generate`
skips whatever it reports. So "which row do I mark as the dead one" is not a
wording choice — mark the last occurrence and the generator drops the row in
effect and writes the superseded one instead. `hl.env` did this (editing a
variable and leaving the old row made the *old* value take effect) and so did
hyprpaper's blocks (a new wallpaper left the old image on screen). Both existing
tests asserted the wrong index, one of them contradicting its own doc comment.
`core::supersede` owns the rule now; a test that only checks *that* something
was flagged will not catch this, so assert the generated value too.

**`pkill -f <pattern>` matches the shell running it**, because the pattern
appears in that shell's own command line. It kills the shell and exits 144. Use
`pkill -x <name>` or a literal PID.

And `pkill -x`/`pgrep -x` is not the safe fallback it looks like: a process name
longer than **15 characters** is truncated in `/proc/<pid>/comm`, so
`pgrep -x hyprforge-settings` matches *nothing at all* and reports success doing
it. That left a stale window running beside the one under test. `hyprforge-lock`
fits in fifteen; `hyprforge-settings` and `hyprforge-displayd` do not. When the
name is too long, or when the real session is running a copy of the same binary,
match on `/proc/<pid>/cmdline` and `/proc/<pid>/environ` instead — the nested
instance is the one whose environ says `WAYLAND_DISPLAY=wayland-2`.

**iced reports three things for a key press and only one of them is what was
typed.** `key` is the *unmodified* logical key, `modified_key` has modifiers
applied, `text` is what the press produced. Reading `key` turns `SHIFT + j` into
`j`, so a password loses every capital and symbol, PAM rejects a password the
user typed correctly, and each attempt spends a `faillock` slot. Use `text` for
character input and `key` only to recognise Enter, Escape and Backspace — which
produce text of their own (`\r`, `\u{1b}`, `\u{8}`) and must never reach a
password field.

**`hl.exec_cmd` runs while the config is being parsed**, which is before Hyprland
accepts clients. Anything it launches that needs a Wayland connection has to wait
for `$XDG_RUNTIME_DIR/$WAYLAND_DISPLAY` to exist first. Losing that race made the
greeter exit silently with status 0, which greetd then reported as
`conversation failed` for a password nobody had been asked for.

**Find a Hyprland instance with `hyprctl instances -j`**, keyed on `wl_socket`.
Picking the newest directory under `/run/user/*/hypr/` can hand you the real
session, because it writes its log continuously. Also: `hyprctl keyword` does
not work against a Lua config — use `hyprctl eval`.

**A test that cannot run must not report as one that passed.** libtest has no
skipped state: a test that returns early prints `ok`, indistinguishable from
one that asserted something. With hyprpaper running, a parse test passed
having verified nothing at all. The fix is the `HYPRFORGE-SKIP:` marker
convention — a skipped check `eprintln!`s `HYPRFORGE-SKIP: <reason>` before
returning early, in `crates/hyprforge-ecosystem/tests/generated_configs_parse.rs`
and `crates/hyprforge-network/tests/live_networkmanager.rs` — and `check.sh`
greps `--nocapture` output for the marker and reports each one in yellow, so
a green run still shows what it didn't actually check.

The other half of the same mistake is gating a check on the wrong thing. The
parse tests rode tier 2's one `--ignored` run, so they needed *Hyprland*
running to ask *hyprpaper* a question, and on a machine with the daemons and
no compositor they never ran at all. Splitting them out fixed it — and then
the first live NetworkManager test went straight back into that same run,
gating a check on the network service on whether a compositor was up. Every
tier gates on the thing it actually asks. When you add a live test, the
question to answer before writing it is *what has to be running for this to
mean anything*, and that is rarely what the tier above it needed.

## Never do this to the machine you are working on

**Never point the lock screen at the live session.** `--fake-password` is
compiled out of release builds entirely, and in debug builds refuses a display
the process would have connected to anyway. Test against a nested compositor:
`./crates/hyprforge-lock/testing/nested.sh`, then `--display wayland-2`. Run
that script before *every* attempt — a lock client killed while holding the lock
leaves the session locked by design, and restarting the nested compositor is the
clean way back (`hyprctl --instance N eval 'hl.clear_crashed_lockscreen()'` is
the other).

**Never test wrong passwords against real PAM.** `pam_faillock` is active here
at three attempts, so two typos lock the *account*, which is far worse than a
locked screen. `--type-in <wrong>` exercises the failure path against the fake
backend instead.

**The way into a greeter is its compositor, not its code.** Every keybind the
compositor running the greeter has belongs to whoever is standing at the keyboard,
authenticated or not — so `SUPER + Q -> terminal` is a shell as the greeter user
without logging in. `crates/hyprforge-greet/config/hyprland-greeter.lua` has no
binds at all, deliberately, and adding a "harmless" one breaks that property. No
amount of care in the greeter program can compensate.

**Run the Settings app against an isolated `XDG_CONFIG_HOME`.** It writes real
config, and one of its jobs is editing `hyprland.lua`.

**Do not start a second `hyprpaper`** to validate a generated file — it takes
over the IPC socket of the running one. Skip the check when it is running.

## The shape a D-Bus-backed module takes

Four of these now exist — Network over NetworkManager, Bluetooth over
BlueZ, and the tray's item and menu over StatusNotifierItem and
dbusmenu — and they were each better for being built in this order. It is
written down so a fifth does not re-derive it.

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

**Every service gets its own tier in `check.sh`, gated on the thing it
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
registration; `GetLayout`'s reply deserializes as `(u(ia{sv}av))`.
Every one of those is a claim this code makes about somebody else's, and
the compiler cannot see any of them.

**Two failures that always need naming, in both directions.** The daemon
not running is not an empty list — it is its own state with its own
message and a way out of it. And a connection that failed is never
cached, because the message telling the user to start the service keeps
being shown after they do. That one has been got wrong three times.

## Every component runs alone, and is better together

This is a suite, and someone must still be able to install only the
clipboard. Those pull against each other only if "shared at build time"
is confused with "required at run time". They are different things.

**Shared at build time** is what makes it a suite. One
`hyprforge_look::Theme`, one widget vocabulary, one keyboard grammar —
and one place to fix them, which is why `web-colors` was a single
`default-features = false` rather than the same edit in five apps.

**Split at install time** is what makes it honest. `packaging/arch`
builds one source tree into separate packages, so `hyprforge-clipboard`
installs two binaries and nothing else. Packaging granularity and
repository granularity are different questions, and only the first
decides what a user can install.

**So every component has to work with its siblings absent.** That is the
rule, and it is the one that breaks quietly:

- A missing `appearance.toml` is first-run, not an error. The clipboard
  popup draws correctly themed on a machine where the Settings app has
  never been installed, let alone run.
- A service that is not running is a state with a message, never a
  failure. The tray says "NetworkManager isn't running" and keeps its
  other icons.
- A sibling binary that is not installed is a logged warning. Clicking a
  tray icon when `hyprforge-settings` is absent fails to spawn and says
  so; it does not take the daemon down.

The test for a new component is: **install only this package on a clean
machine. Does it work?** If it needs another Hyprforge component to
start, that dependency belongs in the package metadata or the need
belongs in the design — not in a user's surprise.

## Checking your work

```
./check.sh          # everything available on this machine
./check.sh --quick  # tier 1 only: clippy + unit tests, no compositor
```

Clippy must be silent and every test must pass before a commit. Three tiers, and
the later ones exist because code can be internally consistent and wrong about
the system it is talking to: tier 2 checks catalogue claims against the running
compositor, tier 3 hands generated files to the real daemons.

`check.sh` does **not** cover the lock screen's live behaviour. Its unit tests
run in tier 1, but proving it locks, draws and unlocks needs the nested
compositor, by hand.

## What the layering is for

```
hyprforge-paths     no dependencies at all
hyprforge-process   a bounded subprocess wait (Command::output that gives up);
                    no dependencies at all — a client of NetworkManager, BlueZ
                    or systemd-logind needs this and nothing Hyprland-shaped
hyprforge-look      Color + the runtime Theme; no iced, because the lock screen
                    and greeter paint into a raw Wayland buffer
hyprforge-ui        the iced layer; knows nothing about Hyprland
hyprforge-core      Hyprland config machinery — a new app should never need it
```

`hyprforge-core` re-exports `hyprforge-process` as `hyprforge_core::command` so
the ~20 existing call sites did not have to churn when it moved out; new code
should depend on `hyprforge-process` directly.

`hyprforge-ui` and `hyprforge-look` are **leaves**. Neither may depend on
`hyprforge-appearance`: `settings -> appearance` already exists, so that edge
would cycle. Each app's composition root does the joining.

**Never add a colour constant to an app.** Every app reads one
`hyprforge_look::Theme`, resolved from settings the user already controls — the
accent is Hyprland's `general:col:active_border`, fonts come from gsettings. If
a colour is missing, it belongs in the Theme. The Settings app and the lock
screen each grew their own palette once and drifted to different accents, which
is the exact failure this suite exists to prevent, occurring inside the suite.

Do not resolve the accent from the catalogue default: `general:col:active_border`
defaults to white, which is a checked claim about what Hyprland does, not a
design choice.

## Writing code here

Match the surrounding style, which is unusually comment-heavy on purpose:
comments explain *why*, especially where the obvious thing is wrong. If a line
looks like a bug and is not — the lock screen's buffer needs no channel swizzle
because `iced_tiny_skia` writes BGRA deliberately — say so where someone would
otherwise "fix" it, and add a test that fails if they do.

Tests are named as sentences describing the property, not the function under
test. Prefer a test that pins a decision (`a_failure_never_blames_the_password`)
over one that exercises a code path.
