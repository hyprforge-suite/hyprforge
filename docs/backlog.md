# Backlog

What is left to do, in one place. The `backlog` skill keeps it: an item
is added when work is deferred or an idea is parked, moved to **Done**
when it lands, and checked against the code before it is reported as
still open — an item is a claim about the repository, and claims go
stale.

Each item is one bullet: a bold title, why it matters or what it waits
on, and the date it was added. `tools/hf lint` holds the file to that
shape.

## Needs you

Things only the user can do — an account, a password, a decision, or
someone watching the screen. In the order of what each one unblocks.

- **A first look at procman, together.** The window has only ever been
  drawn offscreen (`render_every_tab`); open it with the user watching
  and go through every tab. *Added 2026-10-10.*
- **The AUR.** No AUR account or SSH key yet, so nothing from this suite
  has been pushed there. When there is one, push the latest tag's
  render only (`packaging/aur/hyprforge/`, the commands `hf release`
  prints at B4); skipped versions need no replay. Install `devtools`
  first, so the release's clean-chroot build stops skipping.
  *Added 2026-10-11.*
- **`pam-fprint-grosshack`, or not.** The only way to offer a finger and
  a password at the same moment, in the lock screen or polkit prompt.
  It is a third-party AUR package, so installing it is the user's call.
  *Added 2026-10-10.*
- **Files' last two screenshots.** `files-devices` and `files-dialog` are
  still placeholders in `site/pages/files.html`; the live screenshot rig
  needs the user present (`live-screenshots` skill). *Added 2026-10-10.*
- **Does cancelling a fingerprint cost a faillock slot?** Read from the
  source, probably yes, about ten seconds later. Measure only with the
  user's OK, and never with a wrong password. Optional.
  *Added 2026-10-10.*
- **Files' Open as administrator, live.** The pkexec helper has passed
  its tests but never a real round trip with the user's password.
  *Added 2026-10-06.*
- **Phones and cameras with a real device.** Files' gvfs code was built
  from real published `gio` output and has never seen a phone plugged
  in. *Added 2026-10-07.*

## Engineering

Open work Claude can do unattended. Nothing here blocks a release.

- **Procman's journal leaves three things unsaid.** Whether delay
  accounting was on (block-I/O delay reads 0, not unknown, when it was
  off), which frames dropped exits (ENOBUFS), and that `run_delay` and
  `open_files` fall back to 0 and show as measured. All three need a
  journal format change. *Added 2026-10-10.*
- **`hf hyprland up --for-lock`.** The tooling plan had the disposable
  instance replace `crates/hyprforge-lock/testing/nested.sh`; the lock
  screen still uses the script. *Added 2026-10-11.*
- **Does `hyprforge-mime` need a package of its own?** The installer's
  `--mime` installs `hyprforge-mimetype` and `hyprforge-mimeopen`; the
  PKGBUILD has no package for them. Decide, then make the two agree.
  *Added 2026-10-11.*
- **Reason strings on `#[ignore]`.** Skipped when the tiers arrived,
  because `.config/nextest.toml` is what enforces a test's tier; a
  reason would still say why at the test. *Added 2026-10-11.*
- **Time `hf check` against what it replaced.** `check.sh` was never
  timed, so "lighter" is measured on one side only (a warm
  `hf check --fast` is 102 s). *Added 2026-10-11.*
- **Media's model view, live.** Drag to rotate and pan has never been
  exercised with a real pointer, and view3d's `--screenshot` was not
  carried over. *Added 2026-09-26.*

## Ideas

Parked on purpose — written down so they are not lost, not scheduled.

- **The GPU broker, `hyprforge-gpud`.** Launch routing by policy, the
  discrete GPU kept asleep, and per-app GPU choice moved here from
  procman. `docs/design/vision.md` has it; moving an arbitrary running
  app between GPUs is impossible on any OS. *Added 2026-10-10.*
- **Drops from other apps into Files.** Hyprland 0.56 sends a drag only
  to the first data device a client made, which is iced's clipboard's.
  A fork of that was declined; this waits on Hyprland or iced upstream.
  *Added 2026-10-03.*
- **Procman: io_uring waits are not I/O trouble.** Ghostty idles in
  `io_cqring_wait`, which the kernel counts as iowait, so raw I/O
  pressure reads 80–90% with idle disks. Don't call that "struggling".
  *Added 2026-10-10.*

## Done recently

Kept for a few weeks so "what happened to X" has an answer, then
pruned; git history has the rest.

- **0.1.14 released**, the first two-phase release, with `tools/hf`,
  procman as the twelfth component, and the exits fix for
  single-threaded processes. *Added 2026-10-11.*
- **The disposable and root tiers passed** on this machine (33/33; 3/3
  and the recorder's self-test inside its sandbox). *Added 2026-10-11.*
- **Files' Retry as administrator** and **model thumbnails** in Files
  and Media — both built, both once listed here as open.
  *Added 2026-10-11.*
