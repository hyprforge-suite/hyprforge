# Splitting Hyprforge into repositories

The goal, in the owner's words: *"I want them to feel like they are
intertwined and connected like a software suite, but if someone wants our
clipboard they should be able to install that without needing
everything."*

Two things have to be true at once, and they are not the same thing:

- **Feels like one suite** — one `hyprforge_look::Theme`, one widget
  vocabulary, one keyboard grammar, and *one place to fix them*. The
  `web-colors` bug was a single `default-features = false` rather than
  the same edit in five apps.
- **Install one part** — already solved, and not by repository layout.
  `packaging/arch/PKGBUILD` builds one source tree into seven packages;
  `pacman -S hyprforge-clipboard` installs two binaries and no other
  Hyprforge package. Packaging granularity and repository granularity are
  different questions.

What repositories add on top of that is **identity**: a project someone
can star, file an issue against, and clone without the other twenty-two
crates. That is a real thing to want, and it is what this plan is for.

## The mechanism

`git subtree`, in both directions:

```
git subtree add   --prefix=notif <repo> <branch>   # bring one in, history intact
git subtree push  --prefix=notif origin main       # send changes back out
git subtree split --prefix=notif -b extracted      # take it out again, history intact
```

So a component can live in its own repository *and* in the master
repository, with changes flowing between them. Development happens in the
master repo, where a commit can cross a boundary and the whole suite is
tested together; the component repos are where the world sees it.

Every step below is reversible. That is deliberate — this is a decision
made under uncertainty, and the cheap-to-undo version is the right one.

## The blocker, and why it is nearly gone

A standalone `hyprforge-clipboard` repository still needs
`hyprforge-paths` and `hyprforge-secret`. Those have to come from
somewhere, or the repository is not standalone at all — it is a directory
that only builds inside the monorepo.

So **the foundation has to be published before any app can leave.** The
good news is how small it is:

| crate | public items | lines |
|---|---|---|
| `hyprforge-paths` | 11 | 396 |
| `hyprforge-look` | 20 | 721 |
| `hyprforge-secret` | 8 | 199 |
| `hyprforge-process` | 2 | 142 |

Roughly 1,460 lines and 41 public items — small enough to stabilise
honestly. It is only this clean because `process` and `secret` were
extracted out of `hyprforge-core`; before that, `hyprforge-network`
depended on 4,450 lines of Hyprland config machinery to get a
134-line subprocess helper, and no honest split was possible.

## The order

Computed from the dependency graph, no cycles:

```
layer 0   lua-import, paths, power, process, secret
layer 1   bluetooth, clipboard, look, network
layer 2   core, ui
layer 3   appearance, authui, displayd, ecosystem, input,
          session, shortcuts, system, windowrules
layer 4   clipmenu, greet, lock, tray
layer 5   settings
```

A crate can only be published after everything it depends on. The same
order applies to splitting: a component can only leave once everything it
needs is reachable from outside.

## Steps

### 1. Make the foundation publishable

Nothing here changes behaviour; it is metadata and discipline.

- Add `description`, `repository`, `documentation`, `keywords`,
  `categories` to each foundation crate. **All five are currently
  missing from every crate in the workspace** — crates.io rejects a
  publish without at least a description and a license.
- Give the publishable crates their own `version` rather than
  `version.workspace = true`. All 23 crates are currently locked to
  0.1.0 in step; a published crate needs to move at its own pace. This
  is one line each and buys independent cadence the day it is needed.
- Decide the public API is what you want to support. `Secret<T>`'s
  one-directional `From` and `Color`'s single `rgba()` parser are the
  sort of decisions that become permanent on publish.

Do this for `paths`, `process`, `secret`, `look`, `ui` — layers 0–2,
minus the crates that are apps or Hyprland-specific.

### 2. Publish them

In layer order. Verify each with `cargo publish --dry-run` first.

Once published, a component repository can depend on
`hyprforge-look = "0.1"` from crates.io and build anywhere.

### 3. Split the first app out

**`hyprforge-clipboard` is the right first one** — it depends on
`paths` and `secret` only, both in layer 0. Its repository would build
against published crates with nothing vendored and nothing pathed.

```
git subtree split --prefix=crates/hyprforge-clipboard -b clipboard-split
# push that branch to a new repository
```

Then decide whether the master repo keeps it as a subtree or a
crates.io dependency. The subtree keeps atomic commits; the dependency
is cleaner but slower to iterate on.

### 4. Judge it before doing the rest

After one split, the questions stop being hypothetical:

- Does a change spanning the boundary still feel cheap?
- Does `check.sh` still prove what it used to?
- Did anything drift?

If the answer is bad, `git subtree add` puts it back. If it is good,
work up the layers: `tray`, `lock`, `greet`, then `settings` last,
because it depends on fifteen crates and will be the hardest.

## What must stay true

- **Leaf crates stay runtime-free.** `hyprforge-look` depends on no async
  runtime and no D-Bus, which is exactly why `notif` can share it while
  keeping smol where the rest of the suite uses tokio. A leaf crate that
  acquires a runtime stops being shareable, and the split quietly
  becomes impossible again.
- **Every component runs alone.** See CLAUDE.md. A split repository makes
  that rule load-bearing rather than aspirational: someone will install
  exactly one package.
- **One place to fix a shared thing.** If a change to the theme starts
  needing five pull requests, the split has gone too far and should be
  walked back.

## Status

- [x] Foundation extracted so the leaves are genuinely leaves
- [x] Split packaging, so installing one component already works
- [x] `notif` merged as a nested workspace — the trial run of the
      mechanism, and proof that a differently-shaped project can share
      the theme without sharing a runtime
- [x] Publishing metadata on the foundation crates
- [x] Independent versions for the publishable crates
- [ ] First publish
- [ ] First split
