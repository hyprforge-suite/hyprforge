#!/usr/bin/env python3
# Siblings build from this checkout.
#
# Moved verbatim from check.sh, where it ran as an inline heredoc;
# `tools/hf lint` runs it from the repository root and reads its
# CHECKED / PROBLEM / SKIP lines.
#
# Every component names its Hyprforge siblings by version, as
# crates.io dependencies, so the manifest works unchanged both here and
# as the root of its own repository. The root `[patch.crates-io]` table
# is what points each name back at `crates/` inside this workspace.
#
# Two ways that silently stops being true, and both build fine:
#
# - A name missing from the table. Cargo fetches the sibling from
#   crates.io and builds against the published copy, so a fix here never
#   reaches the component or the tests that use it. Seven were missing at
#   once when these were still git dependencies, and Files ran its tests
#   against days-old copies of three crates without anything failing.
# - A patch whose local version no longer *satisfies* the component's
#   requirement: a component asking for "0.1" while `crates/` is at
#   0.2.0. Cargo ignores a patch that does not match — with a warning
#   nobody reads — and fetches 0.1 from the index. Same failure, new door.
#
# So this asks both, and additionally that nothing in the resolved tree
# comes from a registry or a git source under a hyprforge- name.

import re
import tomllib
from pathlib import Path

root = tomllib.loads(Path("Cargo.toml").read_text())
patched = root.get("patch", {}).get("crates-io", {})
local_version = {}
for manifest in Path("crates").glob("*/Cargo.toml"):
    pkg = tomllib.loads(manifest.read_text()).get("package")
    if pkg is None:
        continue  # a nested workspace's root: its members are not siblings
    v = pkg.get("version")
    if isinstance(v, dict):
        v = root["workspace"]["package"]["version"]
    local_version[pkg["name"]] = v

def satisfies(req, have):
    """Cargo's default (caret) requirement, which is all this suite uses."""
    req = req.strip().lstrip("^")
    want = [int(x) for x in req.split(".")]
    got = [int(x) for x in have.split(".")]
    want += [0] * (3 - len(want))
    # The leftmost non-zero component is the one that must match exactly.
    for i in range(3):
        if want[i] != 0 or i == 2:
            break
    if got[:i + 1] != want[:i + 1]:
        return False
    return got >= want

named = {}
problems = []
for manifest in sorted(Path("crates").glob("*/Cargo.toml")):
    data = tomllib.loads(manifest.read_text())
    tables = [data.get(k, {}) for k in ("dependencies", "dev-dependencies", "build-dependencies")]
    for target in data.get("target", {}).values():
        tables += [target.get(k, {}) for k in ("dependencies", "dev-dependencies", "build-dependencies")]
    for table in tables:
        for name, spec in table.items():
            if not name.startswith("hyprforge-"):
                continue
            if isinstance(spec, dict) and ("path" in spec or spec.get("workspace") is True):
                continue
            if isinstance(spec, dict) and "git" in spec:
                problems.append(f"{manifest.parent.name} names {name} by git; components depend on published versions now")
                continue
            req = spec if isinstance(spec, str) else spec.get("version", "")
            named.setdefault(name, set()).add(manifest.parent.name)
            if name not in patched:
                problems.append(f"{name} is required by {', '.join(sorted(named[name]))} but not in [patch.crates-io] — it builds from crates.io, not from crates/")
            elif name in local_version and not satisfies(req, local_version[name]):
                problems.append(f"{manifest.parent.name} asks for {name} \"{req}\" but crates/ is at {local_version[name]}, so cargo ignores the patch and fetches the published copy")

# A nested workspace (hyprforge-notif) is its own root, so the table
# above never reaches it: its siblings are patched by
# crates/.cargo/config.toml instead, and its requirements live in its
# own [workspace.dependencies]. Same two questions, other table.
config = Path("crates/.cargo/config.toml")
config_patched = tomllib.loads(config.read_text()).get("patch", {}).get("crates-io", {}) if config.is_file() else {}
for manifest in sorted(Path("crates").glob("*/Cargo.toml")):
    data = tomllib.loads(manifest.read_text())
    if "package" in data or "workspace" not in data:
        continue
    for name, spec in data["workspace"].get("dependencies", {}).items():
        if not name.startswith("hyprforge-"):
            continue
        if isinstance(spec, dict) and "path" in spec:
            problems.append(f"{manifest.parent.name} names {name} by path; it would point at nothing in its own repository")
            continue
        req = spec if isinstance(spec, str) else spec.get("version", "")
        named.setdefault(name, set()).add(manifest.parent.name)
        if name not in config_patched:
            problems.append(f"{name} is required by {manifest.parent.name} but not patched in crates/.cargo/config.toml — it builds from crates.io, not from crates/")
        elif name in local_version and not satisfies(req, local_version[name]):
            problems.append(f"{manifest.parent.name} asks for {name} \"{req}\" but crates/ is at {local_version[name]}, so cargo ignores the patch and fetches the published copy")

print(f"CHECKED {len(named)}")
if not named:
    problems.append("no component names a hyprforge- sibling by version — either the manifests changed shape or this check is looking in the wrong place")
for p in dict.fromkeys(problems):
    print(f"PROBLEM {p}")
