# Testing

`./check.sh` is the one command; this is what each of its steps asks,
and why each tier gates on the thing it actually asks.

## Checking everything works

```
./check.sh          # everything available on this machine
./check.sh --quick  # no compositor or daemons needed
```

Tier 1 runs everywhere, no system needed. Besides clippy and the unit and
integration tests, it has two steps worth knowing about by name:
**Renderer colour space** greps `cargo tree` for iced's `web-colors`
feature and fails if it's on, because that fact is invisible to every
Rust test — both renderers report the same `Color` and only the pixels
differ (see the shared-look rule in `CLAUDE.md`). **Standalone crate
dependency pins** exists because the components in the README's table are
their own repositories, which means each hand-copies every third-party
dependency's version and feature set instead of inheriting from
`[workspace.dependencies]` — a crate that is its own repository root has
nothing to inherit from. It checks nine of the ten: `hyprforge-notif`'s
versions are its own on purpose, not copies of this table (see below). If
the root bumps a version or a feature and the copy isn't updated, cargo
resolves both happily: clippy stays silent and every test passes while
the suite quietly builds two versions of the same dependency. A dropped
`features = ["derive"]` was proved invisible to the rest of tier 1
before this step existed. It discovers which crates to check by their
shape — nothing in the manifest inherits from the workspace — rather
than from a hardcoded list, so a newly prepared crate is covered
automatically. It's also why `crates/hyprforge-notif/` gets its own step: that's a
second, separate cargo workspace (`exclude`d from this one — see
Cargo.toml), so `cargo test --workspace` never reaches it, and a
component whose tests silently never run is exactly the failure this
project keeps naming.

Three tier-1 steps hold what `tools/release.sh` needs to be true before
a release can be cut. **Versions agree** runs `tools/versions.py
--check`: the suite has one version, which the libraries inherit and
the components, notif's crates and the PKGBUILD's `pkgver` spell by
hand. It fails on `pkgver`, on a library spelling a version of its own,
and on the clipboard and tray, whose versions crates.io already shows.
The other seven components and notif were never bumped before lockstep
was decided and still say 0.1.0, so they are reported in yellow rather
than failed — a step that is red until the next release is a step nobody
reads — and each one is held to the version from the first release that
tags its repository. **Changelog has a section for the version** asks
that `CHANGELOG.md` has a section for the workspace version and an
Unreleased one above it. **AUR PKGBUILD matches packaging/arch** asks
that `packaging/aur/hyprforge` is exactly what `tools/aur.py` renders
from `packaging/arch/PKGBUILD`, `.SRCINFO` included when makepkg is
installed: two PKGBUILDs kept in step by hand would drift the way the
standalone crates' dependency pins did.

Past tier 1, every further step answers a different question and gates
on the thing it actually asks, rather than sharing one `--ignored` run:

| Step | Asks | Needs |
|---|---|---|
| Live tests against Hyprland | does **Hyprland** agree? | Hyprland running |
| Parse tests against the ecosystem daemons | do **hyprpaper/hypridle** agree? | hyprpaper/hypridle installed, not running |
| Live tests against the system's archive tools | do **tar, unzip and 7z** read what this writes, and this what they write? | at least one of them installed |
| Live tests against NetworkManager | does **NetworkManager** agree? | NetworkManager running |
| Live tests against BlueZ | does **BlueZ** agree? | bluetooth.service running |
| Live tests against hyprsunset | does **hyprsunset** agree? | hyprsunset running |
| Live tests against systemd-logind | does **logind** agree? | something answering on `org.freedesktop.login1` |
| Live tests against UPower | does **UPower** agree? | upower.service running |
| Live tests against power-profiles-daemon | does **power-profiles-daemon** agree? | power-profiles-daemon.service running |
| Live tests against fprintd | does **fprintd** answer the lock screen's questions? | fprintd installed (bus-activatable) |
| Live tests against the Wayland clipboard | does the **compositor's clipboard** agree? | a Wayland session (`WAYLAND_DISPLAY` set) |
| Trash entries written by another implementation | can this crate read the **`.trashinfo` files already on disk**? | a home trash directory with something in it |
| Icon names against the installed theme | do the tray's icon names resolve in the **installed icon theme**? | an icon theme to ask (via `gsettings`) |
| The installed shared MIME database | does this machine's **shared MIME database** say what the parsers expect? | shared-mime-info installed |
| Live tests against a tray host | does a real **tray host** accept these icons? | a `StatusNotifierWatcher` running (a bar with a tray) |
| The AUR package builds in a clean chroot | does the **AUR package** build with nothing but its declared dependencies? It builds this checkout's committed HEAD (`tools/aur.py --local-into`), not the published tag | devtools installed (`extra-x86_64-build`) |

Tier 1 catches a mistake in the code. Every step below it catches the far
nastier kind: code that is internally consistent and wrong about the system
it talks to. Every claim the option catalogues make — that an option exists,
what type it is, what range it accepts — is checked against the running
compositor, the generated hyprlang files are handed to the daemons
themselves, and the NetworkManager, BlueZ, hyprsunset, logind, UPower,
power-profiles-daemon, clipboard and tray steps are each read-only checks
that the real service's interface is the shape the corresponding crate
claims. UPower and power-profiles-daemon are two separate services with
independent lifetimes, so they get two separate steps rather than one
combined "power" step — a machine with UPower masked but
power-profiles-daemon running should still get the second step's coverage,
and vice versa; power-profiles-daemon's own step never calls
`SetActiveProfile`, since changing the active profile changes how loud a
stranger's fans are out from under them. Registering with a tray host is
the one exception to read-only: it really does put an icon in the
user's bar for a fraction of a second, which is the smallest observable
form of "a host accepted it".

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

**A skipped check announces itself instead of counting as a pass.** libtest
has no skipped state — a test that returns early because a daemon isn't
installed, or NetworkManager has no Wi-Fi device to ask, prints `ok`
exactly like one that verified something. Every gated step except live
tests against Hyprland — parse, archive tools, NetworkManager, BlueZ,
hyprsunset, logind, trash, UPower, power-profiles-daemon, fprintd,
clipboard, icon names, MIME database and tray —
`eprintln!`s an `HYPRFORGE-SKIP: <reason>` line before returning early from a check it
couldn't actually run, `check.sh` runs them with `--nocapture` and greps
for the marker, and each one found is reported separately in yellow even
on an otherwise green run.
