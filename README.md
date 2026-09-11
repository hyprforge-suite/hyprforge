# Hyprforge (v1 slice)

Displays daemon + Window Rules, Shortcuts, Input and Appearance libraries
+ Settings GUI for Hyprland. See `hyprforge-vision.md` for the whole-suite context this
session's work fits into; this README covers only what's in this workspace
so far.

## Workspace layout

Four layers, and the order matters: anything lower may be used by an app that
has never heard of Hyprland.

```
Shared by every app in the suite
  hyprforge-paths/         where config lives and how to write it atomically.
                            No dependencies at all.
  hyprforge-look/          the Color type with the one rgba() parser, and the
                            runtime Theme every app draws from. No iced — the
                            lock screen and greeter paint into a raw Wayland
                            buffer and must be able to use this.
  hyprforge-ui/            the iced layer: spacing scale, palette, widgets.
                            Knows nothing about Hyprland.

Hyprland-facing
  hyprforge-core/          the config machinery: hlconfig (the generic
                            hl.config overlay: catalog, storage, codegen,
                            import), hyprlang, Lua codegen, the one-time
                            hyprland.lua setup, monitor geometry, and the
                            D-Bus proxy for displayd
  hyprforge-displayd/      daemon + CLI (hyprforge-displayd, hyprforge-displayctl)
  hyprforge-windowrules/   TOML rule storage, Lua codegen
  hyprforge-shortcuts/     TOML shortcut storage, Lua codegen, live conflict
                            detection against hyprctl binds
  hyprforge-input/         the input option catalog (51 options)
  hyprforge-appearance/    the appearance catalog (88 options), the animation
                            model, the gsettings bridge, theme/font discovery,
                            and `look::resolve()` — which turns all of that
                            into the shared Theme
  hyprforge-ecosystem/     hyprpaper / hyprsunset / hypridle config
  hyprforge-session/       autostart programs and environment variables
  hyprforge-system/        the misc/debug/render catalog
  hyprforge-lua-import/    sandboxed mlua evaluator that imports hand-written
                            hl.bind()/hl.window_rule()/hl.monitor() calls —
                            the only crate depending on mlua

Apps
  hyprforge-settings/      the iced GUI, hosting the settings modules
  hyprforge-authui/        the authentication conversation model, shared by
                            the lock screen and the greeter
  hyprforge-lock/          the ext-session-lock-v1 lock screen
```

### Why the look is one crate

The Settings app and the lock screen each grew their own palette and drifted
to different accents, different backgrounds and different reds — two products
out of one project, which is the failure this suite exists to avoid. They now
read one `hyprforge_look::Theme`.

Nothing in it is invented. `look::resolve()` derives the accent from Hyprland's
own `general:col:active_border`, the fonts and text scale from gsettings. There
is no Hyprforge theme file to discover, because that would be a second place to
set colours that already exist. The old hardcoded violet had a comment saying it
"matches this desktop's own window-border accent" — it was eyeballed, and it was
wrong by a few points. Now it is read.

Settings publishes the resolved theme to `~/.config/hyprforge/lock.toml` on save,
and exports a copy to `/var/lib/hyprforge/greet`. The export exists because a
greeter runs as its own user and a home directory is `drwx------`: it cannot
traverse into `$HOME` at all, so continuity across the login boundary has to be
an export rather than a shared path.

## Nothing waits on another process forever

`Command::output()` waits indefinitely, and every external program here —
`hyprctl`, `gsettings`, `fc-list`, `lua` — is one that can stop answering: a
wedged compositor, a hung dconf, a home directory on a stalled mount. All twenty
call sites go through `hyprforge_core::command::output`, which kills the child
and reports `io::ErrorKind::TimedOut` after five seconds; the async ones use
`tokio::time::timeout`.

This is the same category of mistake as waiting forever for a compositor to
grant a session lock, which is what put a recovery screen in front of the lock
screen for five seconds. Unbounded waits on another process are worth treating
as a class rather than one at a time.

The child's pipes are drained on their own threads rather than after it exits.
A pipe holds about 64KB before it blocks, and `fc-list` on a machine with many
fonts prints more than that, so reading only after exit would deadlock — the
child waiting for the pipe to drain, the parent waiting for the child.

## Checking everything works

```
./check.sh          # everything available on this machine
./check.sh --quick  # no compositor or daemons needed
```

Three tiers, answering different questions:

| Tier | Asks | Needs |
|---|---|---|
| clippy + unit tests | does the code do what *this project* thinks? | nothing |
| live tests | does **Hyprland** agree? | Hyprland running |
| parse tests | do the **ecosystem daemons** agree? | hyprpaper/hypridle installed |

The first tier catches a mistake in the code. The other two catch the far
nastier kind: code that is internally consistent and wrong about the system
it talks to. Every claim the option catalogues make — that an option exists,
what type it is, what range it accepts — is checked against the running
compositor, and the generated hyprlang files are handed to the daemons
themselves.

