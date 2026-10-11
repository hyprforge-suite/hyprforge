#!/usr/bin/env python3
# docs/backlog.md keeps its shape.
#
# The backlog used to be a dozen memory files nobody but Claude could
# read, and the list built from them on 2026-10-11 still named two
# Files features as open that had shipped weeks before. One file in the
# repository fixes the reading; this keeps it parseable, so the
# `backlog` skill can move items between sections without guessing:
# the four sections in order, and every item a bullet with a bold title
# and the date it was added. Prints CHECKED / PROBLEM.

import re
from pathlib import Path

SECTIONS = ["Needs you", "Engineering", "Ideas", "Done recently"]

path = Path("docs/backlog.md")
problems = []
items = 0
if not path.is_file():
    problems.append("docs/backlog.md is missing")
else:
    text = path.read_text()
    found = re.findall(r"^## (.+)$", text, re.M)
    if found != SECTIONS:
        problems.append(f"sections are {found}, expected {SECTIONS} in that order")
    # An item is a top-level bullet and the indented lines under it.
    for block in re.findall(r"^- .*(?:\n  .*)*", text, re.M):
        items += 1
        title = block.splitlines()[0][:70]
        if not re.match(r"- \*\*.+?\*\*", block, re.S):
            problems.append(f"no bold title: {title}")
        if not re.search(r"\*Added \d{4}-\d{2}-\d{2}\.\*", block):
            problems.append(f"no *Added YYYY-MM-DD.*: {title}")

print(f"CHECKED {items}")
for p in problems:
    print(f"PROBLEM {p}")
