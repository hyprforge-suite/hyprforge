# hyprforge-icons

Finds an icon by name the way the freedesktop icon theme specification says to: the configured theme, what it inherits, then hicolor.

Finding an icon's file from its name, through the configured icon
theme.

The freedesktop icon theme specification, and only the parts of it a
lookup needs: a theme's `index.theme` names its directories and what
size each holds, and names the themes it `Inherits=` from; a lookup
searches the configured theme, then what it inherits depth first,
then `hicolor`, then `/usr/share/pixmaps`.

# Why the inheritance walk is the whole point

It is the part that is tempting to skip and the part this machine
depends on. The configured theme here, Dracula, ships no `mimetypes/`
directory at all and names eight parents of which six are not
installed — so every file-type icon resolves through `breeze-dark`,
two levels down, or not at all. A lookup that only searched the named
theme and hicolor would find almost nothing on this machine and look
finished on a tidier one.

# Theme first, then name

Given several names — `text-x-rust`, then `text-x-script`, then
`text-x-generic` — this searches every name in one theme before
moving to the next theme, which is the order the specification's
`FindBestIcon` gives. It means a generic icon from the user's own
theme beats a specific one from a theme it inherits. That is the
right way round: a listing whose icons come from three different
themes looks broken even when every one of them resolved, which the
tray learned the hard way (see CLAUDE.md on two icon names that both
resolve and still look wrong together).

# What this does not do

No `.xpm`: nothing in this suite can draw one, and a path to a file
the caller cannot draw is worse than no answer, since it stops the
caller falling back to something it can.

## Where this lives

Part of [Hyprforge](https://github.com/hyprforge-suite/hyprforge), a suite of native Hyprland desktop
applications. This crate is `crates/hyprforge-icons` there; it is published to
crates.io so the suite's applications can be built from their own
repositories, and its API follows the suite. Issues and pull requests go
to the suite repository.

```
cargo add hyprforge-icons
```

This README is rendered from the crate's `//!` documentation by
`tools/crate-readme.py`; edit `src/lib.rs`, not this file.

## Licence

MIT. See `LICENSE`.
