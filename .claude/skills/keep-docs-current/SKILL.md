---
name: keep-docs-current
description: Bring Hyprforge's docs back in line with the code after a change. Use before committing any change that adds, removes or renames a component, crate, binary, tray item, Settings screen, source file, protocol or check.sh tier; that finishes something a doc calls deferred, planned or not started; that changes a count a doc states; or that changes how something works — how it is built, resolved, released, ordered, installed or talked to — even when no name or number moves. Also use when asked to update, audit or sync the docs, which runs the whole-repository audit.
---

# Keep the docs current

Hyprforge's docs make specific claims — counts, file names, protocols,
what is and isn't done — and nothing fails when one stops being true.
In one session all of these were found stale, each written correctly
once and left behind by a later change:

- README: "Five of these directories" are repositories, when eight were.
- The tray's published README: serves `com.canonical.dbusmenu`, and a
  source file called dbusmenu.rs — the protocol dropped and the file
  deleted long before.
- The vision doc: night light has "no way yet to deep-link" — the deep
  link existed, with a test.
- "four icons" / "four toggles" in five places after a fifth was added.

Those are all nouns and numbers, and the steps below were first written
to find nouns and numbers. A later kind survived them: components
stopped naming their siblings by this repository's git URL and started
depending on crates.io versions. Nothing was renamed and no count moved,
so every search came back clean — while `repo-plan.md` and `sync.sh`
still said the monorepo is pushed first *because every component's CI
resolves its siblings from here*. A doc that explains a reason goes
stale when the mechanism under the reason changes, and it is the one
kind of claim a name search can never find.

`check.sh`'s **"Docs name things that exist"** step catches the part a
machine can: source paths that don't exist; the README's repository
table and its spelled-out counts against the crates that are actually
standalone; CLAUDE.md's gated-tier count against `check.sh` itself. This skill is the part it can't.

## 1. Say what the change made true or false

Before searching, write down the nouns and numbers the change touched:
what was added, removed, renamed, finished, or now counts differently.
"Added a tray item" means: the tray's item count, its list of item
names, the Tray screen's toggle count, the list of backends the daemon
uses, and whatever called that item "deferred".

If the change altered **how** something works, rather than what exists,
also write down what the old mechanism made true and what was done *on
account of* it. "Components depend on crates.io, not the suite's git
URL" means: a component's CI no longer reads this repository; anything
ordered "monorepo first so components see it" has lost its reason; a
new library API now has to be *published* before a component using it
is pushed, which nothing may yet enforce; and every README that tells a
contributor where siblings come from is wrong. Each of those is a
sentence somewhere, and none of them contains the word that changed.

Also ask the reverse: does the new mechanism need a rule the old one
made unnecessary? If so, saying it in a doc is not enough — note it as
a gap for the person you are working with, or a check for step 6.

## 2. Look where claims live

| Where | What it claims | Why it matters |
|---|---|---|
| `crates/<crate>/README.md` | what the crate is, its files, protocols | **Published** — it is the front page of the split repository |
| `crates/<crate>/Cargo.toml` `description` | one-line summary | Published the same way, and usually lists features |
| `README.md` | workspace layout, the repository table, counts | The only place that says the other repositories exist |
| `hyprforge-vision.md` | component inventory (the Status column), "Where things stand" | Its "as of" date is part of the claim — update it with the facts |
| `repo-plan.md` | the Status checklist | A split, a new check, a finished step |
| `CLAUDE.md` | rules, the tier count in "Checking your work", the layering diagram | Add a rule only for a mistake that was not obvious in advance |
| `//!` module docs of every file touched | what the module does and does not do | Lists of items, backends and "the four …" live here too |
| Comments and messages in `check.sh`, `hyprforge`, `.gitmodules` | why a step exists, why it runs in this order | These explain the mechanism they guard, and print it to whoever runs them — a stale reason here is believed |
| `.github/workflows/*.yml`, and each component's own CI | what is built from where, which secret, which crates are excluded | Comments here state the build and release model outright |
| A component README's build and install sections | where siblings come from, what a clone fetches | The first thing an outside contributor follows |
| `packaging/arch/` (`PKGBUILD` and its comments) | which package ships which binaries, from which source | Packaging granularity is a claim about the suite's shape |

## 3. Search, don't recall

