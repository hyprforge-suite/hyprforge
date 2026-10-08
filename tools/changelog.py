#!/usr/bin/env python3
"""Draft release notes from git, and file them into the changelogs.

    tools/changelog.py draft [--since vX.Y.Z] [--until REV]
        print a draft of what changed, suite and components, to stdout
    tools/changelog.py release X.Y.Z [--since vX.Y.Z] [--date YYYY-MM-DD] [--write]
        turn the suite's "Unreleased" section into "X.Y.Z", and prepend a
        drafted "X.Y.Z" section to every component whose pin moved;
        without --write, prints what it would write and changes nothing
    tools/changelog.py components-changed [--since vX.Y.Z] [--until REV]
        print the components whose pin moved, one path per line — what
        tools/release.sh creates GitHub Releases for

A draft, not a changelog. Commit subjects here are written to be read,
which is why drafting from them is worth doing at all, but they say what
a commit did and not whether a user would notice: the person cutting the
release sorts the draft into Added / Changed / Fixed and drops what only
matters to contributors. That step is why this never writes a section
the release script then publishes unread — `release.sh` stops for it.

Two sources, because the suite has two kinds of history. A library's
commits are this repository's own, found by path under `crates/<lib>`.
A component's are in its own repository: what this repository records
is only the pin, so its changes are the submodule's log between the pin
at the previous tag and the pin now. Reading this repository's log for
`crates/<component>` would list "Move four component pins…" and nothing
the component actually did.
"""

import argparse
import datetime
import re
import subprocess
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
SUITE_CHANGELOG = ROOT / "CHANGELOG.md"
UNRELEASED = "## [Unreleased]"


def git(*args, cwd=ROOT):
    return subprocess.run(
        ["git", *args], cwd=cwd, capture_output=True, text=True, check=True
    ).stdout


def last_tag():
    """The newest `v*` tag reachable from HEAD."""
    return git("describe", "--tags", "--abbrev=0", "--match", "v*").strip()


def submodules():
    out = git("config", "-f", ".gitmodules", "--get-regexp", r"submodule\..*\.path")
    return [line.split()[1] for line in out.splitlines()]


def pin(rev, path):
    """The commit `path` was pinned to at `rev`, or None if it was not a
    submodule then (a component added since the last release)."""
    out = git("ls-tree", rev, path).split()
    return out[2] if len(out) >= 3 and out[1] == "commit" else None


def subjects(*args, cwd=ROOT):
    out = git("log", "--no-merges", "--format=%s", *args, cwd=cwd)
    return [s for s in out.splitlines() if s]


def library_dirs():
    subs = set(submodules())
    return sorted(
        str(p.relative_to(ROOT))
        for p in (ROOT / "crates").iterdir()
        if p.is_dir() and str(p.relative_to(ROOT)) not in subs
    )


def component_changes(since, until):
    """{path: [subjects]} for every component whose pin moved."""
    changes = {}
    for path in submodules():
        old, new = pin(since, path), pin(until, path)
        if old == new:
            continue
        if old is None:
            changes[path] = ["First release as a component of the suite."]
            continue
        try:
            changes[path] = subjects(f"{old}..{new}", cwd=ROOT / path) or [
                "Pin moved with no new commits (a rewind?) — check by hand."
            ]
        except subprocess.CalledProcessError:
            # The old pin is not in this clone of the component: fetch it,
            # or this would draft an empty section for a component that
            # changed.
            changes[path] = [f"Could not read {old[:7]}..{new[:7]} — run `git -C {path} fetch` and draft again."]
    return changes


def library_changes(since, until):
    changes = {}
    for path in library_dirs():
        found = subjects(f"{since}..{until}", "--", path)
        if found:
            changes[path] = found
    other = subjects(
        f"{since}..{until}",
        "--",
        ".",
        *(f":(exclude){p}" for p in library_dirs() + submodules()),
    )
    # A commit that touched both a library and the root is listed under
    # the library; the suite list keeps only what is not already said.
    said = {s for v in changes.values() for s in v}
    return changes, [s for s in other if s not in said]


