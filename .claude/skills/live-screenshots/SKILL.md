---
name: live-screenshots
description: Take README or website screenshots of Hyprforge's apps (Settings, Files, Media, popups, the lock screen, notifications) or render demo images. Use when asked for screenshots, when docs/images or a site page needs a picture, or when a `<shot name=...>` placeholder needs filling.
---

# Screenshots

Two ways, in order of preference.

## 1. Rendered, no compositor

Some components draw their own pictures offscreen, as tests in the
`manual` tier — `tools/hf test manual` lists them (polkit's
`render_the_card`, procman's `render_every_tab` and `render_recorded`).
`SHOTS=<dir> tools/hf test manual -- -E 'test(render_the_card)'`. These
touch nothing and need nobody present; a component without one can get
one the same way (a `manual`-tier test writing PNGs to `$SHOTS`).

## 2. A real window: the disposable instance

For an app that has to run, prefer the disposable nested Hyprland
(`disposable-hyprland` skill): the user starts it, the app runs on its
display, it moves to `HF-HEADLESS` and `grim -o HF-HEADLESS` captures it.
Nothing in the session moves.

## 3. A real window on the live session — only with the user present

The 2026-10-09 README shots were taken on an off-screen headless output
of the **live** session; the rig lives in `~/.cache/hyprforge-shoot/`
(`stage.sh up|down`, `run.sh`, `grab.sh`, `key.sh`, `stop.sh`, a demo
home of public-domain photos). It needs explicit consent each time, and
it has three traps:

- **Stop hyprforge-displayd first.** It sees the new output as a new
  topology and rewrites `monitors.lua`. Back up `monitors.lua` and
  `display-profiles.toml`; restore both on teardown.
- **Moving a window there takes focus and the pointer with it** — the
  user's keyboard lands in an invisible window. Dispatch
  `hl.dsp.focus({ monitor = [[eDP-2]] })` after every move.
- **`hyprforge-media --help` opens a window**: Media reads any argument
  as a path.

`hyprctl` reports logical coordinates and `grim` writes physical pixels:
multiply a window's box by the output's scale before cropping, or the
crop lands on some other application.

## Every picture

- **The persona is voldemort** (bellatrix for a second account; Riddle
  House, Nagini Buds, Gringotts for places and things) — never a real
  name, never a generic one. Screenshots are public.
- Run with `HOME=<demo home>` and an isolated `XDG_CONFIG_HOME`, and
  scrub what a copied config leaks: `files-session.toml` (real tab
  paths), `files-portal.toml`, GTK bookmarks, Files' Owner column. An
  isolated config also isolates dconf, so copy `dconf/user` before
  judging icons or fonts.
- A screenshot is evidence only once you have looked at it (Read the
  PNG). Check the centre pixel against the theme when the question is
  "do these two look the same".
