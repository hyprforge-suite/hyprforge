#!/usr/bin/env python3
# docs/backlog.md keeps its shape.
#
# The backlog used to be a dozen memory files nobody but Claude could
# read, and the list built from them on 2026-10-11 still named two
# Files features as open that had shipped weeks before. One file in the
# repository fixes the reading; this keeps it parseable, so the
# `backlog` skill — and any agent brought in later — can move items
# between sections without guessing.
#
# The shape: five sections in order; every item a bullet with a bold
# title and the date it was added; every open item one type tag; and
# two clocks. A "Working on" item older than STALE_DAYS has stalled —
# on 2026-11-07 a whole hardening sweep (84 files) was found sitting
# uncommitted for two weeks with no item anywhere naming it, so being
# in progress is now something the list has to say, and keep saying
# only while it is true. A "Done recently" item older than STALE_DAYS
# is pruned: git history keeps it, and before this rule the Done
# section had grown to half the file. Prints CHECKED / PROBLEM.

import datetime
import re
from pathlib import Path

SECTIONS = ["Working on", "Needs you", "Engineering", "Ideas", "Done recently"]
# Every open item says what kind of work it is, so a reader can pull
# "all the bugs" without reading every paragraph. Ideas are parked and
# Done is history; neither needs one.
TAGGED = {"Working on", "Needs you", "Engineering"}
TAGS = {"bug", "feature", "check", "decision", "chore"}
STALE_DAYS = 14

today = datetime.date.today()
path = Path("docs/backlog.md")
problems = []
items = 0


def date_of(block: str, word: str) -> datetime.date | None:
    m = re.search(rf"\*{word} (\d{{4}}-\d{{2}}-\d{{2}})\.\*", block)
    return datetime.date.fromisoformat(m.group(1)) if m else None


if not path.is_file():
    problems.append("docs/backlog.md is missing")
else:
    text = path.read_text()
    found = re.findall(r"^## (.+)$", text, re.M)
    if found != SECTIONS:
        problems.append(f"sections are {found}, expected {SECTIONS} in that order")
    # Split on the headings so each item is judged by the section it is in.
    parts = re.split(r"^## (.+)$", text, flags=re.M)
    for name, body in zip(parts[1::2], parts[2::2]):
        # An item is a top-level bullet and the indented lines under it.
        for block in re.findall(r"^- .*(?:\n  .*)*", body, re.M):
            items += 1
            title = block.splitlines()[0][:70]
            m = re.match(r"- (?:\[(\w+)\] )?\*\*.+?\*\*", block, re.S)
            if not m:
                problems.append(f"no bold title: {title}")
                continue
            tag = m.group(1)
            if name in TAGGED and tag not in TAGS:
                problems.append(f"{name}: needs one tag of {sorted(TAGS)} before the title: {title}")
            if name not in TAGGED and tag is not None:
                problems.append(f"{name}: items here carry no tag: {title}")
            if date_of(block, "Added") is None:
                problems.append(f"no *Added YYYY-MM-DD.*: {title}")
            if name == "Working on":
                started = date_of(block, "Started")
                if started is None:
                    problems.append(f"Working on: no *Started YYYY-MM-DD.*: {title}")
                elif (today - started).days > STALE_DAYS:
                    problems.append(
                        f"Working on: started {started}, over {STALE_DAYS} days ago — "
                        f"finish it, or move it back with what is left: {title}"
                    )
            if name == "Done recently":
                done = date_of(block, "Done")
                if done is None:
                    problems.append(f"Done recently: no *Done YYYY-MM-DD.*: {title}")
                elif (today - done).days > STALE_DAYS:
                    problems.append(f"Done recently: done {done}, prune it (git keeps it): {title}")

print(f"CHECKED {items}")
for p in problems:
    print(f"PROBLEM {p}")