That matters most where a mistake is *silent*. A misspelled key in a
generated config isn't an error to hyprpaper or hypridle:

```
[ERR] Config has errors:
Config error … config option <listener:this_is_not_a_key> does not exist.
Proceeding ignoring faulty entries
```

It exits 0, drops the line, and the setting simply never happens. So the
parse tests match on the message, not the status — and a negative control
feeds each daemon a key that definitely doesn't exist and insists it
complains, because a validation test that cannot fail is worthless.

**The hyprpaper parse check skips while hyprpaper is running.** A second
instance takes over its IPC socket, and when it exits the socket is gone,
leaving the original alive but unreachable until it's restarted.

## Build

```
cargo build --workspace --release
```

Binaries land in `target/release/`: `hyprforge-displayd`,
`hyprforge-displayctl`, `hyprforge-settings`.

## Config file locations

| File | Purpose |
|---|---|
| `$XDG_CONFIG_HOME/hyprforge/display-profiles.toml` | Display profiles (canonical, hand-editable) |
| `$XDG_CONFIG_HOME/hyprforge/window-rules.toml` | Window rules (canonical, hand-editable) |
| `$XDG_CONFIG_HOME/hypr/hyprforge/window-rules.lua` | Generated from the TOML above — **never edit, never hand-round-trip**; regenerated in full on every save |
| `$XDG_CONFIG_HOME/hyprforge/shortcuts.toml` | Shortcuts (canonical, hand-editable) |
| `$XDG_CONFIG_HOME/hyprforge/input.toml` | Input settings you've taken ownership of (canonical, hand-editable) |
| `$XDG_CONFIG_HOME/hypr/hyprforge/input.lua` | Generated from the TOML above — **never edit**; regenerated in full on every save |
| `$XDG_CONFIG_HOME/hyprforge/appearance.toml` | Appearance settings and animations you've taken ownership of (canonical, hand-editable) |
| `$XDG_CONFIG_HOME/hypr/hyprforge/appearance.lua` | Generated from the TOML above — **never edit**; regenerated in full on every save |
| `$XDG_CONFIG_HOME/hypr/hyprforge/keybinds.lua` | Generated from the TOML above — **never edit, never hand-round-trip**; regenerated in full on every save |
| `$XDG_CONFIG_HOME/hypr/hyprforge/monitors.lua` | Generated by `hyprforge-displayd` from whatever layout is currently live — **never edit**; regenerated on every settled topology. No canonical TOML of its own; see "Surviving the display daemon not running" below |
| `$XDG_CONFIG_HOME/hypr/hyprland.lua` | Your own config — Hyprforge touches exactly one line per module in it, see below |
| `$XDG_CONFIG_HOME/hypr/hyprland.lua.hyprforge.bak` | Backup taken automatically the one time that line is inserted |

`$XDG_CONFIG_HOME` falls back to `~/.config` if unset, per the XDG spec.

## Installing the systemd unit

```
mkdir -p ~/.config/systemd/user
cp crates/hyprforge-displayd/packaging/hyprforge-displayd.service ~/.config/systemd/user/
# Edit ExecStart if your binary isn't at ~/.cargo/bin/hyprforge-displayd
# (e.g. `cargo install --path crates/hyprforge-displayd` puts it there).
systemctl --user daemon-reload
systemctl --user enable --now hyprforge-displayd
systemctl --user status hyprforge-displayd
```

The unit is `PartOf=graphical-session.target` / `WantedBy=graphical-session.target`,
which matches a uwsm-managed Hyprland session out of the box — no
`import-environment` shim needed, since uwsm already exports
`WAYLAND_DISPLAY` into the systemd user manager before that target starts.

## Testing displayd without touching real monitors

The mock backend simulates the whole `wlr-output-management-v1` surface in
memory — connect/disconnect fake outputs with arbitrary EDID identities,
drive the daemon's fingerprinting/matching/auto-learn logic, and inspect
the result, with zero risk to whatever's actually plugged in.

