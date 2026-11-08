# Backlog

What is left to do, in one place. The `backlog` skill keeps it: an item
is added when work is deferred or an idea is parked, moved to **Done**
when it lands, and checked against the code before it is reported as
still open — an item is a claim about the repository, and claims go
stale.

Each item is one bullet: a type tag, a bold title, why it matters or
what it waits on, and the date it was added. Within a section, items are
ranked: the first is the one most worth doing next.

```
- [bug] **A title that names the thing.** Why, and where to start.
  *Added 2026-11-07.*
```

The tags are `bug` (something is wrong), `feature` (something new),
`check` (built, never tried for real — usually needs the user's screen
or hardware), `decision` (the user's call) and `chore` (tooling, docs,
housekeeping). Ideas and Done carry none. `grep '\[bug\]'
docs/backlog.md` is every known bug.

`tools/hf lint` holds the file to this shape, and to two clocks: an item
in **Working on** carries `*Started YYYY-MM-DD.*` and goes stale after
14 days, and one in **Done recently** carries `*Done YYYY-MM-DD.*` and
is pruned after 14 days.

## Working on

Started and not finished — so a second agent, or the next session, sees
what is already in hand before it starts the same thing. Say what is
left, and move an item back to its section if it stops.

Nothing in hand.

## Needs you

Things only the user can do — an account, a password, a decision, or
someone watching the screen. Ranked by what each one unblocks.

- [check] **Switch the pressure icon on once.** Settings → Tray → Pressure
  (tray 623b3b0, Settings on `next`). It should show calm, and change
  only when something waits — the trigger wake has been tested by hand,
  not by a real slowdown. *Added 2026-10-18.*
- [check] **Try procman's window picking once.** Processes → Pick a window,
  then click another window: its app should be selected and procman come
  back to the front (the refocus is a `hyprctl dispatch`, untested
  against a real click). And a bind running `hyprforge-procman
  --end-focused`: it should open asking about the focused app.
  *Added 2026-10-18.*
