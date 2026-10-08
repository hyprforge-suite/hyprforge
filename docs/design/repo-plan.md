# Splitting Hyprforge into repositories

The goal, in the project author's words: *"I want them to feel like they are
intertwined and connected like a software suite, but if someone wants our
clipboard they should be able to install that without needing
everything."*

Two things have to be true at once, and they are not the same thing:

- **Feels like one suite** — one `hyprforge_look::Theme`, one widget
  vocabulary, one keyboard grammar, and *one place to fix them*. The
  `web-colors` bug was a single `default-features = false` rather than
  the same edit in five apps.
- **Install one part** — already solved, and not by repository layout.
  `packaging/arch/PKGBUILD` builds one source tree into eleven packages
  (ten components and the `hyprforge` metapackage);
  `pacman -S hyprforge-clipboard` installs two binaries and no other
  Hyprforge package. Packaging granularity and repository granularity are
  different questions.

What repositories add on top of that is **identity**: a project someone
can star, file an issue against, and clone without the other
forty-three crates. That is a real thing to want, and it is what this plan is for.

## The mechanism now: submodules (since 2026-10-03, issue #2)

Each component's own repository is the real home of its code, and this
repository **references** it as a git submodule at `crates/<component>`,
pinned to a commit. There is one copy, so there is nothing to keep in step:
`split.sh`, `sync.sh` and the `--rejoin` merge commits are gone, and a pull
request against a component repository is a pull request against the code.

What did not change: the libraries live here and are published to
crates.io; components depend on them by version; and the root
`[patch.crates-io]` table (and `crates/.cargo/config.toml` for notif) still
points every one back at `crates/`, so a build here uses the local
libraries whether a component is a submodule or not. Cargo is content with
workspace members inside submodules.

Working in it:

```
git clone --recurse-submodules https://github.com/hyprforge-suite/hyprforge
git submodule update --init            # a clone made without them

# a change to a component: commit inside it, push it, then move the pin
git -C crates/hyprforge-files commit ...
git -C crates/hyprforge-files push origin HEAD:main
git add crates/hyprforge-files && git commit -m "..."   # the pin bump
```

The order matters and `check.sh` holds it. A pin naming a commit its
repository does not have on `main` is a monorepo nobody else can clone, so
"Submodule pins are on their repositories" refuses one. And a component
whose commit calls a library change not yet on crates.io builds here (the
`[patch]` table) and fails its own CI, so "Components build against
published crates" builds each one outside the workspace the way its CI
will — the guard `sync.sh --push` used to apply before pushing, moved to
where the pin is decided. A change to a library and a component together
is therefore: the library here, a release (`vX.Y.Z`, see Status), then the
component's commit, then the pin.

The migration was mechanical because the copies were exact: after
`sync.sh` reported all ten in step, each `crates/<component>` was replaced
by a submodule at its repository's `main`, and each submodule's tree hash
was compared against the tree the monorepo had held — all ten identical,
so not a byte of component code changed in the move. History before it
stays where it was: a component's commits up to the move are in both
repositories, and from the move on, only in its own.

## The mechanism until October 2026: `git subtree`

Kept as history — none of this is how the repository works now.

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

A standalone `hyprforge-clipboard` repository needed
`hyprforge-paths` and `hyprforge-secret` when this was written (six
Hyprforge crates now, since its popup moved in beside the daemon). Those have to come from
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

The one-manifest problem is solved and worth knowing: a component's
`Cargo.toml` names its siblings by crates.io version, and the root
`Cargo.toml` has a `[patch.crates-io]` section redirecting those names back
to `crates/` (every one of them, which `check.sh`'s "Siblings build from
this checkout" step enforces, along with the version still satisfying the
requirement). One file, both contexts, no divergence for `git subtree
push` to conflict on forever, and no network access when building here.
Until 2026-09-28 the same trick ran on the suite's git URL instead, which
worked, but a clone of one component fetched this entire repository and
built against whatever its main branch held that hour.

The good news is still how small the foundation is (measured
2026-09-12; `paths`, `look` and `secret` have grown since, to roughly
2,000 lines between the four):

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
layer 0   emoji, image, keys, listing, lua-import, mesh, paths,
          power, process, secret, thumbnails