```
# Terminal 1
XDG_CONFIG_HOME=/tmp/hyprforge-test cargo run -p hyprforge-displayd \
  --bin hyprforge-displayd -- run --mock

# Terminal 2
BIN=./target/debug/hyprforge-displayctl
$BIN list-profiles
$BIN simulate-topology 'BOE:0x0BC9:,DELL:U2720Q:ABC123'
$BIN list-profiles          # a profile has been auto-learned
$BIN current-fingerprint
$BIN simulate-topology 'BOE:0x0BC9:'     # subset match — partial layout applied
$BIN simulate-topology 'BOE:0x0BC9:,DELL:U2720Q:ABC123'   # back to exact match
$BIN apply <profile-id>     # force-apply regardless of current fingerprint
$BIN rename <profile-id> "Home dual monitor"
$BIN delete <profile-id>    # see the caveat below
$BIN get-profile <profile-id>   # full stored geometry, as JSON
$BIN current-layout             # the live layout, as JSON
$BIN modes MOCK-1               # modes a connected head advertises
$BIN set-geometry <profile-id> MOCK-1 0 0 2560 1440 60000 1.25 Normal
$BIN set-policy <profile-id> mirror        # extend_right | mirror | disable
$BIN swap-heads <profile-id> MOCK-1 MOCK-2
$BIN apply-reversible <profile-id>  # provisional; reverts unless confirmed
$BIN confirm                        # keep it
$BIN revert                         # roll back now, without waiting
```

`delete` is a true forget only for topologies that **aren't** plugged in.
Deleting the profile for your current monitors discards its saved
arrangement, but auto-learn recreates one from the live layout on the next
topology change — "no stored profile matches" is exactly the condition that
triggers learning. The GUI's confirmation dialog says which of the two you're
about to get.

## What the static fallback deliberately does not copy

`monitors.lua` describes enabled outputs only. A disabled one is named in
a comment and given no rule at all.

The reason is that a rule is matched against whatever is plugged in when
*Hyprland* reads the file, not what was plugged in when the daemon wrote
it. Dock with the lid shut and the internal panel is genuinely disabled,
so writing `disabled = true` for it is an accurate record of that moment.
Undock and boot before the daemon starts, and that rule still matches —
switching off the only display present, by the very file whose job is to
stop you losing your display when the daemon is not running.

Omitting the rule means Hyprland applies its own default and the panel
comes up. If the profile really does want it off, the daemon turns it off
a second later. The cost is a panel briefly on when it should be off; the
cost of the alternative is a black screen and a TTY.

## The confirm/revert window on display changes

Applying a layout from the Settings GUI is **provisional for 12 seconds**. A
banner offers "Keep changes" / "Revert now", and if you do neither — because
the change blanked or scrambled the screen the banner is on — the daemon rolls
it back on its own.

The timer lives in `hyprforge-displayd`, not the GUI, so it still fires if the
GUI is killed or wedged. A revert restores **both** the compositor state and
the stored profile: undoing only the former would leave the bad geometry saved,
and the next hotplug would re-apply the layout you just escaped.

On the wire this is `ApplyProfileReversible` (returns the number of seconds),
`ConfirmLayout`, `RevertLayout`, plus the `RevertPending`/`RevertResolved`
signals. Plain `ApplyProfile` stays immediate and irreversible — that's what
`displayctl apply` uses, since a scripted caller has no banner to click and
shouldn't have its change silently undone.

`simulate-topology`'s spec format is a comma-separated list of
`make:model:serial` triples; any field may be left blank (e.g. `BOE:0x0BC9:`
for a blank-serial panel, mirroring real hardware — this machine's own
built-in panel reports an empty EDID serial).

## Plugging a new display into a setup Hyprforge already knows

That's a *superset* match: a stored profile covers some of what's connected,
so it's applied and the uncovered outputs are placed by the profile's
extra-output policy. The daemon then learns a second profile describing the
whole connected set, and that one exact-matches from then on.

The learning step matters more than it sounds. Auto-learn is otherwise
exact-only, so without it nothing ever records the new display — and because
the Settings module edits *stored profiles*, a display absent from every
profile cannot be seen or arranged in the UI at all. Plug in a second
monitor and it would simply never appear.

The original profile is left untouched, so unplugging still gets you the
setup you had before.

Automated coverage of the same flows (exact/superset/subset matching,
tie-breaks, duplicate/blank-serial assignment + `SwapHeads`, auto-learn,
the zero-enabled-outputs safety rail, profile deletion, and the
confirm/revert window) lives in `crates/hyprforge-displayd/tests/matching.rs`,
run via `cargo test --workspace`.

The revert tests drive paused tokio time. Note they must `yield_now` *before*
`tokio::time::advance` as well as after: a freshly `spawn`ed task hasn't been
polled yet, so it has no registered `sleep` for `advance` to fire. A separate, manual-only test
(`crates/hyprforge-displayd/tests/live_smoke.rs`, run with `cargo test
--test live_smoke -- --ignored`) connects to whatever compositor is
actually running and lists real outputs — read-only, never applies a
configuration, safe to run against a live session.

## How the `hyprland.lua` insertion works, and why placement differs per module

Each module owns one generated file, sourced by one `require()` line. **Where
that line goes is not the same for every module**, because Hyprland resolves
the two kinds of thing they write in opposite directions:

