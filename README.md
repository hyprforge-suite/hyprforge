# Hyprforge (v1 slice)

A Displays daemon, the Window Rules/Shortcuts/Input/Appearance config
libraries, a Settings GUI, a clipboard history, an emoji picker, a
StatusNotifierItem tray, Network/Bluetooth/power clients, a file manager,
a photo, video and 3D model viewer, and a shared lock screen and greeter
— all for Hyprland. See `hyprforge-vision.md` for
the whole-suite context this session's work fits into; this README covers
only what's in this workspace so far.

## Workspace layout

Three groups, and the order matters: anything lower may be used by an app that
has never heard of Hyprland.

```
Shared by every app in the suite
  hyprforge-paths/         where config lives and how to write it atomically,
                            and where your Pictures and Downloads are
                            (user-dirs.dirs). No dependencies at all.
  hyprforge-look/          the Color type with the one rgba() parser, and the
                            runtime Theme every app draws from. No iced —
                            config machinery and the theme publisher use it
                            without ever opening a window.
  hyprforge-ui/            the iced layer: spacing scale, palette, widgets.
                            Knows nothing about Hyprland.
  hyprforge-process/       a subprocess wait with a timeout, for talking to
                            any program that might not answer (hyprctl,
                            gsettings, fc-list, ...). No dependencies at all;
                            used by clients of other daemons (NetworkManager,
                            BlueZ) that have no Hyprland config to read or
                            write. hyprforge-core re-exports it as
                            hyprforge_core::command for compatibility.
  hyprforge-secret/        Secret<T>, whose Debug renders a length and
                            nothing else, so every place a password or
                            passphrase leaves the wrapper is one greppable
                            expose() call. No dependencies at all.
  hyprforge-popup/         the shared layer-shell popup shell: surface,
                            event loop, pointer/keyboard handling,
                            placement, the single-instance lock, the
                            pixel-offset scrollbar, and the Dismissal
                            choice — whether a click elsewhere closes the
                            popup or is swallowed by it, which is the same
                            question as how it holds the keyboard. Extracted from
                            hyprforge-clipmenu once a second and third
                            popup (hyprforge-emojimenu, hyprforge-traymenu)
                            needed the same machinery. A leaf: depends
                            only on hyprforge-look and hyprforge-process,
                            no async runtime, no D-Bus, no hyprforge-core.
  hyprforge-emoji/         CLDR-ordered emoji data (generated from
                            Unicode's emoji-test.txt) and a pure search
                            function. No dependencies at all; a picker UI
                            is built on this, not the other way round.
  hyprforge-keys/          the suite's keyboard grammar: one parser for
                            "Ctrl+Shift+N", one merge policy for a user's
                            bindings over the defaults, generic over each
                            app's own action type. No iced — the toolkit
                            adapter is hyprforge-ui::keys — so the file
                            manager and the image viewer read a key the
                            same way because it is the same code.
  hyprforge-listing/       what a directory listing is, how it is read,
                            and what order it is in, with the FsBackend
                            seam and its mock. The leaf under both the file
                            manager and the image viewer, so the two cannot
                            disagree about which picture comes next.
  hyprforge-image/         measuring, orienting and decoding a picture
                            within a budget: the header is read before
                            anything is allocated, the EXIF orientation is
                            applied once, and a 36-megapixel photograph is
                            decoded to fit the window rather than at full
                            size; and what the camera wrote — body, lens,
                            exposure, date, GPS. No iced, no Wayland, no
                            Hyprland.
  hyprforge-video/         videos: playing one into memory through libmpv
                            (opened at run time, so nothing needs mpv to
                            start), and a video's first real frame — past
                            the black most open on — for the thumbnail
                            cache, through ffmpeg. No iced, no async
                            runtime; the only unsafe code in the suite's
                            viewer is in its libmpv binding.
  hyprforge-mesh/          3D models — STL, 3MF and OBJ — loaded into a
                            welded, indexed mesh, with fstl's camera and
                            draw modes: the model half of view3d, brought in
                            so the image viewer opens a model the way it
                            opens a picture. Memory-mapped, parallel
                            parsing. No iced, no wgpu; carries its own MIT
                            LICENSE with fstl's copyright line.
  hyprforge-viewer/        a playing video and a turnable 3D model as panes
                            of an iced window — the player, its frame
                            texture, the GPU renderer and the hands on
                            them — so Media's viewer and Files' Quick Look
                            show both the same way.

Hyprland-facing
  hyprforge-core/          the config machinery: hlconfig (the generic
                            hl.config overlay: catalog, storage, codegen,
                            import), hyprlang, Lua codegen, the one-time
                            hyprland.lua setup, monitor geometry, and the
                            D-Bus proxy for displayd
  hyprforge-displayd/      daemon + CLI (hyprforge-displayd, hyprforge-displayctl)
  hyprforge-windowrules/   TOML rule storage, Lua codegen — window,
                            workspace and layer rules
  hyprforge-shortcuts/     TOML shortcut storage, Lua codegen, live conflict
                            detection against hyprctl binds
  hyprforge-input/         the input option catalog (51 options)
  hyprforge-appearance/    the appearance catalog (155 options), the animation
                            model, the gsettings bridge, theme/font discovery,
                            and `look::resolve()` — which turns all of that
                            into the shared Theme
  hyprforge-ecosystem/     hyprpaper / hyprsunset / hypridle config
  hyprforge-session/       autostart programs and environment variables
  hyprforge-system/        the misc/debug/render catalog
  hyprforge-setup/         the steps that finish an install — services,
                            keybinds, the idle lock, notification blur,
                            default apps, the open/save dialog — each
                            checked, applied through the module that owns
                            its file, and recorded in setup.toml so it can
                            be undone. Behind a System trait with a mock,
                            so no test enables a real service. The
                            Settings app's --setup and Set up page.
  hyprforge-lua-import/    sandboxed mlua evaluator that imports hand-written
                            hl.bind()/hl.window_rule()/hl.monitor() calls —
                            the only crate depending on mlua
  hyprforge-network/       Wi-Fi, wired status and the radio toggle, over
                            NetworkManager. The first module whose backend
                            trait came before its D-Bus client, so the
                            screen is testable without an adapter.
  hyprforge-bluetooth/     adapters, devices and pairing over BlueZ, on the
                            same backend-trait-first shape.
  hyprforge-power/         keeping the machine awake, battery state and the
                            active power profile, over systemd-logind,
                            UPower and power-profiles-daemon — three
                            independent daemons, each behind its own
                            backend trait, because one can be down while
                            the others answer fine.
  hyprforge-clipboard/     a Wayland clipboard history over
                            wlr-data-control/ext-data-control: what was
                            copied and what may be kept. Sensitivity is
                            checked before an offer's bytes are ever read,
                            so a password manager's clipboard contents are
                            never hashed, stored or logged. Also ships
                            hyprforge-clipmenu, the history popup — search,
                            filter tabs, a preview pane, pin and delete —
                            launched
                            per invocation by a keybind rather than run as
                            a daemon — a short-lived process cannot leak a
                            stuck layer surface holding exclusive keyboard
                            focus. A PopupApp consumer of hyprforge-popup.
  hyprforge-fileops/       trash, copy, move and rename. The freedesktop
                            trash spec including the part that bites: a
                            file cannot be renamed across filesystems, so
                            a file on another one needs a trash directory
                            of its own — which is not an external-drive
                            edge case, since btrfs subvolumes report
                            different device numbers. No Hyprland, no
                            async runtime: syscalls and arithmetic.
  hyprforge-mime/          what kind of file this is, what opens it, and
                            which one does by default — the freedesktop
                            shared MIME database, read rather than
                            guessed at. Ships hyprforge-mimetype and
                            hyprforge-mimeopen, drop-ins for the commands
                            of the same name in perl-file-mimeinfo: with
                            neither installed, xdg-open falls back to
                            content sniffing, which calls an STL a stream
                            of bytes and opens 3D models in a browser.
                            A leaf, like hyprforge-look.
  hyprforge-icons/         which file is the icon called image-png: the
                            freedesktop icon theme lookup, through the
                            configured theme's Inherits= chain to hicolor.
                            hyprforge-mime says which name a type's icon
                            has; this finds the file. A leaf.
  hyprforge-thumbnails/    the freedesktop thumbnail cache every program
                            shares: a thumbnail found while its file is
                            unchanged, stored as small as a PNG can be,
                            failures remembered, orphans pruned. A leaf.
  hyprforge-archive/       zip, tar and 7z: what is inside one as a
                            directory tree, extracting from it, making
                            one, and rewriting it. Pure Rust, so a
                            machine with no p7zip still opens a .7z.
                            A leaf: its one Hyprforge dependency is
                            hyprforge-secret (for archive passwords),
                            which has none of its own. It is handed
                            paths and never goes looking for one, which is what lets the file
                            manager, the open/save dialog or a preview
                            pane all ask the same questions without any
                            of them pulling in a GUI toolkit.
  hyprforge-volumes/       drives over UDisks2 and network shares over
                            gvfs: what is plugged in, mounting,
                            unmounting and ejecting it without root, and
                            connecting to an smb:// or sftp:// address
                            through `gio`, answering its password prompt
                            without the password ever reaching a log.
                            Backend traits with mocks; the data and
                            decisions build without the D-Bus client,
                            which is all the browser view takes.
  hyprforge-files-core/    the file browser's model and the browsing view
                            itself (the FsBackend seam it re-exports lives
                            in hyprforge-listing now). The view lives
                            here rather than in the app because the
                            portal's open/save dialog has to render the
                            same one — enforced by the type, not by
                            convention: view() takes a private ViewModel
                            with no mode field, so it cannot branch on
                            which host it is in.
  hyprforge-tray/          the StatusNotifierItem protocol, and
                            hyprforge-trayd, which puts network (Wi-Fi and
                            Ethernet), Bluetooth, keep-awake, night-light,
                            battery/power-profile and display-layout icons
                            in whatever bar is running. A tray icon is a
                            D-Bus object, not a widget — which is why it
                            needs no GTK or Qt. The right-click menu is
                            hyprforge-traymenu, a sibling popup this daemon
                            spawns directly; the item advertises no `Menu`
                            property at all, rather than serving
                            com.canonical.dbusmenu for a bar to draw
                            itself. The cost is real and stated plainly in
                            the crate's own module doc: before this, any
                            spec-compliant tray host could show these
                            menus, styled however that host chose; now
                            only hyprforge-traymenu can, and a bar with no
                            Hyprforge installed sees an icon with no menu
                            at all, its right-click falling back to the
                            icon's primary action.
                            hyprforge-traymenu, a second binary in the same
                            crate, is the tray's own right-click menu,
                            themed like every other Hyprforge popup and
                            anchored below the icon that was clicked — a
                            PopupApp consumer, like hyprforge-clipmenu.
  hyprforge-emojimenu/     an emoji picker popup that appears where the
                            mouse is, filtered by what's typed, built the
                            same way as hyprforge-clipmenu — a PopupApp
                            consumer of hyprforge-popup and
                            hyprforge-emoji, with long-press support for
                            picking a skin tone, frequently used emoji
                            first, and tabs for kaomoji and symbols.
  hyprforge-notif/         the notification daemon, notifd, and notifctl
                            to drive it: toasts on layer-shell surfaces, a
                            notification center, history and
                            do-not-disturb. A cargo workspace of its own,
                            excluded from this one, because it runs zbus
                            on async-io and the suite runs it on tokio,
                            and one workspace would unify the two into a
                            runtime panic. Its two suite dependencies
                            are patched to this checkout by
                            crates/.cargo/config.toml, since this
                            workspace's [patch] cannot reach it.

Apps
  hyprforge-files/         the file manager window: the chrome around the
                            shared browsing view, plus what a dialog
                            deliberately does not have — launching what
                            you double-click, file operations, and drag
                            and drop with other applications. And a
                            second binary, hyprforge-files-portal: the
                            desktop's open/save dialog, served to
                            xdg-desktop-portal and opted into, never
                            switched on by installing it. And a third,
                            hyprforge-files-admin: the small helper
                            pkexec runs for "Open as administrator", so
                            the window itself never runs as root.
  hyprforge-media/        the photo, video and 3D model viewer, in the
                            file manager's shell: a Places sidebar, and
                            Photo, Grid and Library as modes of one window.
                            A picture opened on its own gets a compact,
                            floating viewer instead. Photo pages through
                            the folder in the order Files shows it, with
                            zoom, pan, rotation and a filmstrip; Grid groups
                            it by day; Library is the folder tree as cards.
                            Videos play in the pane through libmpv, with a
                            seek bar; 3D models (STL, 3MF, OBJ) are drawn by
                            view3d's renderer in an iced shader widget. An
                            inspector with the camera's EXIF, a video's
                            length or a model's geometry, a slideshow, and
                            trash with undo, copy, Open With, Show in Files
                            and Set as Wallpaper.
  hyprforge-settings/      the iced GUI, hosting the settings modules
  hyprforge-authui/        the authentication conversation model, shared by
                            the lock screen and the greeter
  hyprforge-lock/          the ext-session-lock-v1 lock screen
  hyprforge-greet/         the greetd greeter, on the same hyprforge-authui
                            conversation model
```

