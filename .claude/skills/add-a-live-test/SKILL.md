---
name: add-a-live-test
description: "Write or move an ignored (`#[ignore]`) test in Hyprforge — one that asks a real service, tool, compositor or the kernel something. Use whenever you add `#[ignore]` to a test, write a test that talks to Hyprland, D-Bus, an installed program or the clipboard, or when `tools/hf` reports a test misclassified or behind no gate."
---

# Adding a live test

An ignored test has to answer two questions before it is written, and
`tools/hf` refuses to commit one that has not: **which tier is it in**,
and **what has to be running for it to mean anything**.

## 1. Pick the tier

| It… | Tier |
|---|---|
| only reads a real service, file, tool or Hyprland (`getoption`, `binds -j`, introspection) | `live` |
| registers something visible for a moment and removes it (a tray icon, a BlueZ agent) | `visible` |
| changes Hyprland (eval, reload, binds, rules, options), a daemon's state (wallpaper, temperature) or the clipboard | `disposable` |
| needs root | `root` |
| draws screenshots or measures this machine — a tool, not a check | `manual` |

When unsure between `live` and `disposable`, it is `disposable`. A
`hyprctl reload` is a change: it wipes the user's runtime settings.

## 2. Write it

- `#[ignore = "<what it needs>"]` — the reason is for people; the tier is
  in `.config/nextest.toml`.
- Hyprland, in a library crate: add the dev-dependency
  `hyprforge-testkit = { path = "../hyprforge-testkit" }` (path only — it
  is never published) and open the test with
  `let live = hyprforge_testkit::Hyprland::read_only();` or
  `::disposable();`. `disposable()` panics unless it is given a nested
  instance, and reloads it on drop, panic or not. Use its `hyprctl`,
  `reload`, `config_errors` and `daemon_running`, never a bare
  `Command::new("hyprctl")`. A component cannot use the testkit (its own
  repository has no path to it); keep compositor-changing tests in
  libraries.
- A check that cannot run returns early **after**
  `eprintln!("HYPRFORGE-SKIP: <reason>")`. Without the marker a skipped
  test reports as a pass.
- Assert what only the real thing can answer — a value's type, a range
  it accepts, that a name is listed — not that a call returned. Add a
  negative control when the failure would be silent (a daemon that exits
  0 on a bad key).
- Read-only means read-only: nothing that can drop the Wi-Fi, unpair a
  device, start a discovery, change a power profile or write the user's
  files.

## 3. Classify it

- Add it to its tier's `default-filter` in `.config/nextest.toml`
  (usually `binary_id(<crate>::<test file>)`, plus `test(...)` when one
  file holds tests of two tiers).
- `live` and `visible` tests also need a gate in `tools/hflib/tiers.py`:
  a `Gate(label, tier, expr, check)` whose `check` asks **the service the
  test asks** — never the compositor on behalf of NetworkManager. Reuse
  an existing gate when the question is the same.
- If you added a gate, the count in CLAUDE.md ("N live gates") and the
  gate table in `docs/testing.md` move with it; `tools/hf lint` checks
  the count.

## 4. Check it

`tools/hf lint` must report every ignored test in one tier and every
live one behind one gate. Then run its tier: `tools/hf check` for
`live`/`visible`; ask the user for `! tools/hf test disposable` or
`! tools/hf test root`. A green run that only skipped is not a pass —
make it fail once on purpose (wrong expected value) to see it can.
