#!/usr/bin/env python3
# Nothing shipped calls `hyprctl dispatch` in hyprlang syntax.
#
# Under a Lua config, `hyprctl dispatch X` is `hl.dispatch(X)`, so the
# hyprlang `hyprctl dispatch exit` is a Lua error and does nothing. The
# greeter's compositor config said exactly that, and every login wrote
# "hl.dispatch: expected a dispatcher" to /tmp/hyprforge-greet.log for
# nobody to read; imported idle commands said `dpms off` and the screen
# never turned off. A command line runs where nobody sees its exit code,
# so this reads the files instead.
#
# Scans the scripts and configs that run commands — Lua configs, shell,
# the installer, unit files, Rust string literals — and skips comment
# lines and lines that explain the old form. Prints CHECKED / PROBLEM.

import re
import subprocess

OLD = re.compile(r"""hyprctl\s+dispatch\s+(?!['"]?hl\.)([a-z_]+)""")
COMMENT = re.compile(r"^\s*(--|#|//|\*|///|//!)")

files = subprocess.run(
    ["git", "ls-files", "--recurse-submodules", "*.lua", "*.sh", "*.rs", "*.service", "*.desktop", "hyprforge"],
    capture_output=True, text=True, check=True,
).stdout.split()

problems, checked = [], 0
for path in files:
    try:
        lines = open(path, encoding="utf-8").read().splitlines()
    except (OSError, UnicodeDecodeError):
        continue
    checked += 1
    for n, line in enumerate(lines, 1):
        if COMMENT.match(line):
            continue
        m = OLD.search(line)
        if not m:
            continue
        # Prose naming the old form in backticks explains it, and the
        # translation and its tests name it on purpose.
        if line[: m.start()].endswith("`") or path.endswith(("ecosystem/src/idle.rs", "setup/src/tests.rs")):
            continue
        problems.append(f"{path}:{n} calls `hyprctl dispatch {m.group(1)}`, which a Lua config rejects — "
                        f"write `hyprctl dispatch \"hl.dsp.{m.group(1)}(...)\"`")

print(f"CHECKED {checked}")
for p in problems:
    print(f"PROBLEM {p}")
