#!/usr/bin/env python3
# Every #[ignore] says why, at the test.
#
# `.config/nextest.toml` is what decides a test's tier, and the
# classification check holds every ignored test to exactly one. But a
# bare `#[ignore]` read in the source says nothing — it used to mean four
# different things at once, which is how one sweep pushed every Hyprland
# option into the live session (CLAUDE.md, "A live test that changes the
# compositor changes the one you are working in"). The reason names the
# tier and, for a live or visible test, the gate it waits behind:
# `#[ignore = "live (UDisks2): changes nothing"]`.
#
# Prints CHECKED / PROBLEM.

import re
import subprocess

BARE = re.compile(r"^\s*#\[ignore\]\s*$")

files = subprocess.run(
    ["git", "ls-files", "--recurse-submodules", "*.rs"], capture_output=True, text=True, check=True
).stdout.split()

problems, checked = [], 0
for path in files:
    try:
        lines = open(path, encoding="utf-8").read().splitlines()
    except (OSError, UnicodeDecodeError):
        continue
    for n, line in enumerate(lines, 1):
        if BARE.match(line):
            problems.append(f"{path}:{n} is a bare #[ignore] — say its tier: #[ignore = \"live (<gate>): changes nothing\"]")
        elif line.strip().startswith("#[ignore"):
            checked += 1

print(f"CHECKED {checked}")
for p in problems:
    print(f"PROBLEM {p}")
