# hyprforge-mime

Reads the freedesktop shared MIME database: what type a file is by name or by its contents, which applications handle a type, which one is the default, and which icon a type has.

What type a file is, which applications can open it, and which one
does by default.

Nearly every question has a data file of its own, all of them
freedesktop's and all of them plain text:

| Question | File | Module |
|---|---|---|
| What type is `part.3mf`? | `mime/globs2` | `globs` |
| What is this file, with no name to go on? | `mime/magic` | `magic` |
| Which of those two wins? | — | `lookup` |
| What is a `model/3mf` a kind of? | `mime/subclasses`, `mime/aliases` | `types` |
| Which icon does a `model/3mf` have? | `mime/icons`, `mime/generic-icons` | `icons` |
| What can open a `model/3mf`? | `applications/mimeinfo.cache` | `apps` |
| Which one does, normally? | `mimeapps.list` | `defaults` |

# Why this crate exists

A double-clicked STL opened in Firefox. Not because anything chose
that — because `xdg-open`, on a desktop it does not recognise
(Hyprland is one), asks `file --mime-type`, which reads bytes and not
names. An STL is `application/octet-stream` to it, a 3MF is a zip, a
`.blend` is a zstd stream; none of those has a default application,
and `xdg-open` answers a lookup that found nothing by walking a
built-in list of web browsers. Every `.stl`, `.3mf`, `.uf2`, `.blend`
and `.sh` in a real Downloads folder went that way.

The database on the machine had the right answer the whole time. So
this reads it.

# What this is not

Not a launcher, and not a fourth opinion about which application
opens a file. It reads what the desktop already recorded and writes
only what a person explicitly chose; running the thing is still
`gio launch` with `xdg-open` behind it. Nothing here parses `Exec=`
field codes, `TryExec` or `Terminal=true` — see `apps` for why that
line is where it is.

# Layering

A leaf, like `hyprforge-look`: no iced, nothing Hyprland-shaped, two
dependencies (`hyprforge-paths`, for the config directory and the
atomic write; `hyprforge-process`, for the bounded wait
`hyprforge-mimeopen` puts on `gio launch`). The file manager
and the Settings app's Default apps page read it today, and any
future viewer needs the same answers.

## Where this lives

Part of [Hyprforge](https://github.com/hyprforge-suite/hyprforge), a suite of native Hyprland desktop
applications. This crate is `crates/hyprforge-mime` there; it is published to
crates.io so the suite's applications can be built from their own
repositories, and its API follows the suite. Issues and pull requests go
to the suite repository.

```
cargo add hyprforge-mime
```

This README is rendered from the crate's `//!` documentation by
`tools/crate-readme.py`; edit `src/lib.rs`, not this file.

## Licence

MIT. See `LICENSE`.
