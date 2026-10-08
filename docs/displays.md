# Displays

`hyprforge-displayd` keeps a saved layout for every set of monitors it has
seen, applies it when that set is plugged in, and asks before keeping a
change. `hyprforge-displayctl` drives it from a script, and the Settings
app's Displays page is a client of the same D-Bus API.

## Installing the systemd unit by hand

Only for a build installed some other way, such as `cargo install`.
`./hyprforge` and the packages both install the unit, and setup enables
it.

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

The snapshot a revert would restore is taken by the first mutation of an
edit, not by the apply — the GUI issues several `SetHeadGeometry` calls
before applying, and snapshotting at apply time would capture the
already-edited profile. It expires after two minutes if nothing arms it,
because several paths mutate without ever applying (`displayctl apply`,
and the GUI's save-without-apply). Without that bound an abandoned edit
would leave a snapshot alive for the daemon's lifetime, and the *next*
reversible change would arm it — so letting the window lapse would roll
back to a state from hours earlier, discarding every profile learned in
between.

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

Before the above was understood, the README carried an unresolved section
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
above — Hyprforge's own generated file is never flagged as competing with
itself.

Like window rules, its require line is inserted **before** the user's own
`require()` calls, so anything you write yourself for a given output can
still take precedence.

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