def bullets(items):
    return "\n".join(f"- {s}" for s in dict.fromkeys(items))


def draft_suite(since, until):
    libs, suite = library_changes(since, until)
    comps = component_changes(since, until)
    parts = [
        "<!-- Drafted by tools/changelog.py from git log "
        f"{since}..{until}. Sort into Added / Changed / Fixed and drop "
        "what only matters to contributors before releasing. -->"
    ]
    if suite:
        parts.append("### The suite\n\n" + bullets(suite))
    for path, items in libs.items():
        parts.append(f"### `{Path(path).name}`\n\n" + bullets(items))
    for path, items in comps.items():
        parts.append(f"### `{Path(path).name}` (component)\n\n" + bullets(items))
    if len(parts) == 1:
        parts.append("Nothing since " + since + ".")
    return "\n\n".join(parts) + "\n"


def draft_component(path, since, until):
    items = component_changes(since, until).get(path)
    if not items:
        return None
    return (
        "<!-- Drafted by tools/changelog.py; sort into Added / Changed / "
        "Fixed before releasing. -->\n\n" + bullets(items) + "\n"
    )


def release_suite_text(text, version, date):
    """The suite changelog with its Unreleased section renamed to
    `version`, and a fresh empty Unreleased above it."""
    if UNRELEASED not in text:
        raise SystemExit(f"{SUITE_CHANGELOG.name} has no '{UNRELEASED}' section to release")
    if re.search(rf"^## \[{re.escape(version)}\]", text, re.M):
        raise SystemExit(f"{SUITE_CHANGELOG.name} already has a [{version}] section")
    body = text.split(UNRELEASED, 1)[1].split("\n## [", 1)[0]
    if not body.strip() or "Drafted by tools/changelog.py" in body:
        # Releasing an untouched draft is exactly what the comment in it
        # asks not to do.
        raise SystemExit(
            "the Unreleased section is empty or still an unedited draft — "
            "edit CHANGELOG.md first (tools/changelog.py draft helps)"
        )
    return text.replace(UNRELEASED, f"{UNRELEASED}\n\n## [{version}] - {date}", 1)


def prepend_component(path, version, date, section):
    file = ROOT / path / "CHANGELOG.md"
    header = (
        "# Changelog\n\nThis component is released with the Hyprforge suite "
        "and shares its version: see the suite's `CHANGELOG.md`.\n"
    )
    text = file.read_text() if file.exists() else header
    if re.search(rf"^## \[{re.escape(version)}\]", text, re.M):
        return file, None
    # After the header (everything before the first "## "), newest first.
    head, sep, rest = text.partition("\n## ")
    new = f"{head.rstrip()}\n\n## [{version}] - {date}\n\n{section}" + (f"\n## {rest}" if sep else "")
    return file, new


def main():
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    sub = ap.add_subparsers(dest="cmd", required=True)
    d = sub.add_parser("draft")
    d.add_argument("--since")
    d.add_argument("--until", default="HEAD")
    c = sub.add_parser("components-changed")
    c.add_argument("--since")
    c.add_argument("--until", default="HEAD")
    r = sub.add_parser("release")
    r.add_argument("version")
    r.add_argument("--since")
    r.add_argument("--date", default=datetime.date.today().isoformat())
    r.add_argument("--write", action="store_true")
    args = ap.parse_args()
    since = args.since or last_tag()

    if args.cmd == "draft":
        sys.stdout.write(draft_suite(since, args.until))
    elif args.cmd == "components-changed":
        for path in component_changes(since, args.until):
            print(path)
    else:
        new_suite = release_suite_text(SUITE_CHANGELOG.read_text(), args.version, args.date)
        writes = [(SUITE_CHANGELOG, new_suite)]
        for path in component_changes(since, "HEAD"):
            section = draft_component(path, since, "HEAD")
            file, text = prepend_component(path, args.version, args.date, section)
            if text is not None:
                writes.append((file, text))
        for file, text in writes:
            if args.write:
                file.write_text(text)
                print(f"wrote {file.relative_to(ROOT)}")
            else:
                print(f"would write {file.relative_to(ROOT)}")


if __name__ == "__main__":
    main()
