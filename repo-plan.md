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

`git subtree`, in both directions — and `./split.sh` wraps the outbound
half, because the incantation has one non-obvious flag in it:

```
git subtree add   --prefix=notif <repo> <branch>   # bring one in, history intact
git subtree push  --prefix=notif origin main       # send changes back out
./split.sh hyprforge-clipboard                     # take one out, history intact
```

**`--rejoin` is not optional, and this was got wrong first.** A plain
`git subtree split --prefix=... -b extracted` produces a perfectly good
branch and records *nothing* in this repository. That works in one
direction and fails silently in the other: a change made in the
component repository cannot come back, because `git subtree pull` looks
for a common ancestor, finds none, and stops with `fatal: refusing to
merge unrelated histories`. The split commits have the same trees as
ours and different hashes, and nothing connects the two.

`--rejoin` leaves a merge commit here — `Split 'crates/<crate>/' into
commit '<sha>'` — recording which commit the split produced, so a later
pull has an ancestor to find. That merge commit is the entire cost, and
it is what makes this bidirectional rather than a one-way export. Both
directions were tested against a local clone before this was written
down; the failure above is what actually happened.

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

So **the foundation has to be reachable from outside before any app can
leave** — which is not the same as published. A git dependency on this
repository is reachable, builds anywhere, and needs no crates.io account
at all; `hyprforge-clipboard` uses exactly that today and builds
standalone. Publishing is still the destination, because a git
dependency cannot itself be published and pins consumers to a repository
rather than a version. But it is no longer the gate, and the order below
is now about *convenience* rather than *possibility*.

The one-manifest problem is solved and worth knowing: the clipboard's
`Cargo.toml` names the git URL, and the root `Cargo.toml` has a
`[patch."https://github.com/apost/hyprforge"]` section redirecting those
two dependencies back to `crates/`. One file, both contexts, no
divergence for `git subtree push` to conflict on forever, and no network
access when building here.

The good news is still how small the foundation is:

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
./split.sh hyprforge-clipboard
git push <new-repo-url> clipboard-split:main
```

Done, minus the push. The branch builds standalone: `cargo build`,
`cargo clippy --all-targets` silent, 53 tests green and
`hyprforge-clipd` linking, verified in a clone with the git dependencies
pointed at a local stand-in for GitHub.

Three things only doing it revealed. The `LICENSE` symlink dangled —
fine for a crate that is only ever a tarball, since `cargo package`
dereferences it, and broken for one that becomes a repository, since git
carries a symlink as a symlink and `../../LICENSE` is above the root.
There was no `.gitignore`, because the root one does not come along. And
there was no README, which matters most of the three: identity is the
entire point of splitting, and a repository with no front page has none.
`split.sh` now refuses to split a crate missing any of them.

**Only the crate leaves, not the product.** The `hyprforge-clipboard`
*package* installs two binaries, and the second is `hyprforge-clipmenu`,
which needs `hyprforge-appearance` for `look::resolve()`. So the popup
stays here for now and the split repository is the clipboard engine plus
its daemon. Worth being honest that this is a smaller thing than
"the clipboard has its own repository" sounds.

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
- [x] First split prepared and verified standalone; not yet pushed