Ten of these directories — clipboard, lock, greet, tray, settings,
displayd, emojimenu, files, media and notif — are also their own repositories, published
separately from this one. See "Eleven repositories, one workspace" below for what that means and where a change to
one of them should actually be made.

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

## Eleven repositories, one workspace

The suite lives in the [hyprforge-suite](https://github.com/hyprforge-suite)
GitHub organisation (`hyprforge` itself was already taken by an unrelated
project), and every repository in it is public. Ten components have
repositories of their own, each with green CI, and each appears here as a
git submodule at `crates/<component>`; this repository is where the
libraries they share live, and the only place that says how the eleven fit
together.

| Repository | What it is |
|---|---|
| [hyprforge-clipboard](https://github.com/hyprforge-suite/hyprforge-clipboard) | A Wayland clipboard history library over `wlr-data-control`/`ext-data-control`, plus `hyprforge-clipd`, the daemon that watches the compositor's clipboard and writes its history, and `hyprforge-clipmenu`, the popup that shows it and pastes what you pick. |
| [hyprforge-lock](https://github.com/hyprforge-suite/hyprforge-lock) | An `ext-session-lock-v1` lock screen for Hyprland: PAM and fingerprint unlock, a status line, media and notification counts, a power menu, and a look shared with the greeter. |
| [hyprforge-greet](https://github.com/hyprforge-suite/hyprforge-greet) | A greetd greeter for Hyprland, sharing its look and authentication conversation with the lock screen. |
| [hyprforge-tray](https://github.com/hyprforge-suite/hyprforge-tray) | A StatusNotifierItem tray library, plus `hyprforge-trayd`, the daemon that puts network (Wi-Fi and Ethernet), Bluetooth, keep-awake, night-light, battery/power-profile and display-layout icons in whatever bar is running, and draws its own right-click menu through `hyprforge-traymenu` rather than `com.canonical.dbusmenu`. |
| [hyprforge-settings](https://github.com/hyprforge-suite/hyprforge-settings) | The Settings app: an iced GUI over Hyprland's config, appearance, displays, network, Bluetooth, shortcuts and more. |
| [hyprforge-displayd](https://github.com/hyprforge-suite/hyprforge-displayd) | A monitor-arrangement daemon: it watches `wlr-output-management`, recognises a set of displays it has seen before and applies the layout saved for it, plus `hyprforge-displayctl` to drive it from a script. |
| [hyprforge-emojimenu](https://github.com/hyprforge-suite/hyprforge-emojimenu) | An emoji picker: a layer-shell popup at the pointer with type-to-filter search over the full Unicode set, frequently used first, skin tones and a remembered default tone, plus kaomoji and symbols. |
| [hyprforge-files](https://github.com/hyprforge-suite/hyprforge-files) | A file manager: tabs, a sidebar, list, grid and column views with thumbnails, a preview pane, the freedesktop trash, copy, paste and drag with other applications, and zip/tar/7z archives browsed and edited in place. |
| [hyprforge-media](https://github.com/hyprforge-suite/hyprforge-media) | A photo, video and 3D model viewer: a folder browsed in the file manager's order, a date-grouped grid and library, EXIF and orientation done right, videos through libmpv, STL/3MF/OBJ models on the GPU, and a slideshow. |
| [hyprforge-notif](https://github.com/hyprforge-suite/hyprforge-notif) | A notification daemon, `notifd`, replacing dunst or mako: the full `org.freedesktop.Notifications` server, layer-shell toasts, a notification center with history, do-not-disturb, and `notifctl` to drive them. A cargo workspace of its own, on smol rather than tokio. |

Nine of the ten are meant to be installed on their own: clone
`hyprforge-clipboard` and you get a clipboard daemon and nothing else — no
Settings app, no tray, no Hyprland config machinery. `hyprforge-settings` is
the exception and its own README says so: it depends on seventeen other
Hyprforge crates, which makes it the hub rather than a small standalone thing.
"Every component runs alone, and is better together" in `CLAUDE.md` is the
rule this is built around; `repo-plan.md` opens with the owner's own framing
of why repository identity and installable-alone are worth keeping as separate
questions, rather than restated here.

Each of these repositories takes its Hyprforge dependencies as **versions
from crates.io** — `hyprforge-look = "0.1"` — where every library crate in
this workspace is published. So a clone of one component fetches the
published libraries it uses and nothing else; it never needs this
repository. Inside this workspace the root `[patch.crates-io]` table points
every one of those names back at `crates/`, so a change to a library
reaches every app built here at once. The libraries share one version and
are published together; see `repo-plan.md`.

### Where development happens

Each component's own repository is where its code lives. This repository
**references** each one as a git submodule at `crates/<component>`, pinned to
a commit, so clone it with its components:

```
git clone --recurse-submodules https://github.com/hyprforge-suite/hyprforge
# or, in a clone made without them:
git submodule update --init
```

`./check.sh` says so before anything else when one is missing, rather than
letting cargo fail to load the workspace.

A pull request against a component repository — `hyprforge-lock`, say — is a
pull request against the code; there is no second copy for it to be carried
back into. A change here to a component is two commits: one inside
`crates/<component>`, pushed to that component's repository, and one here
moving the pin. `./check.sh` refuses a pin that names a commit its repository
does not have on `main` (nobody else could clone it), and builds every
component on its own against the libraries on crates.io, the way the
component's CI is about to — so a component that needs a library change not
yet released is caught here rather than on GitHub. A change to a library and
a component together is the library here, a release, then the component and
the pin; `repo-plan.md` has the order.

Until October 2026 the components were copies instead, written here and
pushed out with `git subtree split` by `split.sh` and kept in step by
`sync.sh`; both are retired, and `repo-plan.md` keeps the history of why.

If you only care about one component, you don't need any of the above: clone
its repository and `cargo test`.

## Nothing waits on another process forever

`Command::output()` waits indefinitely, and every external program here —
`hyprctl`, `gsettings`, `fc-list`, `lua` — is one that can stop answering: a
wedged compositor, a hung dconf, a home directory on a stalled mount. Every
call site goes through `hyprforge_process::output` (re-exported as
`hyprforge_core::command::output`), which kills the child
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

Tier 1 runs everywhere, no system needed. Besides clippy and the unit and
integration tests, it has two steps worth knowing about by name:
**Renderer colour space** greps `cargo tree` for iced's `web-colors`
feature and fails if it's on, because that fact is invisible to every
Rust test — both renderers report the same `Color` and only the pixels
differ (see the shared-look rule in `CLAUDE.md`). **Standalone crate
dependency pins** exists because the nine components in the table
above are their own repositories, which means each hand-copies every third-party dependency's version and
feature set instead of inheriting from `[workspace.dependencies]` — a
crate that is its own repository root has nothing to inherit from. If
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

## Build

```
cargo build --workspace --release
```

Binaries land in `target/release/`: `hyprforge-displayd`,
`hyprforge-displayctl`, `hyprforge-settings`, `hyprforge-files`,
`hyprforge-media`, `hyprforge-clipd`,
`hyprforge-clipmenu`, `hyprforge-emojimenu`, `hyprforge-trayd`,
`hyprforge-traymenu`, `hyprforge-mimetype`, `hyprforge-mimeopen`,
`hyprforge-lock`, `hyprforge-greet`.

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
| `$XDG_CONFIG_HOME/hyprforge/lock.toml` | The lock screen's theme, written by Settings, read by `hyprforge-lock` (the greeter reads an exported copy — it runs as its own user and cannot read your home) |
| `$XDG_CONFIG_HOME/hyprforge/tray.toml` | Which tray icons show, how far below the bar their menu opens, and whether clicking away dismisses it — written by Settings' Tray screen and re-read by `hyprforge-trayd` on every poll (and by `hyprforge-traymenu` on every right click), so a change takes effect without restarting anything |
| `$XDG_CONFIG_HOME/hyprforge/emojimenu.toml` | The emoji picker's default skin tone, and how often each emoji is picked (its "Frequently used" section) |
| `$XDG_CONFIG_HOME/hyprforge/files.toml` | The file manager's remembered state — sort order, view mode, pinned folders, whether the preview pane is on, window size |
| `$XDG_CONFIG_HOME/hyprforge/files-config.toml` | The file manager's hand-written configuration — key bindings, the sidebar's places and their icons. Only ever read, so your comments survive |
| `$XDG_CONFIG_HOME/hyprforge/media.toml` | The photo viewer's remembered state — window size, which panels are showing, slideshow and 3D model view settings |
| `$XDG_CONFIG_HOME/hyprforge/media-config.toml` | The photo viewer's hand-written key bindings. Only ever read. The order pictures page in is not here: it is read from `files.toml`, so the two apps agree |
| `$XDG_CONFIG_HOME/hyprforge/clipboard/` | The clipboard history: `history.toml` for the index, `images/` for one file per image entry — never image bytes in the index itself |

`$XDG_CONFIG_HOME` falls back to `~/.config` if unset, per the XDG spec.

## Installing

Everything that lives outside this repository — binaries in
`/usr/local/bin`, the greeter's compositor config, the PAM stack, the
exported-look directory — is handled by `./hyprforge`:

```sh
./hyprforge --status                   # what is installed, and is it stale
./hyprforge --install --greeter        # install, or update in place
./hyprforge --install --all --dry-run  # show what that would do
./hyprforge --uninstall --lock --force-cleanup
```

Install and update are the same command: it builds, compares each file
against what is installed, copies only what differs, and then verifies
the copy landed. That last check is not ceremony — a stale binary in
`/usr/local/bin` looks exactly like a code change that did not work, and
`--status` is the fastest way to rule it out.

`--install` ends by running `hyprforge-settings --setup` when Settings is
installed. It wires the suite into the desktop: it enables the daemons'
user services, binds the popups, points hypridle's `lock_cmd` at the
lock screen, sets Files and Media as default applications, and sets up
the open/save dialog. It asks about each item. Under `--yes` it applies
each item's default, and under `--dry-run` it only lists what it would
do. It is the same code as the Settings app's **Set up** page, and it
records what it changed, so `hyprforge-settings --setup --undo` puts it
back. `--uninstall` offers that undo before it removes anything, and
`--status` shows each item's state.

It deliberately does **not** install `/etc/greetd/config.toml` or enable
`greetd`. That file decides which VT the machine logs in on, so it is the
one step that stays manual; see `crates/hyprforge-greet/INSTALL.md`.

From the Arch packages (`packaging/arch/PKGBUILD`), the four daemons —
displayd, trayd, clipd and notifd — are enabled for every user on
install, through systemd presets. notifd is the exception when dunst,
mako or swaync is installed: only one notification daemon can run, so
its package leaves that choice to the Set up page. The Settings package
points at the Set up page for the rest. Nothing in the user's own
config is written as root.

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

Each row shows the setting as `key = value` and says, in a chip beside
it, which of three things that value is — and the distinction is the
point of the screen:

- **set by hyprforge** — this app writes it, and it wins.
- **your config** — your config sets it; the app is only reporting it
  back.
- **hyprland default** — nobody set it.

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

### What the screen shows

The layout is the "Hyprlock: glass card" mockup, drawn by
`hyprforge-authui` for both hosts, with every colour taken from the shared
theme at an alpha:

- **Idle**: the clock alone, large, with "Type to unlock" — or "Type, or
  touch the sensor, to unlock" while the fingerprint reader is listening.
  Any key brings the card up; Enter, Escape or Backspace on the idle clock
  only wake it, and are never sent as an empty password (which would spend
  a `pam_faillock` attempt). Twelve quiet seconds with nothing typed puts
  it back, and Escape on an empty card does so at once.
- **The card**: avatar (`~/.face`, then AccountsService, else the initial),
  name, a dot per character with a caret, and under it the keyboard layout
  and Caps Lock. A rejected password turns the field red, keeps *that
  attempt's* dots up while it shakes three times, then clears; "attempt N"
  sits beside PAM's own message.
- **Status**, top right: layout from the keymap the compositor sent, the
  network from NetworkManager, the battery from UPower — each part simply
  absent when its daemon is. A low battery turns the reading orange and
  adds a warning to the card.
- **Extras on the idle clock**: what an MPRIS player is playing, with
  previous / play-pause / next (media keys work too), and notification
  counts per application — the app and the count, never the text.
- **Power menu** from ⏻, or Tab: Suspend, Hibernate, Reboot, Shut down,
  each only when logind answers `yes`, chosen by pointer, arrows and
  Enter, or its letter. While it is open, nothing typed reaches the
  password. Switch user is not offered: greetd has no session switching
  to hand over to.
- **Several monitors**: the card, status and ⏻ are drawn on the output
  that has the keyboard; every other output shows the clock alone.

Notification counts come from watching `Notify` calls on the session bus
as a D-Bus monitor, because the freedesktop protocol has no way to ask a
server how many are unread. That also makes them mean the right thing
for a lock screen: what arrived while you were away. Summary and body are
dropped the moment the application's name has been read.

### The fingerprint goes to fprintd, not PAM

PAM asks its modules in turn, so with `pam_fprintd` in the stack either
the password prompt waits for the reader to give up or the reader waits
for a password to be refused — it cannot offer both at once, which is
what the card does. So, like hyprlock, the lock talks to fprintd on the
system bus beside the password conversation (`src/fingerprint.rs`).

It is not a way round PAM's *policy*: a match is followed by the PAM
account stack (`pam_acct_mgmt`) before anything unlocks, so an account
PAM would bar on a correct password is barred on a correct finger. Only
`VerifyStatus` signals from the process that owns fprintd's bus name,
about the device that was claimed, are believed. Three unrecognised
fingers — `pam_fprintd`'s own default — stop the reader, because fprintd
keeps no count of its own and `pam_faillock` never sees these; a bad
*read* (too short, off-centre) is not counted. `check.sh`'s fprintd tier
asks the running daemon the lock's own questions, read-only; the first
version of this code named the wrong manager path and passed every unit
test.

### It draws at the output's real resolution

The lock used to paint a logical-size buffer at scale 1.0 and let the
compositor stretch it — blurry on a 1.6 output, the mistake CLAUDE.md
records for the popups. It now uses `wp_fractional_scale_v1` with
`wp_viewporter`, per output.

At a real resolution, resampling a 4K wallpaper onto the output every
frame was more than half of each frame: 45ms with it, 20ms without, at
1440×900 physical. So each output's wallpaper is prepared once on a
worker thread (`src/backdrop.rs`) — decoded through `hyprforge-image`'s
budget, cropped to cover, dim baked in — and drawn one-to-one: 32ms. The
first frames still draw from the path, since the session is not locked
until they exist.

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

A test lock still talks to the real system bus, so under
`--fake-password` the power menu only *rehearses* — "would Shut down now"
on stderr — rather than asking logind to act on the machine the test is
running on. The fingerprint reader stays on for a hand-driven run (a
real finger unlocks the nested session) and off under `--type-in`, so a
self test's result never depends on who touched the sensor.

Hyprland does not deliver synthetic keys or clicks to a lock surface —
`send_shortcut` targets windows, and a lock is not one — so the nested
compositor cannot be typed at from a script. `--type-in` is the way in;
the card, the shake and the settled failure can be screenshotted with
`grim -o <output>` after `--type-in wrong`. Screenshot a headless output
(`hyprctl output create headless`), not the nested window, whose frames
go stale while the host is not showing it.

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

Four gaps, none of them a security hole, the first three things an
established lock screen has:

- **No input-method support.** A password typed through an IME cannot be
  entered. swaylock is the same; it still means some users cannot log in.
- **The password is erased on this side, and PAM keeps its own copy.**
  `Secret<T>` zeroes its value when it goes out of scope, which matters
  more than it looks: a typed password is not appended to in place — the
  host hands the whole string over on every keystroke and the old one is
  dropped — so eight characters allocate eight strings, each holding a
  prefix, and every one of them is now cleared as it is displaced. The
  clone `submit` makes for the backend is wrapped too. What remains is
  outside this code: PAM and greetd copy the answer once it is handed
  over, and a core dump or a swapped page taken while they hold it can
  still contain it.
- **No attempt limiting of its own for passwords**, on purpose: rate
  limiting belongs in `/etc/pam.d`, where an administrator can see and
  change it, rather than hidden in a settings app. The fingerprint path is
  the exception, because nothing in PAM sees it — see above.
- **No blur behind the card.** The mockup's frosted glass is a
  `backdrop-filter`; a software renderer that draws each widget once has
  no equivalent, and the translucent tint does most of the work of
  keeping text legible over a photograph.

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