- [check] **Look at procman's Recorded tab once.** Redesigned 2026-10-18 (procman
  7486ee0) but never rendered: its journal is root's. `sudo cp
  /var/log/hyprforge/procman/$(date +%F)* ~/`, then `JOURNAL=~/<file>
  SHOTS=… cargo test -p hyprforge-procman --bin hyprforge-procman
  render_recorded -- --ignored`. *Added 2026-10-18.*
- [check] **Try procman's limits, efficiency mode and rules once.** Both were built
  against a mock systemd and checked on throwaway processes; neither has
  touched one of the user's apps. Limits: Details → Limits… on a test
  app (a second Ghostty), 1 GB, then `systemctl --user show` on the
  scope it names. Efficiency: `hyprforge-procman-efficiency --dry-run`
  first, then the Processes tab's toggle, a busy app sent to another
  workspace, and the toggle off again — every weight should be given
  back (`systemctl --user show -p CPUWeight`). Rules: one for `cargo`
  (priority 10, 8 GB), `hyprforge-procman-rules --dry-run`, the tab's
  switch, then a build — `ps -o ni` and its scope should show both.
  *Added 2026-10-17.*
- [check] **Close Files a few dozen times in the nested Hyprland.** Two fixes
  for its 33 segfaults: the drag worker's connection is never torn down
  (8233aaa), and since 2026-10-18 the worker is stopped before the
  window closes (Files 0c218cd) — the one crash after the first fix was
  the worker still dispatching on the display winit had just freed.
  Proof is closing it both ways (last tab, titlebar ×) repeatedly with a
  drag done first, then `coredumpctl list hyprforge-files` showing
  nothing new, and the window still closing at once. *Added 2026-10-17.*
- [decision] **The NVIDIA card is held awake by its power setting.**
  `/sys/bus/pci/devices/0000:01:00.0/power/control` is `on`, which
  disables runtime power management outright, though the driver runs
  fine-grained D3 (`DynamicPowerManagement: 2`) and could sleep the card
  when idle. On battery that is a few watts all the time. Setting it to
  `auto` (usually a udev rule) is a system change and the user's call.
  *Added 2026-10-17.*
- [decision] **Procman: container names, and Podman.** Containers are listed
  (procman, 2026-10-18), but Docker only names them for the `docker`
  group, which this user is not in — and that group is root by another
  name, so joining it is a real decision, not a setting. Without it a
  container goes by its first process. Podman is not handled until one
  is installed to read its cgroups from. *Added 2026-10-18.*
- [feature] **Procman: bandwidth per connection, with root.** Details lists each
  process's connections; their bytes need the eBPF counter to key on
  the socket as well as the process, and loading it needs root — the
  root tier, in the user's terminal. *Added 2026-10-17.*
- [bug] **Procman: a right click opens no menu, so start the nested
  Hyprland.** The user reports nothing at all on a right click, on any
  row. The menus are wired (`src/menus.rs`) and a real right press sent
  through the drawn view by `iced_test` opens one (procman cf584b5), so
  the cause is outside what headless iced reaches. Next: `tools/hf
  hyprland up` in the user's terminal, then the installed procman in it
  and a right click from a virtual pointer (`zwlr_virtual_pointer_v1`,
  buttons 0x110/0x111; the nested-pointer-drag-rig memory), after
  checking the rig gets a menu out of Files. *Added 2026-10-17.*
- [check] **An encrypted stick, once, for real.** Unlocking was built against
  UDisks2's interface and a mock, never a real LUKS device. A stick (or
  a LUKS image attached with `udisksctl loop-setup`) clicked in Files:
  the prompt, a wrong passphrase, the right one, then Eject — after
  which `lsblk` should show no `crypt` mapping left. *Added 2026-10-12.*
- [check] **A first look at procman, together, and its screenshots.** The
  window has only ever been drawn offscreen (`render_every_tab`); open
  it with the user watching and go through every tab. Its site page
  (`site/pages/procman.html`) shows three placeholders, one as the hero
  — `procman`, `procman-recorded`, `procman-top` — and the README has no
  procman image. The same session can take them (`live-screenshots`
  skill). *Added 2026-10-10.*
- [decision] **Sound: which audio work comes first.** The suite has no volume,
  output or per-app mixer anywhere: Settings' Sound page is deferred
  (`crates/hyprforge-settings/DESIGN.md`, "needs a backend") and the
  vision's PipeWire module is not started. The biggest visible gap;
  scope (a Settings page, a tray item, both) is the user's call.
  *Added 2026-10-11.*
- [check] **Previous Versions against real snapshots.** Files' sheet has only
  been tested against snapper's documented `info.xml`, never this
  machine's. Apply Set up → "Previous versions" (pkexec, the user's
  password), then look together (`crates/hyprforge-files/DESIGN.md`
  around line 1122). *Added 2026-10-11.*
- [decision] **The greeter: more than one user, and a session choice.** It logs in
  one account named in `hyprland-greeter.lua` (`CHANGE_ME`) and always
  starts Hyprland (`crates/hyprforge-greet/src/main.rs:30`). A user
  picker needs a design decision: how it meets the export directory and
  avatars. *Added 2026-10-11.*
- [decision] **The greeter: a power menu, or deliberately none.** There is no way
  to shut down or reboot from the login screen
  (`crates/hyprforge-greet/src/main.rs:85`). authui already has the
  lock's logind-checked menu; offering power actions before login is a
  security trade-off for the user to accept. *Added 2026-10-11.*
- [decision] **The lock screen: input methods.** The greeter has an IME (fcitx5,
  sandboxed); the lock has none (`docs/lock-and-greeter.md` known gaps),
  so a password that needs an IME can log in but not unlock. Worth doing
  is the user's call; testing is by hand on a nested compositor.
  *Added 2026-10-11.*
- [decision] **Media's Edit button.** A disabled ✎ in the toolbar
  (`crates/hyprforge-media/src/view.rs:116`); crop, straighten, exposure
  and saving a rotation are unbuilt, because `docs/design/media-plan.md`
  phase 3 leaves writing back to someone's original as a decision
  nobody has made. Decide, or remove the dead control.
  *Added 2026-10-11.*
- [decision] **OK to delete two leftovers?** A stale agent worktree
  (`.claude/worktrees/agent-ab5de03282eb204ef`, 2.8 GB, fully merged,
  clean) and the suite repository's `files-phones` branch on GitHub
  (fully merged). Nothing would be lost; deleting is the user's say.
  *Added 2026-10-11.*
- [chore] **The AUR.** No account yet: on 2026-10-16 the AUR was not accepting
  signups, so this waits on them. When there is one, push the latest
  tag's render only (`packaging/aur/hyprforge/`, the commands
  `hf release` prints at B4); skipped versions need no replay.
  `devtools` is installed now, so the next release's clean-chroot build
  runs instead of skipping. *Added 2026-10-11.*
- [chore] **Files' last two screenshots.** `files-devices` and `files-dialog` are
  still placeholders in `site/pages/files.html`; the live screenshot rig
  needs the user present. *Added 2026-10-10.*
- [check] **Files' Open as administrator, live.** The pkexec helper has passed
  its tests but never a real round trip with the user's password.
  *Added 2026-10-06.*
- [check] **Phones and cameras with a real device.** Files' gvfs code was built
  from real published `gio` output and has never seen a phone plugged
  in. *Added 2026-10-07.*
- [feature] **Network: enterprise Wi-Fi, VPN, hotspot.** 802.1X networks say
  "Hyprforge cannot join it yet — use nmcli"
  (`crates/hyprforge-network/src/types.rs:146`); no VPN or hotspot code.
  Large, and testing needs a real enterprise network or VPN.
  *Added 2026-10-11.*
- [decision] **`pam-fprint-grosshack`, or not.** The only way to offer a finger and
  a password at the same moment. A third-party AUR package, so
  installing it is the user's call. *Added 2026-10-10.*
- [check] **Does cancelling a fingerprint cost a faillock slot?** Read from the
  source, probably yes, about ten seconds later. Measure only with the
  user's OK, and never with a wrong password. Optional.
  *Added 2026-10-10.*

## Engineering

Open work Claude can do unattended. Nothing here blocks a release.

- [feature] **Parked mid-pass: the look following Hyprland, and procman's redesign.**
  Done and committed: every procman tab redesigned, data drawn in text
  shades rather than state colours (procman b294a63, on `next`), and the
  look reading both borders from the running Hyprland with surfaces in the
  inactive border's hue (the monorepo commit after fa1e088). Left: render
  Settings, Files and the lock screen under the new surfaces and compare a
  screenshot's pixels against the theme (CLAUDE.md's rule about two hosts
  looking the same); the half-width layouts of the five over-time tabs;
  move the canvas designs' hex values to the derived ones. A start on the
  first sits uncommitted: Settings' `render::render_every_screen` (its
  `src/main.rs` and `Cargo.toml`), with its line in `.config/nextest.toml`
  and the `Cargo.lock` entries it needs. *Added 2026-10-18.*
- [bug] **Why receiving one D-Bus signal costs the tray ~45 wakeups.** After
  the 2026-10-17 work the tray idles at ~1,590 wakeups a minute, from
  ~3,400. Measured again 2026-10-18 (one thread, nothing else in the
  process): 324 voluntary switches one minute and 1,433 the next, so it
  follows what the system bus delivers (Wi-Fi strength, UPower) rather
  than a steady cost. Which part wakes — zbus's reader, tokio's driver or
  the match-rule streams — needs a tracer, and that needs you:
  `ptrace_scope` is 1 here, so an agent cannot attach to the running
  tray, and starting a second one to trace breaks the one-instance rule.
  `sudo strace -c -f -p $(pidof hyprforge-trayd)` for a minute, or
  `perf` installed. *Added 2026-10-17.*
- [feature] **A Keyboard screen for the suite's own bindings.** Media's keys can
  only be changed by editing `media-config.toml`; `docs/design/vision.md`
  plans a Settings screen on the `hyprforge-keys` grammar, and Files'
  `config_edit` is there to reuse. Medium. *Added 2026-10-11.*
- [chore] **`hf hyprland up --for-lock`.** The tooling plan had the disposable
  instance replace `crates/hyprforge-lock/testing/nested.sh`; the lock
  screen still uses the script. *Added 2026-10-11.*
- [check] **Media's model view, live.** Drag to rotate and pan has never been
  exercised with a real pointer, and view3d's `--screenshot` was not
  carried over. *Added 2026-09-26.*
- [chore] **Time `hf check` against what it replaced.** `check.sh` was never
  timed, so "lighter" is measured on one side only (a warm
  `hf check --fast` is 102 s). *Added 2026-10-11.*
- [chore] **Pixel tests for the video and 3D panes.** Media's still picture is
  now drawn and read back in tests, but the video and model panes are
  `shader` widgets, which `iced_tiny_skia` doesn't draw, and CI has no
  GPU for `iced_test`'s wgpu backend. Needs either a software wgpu
  adapter in CI or a test-only CPU path. Medium. *Added 2026-10-12.*
- [feature] **A screenshot tool, like Flameshot.** A new component: drag out a
  region (or pick a window or an output) over a frozen frame, then mark
  it up in place — arrows, boxes, text, highlight, blur and numbered
  steps — and copy, save or pin it. The suite has nothing here; today
  it's grim and slurp, which have no editing step. Built on the layer
  shell and `wlr-screencopy`, and themed like everything else. Large.
  *Added 2026-10-12.*

## Ideas

Parked on purpose — written down so they are not lost, not scheduled.

- **The vision's unbuilt components.** Users and time (accountsservice,
  timedated), a bar to replace waybar (and be the tray host), Disk
  Utility, Notepad, Calculator, Calendar — `docs/design/vision.md`.
  Audio is under Needs you. *Added 2026-10-11.*
- **Settings' "needs a backend" list.** Network's IP/DNS panel and
  throughput graph, Power's 24-hour chart and charge limit, a live
  tiling preview, Displays' VRR and 10-bit rows, the palette's inline
  toggle, Wi-Fi Disconnect (`crates/hyprforge-settings/DESIGN.md`).
  *Added 2026-10-11.*
- **The GPU broker, `hyprforge-gpud`.** Launch routing by policy, the
  discrete GPU kept asleep, and per-app GPU choice moved here from
  procman. `docs/design/vision.md` has it; moving an arbitrary running
  app between GPUs is impossible on any OS. *Added 2026-10-10.*
- **Drops from other apps into Files.** Hyprland 0.56 sends a drag only
  to the first data device a client made, which is iced's clipboard's.
  A fork of that was declined; this waits on Hyprland or iced upstream.
  *Added 2026-10-03.*
- **An animation curve editor.** Settings can only pick curves the
  config already defines. Large. *Added 2026-10-11.*
- **Files' deliberate omissions.** Pausing transfers, a content index,
  `network://` browsing and remembered servers, thumbnails on phones,
  git and a terminal drawer, writing single-file `.gz` archives; and
  bulk rename on network shares, unmeasured and outside the job queue
  (`crates/hyprforge-files/DESIGN.md`). *Added 2026-10-11.*
