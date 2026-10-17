# Backlog

What is left to do, in one place. The `backlog` skill keeps it: an item
is added when work is deferred or an idea is parked, moved to **Done**
when it lands, and checked against the code before it is reported as
still open — an item is a claim about the repository, and claims go
stale.

Each item is one bullet: a bold title, why it matters or what it waits
on, and the date it was added. `tools/hf lint` holds the file to that
shape. Within a section, items are ranked: the first is the one most
worth doing next.

## Needs you

Things only the user can do — an account, a password, a decision, or
someone watching the screen. Ranked by what each one unblocks.

- **The NVIDIA card is held awake by its power setting.**
  `/sys/bus/pci/devices/0000:01:00.0/power/control` is `on`, which
  disables runtime power management outright, though the driver runs
  fine-grained D3 (`DynamicPowerManagement: 2`) and could sleep the card
  when idle. On battery that is a few watts all the time. Setting it to
  `auto` (usually a udev rule) is a system change and the user's call.
  *Added 2026-10-17.*
- **Procman: containers, once Podman or Docker is installed.** Neither
  is on this machine, so a container's cgroup and its name can't be
  checked against the real thing, and a fixture written from memory
  tests the memory. Installing one is the user's call.
  *Added 2026-10-17.*
- **Procman: bandwidth per connection, with root.** Details lists each
  process's connections; their bytes need the eBPF counter to key on
  the socket as well as the process, and loading it needs root — the
  root tier, in the user's terminal. *Added 2026-10-17.*
- **Procman: a right click opens no menu, so start the nested
  Hyprland.** The user reports nothing at all on a right click, on any
  row. The menus are wired (`src/menus.rs`) and a real right press sent
  through the drawn view by `iced_test` opens one (procman cf584b5), so
  the cause is outside what headless iced reaches. Next: `tools/hf
  hyprland up` in the user's terminal, then the installed procman in it
  and a right click from a virtual pointer (`zwlr_virtual_pointer_v1`,
  buttons 0x110/0x111; the nested-pointer-drag-rig memory), after
  checking the rig gets a menu out of Files. *Added 2026-10-17.*
- **An encrypted stick, once, for real.** Unlocking was built against
  UDisks2's interface and a mock, never a real LUKS device. A stick (or
  a LUKS image attached with `udisksctl loop-setup`) clicked in Files:
  the prompt, a wrong passphrase, the right one, then Eject — after
  which `lsblk` should show no `crypt` mapping left. *Added 2026-10-12.*
- **A first look at procman, together, and its screenshots.** The
  window has only ever been drawn offscreen (`render_every_tab`); open
  it with the user watching and go through every tab. Its site page
  (`site/pages/procman.html`) shows three placeholders, one as the hero
  — `procman`, `procman-recorded`, `procman-top` — and the README has no
  procman image. The same session can take them (`live-screenshots`
  skill). *Added 2026-10-10.*
- **Sound: which audio work comes first.** The suite has no volume,
  output or per-app mixer anywhere: Settings' Sound page is deferred
  (`crates/hyprforge-settings/DESIGN.md`, "needs a backend") and the
  vision's PipeWire module is not started. The biggest visible gap;
  scope (a Settings page, a tray item, both) is the user's call.
  *Added 2026-10-11.*
- **Previous Versions against real snapshots.** Files' sheet has only
  been tested against snapper's documented `info.xml`, never this
  machine's. Apply Set up → "Previous versions" (pkexec, the user's
  password), then look together (`crates/hyprforge-files/DESIGN.md`
  around line 1122). *Added 2026-10-11.*
- **The greeter: more than one user, and a session choice.** It logs in
  one account named in `hyprland-greeter.lua` (`CHANGE_ME`) and always
  starts Hyprland (`crates/hyprforge-greet/src/main.rs:30`). A user
  picker needs a design decision: how it meets the export directory and
  avatars. *Added 2026-10-11.*
- **The greeter: a power menu, or deliberately none.** There is no way
  to shut down or reboot from the login screen
  (`crates/hyprforge-greet/src/main.rs:85`). authui already has the
  lock's logind-checked menu; offering power actions before login is a
  security trade-off for the user to accept. *Added 2026-10-11.*
