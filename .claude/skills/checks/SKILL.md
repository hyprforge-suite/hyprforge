---
name: checks
description: Choose and run Hyprforge's checks with tools/hf, and read what they say. Use before committing, after any code change you want to verify, when a test fails or a check reports yellow, when the machine slows down during a build, or when deciding whether a change needs the live, disposable or root tier. Also use instead of reaching for `cargo test` directly.
---

# Running the checks

`tools/hf` is the only test entry point. `docs/testing.md` has the tier
table; this is which command to run and how to read it.

## Which command

| You changed | Run | Who runs it |
|---|---|---|
| anything, about to commit | nothing extra — `git commit` runs the hook (`tools/hf precommit`: staged pins, then `check --fast`) | you |
| code, iterating on one crate | `tools/hf test unit -- -E 'package(hyprforge-x)'` | you |
| a D-Bus client, a parser of system data, icon names, anything a `live` test covers | `tools/hf check` (full — read-only, safe on the live session) | you |
| a Hyprland option catalogue or code generator (appearance, input, system, session, shortcuts, windowrules, ecosystem, core), the nested clipboard | `tools/hf test disposable` | **the user**, from a terminal |
| procman's exits/eBPF, or a shipped unit's sandbox | `tools/hf test root` / `tools/hf sandbox` | **the user** (sudo) |
| a screenshot tool or benchmark | `tools/hf test manual -- -E 'test(name)'` | you, with `$SHOTS` set |

Do not run `tools/hf check --fast` right before committing: the hook
runs it, and two runs is two builds. Never run two cargo builds at once
on this machine (disk-full-build-stall memory).

**Always go through `tools/hf`, never `cargo test`/`cargo nextest` by
hand.** hf passes `--workspace` and procman's `tui,netbpf` features to
every call so one build serves every tier; a hand-typed command with a
different feature set rebuilds procman and everything it pulls in. And
`cargo test -- --ignored` runs tests that were never meant to run
together, ungated.

**The disposable and root tiers are never yours to start.** The Bash
tool has no terminal, `hf hyprland up` refuses without one, and the
2026-10-08 freeze happened while an unattended agent cycled nested
compositors. Ask the user to run `tools/hf test disposable` in a terminal of their
own — not with the `!` prefix, which has no terminal either and is
refused the same way. Never pass `--unattended-ok`.

## Reading the result

- `✓` passed. `✗` failed — the lines under it are the failing tests'
  last output.
- Yellow `–` is **not checked**, never a pass: a gate whose service is
  absent, or a test that printed `HYPRFORGE-SKIP:`. Say so when you
  report a green run that had any.
- "zero tests ran" and "nextest did not finish" are failures — a killed
  build prints no FAILED line.
- **"The session is as it was" failing** means a live test changed the
  running Hyprland (or the user changed a setting mid-run). Treat it as
  a bug in a test that should be in `disposable`; the diff names the key.
- "N ignored tests … misclassified" — an `#[ignore]` test is in no tier
  or two. See the `add-a-live-test` skill.

## The machine

- `tools/hf check` warns when `target/` passes 200GB and refuses under
  20GB free. `tools/hf clean --dry-run` shows what it would remove.
- When the whole computer slows during a build: `df -h /home`,
  `cat /proc/pressure/io`, `du -sh target .claude/worktrees/*/target`.
  80–90% I/O pressure with idle disks is Ghostty's io_uring, not us.
- `tools/hf doctor` says what is missing (nextest, clang, devtools…).

## Finding out more

`tools/hf <command> --help`; each step's reason is in a comment in
`tools/hflib/checks.py` or at the top of its script in
`tools/hflib/lints/`.