- **Procman: io_uring waits are not I/O trouble.** Ghostty idles in
  `io_cqring_wait`, which the kernel counts as iowait, so raw I/O
  pressure reads 80–90% with idle disks. Don't call that "struggling".
  *Added 2026-10-10.*
- **Blur behind the lock's card.** A software-renderer limit, accepted
  in `docs/lock-and-greeter.md`. Large, cosmetic. *Added 2026-10-11.*
- **notif on sway.** Its README claims any layer-shell compositor; the
  sway check is deferred and sway isn't installed. *Added 2026-10-11.*

## Done recently

Landed items, each with what closed it, kept for two weeks and then
pruned — git history has the rest.

- **The hardening sweep, committed and landed**: 84 files written
  2026-10-18 to 10-22 — symlink-safe writes and opens, archive and mesh
  budgets, escaping in every config writer, bounded subprocess groups,
  a private clipboard history, CI actions pinned by commit — as 28
  library commits (b6d11b8..3570463). displayd, lock, notif and polkit
  pushed to `main` and pinned; clipboard, files, media, procman and
  settings pushed to `next` for the release. *Added 2026-11-07.*
  *Done 2026-11-07.*
- **The greeter's look follows a hand-edited border**: Settings refreshes
  the greeter's export as it starts, as it did the lock screen's, when
  the config is the user's own `~/.config` or the export directory was
  named on purpose — so an isolated test run still cannot restyle the
  login screen (Settings c36a9ca, on `next`). Both copies catch up when
  Settings next opens, not the moment the file is edited.
  *Added 2026-11-07.* *Done 2026-11-07.*
