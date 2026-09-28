# hyprforge-photos

A photo, video and 3D model viewer for Hyprland.

Open one picture and it floats, sized to the picture, on the monitor the
pointer is on. Open a folder and it is a viewer over that folder, in the
**same order the file manager shows it** — "next" is asked of
`hyprforge-listing`, the leaf under both, so two windows over one folder
never disagree about which picture follows this one. Behind the single
picture there is a date-grouped grid, a library of folders, a filmstrip,
an inspector reading EXIF, a slideshow, and Back and Forward through
what was looked at.

Pictures are decoded **within a budget** and orientation-correct, through
`hyprforge-image`; a 36-megapixel file is measured before it is decoded,
not after. Videos play through libmpv, opened at run time rather than
linked, so a machine without mpv still runs the viewer and a video says
what is missing. STL, 3MF and OBJ models are loaded into a welded mesh
by `hyprforge-mesh` and drawn on the GPU with fstl's camera, inside the
same window.

Copy, "Open With…", trash, "Show in Files" and "Set as Wallpaper" each go
through the suite's own library for that job rather than a copy — the
clipboard *library* (never the history daemon, which need not be
installed), the shared MIME database, the freedesktop trash, and the
wallpaper settings the Settings app's Desktop screen edits, so the two
never disagree.

Part of [Hyprforge](https://github.com/hyprforge-suite/hyprforge), a suite of
native Hyprland desktop applications. This repository is a split of the
`crates/hyprforge-photos` directory there; development happens in the
monorepo and `sync.sh` keeps this copy in step.

## What is here

A library as well as an application. Every *decision* — what is in the
folder, what is next, how a picture fits the window, what a key means,
which tiles a date heading groups, what the slideshow shows next — lives
under `src/lib.rs` where a test can reach it without a display. The
window itself is `src/main.rs` and `src/view.rs`, with `src/film.rs`
drawing a playing video's frames and `src/model/` a 3D model on the GPU,
and is deliberately thin.

## Installing

```
cargo install --path .
```

Arch users can build the `hyprforge-photos` package from the monorepo's
`packaging/arch` instead, which also installs the desktop entry.

Nothing else in the suite is required. A missing `appearance.toml` is
first-run, not an error, and the app draws correctly themed on a machine
where the Settings app has never been installed.

Optional, each degrading to a sentence rather than a viewer that fails
to start: `mpv` for playing videos, `ffmpeg` for a video's thumbnail,
`hyprpaper` for a wallpaper that changes now rather than at next login,
and `hyprforge-files` for "Show in Files".

## Licence

MIT. See `LICENSE`.
