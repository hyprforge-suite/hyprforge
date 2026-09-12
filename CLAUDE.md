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
hyprforge-paths   no dependencies at all
hyprforge-look    Color + the runtime Theme; no iced, because the lock screen
                  and greeter paint into a raw Wayland buffer
hyprforge-ui      the iced layer; knows nothing about Hyprland
hyprforge-core    Hyprland config machinery — a new app should never need it
```

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
