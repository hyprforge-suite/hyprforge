---
name: backlog
description: "Read, add to and close items in Hyprforge's backlog and to-do list, docs/backlog.md. Use when the user asks what is on the backlog, the to-do list, what is left or what to do next; when work is deferred, parked, skipped or left for later; when the user floats an idea to keep; when something only the user can do comes up (a password, an account, a decision, watching the screen); when a backlog item lands; and at the end of a long task, before reporting what is left."
---

# The backlog

`docs/backlog.md` is the one list of what is left. Not memory files, not
a summary in chat: those go stale where nobody can see them, and the
list built from memory on 2026-10-11 still named two shipped Files
features as open.

Four sections, in this order (`tools/hf lint` checks the shape):

| Section | What goes there |
|---|---|
| Needs you | only the user can do it: an account, sudo, a decision, a live look, an install of something third-party. Ordered by what each unblocks. |
| Engineering | open work Claude can do unattended |
| Ideas | parked on purpose, not scheduled |
| Done recently | landed items, kept a few weeks, then pruned |

Each item is one bullet, continuation lines indented two spaces:

```
- **A title that names the thing.** Why it matters or what it waits on,
  with a path or command to start from. *Added 2026-10-11.*
```

## Reporting it

When asked what is on the backlog or what to do next:

1. Read `docs/backlog.md`.
2. **Check each Engineering and Needs-you item against the code before
   calling it open** — a grep for the feature, `git log -S`, the
   component's DESIGN.md. An item is a claim about the repository. If it
   already landed, move it to Done and say so; don't report it.
3. Answer by section, Needs-you first, and end with the one item most
   worth doing next and why.

## Changing it

- **Deferring or skipping something** mid-task, or finishing a task
  with loose ends: add an item in the same commit as the work, not later.
  A loose end mentioned only in the final message is lost once the
  conversation ends.
- **An idea the user floats** and doesn't ask to build now: Ideas, in
  their words where possible.
- **Landing an item**: move it to Done recently, with what closed it (a
  version, a commit). Prune Done items older than about a month.
- **Something needs the user**: Needs you, placed by what it unblocks,
  and say plainly in chat that it is waiting on them.
- Never delete an open item because it looks unlikely — ask, or move it
  to Ideas. Deleting one is the user's decision.

Commit the file with only the paths you wrote (`git add docs/backlog.md`),
alongside the work that changed it when there is some. A
memory file may point here; it must not hold its own copy of the list.
