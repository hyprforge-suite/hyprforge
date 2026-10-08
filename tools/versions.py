#!/usr/bin/env python3
"""Every version the suite states, read and (at release time) written.

    tools/versions.py --check          exit 1 naming each version that must
                                       match the workspace's and does not;
                                       NOTE lines for drift not yet enforced
    tools/versions.py --set X.Y.Z      rewrite every one of them (release.sh)
    tools/versions.py --set X.Y.Z --dry-run

The suite is versioned as one (docs/design/repo-plan.md): the libraries
inherit `[workspace.package].version`, and the components, notif's
crates and the PKGBUILD's `pkgver` spell the same number by hand,
because a component's repository has no workspace to inherit from.
Spelled by hand, they drift with no symptom — which is the whole reason
this exists.

What `--check` enforces, and what it only reports. The components were
never bumped with the libraries before lockstep was decided (2026-10-08):
seven of them and notif still say 0.1.0 through eight library releases.
Failing on that would turn every run red over a fact only a release can
fix, and a check that is always red is a check nobody reads. So:

- always enforced: `pkgver`, the libraries inheriting rather than
  spelling a version, and the clipboard and tray — published to
  crates.io, so their number is already public;
- enforced per component once it has been released in lockstep, which
  is when its repository has a `v*` tag (tools/release.sh makes one):
  from then on that component drifting is a failure;
- otherwise a NOTE, so a green run still says what it did not hold.

A missing tag (a clone that never fetched the component's tags) can
only downgrade a failure to a note, never invent one.

`--set` also moves Hyprforge sibling requirements when the new version
is not caret-compatible with the old (0.1.x to 0.2.0): cargo silently
ignores a `[patch]` whose version does not satisfy the requirement, so
leaving `hyprforge-look = "0.1"` behind would build the published 0.1
instead of this checkout — the failure CLAUDE.md describes, which
check.sh's "Siblings build from this checkout" then catches.
"""

import argparse
import re
import subprocess
import sys
import tomllib
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
PKGBUILD = ROOT / "packaging" / "arch" / "PKGBUILD"
PUBLISHED_COMPONENTS = {"crates/hyprforge-clipboard", "crates/hyprforge-tray"}
NOTIF = "crates/hyprforge-notif"
VERSION_RE = re.compile(r'^(version\s*=\s*")([^"]+)(")', re.M)


def git(*args, cwd=ROOT):
    return subprocess.run(["git", *args], cwd=cwd, capture_output=True, text=True).stdout


def submodules():
    out = git("config", "-f", ".gitmodules", "--get-regexp", r"submodule\..*\.path")
    return [line.split()[1] for line in out.splitlines()]


def workspace_version():
    return tomllib.loads((ROOT / "Cargo.toml").read_text())["workspace"]["package"]["version"]


def manifests(component):
    """The manifests whose [package].version a component spells."""
    if component == NOTIF:
        return sorted(
            p for p in (ROOT / NOTIF).rglob("Cargo.toml")
            if "target" not in p.relative_to(ROOT / NOTIF).parts
            and "package" in tomllib.loads(p.read_text())
        )
    return [ROOT / component / "Cargo.toml"]


def stated():
    """[(label, version, enforced)] for everything that must match."""
    want = workspace_version()
    out = []
    pkgver = re.search(r"^pkgver=(\S+)", PKGBUILD.read_text(), re.M)
    out.append(("packaging/arch/PKGBUILD pkgver", pkgver.group(1) if pkgver else "none", True))
    subs = set(submodules())
    for crate in sorted((ROOT / "crates").iterdir()):
        rel = str(crate.relative_to(ROOT))
        if rel in subs or not (crate / "Cargo.toml").exists():
            continue
        version = tomllib.loads((crate / "Cargo.toml").read_text())["package"].get("version")
        if version != {"workspace": True}:
            out.append((f"{rel} (a library, which should inherit)", str(version), True))
    for component in sorted(subs):
        released = bool(git("tag", "--list", "v*", cwd=ROOT / component).strip())
        for manifest in manifests(component):
            version = tomllib.loads(manifest.read_text())["package"]["version"]
            if isinstance(version, dict):
                version = "inherits"
            out.append((
                str(manifest.relative_to(ROOT)),
                version,
                component in PUBLISHED_COMPONENTS or released,
            ))
    return want, out


