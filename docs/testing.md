# Testing

`tools/hf` is the one tool. This is what it asks, which tier each test
is in, and why each live check waits on the thing it asks and nothing
else.

```
tools/hf check --fast      # clippy, docs, static checks, unit tier — the pre-commit hook and CI
tools/hf check             # + the live and visible tiers, and the session guard
tools/hf test disposable   # the tests that change a compositor, on a nested one
tools/hf test root         # the tests that need root, and the recorder inside its sandbox
tools/hf lint              # the static checks alone
tools/hf sandbox           # every shipped unit verified; self-tests inside their sandbox
tools/hf doctor            # which tools are missing, and whether the hook is on
tools/hf clean             # what build output weighs; removes what is always safe to
```

Nothing `hf check` runs changes the machine's state. That was not true
of the `check.sh` it replaced: on 2026-10-10 its live step pushed every
Hyprland option, at extreme values, into the session in use, and six
runs in one evening left the desktop frozen until a re-login. The rule
in CLAUDE.md ("A live test that changes the compositor changes the one
you are working in") has the whole story.

## The tiers

Every test is in exactly one tier. A tier is a nextest profile in
`.config/nextest.toml` whose `default-filter` says which tests it holds,
and `tools/hf` fails when an `#[ignore]`d test is in no tier or in two —
so a new one has to be classified before it can be committed.

| Tier | What is in it | Run by | Changes anything? |
|---|---|---|---|
| `unit` | every test not `#[ignore]`d | `hf check --fast`, the hook, CI | no |
| `live` | asks a real service, installed tool or the running Hyprland, read-only | `hf check` | no |
| `visible` | registers with a tray host or BlueZ for a moment, then unregisters | `hf check` | briefly, then undone |
| `disposable` | flips Hyprland options, loads binds and rules, sets the wallpaper and temperature, writes the clipboard | `hf test disposable` | yes — a nested instance only |
| `root` | procman's taskstats and eBPF tests | `hf test root` | loads an eBPF program |
| `manual` | screenshot generators and benchmarks | `hf test manual -- -E '…'` | writes PNGs to `$SHOTS` |

**One build serves every tier.** Every cargo and nextest call is
`--workspace` with the same features, so cargo resolves one feature set
and builds one set of test binaries. check.sh ran `cargo test -p <crate>`
per live step, and each `-p` resolved features for that crate alone — a
partial rebuild per step, on top of a separate build each for `publish
--dry-run` and every component built against crates.io. Those two moved
to `hf land` and `hf release`, the two moments their answers decide
something. procman's `tui` and `netbpf` features are on in every build
(netbpf only with clang and libbpf's headers installed, said in yellow
otherwise; CI installs both), because
check.sh never compiled either.

`[profile.dev] debug = "line-tables-only"` with dependencies at no debug
info at all is the other half: debug info was most of the 439GB and then
666GB `target/` that stalled this machine twice.

## Tier 1: `hf check --fast`

Runs everywhere, no system needed. Beside clippy, rustdoc and the unit
tier:

- **The pins build together** — CI builds every component at its pinned
  commit, and this checkout often has one on `next`, so a library change
  the pinned component cannot compile against passes every local check
  and fails CI (`738b5ab`, four days red). When a component is checked
  out away from its pin, this `cargo check`s a worktree of HEAD plus the
  uncommitted changes with every component at its pin
  (`tools/hflib/pins.py`); otherwise the build above already is that tree.
- **Renderer colour space** greps `cargo tree` for iced's `web-colors`
  feature and fails if it is on: both renderers report the same `Color`
  and only the pixels differ (the shared-look rule in CLAUDE.md).
- **Standalone crate dependency pins** — the components are their own
  repositories and hand-copy every third-party dependency's version and
  features; a copy that drifts from `[workspace.dependencies]` builds two
  versions of one crate with no symptom. `hyprforge-notif`'s versions are
  its own on purpose, and it is its own cargo workspace, so it gets its
  own clippy, fmt and test step.
- **Siblings build from this checkout**, **Every package is a
  repository**, **Docs name things that exist**, **Library READMEs match
  their crate docs** — each a script in `tools/hflib/lints/` or `tools/`,
  with its reason at the top.
- **Versions agree**, **Changelog has a section for the version**, **AUR
  PKGBUILD matches packaging/arch** hold the one suite version, its
  changelog and the rendered AUR package to what `hf release` needs.
- **The installer can plan every component** and **The installer's dry run
  touches nothing** — the second compares a scratch home byte for byte
  before and after every component's dry-run uninstall.
- **The installer and the package install the same files** — the
  greeter's package once shipped its binary without the sysusers and
  tmpfiles entries the installer always put down.
- **Every Set up item is undone by some uninstall** — three items were in
  no component's list, so uninstalling Files or the polkit agent left
  their changes behind.
- **Nothing shipped dispatches in hyprlang syntax** — under a Lua config
  `hyprctl dispatch exit` is a Lua error; the greeter's compositor config
  said exactly that, and every login logged the failure for nobody.
- **Every ignored test has one tier**, and every `live` or `visible` test
  sits behind exactly one gate.
- **The backlog keeps its shape** — `docs/backlog.md`'s four sections in
  order, every item with a title and the day it was added, so the
  `backlog` skill can move items without guessing.

## The live gates

Each gate in `tools/hflib/tiers.py` names the tests that ask one thing
and checks that thing is there first — never a neighbour. The parse
tests once rode the Hyprland run and needed a compositor to ask hyprpaper
a question; on a machine with the daemons and no compositor they never
ran at all.

| Gate | Asks | Open when |
|---|---|---|
| Hyprland, read-only | does Hyprland have every catalogued option, with the declared type? | Hyprland answers `hyprctl version` |
| hyprsunset | does the binary and the instance's socket agree? | Hyprland running |
| the ecosystem daemons' parsers | do hyprpaper and hypridle parse what this writes? | either installed |
| the system's archive tools | do tar, unzip and 7z read what this writes, and this what they write? | any installed |
| NetworkManager, BlueZ, UPower, power-profiles-daemon | do they have the interface the crate claims? | the service is active |
| systemd-logind, systemd, polkitd, fprintd, UDisks2 | likewise | the name is on the system bus |
| gio's phones and cameras | does `gio` list what the volumes crate reads? | `gio` installed |
| the Wayland compositor's protocols | data-control, output management, text-input-v3 offered? | `WAYLAND_DISPLAY` set |
| another implementation's trash | can this read the `.trashinfo` already on disk? | a home trash exists |
| the installed icon theme | do the tray's icon names resolve in one theme? | `gsettings` to name it |
| the shared MIME database | does this machine's database say what the parsers expect? | `globs2` readable |
| the portal's FileChooser interface | does the dialog serve what xdg-desktop-portal calls? | the XML and `dbus-daemon` |
| ffmpeg and libmpv | does a thumbnail come from the right frame? | `ffmpeg` installed |
| BlueZ, registering an agent *(visible)* | does BlueZ accept the pairing agent? | bluetooth active |
| a tray host *(visible)* | does a real host list the item, with no `Menu`? | a `StatusNotifierWatcher` on the bus |

notif's two font tests run behind their own check — a font with 标 in it
and one with an emoji — because check.sh never ran them and the first run
failed on a machine with no CJK font rather than on anything notif does.

**A skipped check announces itself.** libtest has no skipped state — a
test that returns early because a device is absent prints `ok` exactly
like one that verified something. Every such test `eprintln!`s
`HYPRFORGE-SKIP: <reason>` first; nextest's JUnit report keeps passing
tests' output (`store-success-output`), and `tools/hf` reports each
marker in yellow even on a green run.

**A silent mistake needs a negative control.** A misspelled key in a
generated config is not an error to hyprpaper or hypridle — they print
`Config error … does not exist. Proceeding ignoring faulty entries` and
exit 0. So the parse tests match on the message, and a control feeds each
daemon a key that does not exist and insists it complains.

The parse tests run each daemon with a runtime directory of its own, a
signature nobody else has and no `WAYLAND_DISPLAY`. A second hyprpaper
under the session's signature takes over the session's IPC socket and
leaves the original unreachable; check.sh avoided that by skipping
whenever hyprpaper was running, which on any machine using hyprpaper
meant the check never ran.

## The session guard

Before the live tiers and after them, `hf check` snapshots the running
Hyprland — every option's current value (`hyprctl descriptions -j`),
binds, workspace rules, animations, monitor modes, hyprpaper's wallpapers
and hyprsunset's temperature — and fails naming anything that moved. A
person changing a setting during the run trips it too, and the message
says so.

## The disposable tier

`tools/hf test disposable` starts a nested Hyprland (`hf hyprland up`),
runs the tier against it and takes it down. There is no headless-only
Hyprland: without a parent compositor it reaches for the DRM device the
session holds. So the instance is a window in the session, kept as safe
as that can be — see `tools/hflib/hyprland.py` and the
`disposable-hyprland` skill: one at a time, a fifteen-minute hard
lifetime, its own hyprpaper and hyprsunset under its own signature, logs
in `target/hf/logs`, and it will not start without a terminal.

Every test in it opens with `hyprforge_testkit::Hyprland::disposable()`,
which panics unless the instance it is given is nested — a compositor
with no `WAYLAND_DISPLAY` in its environment is driving hardware, which
makes it a session. Its handle reloads the instance when dropped, during
a panic too. A pass is stamped in `target/hf/stamps/disposable.json` with
the tree of every crate it covers, and `hf release` refuses a release
those crates changed since.

## Root, and the sandbox

`hf test root` builds as you and runs only the test binaries as root,
through cargo's target runner, so `target/` gains no root-owned file. It
then runs the procman recorder's `--self-test` inside its unit's sandbox
(`hf sandbox`): `systemd-run` with every property the unit file sets,
read from the file. The recorder once shipped with `PrivateNetwork=yes`,
which hides every exit from it, and every test passed — outside the
sandbox.

## Not covered

The lock screen's live behaviour and Files' copy and paste between
applications need the nested compositor by hand; the `disposable-hyprland`
skill has both. The AUR package's clean-chroot build and every component
built against crates.io run in `hf release`; a component is built that
way by `hf land` too.