layer 1   archive, bluetooth, fileops, icons, look, mime, network,
          video
layer 2   core, popup, ui
layer 3   appearance, authui, displayd, ecosystem, files-core, input,
          session, shortcuts, system, windowrules
layer 4   clipboard, greet, lock, tray
layer 5   emojimenu, files, media, settings
```

A crate can only be published after everything it depends on. The same
order applies to splitting: a component can only leave once everything it
needs is reachable from outside.

## Steps

### 1. Make the foundation publishable

Nothing here changes behaviour; it is metadata and discipline.

- Add `description`, `repository`, `documentation`, `keywords`,
  `categories` to each foundation crate. **All five were missing from
  every crate in the workspace** when this was written — crates.io
  rejects a publish without at least a description and a license.
- Give the publishable crates their own `version` rather than
  `version.workspace = true`. All 23 crates were then locked to
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
"the clipboard has its own repository" sounds. (Since reversed: it
published a daemon with no way to see its history — see "The shape a
package has to have" below — and `hyprforge-clipmenu` is now a second
`[[bin]]` in the clipboard crate.)

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

All five are done and verified standalone — clipboard, lock, greet,
tray, settings. `settings` was the largest and not the hardest: none of
its fifteen dependencies needed changing, because a git dependency on a
crate that still inherits from the workspace resolves fine.

It did surface the one constraint that only appears once two of these
exist in a chain. `settings` depends on `tray`, which is itself
standalone-ready and then named the git URL in its own manifest — so
resolving `settings` standalone reads two manifests, and cargo keys a
git source on the URL *string*. Two spellings of the same repository
are two sources, and the second gets fetched over the network. The URL
can change; it has to change everywhere at once, and `split.sh` now
refuses to split while more than one spelling exists.

Three more followed — displayd, emojimenu and files — and the one thing
they taught is that a split can succeed with nothing to check it. All
three were split, pushed and published before their CI workflows were
committed, so for a day they were live repositories with no automated
checking at all: the extreme form of a check that silently never runs,
arriving through the one door `split.sh` had not guarded. It refuses now
unless `.github/workflows/ci.yml` and `rust-toolchain.toml` are
committed in the crate. Their first runs also needed the `SUITE_READ`
secret on each new repository, which cannot be copied from another one —
GitHub never reads a secret back. (Moot since the move to the public
organisation on 2026-09-27: the step and the secret are gone.)

**Judged, and the answers are: cheap, yes, and not yet.**

*Cheap* — the clipboard is still an ordinary workspace member. One
`cargo test --workspace` still covers it, `check.sh` still builds it, and
a commit spanning the boundary is mechanically what it was. The `[patch]`
section adds no friction to editing `hyprforge-paths` and the clipboard
together.

*`check.sh` proves what it did* — its clipboard steps were traced back to
the crate's own creation commit, so nothing about the gating changed. But
it only proves it **here**. `check.sh` does not come along in a split, so
an extracted repository started life with no automated checking at all —
which, by this project's own standard, is the extreme form of a check
that silently never runs. Each split-ready crate now carries its own
CI workflow for that reason.

*Drift* — none today: across the ten dependencies `hyprforge-clipboard`
shares with the root table, every version and feature set still matches,
verified by comparison rather than by eye. But nothing was watching. A
standalone crate cannot inherit from `[workspace.dependencies]`, so its
versions are hand-copied, and if the root bumps one nothing fails: cargo
resolves both, clippy stays silent, the tests stay green, and the suite
quietly builds two versions of the same dependency. That is "one place to
fix a shared thing" failing without a symptom, so `check.sh` now compares
every standalone-ready manifest against the workspace table in tier 1.

*Reversibility, which the plan asserts and nobody had checked* — at the
time, nothing had left this machine; there was still no remote. (All
nine components have been pushed since.) Undoing it is deleting
the branches and reverting the rejoin commits. Nothing is irreversible
until something is pushed and somebody clones it.

**The layer order is looser than it looked.** A git dependency on a crate
that still inherits from the workspace resolves fine, because cargo
clones the whole repository and brings the workspace root with it. So a
crate can be extracted before anything it depends on is prepared — only
the crate being extracted needs a self-contained manifest. `lock` and
`greet` needed no changes to `authui`, `look` or `paths` at all.

## What must stay true

- **One installable package is one crate directory is one repository.**
  This is the rule the rest of this document assumes and did not say, and
  it went wrong twice before anyone wrote it down — see "The shape a
  package has to have" below for what the two failures looked like.
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

## The shape a package has to have

`git subtree split --prefix=` takes exactly one directory. That single
fact decides the whole layout, and ignoring it is what produced two
repositories that were not the component they claimed to be.

**One installable package is one crate directory is one repository.** A
package that installs several *binaries* ships them as several `[[bin]]`
targets in that one crate — `hyprforge-displayd` already does it, with
`src/bin/displayd.rs` and `src/bin/displayctl.rs` beside one `lib.rs`.

The reason is narrow and easy to miss, which is why it has to be written
down rather than remembered:

> A library a component needs can live anywhere, because Cargo fetches
> it. **A binary cannot.** There is no dependency edge that makes a
> second executable appear in somebody's `$PATH`.

That is the whole difference between the two failures and the three
non-failures. `hyprforge-settings`'s repository does not contain
`windowrules`, `input`, `session`, `shortcuts` or `system` either — and
nobody noticed, because those are libraries its manifest pulls from
crates.io (from this repository by URL, at the time). `hyprforge-traymenu` is a *binary*, so when
`hyprforge-tray` was split without it, the published repository built a
daemon whose right-click spawns a program that is not in it. Same for
`hyprforge-clipboard` and `hyprforge-clipmenu`: a clipboard daemon with
no way to see the history.

Neither errored. The daemon logs a warning and carries on, exactly as
CLAUDE.md's "a sibling binary that is not installed is a logged warning"
says it should — which is why this was invisible for as long as it was.
Degrading well hid it.

So: before splitting anything, the question is not "is this crate
ready", it is **"does this crate directory contain every binary its
package installs"**. `check.sh`'s "Every package is a repository" step
asks it now, discovered from the PKGBUILD rather than from a list
someone has to remember to update.

### notif is a workspace, not a crate

notif arrived as a repository of its own (`git subtree add` of
`github.com/adamrpostjr/notif`) and sat at `notif/` as a nested
workspace, outside everything `split.sh` and `sync.sh` look at. On
2026-10-01 it became the tenth component: moved to
`crates/hyprforge-notif`, packaged as `hyprforge-notif` in
`packaging/arch` (replacing `notif-git`), given `--notif` in the
installer and CI of its own, and split to `hyprforge-suite/hyprforge-notif`.

It is still a nested workspace, excluded from this one — zbus picks its
runtime by feature, and the suite's tokio would unify into notif's
async-io and panic — and three things follow from that shape:

- **Its siblings are patched somewhere else.** Cargo reads `[patch]` only
  from the root of the workspace being built, and notif is its own root,
  so the root `[patch.crates-io]` never reaches it. `crates/.cargo/config.toml`
  does it instead, from outside the split directory, where the
  standalone repository will never see paths that point at nothing.
  `check.sh`'s "Siblings build from this checkout" asks the same two
  questions of that table as of the root one.
- **Its versions are its own.** The dependency-pin step skips it: its
  `[workspace.dependencies]` pins zbus to async-io where the root table
  carries tokio, and its own `Cargo.lock` keeps it reproducible.
- **The repository kept its history across a move.** `git subtree split`
  does not follow a directory rename, so a split of
  `crates/hyprforge-notif` begins at the move and shares nothing with
  the archived repository. A merge commit joins the two — the archived
  repository's last commit and the first split commit as parents — and
  is recorded here with `git subtree merge`, so later splits descend from
  it and the published history fast-forwards from what was archived.

## Releases: one version, cut by `tools/release.sh`

The whole suite shares one version — the libraries (which inherit
`[workspace.package].version`), the ten components, notif's crates and
the PKGBUILD's `pkgver`. The PKGBUILD already builds one tree into
eleven packages at one version, so "Hyprforge 0.2.0" should name one
tree too. Before this was decided (2026-10-08) only the libraries moved:
by v0.1.8 the clipboard and tray (published, so they had to) were at
0.1.8 and the other seven components and notif still said 0.1.0.

`tools/release.sh X.Y.Z` prints every step and does nothing; with
`--execute` it does them, stopping at the first failure:

1. `./check.sh`, in full.
2. Every version moved (`tools/versions.py --set`), the changelogs filed
   (`tools/changelog.py release`, which refuses an Unreleased section
   that is still an unedited draft), the AUR package re-rendered
   (`tools/aur.py --write`), and `./check.sh --quick` over the result.
   A bump that is not caret-compatible (0.1.x to 0.2.0) also moves every
   Hyprforge sibling requirement, because cargo silently ignores a
   `[patch]` whose version does not satisfy one.
3. Each component committed and pushed to its `main`.
4. The suite — pins, versions, changelog, packaging — in one
   "Release vX.Y.Z" commit, pushed.
5. The tag, pushed; `publish.yml` publishes the libraries.
6. Every component tagged at its pin; a GitHub Release for the suite and
   for each component whose pin moved since the previous tag (worked out
   before step 3, which moves them all).
7. The AUR push, printed for a person to run.

**Changelogs.** `CHANGELOG.md` here and one per component, Keep a
Changelog style. `tools/changelog.py draft` drafts from git: a library's
commits by path here, a component's from its own repository between the
pin at the previous tag and the pin now — this repository's log for a
component's path is only pin bumps.

**The AUR.** `packaging/arch/PKGBUILD` builds the checkout it sits in.
`tools/aur.py` renders `packaging/aur/hyprforge` from it with real
sources: the suite at `#tag=v$pkgver` and the ten component repositories,
each submodule pointed at makepkg's copy in `prepare`. A render only
builds when the tag carries the same packaging, which is why it is made
at release time: today's render at 0.1.8 fetches and prepares against
`v0.1.8`, then would fail in `package()`, because `v0.1.8` predates the
files helper and polkit policy the current packaging installs.

