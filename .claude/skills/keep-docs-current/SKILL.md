---
name: keep-docs-current
description: Bring Hyprforge's docs back in line with the code after a change. Use before committing any change that adds, removes or renames a component, crate, binary, tray item, Settings screen, source file, protocol or check.sh tier; that finishes something a doc calls deferred, planned or not started; or that changes a count a doc states. Also use when asked to update, audit or sync the docs.
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
