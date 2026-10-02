#!/usr/bin/env python3
"""Render each library crate's `//!` doc into its README.md, or check that it
already matches.

    tools/crate-readme.py --write   rewrite every library README
    tools/crate-readme.py --check   exit 1 naming any README that differs

Why a README is generated rather than written: every library crate here
already has a crate-level doc that says what it is and what it is not, in
the voice this project writes in, and it is what docs.rs shows. A README
written by hand beside it would be a second copy of the same paragraphs,
and a second copy is a thing that drifts — the class of failure
`check.sh`'s "Docs name things that exist" step exists to catch. So the
README *is* the crate doc, plus the one thing the doc cannot say about
itself: where it lives and how to install it.

Only library crates. The nine components carry hand-written READMEs,
because those are the front page of a repository someone clones, and say
things (installing, what is next door) a crate doc has no reason to.

What the rendering does to rustdoc's Markdown, and why:

- `[`Foo`]` and `[`Foo`](path::to::Foo)` — intra-doc links, which resolve
  only inside rustdoc — become plain `Foo` in code font. A README on
  crates.io or GitHub has nowhere for them to point.
- Everything else is Markdown already and passes through.
"""

import re
import sys
import tomllib
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
COMPONENTS = {
    "hyprforge-clipboard", "hyprforge-lock", "hyprforge-greet", "hyprforge-tray",
    "hyprforge-settings", "hyprforge-displayd", "hyprforge-emojimenu",
    "hyprforge-files", "hyprforge-media",
}
SUITE = "https://github.com/hyprforge-suite/hyprforge"

# `[`Foo`](anything)` or `[`Foo`]` -> `Foo`. The link text is kept exactly,
# backticks included, so `[`Theme::resolve`]` reads as `Theme::resolve`.
INTRA_DOC_LINK = re.compile(r"\[(`[^`\]]+`)\](?:\([^)]*\))?")


def crate_doc(lib_rs: Path) -> str:
    lines = []
    for line in lib_rs.read_text().splitlines():
        if line.startswith("//!"):
            lines.append(line[3:].removeprefix(" "))
        elif line.strip() == "" and lines:
            # A blank line inside the doc block is written as a bare `//!`,
            # so a truly blank line means the block is over.
            break
        elif lines:
            break
    return "\n".join(lines).strip("\n")


def render(crate_dir: Path) -> str:
    manifest = tomllib.loads((crate_dir / "Cargo.toml").read_text())["package"]
    name = manifest["name"]
    doc = crate_doc(crate_dir / "src" / "lib.rs")
    if not doc:
        raise SystemExit(f"{crate_dir.name}: src/lib.rs has no `//!` crate doc to render")
    body = INTRA_DOC_LINK.sub(r"\1", doc)
    description = manifest.get("description", "").rstrip(".")
    return (
        f"# {name}\n\n"
        f"{description}.\n\n"
        f"{body}\n\n"
        "## Where this lives\n\n"
        f"Part of [Hyprforge]({SUITE}), a suite of native Hyprland desktop\n"
        f"applications. This crate is `crates/{name}` there; it is published to\n"
        f"crates.io so the suite's applications can be built from their own\n"
        "repositories, and its API follows the suite. Issues and pull requests go\n"
        "to the suite repository.\n\n"
        "```\n"
        f"cargo add {name}\n"
        "```\n\n"
        "This README is rendered from the crate's `//!` documentation by\n"
        "`tools/crate-readme.py`; edit `src/lib.rs`, not this file.\n\n"
        "## Licence\n\n"
        "MIT. See `LICENSE`.\n"
    )


def libraries():
    for manifest in sorted((ROOT / "crates").glob("*/Cargo.toml")):
        package = tomllib.loads(manifest.read_text()).get("package")
        # No [package] is a nested workspace (hyprforge-notif): a
        # component, whose README is written by hand like the others'.
        if package is not None and package["name"] not in COMPONENTS:
            yield manifest.parent


def main() -> int:
    mode = sys.argv[1] if len(sys.argv) == 2 else ""
    if mode not in ("--write", "--check"):
        print(__doc__.split("\n\n")[0], file=sys.stderr)
        return 2
    stale = []
    for crate_dir in libraries():
        want = render(crate_dir)
        readme = crate_dir / "README.md"
        have = readme.read_text() if readme.exists() else None
        if have == want:
            continue
        if mode == "--write":
            readme.write_text(want)
            print(f"wrote {readme.relative_to(ROOT)}")
        else:
            stale.append(readme.relative_to(ROOT))
    for path in stale:
        print(f"STALE {path}")
    return 1 if stale else 0


if __name__ == "__main__":
    raise SystemExit(main())