## Status

- [x] Foundation extracted so the leaves are genuinely leaves
- [x] Split packaging, so installing one component already works
- [x] `notif` merged as a nested workspace — the trial run of the
      mechanism, and proof that a differently-shaped project can share
      the theme without sharing a runtime
- [x] Publishing metadata on the foundation crates
- [x] Independent versions for the publishable crates — since replaced:
      every library inherits one workspace version, like iced's crates,
      because re-exports chain across them and a breaking change rarely
      stays in one
- [x] First split, `clipboard`, prepared and verified standalone
- [x] Step 4 judged: continue, with drift detection and per-repo CI added
- [x] `lock`, `greet`, `tray` and `settings` prepared and verified
      standalone
- [x] All five pushed: clipboard, lock, greet, tray, settings, all green CI
- [x] displayd, emojimenu and files split and pushed; CI committed after
      the fact and green on 2026-09-23, and `split.sh` now refuses a
      crate whose CI is not committed
- [x] Day-2 drift detection (`./sync.sh`), monorepo included
- [x] media (then called photos) prepared standalone and split (2026-09-27), the last of the
      nine packaged components; born with its CI committed, and without
      the `SUITE_READ` step, since it is published into a public suite
- [x] Moved to the `hyprforge-suite` organisation, every repository
      public (2026-09-27). One commit renamed the URL everywhere at
      once — the [patch] key, ninety-odd manifest lines, the scripts and
      the docs — after the transfers, so the old URL redirected the
      whole way through
