# Architecture

How the workspace is put together: which crate depends on which, why the
layers fall where they do, and how the component repositories relate to
this one. `CLAUDE.md` has the rules that came from getting these wrong.

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
                            Phones and cameras through gvfs's MTP and
                            gPhoto2 backends, and the kernel's USB
                            listing for one no backend here can read.
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
                            needs no GTK or Qt. On Hyprland the right-click
                            menu is hyprforge-traymenu, a sibling popup
                            this daemon spawns directly, and the item
                            advertises no `Menu` property at all. Where the
                            popup cannot run — another compositor, or a
                            bar that never calls ContextMenu — the same
                            menu is served as com.canonical.dbusmenu for
                            the bar to draw itself. tray.toml's `menu`
                            (auto, popup or dbusmenu) chooses; auto does
                            the above. Popup-only was tried first, and on
                            any other compositor it left icons with no
                            menu at all.
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
  hyprforge-polkit/        the polkit authentication agent: registers for
                            the session, relays polkit's helper's PAM
                            conversation, and draws it as the lock
                            screen's card (hyprforge-authui) on a
                            full-monitor PopupApp surface of its own
                            namespace, so a layer rule can blur behind it.
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

Eleven of these directories — clipboard, lock, greet, tray, settings,
displayd, emojimenu, files, media, notif and polkit — are also their own repositories, published
separately from this one. See "Twelve repositories, one workspace" in the [README](../README.md) for
the list, and "Repositories, crates.io and where a change goes" below for
where a change to one of them should actually be made.

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

## Repositories, crates.io and where a change goes

Each of these repositories takes its Hyprforge dependencies as **versions
from crates.io** — `hyprforge-look = "0.1"` — where every library crate in
this workspace is published. So a clone of one component fetches the
published libraries it uses and nothing else; it never needs this
repository. Inside this workspace the root `[patch.crates-io]` table points
every one of those names back at `crates/`, so a change to a library
reaches every app built here at once. The libraries share one version and
are published together; see `docs/design/repo-plan.md`.

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
the pin; `docs/design/repo-plan.md` has the order.

Until October 2026 the components were copies instead, written here and
pushed out with `git subtree split` by `split.sh` and kept in step by
`sync.sh`; both are retired, and `docs/design/repo-plan.md` keeps the history of why.

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
