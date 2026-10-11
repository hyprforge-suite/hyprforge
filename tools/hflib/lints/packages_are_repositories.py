#!/usr/bin/env python3
# Every package is a repository.
#
# Moved verbatim from check.sh, where it ran as an inline heredoc;
# `tools/hf lint` runs it from the repository root and reads its
# CHECKED / PROBLEM / SKIP lines.
#
# The rule this exists for is in docs/design/repo-plan.md under "The shape a package
# has to have", and in CLAUDE.md: one installable package is one crate
# directory is one repository, because a library can be fetched and a
# binary cannot.
#
# It was rediscovered twice. `hyprforge-tray` shipped without
# `hyprforge-traymenu` and `hyprforge-clipboard` without
# `hyprforge-clipmenu`, and neither errored — the daemon logs a warning
# about the absent sibling and carries on, which is correct behaviour
# that happened to hide a published repository being incomplete. A rule
# nothing checks is a rule that gets rediscovered, so this asks.
#
# Discovered from the PKGBUILD, not from a list here: the next package
# somebody adds is covered without anyone remembering to add it, the same
# way the dependency-pin step above discovers its own crates by shape.

import re
import sys
import tomllib
from pathlib import Path

PKGBUILD = Path("packaging/arch/PKGBUILD")
if not PKGBUILD.is_file():
    print("SKIP no packaging/arch/PKGBUILD to read")
    sys.exit(0)

text = PKGBUILD.read_text()

# The split packages, and the binaries each one installs. A `package_x`
# function body is read for `install -Dm755 .../<name>` and friends; what
# matters is which `hyprforge-*` executables end up in the package.
names = re.search(r"pkgname=\((.*?)\)", text, re.S)
packages = names.group(1).split() if names else []

problems = []
checked = 0
for package in packages:
    # The umbrella package installs nothing of its own.
    if package == "hyprforge":
        continue
    body = re.search(
        rf"^package_{re.escape(package)}\(\)\s*\{{(.*?)^\}}",
        text,
        re.S | re.M,
    )
    if not body:
        problems.append(f"{package}: no package_{package}() in the PKGBUILD")
        continue

    # Binaries, as the PKGBUILD's own `_bin` helper installs them, plus
    # any spelled out as a path. Both forms, because matching only the
    # one that happens to be used today is how this check silently stops
    # checking: the first version matched `usr/bin/...` and reported
    # "0 packages" in green.
    # Any name, not only hyprforge-*: notif's binaries are `notifd` and
    # `notifctl`, kept so users' keybinds and layer rules keep working,
    # and matching only the prefix would have reported its package as
    # installing nothing.
    binaries = sorted(
        set(re.findall(r"^\s*_bin\s+([a-z0-9][a-z0-9-]*)", body.group(1), re.M))
        | set(re.findall(r"usr/bin/([a-z0-9][a-z0-9-]*)", body.group(1)))
    )
    if not binaries:
        problems.append(f"{package}: installs no binary this check can see")
        continue

    checked += 1

    # Which crate directory declares each binary. One directory for the
    # whole package is the rule; several means the split cannot produce
    # a repository that is the package.
    # A nested workspace's binaries live in its members, and the
    # directory that becomes the repository is the workspace's, so each
    # member is credited to the top-level crate directory it sits in.
    owners = {}
    for top in sorted(Path("crates").glob("*/Cargo.toml")):
        top_data = tomllib.loads(top.read_text())
        manifests = [top] + [
            top.parent / member / "Cargo.toml"
            for member in top_data.get("workspace", {}).get("members", [])
        ]
        for manifest in manifests:
            data = tomllib.loads(manifest.read_text())
            declared = {b.get("name") for b in data.get("bin", [])}
            if not declared:
                # A crate with no [[bin]] but a src/main.rs builds one named
                # after the package.
                if (manifest.parent / "src" / "main.rs").is_file():
                    declared = {data.get("package", {}).get("name")}
            for binary in binaries:
                if binary in declared:
                    owners.setdefault(binary, top.parent.name)

    missing = [b for b in binaries if b not in owners]
    if missing:
        problems.append(f"{package}: no crate declares {', '.join(missing)}")
        continue

    directories = sorted(set(owners.values()))
    if len(directories) > 1:
        problems.append(
            f"{package}: installs {', '.join(binaries)} from {len(directories)} crate "
            f"directories ({', '.join(directories)}) — a component's repository is "
            f"one directory (one submodule), so this package cannot be one repository"
        )