- [x] First publish: all 31 libraries at 0.1.0, plus the clipboard and
      tray (components other components use as libraries), 2026-09-28
- [x] Components name their siblings by version, not by this
      repository's URL (2026-09-28): a clone of one fetches published
      crates and never this repository, and the root's
      `[patch.crates-io]` points every one back at `crates/` here
- [x] `CARGO_REGISTRY_TOKEN` set as an organisation secret readable by
      this repository only (2026-10-01), so
      `.github/workflows/publish.yml` can publish later versions on a tag
- [x] notif made the tenth component (2026-10-01): moved to
      `crates/hyprforge-notif`, its siblings by crates.io version with
      `crates/.cargo/config.toml` pointing them here, packaged as
      `hyprforge-notif` in `packaging/arch`, `--notif` in the installer,
      its own CI, and `split.sh`, `sync.sh` and `check.sh` taught that a
      component can be a workspace
- [x] notif's history joined to the archived repository, which was
      unarchived and transferred into the organisation as
      `hyprforge-notif` (the old URL redirects), and brought in step by
      `./sync.sh --push` as a fast-forward from its archived head, with
      green CI on the first run (2026-10-01)
- [x] First tagged release through that workflow, v0.1.1 (2026-10-03):
      the secret held a value (`CARGO_REGISTRY_TOKEN: ***` in the run),
      and the release hit exactly the limit `publish.yml` was prepared
      for — crates.io's burst of 30 updates ran out on the 31st crate
      with a 429. Re-running the job after the limit's stated time plus
      a minute per remaining crate skipped the 30 already on the index
      and published the last three; all 33 are at 0.1.1. Expect the same
      on every release while there are more than 30 crates
