---
name: backlog
description: "Read, add to and close items in Hyprforge's backlog and to-do list, docs/backlog.md. Use when the user asks what is on the backlog, the to-do list, what is left or what to do next; when work is deferred, parked, skipped or left for later; when the user floats an idea to keep; when something only the user can do comes up (a password, an account, a decision, watching the screen); when a backlog item lands; and at the end of a long task, before reporting what is left."
---

# The backlog

`docs/backlog.md` is the one list of what is left. Not memory files, not
a summary in chat: those go stale where nobody can see them, and the
list built from memory on 2026-10-11 still named two shipped Files
features as open.

Five sections, in this order (`tools/hf lint` checks the shape):

| Section | What goes there |
|---|---|
| Working on | started and not finished; what is left; `*Started …*`, stale after 14 days |
| Needs you | only the user can do it: an account, sudo, a decision, a live look, an install of something third-party. Ordered by what each unblocks. |
| Engineering | open work Claude can do unattended |
| Ideas | parked on purpose, not scheduled; no tag |
| Done recently | landed, with what closed it; `*Done …*`, pruned after 14 days; no tag |

Each item is one bullet, continuation lines indented two spaces, and
every open item starts with one type tag:

```
- [bug] **A title that names the thing.** Why it matters or what it
  waits on, with a path or command to start from. *Added 2026-11-07.*
```

`bug` something is wrong · `feature` something new · `check` built but
never tried for real (usually the user's screen or hardware) ·
`decision` the user's call · `chore` tooling, docs, housekeeping. The
section says *who* can do it; the tag says *what kind* of work it is.

## Reporting it

When asked what is on the backlog or what to do next:

1. Read `docs/backlog.md`.
2. **Check each Engineering and Needs-you item against the code before
   calling it open** — a grep for the feature, `git log -S`, the
   component's DESIGN.md. An item is a claim about the repository. If it
   already landed, move it to Done and say so; don't report it.
3. Answer by section — Working on, then Needs you — and end with the one item most
   worth doing next and why.

## Changing it

- **Starting an item**: move it to Working on first, with `*Started
  <today>.*`, and commit that — another agent reading the list must see
  it is taken. Keep its "what is left" true as you go. If you stop
  before it is done, move it back to its section with what is left; an
  item does not sit in Working on for an agent that has gone.
- **Finding work nobody listed** (an uncommitted diff, a half-made
  branch): add it to Working on before touching it, so it exists
  somewhere other than the working tree.
- **Deferring or skipping something** mid-task, or finishing a task
  with loose ends: add an item in the same commit as the work, not later.
  A loose end mentioned only in the final message is lost once the
  conversation ends.
- **An idea the user floats** and doesn't ask to build now: Ideas, in
  their words where possible.
- **Landing an item**: move it to Done recently, untagged, with what
  closed it (a version, a commit) and `*Done <today>.*`. Delete Done
  items past 14 days — the lint fails on them, and git keeps them.
- **Something needs the user**: Needs you, placed by what it unblocks,
  and say plainly in chat that it is waiting on them.
- Never delete an open item because it looks unlikely — ask, or move it
  to Ideas. Deleting one is the user's decision.

Commit the file with only the paths you wrote (`git add docs/backlog.md`),
alongside the work that changed it when there is some. A
memory file may point here; it must not hold its own copy of the list.