```sh
# Counts: every spelled-out number next to the thing that changed.
grep -rniE '\b(two|three|four|five|six|seven|eight|nine|ten|eleven|twelve)\b[^.]{0,20}(icons?|items?|components?|screens?|tiers?|toggles?|repositories)' \
  --include='*.md' --include='*.rs' --include='*.toml' --include='*.sh' . | grep -v '^./target'

# The old name, file or protocol, anywhere.
grep -rn '<old-name>' --include='*.md' --include='*.rs' --include='*.toml' . | grep -v '^./target'

# Anything that says the thing you just did isn't done.
grep -rniE 'not yet|no way yet|deferred|planned|not started|TODO' --include='*.md' . \
  | grep -i '<component>'
```

For a mechanism change, search for the *reasoning*, using the
consequences you wrote down in step 1 as the vocabulary:

```sh
# Sentences that justify something by the mechanism. Swap in its words.
grep -rniE '(resolves?|fetch(es)?|builds?|comes?) (its |their )?(siblings?|dependenc)[^.]{0,60}from' \
  --include='*.md' --include='*.rs' --include='*.toml' --include='*.sh' --include='*.yml' . | grep -v '^./target'
grep -rniE '(so|because|which is why)[^.]{0,80}(first|before|after|goes)' \
  --include='*.md' --include='*.sh' --include='*.yml' . | grep -v '^./target'
```

These return far more than the change touched; that is the point. Read
each hit and ask whether its *because* is still true.

They will not find a doc that simply *describes the old mechanism*,
because a description need not give a reason. Search for the old
mechanism in its own words as well — whatever the code used to say, and
how a doc would put it in prose. For the crates.io change that was the
URL, and both stale descriptions only matched on that:

```sh
grep -rniE 'URL for every sibling|siblings? by (git|URL)|git = ' \
  --include='*.md' --include='*.sh' --include='*.toml' . | grep -v '^./target'
```

Tested against that change: the reasoning searches above find `sync.sh`
and miss `repo-plan.md` and CLAUDE.md; this one finds those two. Run
both kinds.

grep reads one line at a time, and prose wraps: "Twelve gated" ending
one line and "tiers" starting the next is invisible to the first search,
which is exactly how CLAUDE.md said twelve tiers while `check.sh` had
fourteen. In the docs you are already editing, also search for the bare
number words and read what follows them.

Read each hit in context. A historical sentence ("the first split
produced …") is fine as it stands; a claim about now is not.

## 4. Check each claim against the code

Confirm a claim by reading the code, not by remembering it — the rule
in CLAUDE.md that an instruction is not evidence applies to a doc too.
The night-light "no deep link" claim was disproved by one grep for
`"night-light"` in `hyprforge-settings/src/main.rs`.

## 5. Fix it in the same commit

A doc fix belongs with the change that made it necessary, so the
commit message and the docs describe the same thing. Stage the paths
you edited by name, never `git add -A`.

## 6. Move what can be mechanical into check.sh

If you found a claim a script could have checked — a count derived
from files, a name that must exist — extend the "Docs name things that
exist" step instead of relying on this skill next time. Plant the
error once to prove the check fails on it, then take it back out.

## The whole-repository audit

Steps 1–6 start from one change and so only find what that change
broke. Drift that an earlier change left behind stays found by nobody.
When asked to audit, update or sync the docs without a specific change
in mind, work the other way round: start from the docs, not the diff.

1. **Find the changes nobody followed up.** Read `git log` since the
   last commit that touched docs broadly ("Bring every doc back in line
   with the code" is the usual message). For each commit, do step 1
   above in a sentence — especially for mechanism changes, which are
   the ones most likely to have been missed.
2. **Go document by document** through the table in step 2, crate
   README by crate README. For every sentence that claims something
   about *now* — a count, a name, a status, a reason, an order, where
   something comes from — check it against the code. Historical
   sentences ("the first split produced …", a rule's story in
   CLAUDE.md) are left alone unless they are phrased in the present
   tense about something that is no longer so.
3. **Report before fixing.** List each stale claim with its file and
   line, what it says, what is true, and the code that shows it. Group
   them by the change that made them stale. Some will turn out to be
   gaps in the code rather than the doc — the doc describing a guard
   that no longer guards anything — and those are decisions for the
   person you are working with, not edits.
4. **Then fix, and do step 6** for every finding a script could have
   caught.

An audit is large; work through it crate by crate rather than holding
every doc in mind at once, and say which ones you have not reached yet
if you stop.