- [x] Components are submodules (2026-10-03, issue #2): after v0.1.5 was
      published and `sync.sh` reported all ten in step, each
      `crates/<component>` became a submodule of its repository at the
      same tree. `split.sh` and `sync.sh` retired; `check.sh` asks that
      every standalone crate is an initialised submodule, that every pin
      is on its repository's `main`, and that every component builds
      against the published libraries; CI and the PKGBUILD check out
      the submodules
- [x] Release tooling (2026-10-08): `tools/release.sh`,
      `tools/versions.py`, `tools/changelog.py`, `tools/aur.py`, the
      suite's `CHANGELOG.md`, and check.sh's "Versions agree", "Changelog
      has a section for the version", "AUR PKGBUILD matches
      packaging/arch" and the gated clean-chroot tier
- [ ] First lockstep release through `tools/release.sh`, and the first
      push to the AUR

## Staying in sync after the push (retired with `sync.sh`)

History: this is the problem submodules removed. There is no second copy
to fall behind now, and the one guard here worth keeping — building each
component against crates.io before it goes out — moved into `check.sh`.

Publishing with `split.sh` and a hand-typed `git push` was step 3's
answer to *getting a component out the door once*. It has no opinion on
day 2: nothing re-checks that a published repository still matches what
the monorepo would produce, so a component repository can fall behind
silently — it does not error, it just quietly stops being true, which is
the exact class of failure CLAUDE.md keeps naming.

`./sync.sh` is that check. With no arguments it is read-only: for every
standalone-ready crate (discovered the same way `check.sh`'s dependency-pin
step discovers its list) it computes, without pushing anything, the tree
`git subtree split` would produce right now and compares it against the
published repository's `main`. `./sync.sh --push` re-splits and pushes
only the components that have actually drifted, after refusing on a dirty
tree or a failing `./check.sh --quick` — publishing code that hasn't
passed tier 1 to nine repositories is worse than not publishing. It never
force-pushes: a component whose published history is not an ancestor of
the new split has diverged (an outside contributor, a direct push) and is
left for a human with `git subtree pull`, not resolved automatically.

It checks the monorepo first, and pushes that before any component. That
used to be the whole defence against one failure: while every standalone
manifest named this repository's URL for its siblings, a component
published while this repository was behind got built by its own CI against
siblings from whenever it was last pushed, and the failure named the
component rather than the stale dependency it resolved. It happened on the
first real use of `--push`: three components published, five green ticks,
and a red build twenty seconds later on a function sitting in the diff
that had just gone out.

Since 2026-09-28 components name their siblings by crates.io version, so
their CI never reads this repository, and the same failure arrives through
crates.io instead: a component calling a library function added since the
last release builds here, where `[patch.crates-io]` points it at
`crates/`, and fails on GitHub. Pushing the monorepo first cannot prevent
that — only a release can. So before `--push` sends a component out, it
extracts the split tree outside this workspace and runs `cargo check
--all-targets` on it the way its CI will, resolving fresh from crates.io,
and refuses the push if that fails. The monorepo still goes first, because
it is where every component's README says its code comes from.