- **The lock screen: input methods.** The greeter has an IME (fcitx5,
  sandboxed); the lock has none (`docs/lock-and-greeter.md` known gaps),
  so a password that needs an IME can log in but not unlock. Worth doing
  is the user's call; testing is by hand on a nested compositor.
  *Added 2026-10-11.*
- **Media's Edit button.** A disabled ✎ in the toolbar
  (`crates/hyprforge-media/src/view.rs:116`); crop, straighten, exposure
  and saving a rotation are unbuilt, because `docs/design/media-plan.md`
  phase 3 leaves writing back to someone's original as a decision
  nobody has made. Decide, or remove the dead control.
  *Added 2026-10-11.*
- **OK to delete two leftovers?** A stale agent worktree
  (`.claude/worktrees/agent-ab5de03282eb204ef`, 2.8 GB, fully merged,
  clean) and the suite repository's `files-phones` branch on GitHub
  (fully merged). Nothing would be lost; deleting is the user's say.
  *Added 2026-10-11.*
- **The AUR.** No account yet: on 2026-10-16 the AUR was not accepting
  signups, so this waits on them. When there is one, push the latest
  tag's render only (`packaging/aur/hyprforge/`, the commands
  `hf release` prints at B4); skipped versions need no replay.
  `devtools` is installed now, so the next release's clean-chroot build
  runs instead of skipping. *Added 2026-10-11.*
- **Files' last two screenshots.** `files-devices` and `files-dialog` are
  still placeholders in `site/pages/files.html`; the live screenshot rig
  needs the user present. *Added 2026-10-10.*
- **Files' Open as administrator, live.** The pkexec helper has passed
  its tests but never a real round trip with the user's password.
  *Added 2026-10-06.*
- **Phones and cameras with a real device.** Files' gvfs code was built
  from real published `gio` output and has never seen a phone plugged
  in. *Added 2026-10-07.*
- **Network: enterprise Wi-Fi, VPN, hotspot.** 802.1X networks say
  "Hyprforge cannot join it yet — use nmcli"
  (`crates/hyprforge-network/src/types.rs:146`); no VPN or hotspot code.
  Large, and testing needs a real enterprise network or VPN.
  *Added 2026-10-11.*
- **`pam-fprint-grosshack`, or not.** The only way to offer a finger and
  a password at the same moment. A third-party AUR package, so
  installing it is the user's call. *Added 2026-10-10.*
- **Does cancelling a fingerprint cost a faillock slot?** Read from the
  source, probably yes, about ten seconds later. Measure only with the
  user's OK, and never with a wrong password. Optional.
  *Added 2026-10-10.*

## Engineering

Open work Claude can do unattended. Nothing here blocks a release.

- **Files segfaults, 32 times in 30 days.** Procman's new Crashes tab
  counted them: all SIGSEGV — 16 in the main thread, 14 in `files-dnd`
  (`src/dnd.rs`) and 2 in `files-drag-out`, spread from 2026-09-26 to
  10-16. The latest (pid 16869) died in `wl_proxy_destroy` two minutes
  after a reinstall, so at least one is a proxy outliving its
  connection. `coredumpctl list hyprforge-files` and `coredumpctl info
  <pid>` have the stacks. *Added 2026-10-17.*
- **The tray daemon wakes in a burst every ten seconds.** About 550
  wakeups each time, ~60 a second on average with nothing changing
  (measured from `/proc/<pid>/task/*/schedstat`, 2026-10-17, while the
  screen was locked) — most likely a periodic poll in
  `hyprforge-trayd`. Find which item polls and whether a signal can
  replace it. *Added 2026-10-17.*
- **Procman's run delay counts only each process's first thread.**
  `/proc/<pid>/schedstat` is the leader thread's alone: Hyprland's said
  84.5 s where its threads had waited 176.4. The Advanced view, the
  Recorded tab and `hyprforge-top` all show it. Summing `task/*` fixes
  it, but changes what recorded days mean, so a day spanning the
  upgrade needs a marker (or a format bump) so no interval compares
  one against the other. *Added 2026-10-17.*
- **Procman: limits per app.** Cap an app's memory, CPU and I/O through
  systemd's cgroup properties, on a running app or at launch.
  Approved 2026-10-17. *Added 2026-10-17.*
- **Procman: efficiency mode.** Background windows get less CPU while
  another has focus, from Hyprland's focus events. Approved 2026-10-17.
  *Added 2026-10-17.*