| Module | Placement | Because |
|---|---|---|
| Window Rules | before your requires | rules are *ordered*, last match wins — going first lets your own rules override |
| Displays (`monitors.lua`) | before your requires | same: a hand-written `hl.monitor()` should win |
| Shortcuts | after everything | binds are keyed by chord; the app owns the ones it wrote |
| Input | after everything | `hl.config` is a scalar overwrite — see below |
| Appearance | after everything | same, plus `hl.animation`, which is also last-wins per leaf |

Getting this backwards is not a cosmetic mistake in either direction. A rule
module placed last silently overrides rules you wrote by hand. A settings
module placed *first* is silently overridden by your own `hl.config` block —
and then the app reports saves that changed nothing, with no way to explain
itself.

Both facts were measured on Hyprland 0.56.1 rather than assumed: a later
`hl.config` call beats an earlier one for the keys it passes, and a partial
call leaves every other key alone. `hyprforge-input/src/setup.rs` has a test
pinning its placement as the *opposite* of window rules', so the two can
never quietly become the same constant.

### Window rules go first

The Window Rules module owns `$XDG_CONFIG_HOME/hypr/hyprforge/window-rules.lua`
entirely, sourced into your config via one `require("hyprforge/window-rules")`
line. That line is inserted **before your own existing `require()` calls**,
not appended after them — this is the opposite of "append last" and is
deliberate:

Hyprland 0.56 evaluates window rules in two tiers — all **named** rules
first, then all **anonymous** ones — and within a tier, top-to-bottom, with
the **last** match winning. Every rule Hyprforge generates is named (that's
how the app finds "the rule it made for Discord" again on a later edit).
Appending the `require()` last would put Hyprforge's named rules *after*
your own named rules in evaluation order, letting Hyprforge silently
override rules you wrote yourself. Inserting it first puts Hyprforge's
rules earliest, so your own rules — named or anonymous — win by default,
which is what "the user's hand-written config takes precedence" actually
requires under this evaluation model.

The GUI performs this insertion automatically, once, the first time the
Window Rules module starts — no confirmation dialog, since the line only
ever points at a file Hyprforge itself owns and never touches anything
you wrote. A backup (`hyprland.lua.hyprforge.bak`) is written first, and
a write failure (permissions, read-only filesystem) surfaces as an error
in the module rather than being silently swallowed.

### Modules sharing a placement are grouped, not scattered

Window Rules and the Displays module's `monitors.lua` fallback both use
`Placement::BeforeUserRequires`; Shortcuts, Input and Appearance use
`AtEnd`. Rather than each
module's install hunting for "whichever `require()` line is textually
first right now" and inserting immediately above it — which would leave
the second module installed landing *above* the first one, in
installation order rather than any meaningful order — every module
sharing a placement is grouped under one labeled block:

```lua
-- Hyprforge-managed requires (evaluated first) — do not edit by hand.
require("hyprforge/window-rules")
require("hyprforge/monitors")
-- end Hyprforge-managed requires
```

and, at the end of the file:

```lua
-- Hyprforge-managed requires (evaluated last) — do not edit by hand.
require("hyprforge/keybinds")
require("hyprforge/input")
require("hyprforge/appearance")
-- end Hyprforge-managed requires
```

A second module installed later joins the existing block (as its last
line) instead of creating a new one. This is handled entirely inside
`hyprforge_core::lua_setup` — no module-specific code needed. Bare,
unmarked `require("hyprforge/...")` lines from before this existed are
still recognized as installed (so nothing gets duplicated) but aren't
retroactively wrapped into a block; only new installs use the grouped
format.

### If you don't have a `hyprland.lua`

Hyprland's config language became Lua in 0.55, and `require()` — how
Hyprforge sources its generated rules — only exists there. The module checks
which config you actually have before doing anything:

| What's in `~/.config/hypr/` | What the module does |
|---|---|
| `hyprland.lua` | The normal path: the require line is inserted automatically. |
| Neither file | Creates a minimal `hyprland.lua` containing only the require line automatically. Everything else stays at Hyprland's defaults. |
| Only `hyprland.conf` | Explains that the generated rules are Lua and points at migrating. **It will not touch your `.conf`** — that's the "never round-trip the user's own config" rule. |

Rules can be created and saved in all three cases; they're stored in the
canonical TOML either way and take effect as soon as setup is finished.

### Rule fields

The basic form covers class/title matching plus workspace assignment, float,
blur, rounding, and border color. "Show advanced" adds the rest of what the
generator supports:

- **Workspace** — which workspace a matching window opens on: an ID (`3`), a
  name (`name:coding`), or a special workspace (`special:scratchpad`). This
  is in the *basic* form, not behind "advanced", because it's the rule most
  people come here to write. "Open there without switching to it" emits
  Hyprland's ` silent` suffix; it's a checkbox rather than something you type,
  since the suffix is an encoding detail of Hyprland taking the whole thing as
  one string. The suffix is never applied to the `unset` keyword, which isn't
  a workspace, and the flag is dropped entirely if the field is left blank.
