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
```

`simulate-topology`'s spec format is a comma-separated list of
`make:model:serial` triples; any field may be left blank (e.g. `BOE:0x0BC9:`
for a blank-serial panel, mirroring real hardware — this machine's own
built-in panel reports an empty EDID serial).

Automated coverage of the same flows (exact/superset/subset matching,
tie-breaks, duplicate/blank-serial assignment + `SwapHeads`, auto-learn,
the zero-enabled-outputs safety rail) lives in
`crates/hyprforge-displayd/tests/matching.rs`, run via `cargo test
--workspace`. A separate, manual-only test
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
list (`ListProfiles`, `ApplyProfile`, `RenameProfile`, `SwapHeads`,
`SetExtraOutputPolicy`, `GetCurrentFingerprint`, `GetCurrentLayout`, the
`ProfileApplied`/`NewTopologySeen` signals, and the `CompetingMonitorRules`
property).
