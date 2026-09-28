# hyprforge-authui

The one authentication screen the lock screen and greeter share: background, clock, prompt and a Backend seam for PAM or greetd, drawn with iced 0.14 through iced_tiny_skia.

The one screen that asks who you are.

A greeter and a lock screen are the same screen. Both show a
background, a clock and a name, both ask a question, both accept or
refuse. What differs is only *how* the question travels — a lock talks
to PAM directly, a greeter talks to greetd over a socket — and *where*
it is drawn: a lock gets a surface handed to it by the compositor, a
greeter is an ordinary window.

So this crate is that screen, and nothing else. Two thin hosts
(`hyprforge-lock` and `hyprforge-greet`) supply a surface and a
`conversation::Backend`. Neither knows the other exists, and there
is no "matching them up" to do, because there is only one of them.

The other half of feeling like one system is the `Theme` — and that
has a hard constraint behind it. A greeter runs as its own user, and a
home directory is `drwx------`, so it cannot read your wallpaper or
your settings *at all*. Continuity therefore isn't a styling exercise;
it's an export. See `Theme::export`, and `hyprforge_look::theme`,
where it lives.

## Where this lives

Part of [Hyprforge](https://github.com/hyprforge-suite/hyprforge), a suite of native Hyprland desktop
applications. This crate is `crates/hyprforge-authui` there; it is published to
crates.io so the suite's applications can be built from their own
repositories, and its API follows the suite. Issues and pull requests go
to the suite repository.

```
cargo add hyprforge-authui
```

This README is rendered from the crate's `//!` documentation by
`tools/crate-readme.py`; edit `src/lib.rs`, not this file.

## Licence

MIT. See `LICENSE`.
