#!/usr/bin/env python3
# Every Set up item is undone by uninstalling something.
#
# `./hyprforge --uninstall --<component>` offers to undo the Set up items
# that component made (`setup_ids_for` in the installer), and removing
# Settings undoes all of them. An item in no component's list is undone
# only when Settings goes: uninstalling Files left its GTK portal
# setting and its snapper permission behind, and uninstalling the polkit
# agent left the fingerprint PAM edit, because those three — and
# `wiring` — were never in a list. Nothing failed; the leftovers just
# stayed.
#
# So every id in hyprforge-setup's ITEMS must be in some component's
# list, or in ON_SETTINGS_ONLY with a reason. Prints CHECKED / PROBLEM.

import re
from pathlib import Path

# Items that belong to no one component, undone only with Settings.
ON_SETTINGS_ONLY = {
    "wiring": "the require lines serve every page",
    "idle-dispatch": "the idle commands are the user's own",
}

items = re.findall(r'Item::new\(\s*"([^"]+)"', Path("crates/hyprforge-setup/src/items/mod.rs").read_text())
installer = Path("hyprforge").read_text()
body = re.search(r"^setup_ids_for\(\) \{\n(.*?)^\}", installer, re.M | re.S)
mapped = set()
if body:
    for line in body.group(1).splitlines():
        m = re.match(r"\s*\w+\)\s+echo (.*?) ;;", line)
        if m:
            mapped.update(m.group(1).split())

problems = []
if not items:
    problems.append("found no Item::new(...) in crates/hyprforge-setup/src/items/mod.rs")
if not body:
    problems.append("found no setup_ids_for() in ./hyprforge")
for item in items:
    if item not in mapped and item not in ON_SETTINGS_ONLY:
        problems.append(f"Set up item `{item}` is in no component's setup_ids_for list in ./hyprforge, so uninstalling its component leaves it behind")
for gone in sorted(mapped - set(items)):
    problems.append(f"./hyprforge's setup_ids_for names `{gone}`, which hyprforge-setup no longer has")
for gone in sorted(set(ON_SETTINGS_ONLY) - set(items)):
    problems.append(f"setup_ids.py allow-lists `{gone}`, which hyprforge-setup no longer has")

print(f"CHECKED {len(items)}")
for p in problems:
    print(f"PROBLEM {p}")