- **Procman: rules.** Remembered per-program actions applied as a
  process starts (a priority, a limit, a warning past a threshold),
  ananicy's and Process Lasso's job. Approved 2026-10-17.
  *Added 2026-10-17.*
- **Procman: see deeper, the rest.** Hyprland touches — click a window
  to inspect it, end the focused window's tree, a tray item for
  pressure — want the user at the screen. Memory, Flatpaks and
  connections are done (Details already listed sockets). Approved
  2026-10-17. *Added 2026-10-17.*
- **A Keyboard screen for the suite's own bindings.** Media's keys can
  only be changed by editing `media-config.toml`; `docs/design/vision.md`
  plans a Settings screen on the `hyprforge-keys` grammar, and Files'
  `config_edit` is there to reuse. Medium. *Added 2026-10-11.*
- **`hf hyprland up --for-lock`.** The tooling plan had the disposable
  instance replace `crates/hyprforge-lock/testing/nested.sh`; the lock
  screen still uses the script. *Added 2026-10-11.*
- **Media's model view, live.** Drag to rotate and pan has never been
  exercised with a real pointer, and view3d's `--screenshot` was not
  carried over. *Added 2026-09-26.*
- **Time `hf check` against what it replaced.** `check.sh` was never
  timed, so "lighter" is measured on one side only (a warm
  `hf check --fast` is 102 s). *Added 2026-10-11.*
- **Pixel tests for the video and 3D panes.** Media's still picture is
  now drawn and read back in tests, but the video and model panes are
  `shader` widgets, which `iced_tiny_skia` doesn't draw, and CI has no
  GPU for `iced_test`'s wgpu backend. Needs either a software wgpu
  adapter in CI or a test-only CPU path. Medium. *Added 2026-10-12.*
- **A screenshot tool, like Flameshot.** A new component: drag out a
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

Kept for a few weeks so "what happened to X" has an answer, then
pruned; git history has the rest.

- **`hf check` builds the pins**: "The pins build together" builds
  every component at its pinned commit whenever one is checked out
  elsewhere, as CI does — 63 s cold, 3 s warm. Recreating 738b5ab's
  break made it fail naming the unmatched variant. *Added 2026-10-17.*
- **Procman sees deeper**: Details splits a process's memory into its
  own, shared and swapped (`smaps_rollup`), and a Flatpak app is one
  group with its whole sandbox. In procman's repository; local until
  landed. *Added 2026-10-17.*
- **Procman says what is using power**: on the Battery page, estimated
  watts on battery and the share of activity plugged in, with every
  thread's wakeups, averaged over ten seconds. In procman's repository;
  local until landed. *Added 2026-10-17.*
- **Procman's Crashes tab**: crashes from systemd-coredump with their
  stack traces, and kills for memory from the kernel, systemd and
  systemd-oomd, with what each was using. In procman's repository;
  local until landed. *Added 2026-10-17.*
- **0.1.15 released**: Files' redo and encrypted drives, GIFs and sharp
  zoom in Media, the pair editor and layer rules in Settings, the
  `hyprforge-mime` package; every `next` merged, and `main`'s CI green
  again for the first time since `738b5ab`. The AUR push still waits on
  an account. *Added 2026-10-16.*
- **Redo, animated GIFs and sharp zoom, seen on screen**: the user
  tried them before 0.1.15 and they work. *Added 2026-10-16.*
- **`hyprforge-mime` is a package**: the user's call. It installs
  `hyprforge-mimetype` and `hyprforge-mimeopen`, needs
  `shared-mime-info`, and joins the metapackage; it doesn't take the
  plain `mimetype`/`mimeopen` names, which stay the installer's opt-in
  `--provide-mimetype`. *Added 2026-10-16.*
- **The pair settings load in a real compositor**: `tools/hf test
  disposable` (33/33) on 2026-10-16 evaluated a generated file holding
  every catalogued `vec2`, the shadow offset and the tablet's included,
  and both range extremes, without an error. *Added 2026-10-16.*
- **Every `#[ignore]` says why**: 100 bare ones got their tier and,
  for live and visible tests, their gate — read from nextest and
  `tools/hflib/tiers.py`, not written by hand — and `hf lint` now
  refuses a bare one. *Added 2026-10-11.*