- **Match on more** — `initial_class`/`initial_title` (the class/title the
  window had when it opened, for apps that rename themselves afterwards),
  three-way `fullscreen`/`floating`/`xwayland`, plus `tag` and `content`
  (e.g. `game`). Three-way rather than a checkbox because "don't match on
  this" and "match windows where this is false" are different rules —
  `floating = false` selects tiled windows. (Hyprland spells that matcher
  `float`, same as the effect; the TOML key stays `floating` for readability
  and the generator translates.)
- **Pick a window…** — next to the Class field. Lists what's currently open
  (read from `hyprctl clients -j`) and fills the class from whichever you
  choose. The title is a separate click, because a title-matched rule stops
  matching as soon as the app renames its window. Without Hyprland running
  the button reports that in the panel and the fields still work by hand.
- **Tag** — applies a tag: `+name` adds, `-name` removes, bare toggles.
  Tagged windows can then be selected by another rule's `tag` matcher, which
  is how one rule's effect becomes another's criteria.
- **Position & size** — `move` and `size`. A plain number is emitted as
  pixels; anything else is passed through to Hyprland as an expression
  (`cursor_x-(window_w*0.5)`, `60%`). Both halves of a pair are required,
  since Hyprland's `{ x, y }` form can't express one without the other.
- **Opacity** — per-state active/inactive/fullscreen, with the `override`
  flag for absolute rather than multiplied values.
- **Appearance** — `opaque`, `no_anim`, `dim_around`, `border_size` and
  `animation`. Hyprland has no `no_border` effect; a border size of 0 is how
  that's spelled, so 0 is kept rather than treated as unset.
- **How it opens** — the *static* effects, applied as the window appears
  rather than toggled on one already up: `tile`, `fullscreen`, `maximize`,
  `center`, `pin`, `no_initial_focus`, per-window `monitor`, plus
  `suppress_event`, `group` and `no_close_for`. The monitor is stored as
  `desc:<description>` like a workspace pin, so it survives a replug.

  `suppress_event` and `group` are the two fields Hyprland does **not**
  validate: it accepts any string and silently does nothing with one it
  doesn't recognise, so a typo there reports no error at all. The known-good
  values are shown as placeholders for that reason.
- **Focus & sizing** — `no_focus`, `stay_focused`, `keep_aspect_ratio`,
  `min_size`/`max_size` (plain pixels, no expressions — unlike move/size),
  and `idle_inhibit`. That last one is a dropdown rather than a text field
  because Hyprland validates it: an unknown mode is rejected outright, and a
  rejected field aborts the entire generated file.

### Workspaces on monitors

A window rule can say which workspace a window opens on, but not which
*monitor*. Hyprland spells that as a separate call, `hl.workspace_rule`, and
the "Workspaces on monitors" section of the module writes it. The two compose:
the rule sends Steam to `name:gaming`, the pin puts `name:gaming` on the
external display.

Monitors are stored as `desc:<description>` rather than as a connector.
Connectors are assigned in probe order and can move between boots, while the
description comes from the EDID and identifies the physical panel — so a pin
still applies after a replug. The description has to be exactly what `hyprctl
monitors` reports (`BOE 0x0BC9`), which is why the dropdown is populated from
there rather than from displayd, whose own formatting differs.

Both rule kinds live in the same `window-rules.toml`, as `[[rule]]` and
`[[workspace_rule]]` arrays. A file written before workspace rules existed
loads fine — it simply has none.

## Shortcuts module

A shortcut is a modifier set, a key, and a Hyprland dispatcher call under
`hl.dsp.*` (e.g. `window.close`, `exec_cmd`) with a Lua argument expression
passed through verbatim — there are dozens of dispatchers taking different
shapes, so the module stores the call rather than trying to model every
one's arguments.

Every generated bind carries a `hyprforge: ` -prefixed description. That
prefix is the only way the module recognises its own binds when it reads
`hyprctl binds -j` back — Hyprland doesn't report which file defined a
bind, so there's no other runtime signal. It's also what conflict
detection excludes when you're editing a shortcut that's already applied,
so re-saving it without changing the chord doesn't report a conflict with
itself.

