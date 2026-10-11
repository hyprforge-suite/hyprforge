---
name: release
description: Cut, resume or recover a Hyprforge release with tools/hf release — the two-phase lockstep release of every library, component, crates.io crate, GitHub release and the AUR package. Use when asked to release, tag, publish to crates.io, bump the version, push to the AUR, or when a release stopped halfway or publish.yml failed.
---

# Releasing

`tools/hflib/release.py`'s module doc has the why; `docs/design/repo-plan.md`
"Releases" has the shape. This is the procedure.

Almost nothing here can be taken back — a crates.io version is
permanent, a tag is public, the AUR reaches users at once. The user
decides when to release; confirm before every `--execute`.

## Before

1. The Unreleased section of `CHANGELOG.md` has been written by a person
   (`python3 tools/changelog.py draft --since <last tag>` drafts it; the
   release refuses an unedited draft).
2. Everything for the release is committed and landed: every component
   at its pin on `main`, and work that needed this release's library
   changes on each component's `next` branch (`component-change` skill).
3. The disposable tier has passed on the current commit — the release
   refuses otherwise. Ask the user to run `tools/hf test disposable` in a terminal of
   their own — the `!` prefix has no terminal and is refused.

## Run it

```
tools/hf release 0.1.14            # dry run: every action it would take, nothing done
tools/hf release 0.1.14 --execute  # after the user has read the dry run
```

Phase A merges every `next` locally, runs the full check, packager and
clean-chroot AUR build, moves every version, pushes the clipboard and
tray, commits "Release vX.Y.Z (libraries)" and pushes the tag
`libs-vX.Y.Z`. publish.yml then publishes and **waits out crates.io's
rate limit by itself** (burst of 30, then one a minute) — no manual
`gh run rerun` any more. `hf release` waits until the sparse index lists
every library.

Phase B builds each remaining component alone against the just-published
crates, pushes it, deletes its `next`, commits "Release vX.Y.Z" with every
pin, tags `vX.Y.Z` on the suite and every component, and creates GitHub
Releases for what changed.

Long waits: run it with `run_in_background` and a Monitor on its output,
not a foreground call that times out. Follow the publish job with
`gh run watch -R hyprforge-suite/hyprforge`.

## When it stops

It stops at the first failure, says what failed, and records every
finished step in `target/hf/release-X.Y.Z.json`. Fix the cause and
**re-run the same command** — it continues from the first unfinished
step. Never finish a release by hand while the state file exists, and
never delete the state file to "start over" after anything was pushed.

- Phase A check fails with every `next` merged: the bug is on a
  component's `next`; fix it there (land `--next`), then re-run.
- A component fails its standalone build in phase B: every library is
  already published, so nothing upstream moves — fix it on the
  component's `main`, land it, re-run.
- publish.yml failed for something other than 429: read the log
  (`gh run view --log-failed`); after a fix, re-run the job
  (`gh run rerun <id> --failed`) — it skips what the index already has.
- A brand-new crate name: crates.io rate-limits new names per account;
  the first publish of a new library is done by hand before the release
  (github-org-move memory).

## After

The AUR push is printed, not done — it needs the user's SSH key for
aur.archlinux.org. Hand them the commands it printed. Then tell them
which installed binaries are stale (`./hyprforge --install` — the
`install-locally` skill).