- **Media's model autoreload has a switch**: Live in the model's
  toolbar beside Axes, and Ctrl+Shift+R, saved like the other model
  settings. *Added 2026-10-11.*
- **The tray's popup opens submenus**: a row with children shows `›`
  and opens them in place under a Back row (Right and Left from the
  keyboard); the popup is sized for its largest level. Before, the tree
  was flattened into one list. *Added 2026-10-11.*
- **Media's picture is pixel-tested**: three `iced_test` tests draw
  the canvas headless on tiny-skia and check each quadrant's place and
  exact colour (upright and turned) and the theme's backdrop; `App::blank`
  is the one constructor `main` and the tests share. *Added 2026-10-11.*
- **Files' split divider drags**, through a new shared
  `split_handle` in `hyprforge-ui`; double click for even halves, and
  the session keeps the ratio. *Added 2026-10-11.*
- **The keep-or-revert prompt names the change**: displayd's new
  `PendingChanges` describes each output that moved (mode, scale,
  rotation, position, on/off), and Settings' prompt lists up to three.
  There was never a backup file to show — the snapshot is in memory.
  *Added 2026-10-11.*
- **Settings edits pairs of numbers**: a `vec2` kind in
  `hyprforge-core` (two fields, applied together, written as `{ x, y }`
  as Hyprland 0.56.2's parser reads it), which brings in the shadow
  offset and the whole drawing-tablet section. *Added 2026-10-11.*
- **Procman's journal says what it couldn't see** (format 3): whether
  delay accounting was on, whether exits were dropped or unheard, and
  unknown rather than 0 for run delay and open files; format 2 days
  still read. *Added 2026-10-10.*
- **Encrypted drives open in Files**: a click (or Mount) on a locked
  LUKS or BitLocker drive asks for its passphrase, unlocks it through
  UDisks2 and opens it; a wrong passphrase asks again; unmounting or
  ejecting locks it again. An unlocked stick still shows as removable,
  with its eject. The open/save dialog says to unlock it in Files.
  *Added 2026-10-11.*
- **Zooming in on a photo is sharp**: once a zoom settles, Media
  decodes the visible part (plus a margin) at the density it is drawn
  at, through the new `hyprforge_image::decode_region`, and lays it
  over the enlarged picture; what is kept stays inside the same budget.
  *Added 2026-10-11.*
- **Layer rules are editable** in Settings' Window rules page: a row
  each for the namespace, blur and on/off, add and remove (Settings
  b69919c). *Added 2026-10-11.*
- **Animated GIFs play** in Media and Files' Quick Look:
  `hyprforge_image::decode_animation` (every frame inside one still's
  budget) and `hyprforge_viewer::animation` on the film. Media and
  Files carry it on their `next` branches until the release publishes
  both libraries. Not yet watched on screen. *Added 2026-10-11.*
- **Files has redo**: Ctrl+Shift+Z and Ctrl+Y, for everything undo
  covers; a copy, extraction or archive comes back out of the Trash
  from where its undo put it. On Files' `next` until the next release
  publishes `hyprforge-files-core`'s `Undone`. *Added 2026-10-11.*
- **The lock no longer borrows hyprlock's PAM file**, whose missing
  account stack refused every correct password; it falls back to
  `login` (`hyprforge-lock` b4dcd4d). *Added 2026-10-11.*
- **The stale-docs sweep**: repo-plan, displays, media-plan, Files'
  DESIGN, greet's README, notif's PLAN and example, Settings'
  Bluetooth comments and allows. `tools/changelog.py` no longer writes
  "sort before releasing" into a section it is releasing; the old notes
  are HTML comments and were left. *Added 2026-10-11.*
- **The whole-suite survey** of 2026-10-11: five agents read every
  crate, doc and submodule for unfinished work; their findings are the
  items above dated that day. GitHub had no open issues or pull
  requests, and every repository's CI was green. *Added 2026-10-11.*
- **0.1.14 released**, the first two-phase release, with `tools/hf`,
  procman as the twelfth component, and the exits fix for
  single-threaded processes. *Added 2026-10-11.*
- **The disposable and root tiers passed** on this machine (33/33; 3/3
  and the recorder's self-test inside its sandbox). *Added 2026-10-11.*
- **Files' Retry as administrator** and **model thumbnails** in Files
  and Media — both built, both once listed here as open.
  *Added 2026-10-11.*