Conflict checking reads the compositor's *actual* live binds rather than
parsing `hyprland.lua`: a hand-written config binds things like
`hl.bind(mainMod .. " + C", ...)`, and statically parsing that yields the
literal Lua expression, not the resolved chord. Hyprland has already
evaluated it by the time `hyprctl binds -j` reports it, so that's what the
module asks. Saving a shortcut that collides with an existing bind (yours
or Hyprforge's own) is blocked with the name of what's already there; if
Hyprland isn't running to ask, the save proceeds anyway rather than
blocking on a check that can never complete.

Unlike window rules, the require line for shortcuts goes **last** in
`hyprland.lua`, not first — the opposite placement, and deliberate.
Hyprland resolves a repeated bind by last-one-wins, so appending means a
shortcut you set here takes effect even if your own config already binds
that chord elsewhere, since you changed it here, most recently.

## Input and Appearance modules

Both edit Hyprland's `hl.config` categories, and both work the same way
because `hl.config` does: **a generated file is an overlay**, naming only
the settings you have taken ownership of. Everything else stays with
Hyprland's default or your own config. That is what makes them safe to
adopt one setting at a time rather than all at once.

Each row says which of three things it is showing, and the distinction is
the point of the screen:

- **Set by Hyprforge** — this app writes it, and it wins.
- **From your Hyprland config** — your config sets it; the app is only
  reporting it back.
- **Hyprland default** — nobody set it.

**Reset is not "set it to the default".** It stops writing the key
entirely, so Hyprland and your own config decide again.

Import reads your config file, not the running compositor. `hyprctl
getoption` reports whether an option is *set*, but not whether *your
config* set it — anything written at runtime by `hyprctl`, a script or a
test run looks identical. Evaluating the config has neither problem, and
lets the import skip Hyprforge's own generated file, which is `require()`d
from `hyprland.lua` and would otherwise offer Hyprforge's own values back
as if you had written them.

### What Input covers

51 options: keyboard layout/variant/model, repeat rate and delay, pointer
sensitivity and acceleration, scrolling, focus-follows-mouse, and the
touchpad, touchscreen, virtual keyboard and tablet stylus subcategories.

`input:tablet:*` is deliberately absent — four of its nine fields are
`vec2`, which needs an editor nothing else here uses. It is listed as
unsupported rather than omitted, so a tablet key in a hand-edited file is
reported as "not editable here" instead of as a typo.

### What Appearance covers

88 options across `general`, `decoration`, blur, shadow, glow, motion blur
and `cursor`, plus border colours — and two things that aren't `hl.config`
at all:

- **Animations.** Speed, enable and curve per animation. `hl.animation`
  looks append-shaped but is last-wins per leaf, so it is the same overlay
  in the same file. Curves are read, never written: authoring a bezier
  needs a control-point editor with a preview to be anything but
  guesswork, so the picker offers the curves your config already defines.
- **The desktop theme** — GTK theme, icons, cursor theme and size, fonts,
  light/dark. These live in gsettings, which is shared desktop-wide state
  with one value and no layering. Writing one *is* the change: there is no
  generated file to delete to undo it, so nothing is written unless you
  change that control, and the previous value is offered back afterwards
  as the only undo available.

`decoration:wobble:*` is documented by the Hyprland wiki but **does not
exist in 0.56.1** (`hyprctl getoption` says "no such option"), so it is
declared unsupported rather than catalogued. A live test fails if it ever
ships, so the note can't outlive its truth.

### Where the option lists come from

Every option's type, range and accepted values lives in one catalog per
module, and `tests/live_*.rs` checks every claim in it against a running
compositor — the same arrangement that keeps the shortcuts dispatcher
catalog honest. Run them with:

```
cargo test --workspace -- --ignored
```

Fields whose values can be discovered are offered as pickers rather than
typed: keyboard layouts, variants and models from
`/usr/share/X11/xkb/rules/evdev.lst`; monitors from `hyprctl`; GTK, icon
and cursor themes and font families from the filesystem and `fc-list`. A
picker never drops a value it doesn't recognise — an unplugged monitor or
a theme outside the search paths stays selectable, marked. A
comma-separated value (`kb_layout = "us,cz"`) falls back to a text field,
because a single-select dropdown can't express it.

## The lock screen

`hyprforge-lock` locks the session with `ext-session-lock-v1` and
authenticates against PAM. It shares `hyprforge-authui`'s conversation
model with the greeter, because the problem being solved is that a
greeter and a lock screen usually look like two different systems.

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
`hyprctl --instance <N> eval 'hl.clear_crashed_lockscreen()'`.

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

Three gaps, none of them a security hole, all of them things an
established lock screen has:

- **No input-method support.** A password typed through an IME cannot be
  entered. swaylock is the same; it still means some users cannot log in.
- **The password is not zeroized.** `entered` is a plain `String`, cloned
  into the backend, and PAM holds its own copy. A core dump or swap could
  contain it.
- **No attempt limiting of its own**, on purpose: rate limiting belongs in
  `/etc/pam.d`, where an administrator can see and change it, rather than
  hidden in a settings app.

Caps Lock *is* shown, which is not decoration: without it a stuck key
looks exactly like a forgotten password, and where `pam_faillock` is
configured that spends attempts against the account rather than the
screen.

## Scales are 120ths, and most of the ones you'd want don't exist

This is the single most surprising thing about display handling here, and it
explains a whole family of symptoms: positions that disagree with `hyprctl`,
monitors the compositor calls overlapping when the arithmetic says they're
flush, and a layout that quietly refuses to apply.

Hyprland rounds an incoming scale onto a grid of **120ths** — the unit
`wp_fractional_scale_v1` is defined in — *before* it checks anything, and
only then requires that the resolution divide cleanly. So a usable scale is
`k/120` where `k` divides both `width*120` and `height*120`. Nothing else
qualifies, however cleanly it divides on paper.

On a 2560x1600 panel, `gcd(2560*120, 1600*120)` is 38400, so the achievable
scales are the divisors of 38400 over 120 — 1.25, 1.6, 5/3, 2.0 and so on.
**150% and 175% are not among them.** Ask for either and Hyprland refuses,
picks its own replacement, and says so only in its log:

```
ERR ]: Invalid scale passed to monitor, 1.5       found suggestion 1.6
ERR ]: Invalid scale passed to monitor, 1.7460938 found suggestion 1.6666666
```

The failure that follows is indirect, which is what makes it confusing. The
setting now disagrees with what the hardware is doing, every position derived
from it is computed against a logical width nothing is using, and neighbours
computed as flush end up a few pixels inside each other — at which point the
daemon refuses to apply the layout at all (see "Never apply a layout with
outputs on top of each other"). Auto-learn then rewrites the profile from the
read-back, and the pair can chase each other in a slow oscillation.

Note the trap: requiring only that `width/scale` and `height/scale` be whole
is *not* the rule, and it's a plausible-looking mistake. It admits any
`gcd(w,h)/k` — for 175% that's 320/183, which divides 2560 into exactly 1464
and is rejected all the same, because it isn't a 120th.

`hyprforge-core::geometry` owns both halves of this:

- `nearest_valid_scale(w, h, nominal)` — the nearest achievable `k/120`. The
  Displays module runs every dropdown step through it, so the scales offered
  are only ones the selected panel can genuinely take (which is why a
  2560x1600 panel offers 160% where you might expect 150%). `build_layout_plan`
  applies it again to whatever a profile asks for, so a scale arriving from
  `displayctl`, a hand-edited TOML, or auto-learn can't get through either. A
  substitution is logged at `warn`.
- `logical_size(pixels, scale)` — the layout space an output occupies. It
  snaps to the nearest 120th first, which matters because a scale can't
  survive `wl_fixed` intact: 1.6 arrives as 1.6015625, and dividing by *that*
  gives 1598.4 for a panel the compositor lays out 1600 wide. Snapping
  recovers the compositor's own integer exactly.

Snapping is idempotent on an already-valid scale, so a read-back-and-reapply
cycle settles instead of walking.

### Earlier readings, now explained

Before the above was understood, this README carried an unresolved section
about `wlr-output-management` and `hyprctl monitors` reporting scales and
positions that didn't agree at the same instant. That is the substitution
above: the daemon's view was self-consistent with the scale it *asked* for,
`hyprctl` reported the one Hyprland had swapped in.

One caveat is worth keeping. Those measurements were taken on a machine that
also has competing `hl.monitor()` rules (see the next section), including a
catch-all `hl.monitor({ output = "", ..., scale = 1 })` that re-asserts itself
on every reload. That confound was never removed, so while the scale
substitution accounts for what was seen, it hasn't been proven to be the
*only* thing that was happening.

## Surviving the display daemon not running

Every other Hyprforge module that touches Hyprland's config works by
owning a generated file Hyprland sources itself — so it takes effect on
Hyprland's own startup/reload with no Hyprforge process involved.
`hyprforge-displayd` didn't: it applies layouts entirely at runtime over
`wlr-output-management-v1`, so if the daemon isn't running (crashed before
systemd's `Restart=on-failure` catches it, not started yet at session
boot, or its unit was simply never installed — that's a manual, opt-in
step above), Hyprland falls back to its own default hotplug behavior and
your chosen scale/resolution/arrangement is lost until the daemon comes
back.

`monitors.lua` closes that gap. Its require line is installed
automatically the first time the Displays module starts — nothing else
about Displays depends on this; it works fully without it — after which
the daemon regenerates the file from whatever layout is actually live
every time a settle confirms the compositor matches the profile in
effect. Hyprland applies it on its own next startup/reload, independent
of whether `hyprforge-displayd` is running at that moment — a "last known
good" snapshot, not the daemon's fingerprint-matching intelligence, but
enough that the display doesn't regress just because the daemon hasn't
had a chance to run yet.

The `desc:` selectors in it come from `hyprctl monitors -j`, never from
displayd's own `wlr-output-management` description — the two are
formatted differently (this project already hit that exact mismatch once,
for window rules' monitor pins) and Hyprland only recognizes its own.
`monitors.lua` lives under `hypr/hyprforge/`, not directly under `hypr/`,
so it's outside the scope of the competing-`hl.monitor()`-rules scan
below — Hyprforge's own generated file is never flagged as competing with
itself.

Like window rules, its require line is inserted **before** the user's own
`require()` calls, so anything you write yourself for a given output can
still take precedence.

## Known interaction with nwg-displays / hand-written `hl.monitor()` rules

If your `hyprland.lua` (or anything it `require()`s) contains `hl.monitor()`
rules — e.g. a `monitors.lua` generated by `nwg-displays`, or Hyprland's own
`{output = "", mode = "preferred", position = "auto"}` hotplug fallback —
those are re-applied by Hyprland itself on every `hyprctl reload`, and can
fight `hyprforge-displayd`'s auto-applied layout.

`hyprforge-displayd` detects `hl.monitor(` in any `.lua` file directly under
`$XDG_CONFIG_HOME/hypr/` at startup, logs a warning, and exposes the
matching file paths via the `CompetingMonitorRules` D-Bus property (visible
as a banner in the Displays module). It never edits these files for you —
if you want Hyprforge to own display configuration, remove or comment out
the competing `hl.monitor()` rules yourself.

## Importing hand-written config

Window Rules, Shortcuts, and Displays each have an "Import from config"
action that reads your existing `hl.window_rule()`/`hl.bind()`/
`hl.monitor()` calls out of `hyprland.lua` and offers them as pre-filled
entries to review and add. Opt-in and re-runnable any time — not part of
first-run setup, and nothing is added without you checking it in the
review list first.

This works by actually evaluating your config in a sandboxed Lua
interpreter (`hyprforge-lua-import`, using `mlua`), with `hl.*` stubbed to
record what was called instead of acting on it, rather than by parsing the
Lua source as text. That distinction matters: a real config almost always
writes binds as `hl.bind(mainMod .. " + Q", ...)`, and `mainMod` is a
variable — no text parser can know its value, but a real interpreter
resolves it the same way Hyprland's own config loader would. The sandbox
has no `os`/`io`/`package` access and a hard instruction-count limit, so
evaluating an untrusted-looking config can't run a shell command or hang.

A few things worth knowing about how it behaves:
- **A checked entry's original line is removed from your config, but only
  once it's safely added, and only if the removal is unambiguous.** After
  a successful import, the line is re-read fresh and removed only if,
  trimmed, it still both starts with `hl.` and ends with `)` — a call
  written on a single line, exactly as it was when the list was
  generated. A call spanning multiple lines, or one you've edited since,
  is left in place rather than guessed at; a fresh backup
  (`<file>.hyprforge-import.bak`) is written before any file with at
  least one real match is touched.
- **Shortcuts' recovered action is only as good as what the bind actually
  calls.** A dispatcher reached through `hl.dsp.*` (the normal case)
  imports with its real dispatcher and argument — something `hyprctl
  binds -j` can never report, since every Lua-defined bind shows there as
  opaque `__lua`. A bind that calls something else entirely has no
  recoverable action under any approach and imports with an empty one for
  you to fill in.
- **Monitors import is informational only**, not a one-click add. A
  stored display profile is keyed on the physical panel's make/model/
  serial; a hand-written `hl.monitor()` rule only ever names an `output`
  selector, which doesn't reliably give you that back. Rather than guess
  an identity and risk a profile that silently never matches anything,
  the Displays module just lists what it found — plugging that display in
  once lets `hyprforge-displayd` learn a correct profile from it
  automatically, same as any other new display.

## D-Bus API

Bus name `dev.hyprforge.Displayd`, object path `/dev/hyprforge/Displayd`,
interface `dev.hyprforge.Displayd1`. See
`crates/hyprforge-core/src/displayd_proxy.rs` for the full method/signal
list (`ListProfiles`, `ApplyProfile`, `ApplyProfileReversible`,
`ConfirmLayout`, `RevertLayout`, `RenameProfile`, `DeleteProfile`,
`SwapHeads`, `SetExtraOutputPolicy`, `SetHeadGeometry`, `GetProfile`,
`GetAvailableModes`, `GetCurrentFingerprint`, `GetCurrentLayout`, the
`ProfileApplied`/`NewTopologySeen`/`RevertPending`/`RevertResolved` signals,
and the `CompetingMonitorRules` property).

`displayctl` covers all of these, which makes the mock-backend workflow above
a complete exercise of the API rather than a subset.

Note that `zbus` is pinned to its **tokio** executor
(`default-features = false, features = ["tokio"]`) workspace-wide. With zbus's
default `async-io` backend, D-Bus method handlers run on zbus's own thread,
outside any tokio runtime — anything they spawn (the revert timer) then panics
with "there is no reactor running". Keep that feature set if you add new
handlers that spawn.
