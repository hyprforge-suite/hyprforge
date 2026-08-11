# Hyprforge (v1 slice)

Displays daemon + Window Rules library + Settings GUI for Hyprland. See
`hyprforge-vision.md` for the whole-suite context this session's work fits
into; this README covers only what's in this workspace so far.

## Workspace layout

```
crates/
  hyprforge-core/          SettingsModule trait, shared theme/widgets, xdg
                            paths, the D-Bus client proxy for displayd
  hyprforge-displayd/      daemon + CLI binaries (hyprforge-displayd,
                            hyprforge-displayctl)
  hyprforge-windowrules/   library: TOML rule storage, Lua codegen, the
                            one-time hyprland.lua setup flow
  hyprforge-settings/      the iced GUI, hosting both modules
```

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
| `$XDG_CONFIG_HOME/hypr/hyprland.lua` | Your own config — Hyprforge touches exactly one line in it, see below |
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

## How the `hyprland.lua` insertion works, and why it goes first

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

The GUI only performs this insertion once, on first use of the Window
Rules module, and only after showing the exact line and its insertion
point in a confirmation dialog — it never edits `hyprland.lua` silently. A
backup (`hyprland.lua.hyprforge.bak`) is written first.

### If you don't have a `hyprland.lua`

Hyprland's config language became Lua in 0.55, and `require()` — how
Hyprforge sources its generated rules — only exists there. The module checks
which config you actually have before offering to insert anything:

| What's in `~/.config/hypr/` | What the module does |
|---|---|
| `hyprland.lua` | The normal path: shows the require-line confirmation above. |
| Neither file | Offers to create a minimal `hyprland.lua` containing only the require line, showing the exact contents first. Everything else stays at Hyprland's defaults. |
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
