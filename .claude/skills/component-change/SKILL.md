---
name: component-change
description: Change code inside one of Hyprforge's twelve component submodules (crates/hyprforge-{clipboard,lock,greet,tray,settings,displayd,emojimenu,files,media,notif,polkit,procman}) and get it committed, pushed and pinned in the right order. Use for any edit under those directories, when a component needs a library change that is not on crates.io yet, or when `git status` shows a submodule as "modified content" or "new commits".
---

# Changing a component

Each component is a git submodule and its own repository. A change is
**two commits, in this order**: inside the component, pushed to its
repository's `main`; then the pin here. A pin naming a commit nobody can
fetch is a monorepo nobody else can clone.

## 1. Before editing

- `git -C crates/<c> branch --show-current`. Empty means a detached HEAD
  at the pin: `git -C crates/<c> checkout -B main` before committing, or
  the commit belongs to no branch.
- A new git worktree starts with every submodule empty:
  `git submodule update --init`, and make `target/` a btrfs subvolume
  (`btrfs subvolume create target`) before the first build.

## 2. Does it need an unreleased library change?

Components name their sibling libraries by crates.io version
(`hyprforge-ui = "0.1"`). Here, `[patch.crates-io]` builds them from
this checkout, so anything compiles — but the component's own CI builds
against crates.io.

- **No** (the usual case): work on `main`.
- **Yes** — it calls something added to a library since the last
  release: the library change is committed here as normal, and the
  component's commit goes on its **`next`** branch
  (`git -C crates/<c> checkout -B next origin/next` if it exists, or
  `-b next` from main). The next release merges every `next` once the
  libraries are published. Do not split it by hand or invent another
  branch name.

## 3. Commit inside the component

Stage the paths you wrote, never `-A` — other agents and the user may
have work in the same tree:

```
git -C crates/<c> add src/foo.rs src/bar.rs
git -C crates/<c> commit
```

The commit message describes *its* repository's change; it will be read
there without this one.

## 4. Land it

```
tools/hf land <c>          # main: builds it alone against crates.io, pushes, stages the pin
tools/hf land <c> --next   # next: pushes to origin/next, stages nothing
```

`hf land` refuses uncommitted changes in the component and a HEAD that
does not descend from `origin/main`. If the standalone build fails with
`cannot find function`/`unresolved import` naming a Hyprforge crate, the
change needs an unreleased library: step 2's "yes", so `--next`.

Pushing is outward-facing: confirm with the user before landing unless
they already said to push.

## 5. Commit the pin here

`git commit` — the staged pin, plus whatever monorepo change goes with
it, staged by path. The pre-commit hook refuses a staged pin that is not
on its repository's `origin/main`. `git status` showing the submodule as
`modified content` means step 3 is not done; `new commits` means the pin
is not staged.

## Docs

A component's README, DESIGN.md and CHANGELOG live in its repository;
`keep-docs-current` covers them. Component CHANGELOGs are filed by the
release from git history — do not edit them by hand.