def caret_compatible(old, new):
    o, n = [int(x) for x in old.split(".")], [int(x) for x in new.split(".")]
    if o[0] != n[0]:
        return False
    if o[0] == 0:
        return o[1] == n[1] if o[1] != 0 else o[2] == n[2]
    return True


def requirement(version):
    """The caret requirement a sibling names this version by: 0.2.0 -> 0.2."""
    major, minor, _ = version.split(".")
    return f"{major}.{minor}" if major == "0" else major


def set_version(new, dry_run):
    old = workspace_version()
    edits = []

    def bump_package(path):
        text = path.read_text()
        # The first `version =` under [package] or [workspace.package],
        # which is the first one in every manifest here — asserted, not
        # assumed.
        section = re.search(r"^\[(workspace\.)?package\]\n(?:(?!^\[).*\n)*?version\s*=", text, re.M)
        if not section:
            raise SystemExit(f"no package version line in {path.relative_to(ROOT)}")
        start = section.start()
        m = VERSION_RE.search(text, start)
        edits.append((path, text[: m.start(2)] + new + text[m.end(2):]))

    bump_package(ROOT / "Cargo.toml")
    for component in submodules():
        for manifest in manifests(component):
            bump_package(manifest)
    text = PKGBUILD.read_text()
    edits.append((PKGBUILD, re.sub(r"^pkgver=\S+", f"pkgver={new}", text, count=1, flags=re.M)))
    text = re.sub(r"^pkgrel=\S+", "pkgrel=1", edits[-1][1], count=1, flags=re.M)
    edits[-1] = (PKGBUILD, text)

    if not caret_compatible(old, new):
        req_old, req_new = requirement(old), requirement(new)
        sibling = re.compile(
            r'^(hyprforge-[a-z0-9-]+\s*=\s*(?:\{[^}\n]*?version\s*=\s*)?")([0-9.]+)(")', re.M
        )
        targets = [ROOT / "Cargo.toml"] + [m for c in submodules() for m in manifests(c)]
        merged = dict(edits)
        for path in targets:
            text = merged.get(path, path.read_text())

            def move(m):
                # Root [workspace.dependencies] spell a full version
                # ("0.1.0"); components spell the caret requirement ("0.1").
                v = m.group(2)
                if v == req_old or v.startswith(req_old + "."):
                    return m.group(1) + (new if v.count(".") == 2 else req_new) + m.group(3)
                return m.group(0)

            merged[path] = sibling.sub(move, text)
        edits = list(merged.items())

    for path, text in edits:
        if path.read_text() == text:
            continue
        if dry_run:
            print(f"would set {path.relative_to(ROOT)}")
        else:
            path.write_text(text)
            print(f"set {path.relative_to(ROOT)}")
    if not caret_compatible(old, new):
        print(f"{old} -> {new} is not caret-compatible: Hyprforge sibling requirements moved to {requirement(new)}")


def main():
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    mode = ap.add_mutually_exclusive_group(required=True)
    mode.add_argument("--check", action="store_true")
    mode.add_argument("--set", metavar="X.Y.Z")
    ap.add_argument("--dry-run", action="store_true")
    args = ap.parse_args()

    if args.set:
        if not re.fullmatch(r"\d+\.\d+\.\d+", args.set):
            raise SystemExit(f"{args.set!r} is not X.Y.Z")
        set_version(args.set, args.dry_run)
        return

    want, found = stated()
    bad = [(l, v) for l, v, enforced in found if enforced and v != want]
    notes = [(l, v) for l, v, enforced in found if not enforced and v != want]
    for label, version in bad:
        print(f"DRIFT {label} says {version}, the workspace {want}")
    for label, version in notes:
        print(f"NOTE {label} says {version} (not released in lockstep yet)")
    print(f"CHECKED {want} {len(found)} {len(notes)}")
    sys.exit(1 if bad else 0)


if __name__ == "__main__":
    main()