# And every package — the metapackage too — installs a licence. None
# did until 0.1.1: the suite shipped as MIT with the text in no package
# at all, and notif-git, which it replaced, had carried its own.
for package in packages:
    body = re.search(
        rf"^package_{re.escape(package)}\(\)\s*\{{(.*?)^\}}",
        text,
        re.S | re.M,
    )
    if body and not re.search(r"^\s*_license\b", body.group(1), re.M):
        problems.append(f"{package}: installs no licence (call _license in package_{package})")

# The daemons come on with their packages: each ships a systemd preset
# and an `install=` scriptlet that applies it. Every piece of that is a
# name in one file that must match a name in another, and none of it
# fails at build time — makepkg packages a scriptlet naming a unit that
# does not exist, and `systemctl --global preset` on a unit nobody
# installed exits quietly. So, per package:
#   * every `install=` names a file next to the PKGBUILD;
#   * every `_unit <src> <cmd>` names a unit file that exists, and starts
#     a binary this same package installs (the units shipped for three
#     releases starting ~/.cargo/bin, which the packages never install);
#   * every `_preset <name> <unit>` is named after its package and
#     enables a unit this package ships — and the package has a
#     scriptlet, or the preset is policy nobody ever applies;
#   * a scriptlet's `_unit=` is a unit this package ships, and it only
#     presets in post_install: `preset` on upgrade re-enables a unit the
#     administrator disabled.
# And no scriptlet in packaging/arch is left that no package names.
named_scripts = set()
for package in packages:
    body = re.search(
        rf"^package_{re.escape(package)}\(\)\s*\{{(.*?)^\}}",
        text,
        re.S | re.M,
    )
    if not body:
        continue
    body = body.group(1)
    package_bins = set(re.findall(r"^\s*_bin\s+([a-z0-9][a-z0-9-]*)", body, re.M))

    units = set()
    for src, cmd in re.findall(r'^\s*_unit\s+(\S+)\s+("[^"]*"|\S+)', body, re.M):
        name = src.rsplit("/", 1)[-1]
        units.add(name)
        if not Path(src).is_file():
            problems.append(f"{package}: _unit names {src}, which does not exist")
        first = cmd.strip('"').split()[0]
        if first not in package_bins:
            problems.append(
                f"{package}: {name} would start /usr/bin/{first}, which this package does not install"
            )
    # A unit installed the old way, verbatim, keeps whatever ExecStart
    # its source had — the exact bug _unit exists to prevent.
    for verbatim in re.findall(r"usr/lib/systemd/user/([\w.@-]+\.service)", body):
        problems.append(f"{package}: installs {verbatim} verbatim — use _unit so ExecStart points at /usr/bin")

    presets = re.findall(r"^\s*_preset\s+(\S+)\s+(\S+)", body, re.M)
    for name, unit in presets:
        if name != package:
            problems.append(f"{package}: preset 80-{name}.preset is not named after its package")
        if unit not in units:
            problems.append(f"{package}: its preset enables {unit}, which this package does not ship")

    script = re.search(r"^\s*install=(\S+)", body, re.M)
    if presets and not script:
        problems.append(f"{package}: ships a preset but no install= scriptlet to apply it")
    if not script:
        continue
    named_scripts.add(script.group(1))
    path = PKGBUILD.parent / script.group(1)
    if not path.is_file():
        problems.append(f"{package}: install={script.group(1)}, which is not in {PKGBUILD.parent}")
        continue
    script_text = path.read_text()
    unit = re.search(r"^_unit=(\S+)", script_text, re.M)
    if presets and not unit:
        problems.append(f"{package}: {path.name} names no _unit= to preset")
    if unit and unit.group(1) not in units:
        problems.append(f"{package}: {path.name} presets {unit.group(1)}, which this package does not ship")
    upgrade = re.search(r"^post_upgrade\(\)\s*\{(.*?)^\}", script_text, re.S | re.M)
    if upgrade and "preset" in upgrade.group(1):
        problems.append(f"{package}: {path.name} presets in post_upgrade, which re-enables what an administrator disabled")

for orphan in sorted(p.name for p in PKGBUILD.parent.glob("*.install")):
    if orphan not in named_scripts:
        problems.append(f"{orphan}: no package names it with install=")

print(f"CHECKED {checked}")
for problem in problems:
    print(f"PROBLEM {problem}")
