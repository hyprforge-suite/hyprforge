#!/usr/bin/env python3
# Docs name things that exist.
#
# Moved verbatim from check.sh, where it ran as an inline heredoc;
# `tools/hf lint` runs it from the repository root and reads its
# CHECKED / PROBLEM / SKIP lines.
#
# The half of keeping docs current that a machine can do. The other half
# — "four icons" when there are five — is judgement, and lives in the
# `keep-docs-current` skill; this step catches what needs none.
#
# Both checks here were written after the drift they catch shipped: a
# published README listing `src/dbusmenu.rs` long after the file was
# deleted, and a README saying five components were split when there
# were eight.

import re
import subprocess
import tomllib
from pathlib import Path

problems = []

# --- every source path a doc names is one that exists ----------------
#
# A crate's own docs resolve `src/`, `tests/` and the like against that
# crate, strictly: it is what someone reading the published repository
# will look for. `crates/`, `packaging/` and `docs/` in those same docs
# may point back at the monorepo ("the suite's `docs/displays.md`"), so
# they also resolve from the root. `docs/` is here because the README's
# reference sections moved there, and a doc naming one that has since
# moved again is exactly the drift this exists to catch. The root docs name a crate in prose and then
# give a path inside it, so a path there passes if any crate has it —
# which still catches a file deleted outright, the case that actually
# happened.
#
# `--recurse-submodules`, because the components are submodules and a
# plain `ls-files` lists nothing inside one: when they became submodules
# this check went from 149 named paths to 98 and still passed, every
# component's own README and DESIGN.md silently out of its reach.
docs = subprocess.run(
    ["git", "ls-files", "--recurse-submodules", "*.md"], capture_output=True, text=True, check=True
).stdout.split()
docs = [Path(d) for d in docs]
crates = sorted(p for p in Path("crates").iterdir() if p.is_dir())
path_in_backticks = re.compile(
    r"`((?:crates/|src/|tests/|testing/|config/|packaging/|docs/)[A-Za-z0-9_./-]+)`"
)
named = 0
for doc in docs:
    text = doc.read_text()
    in_crate = doc.parts[0] == "crates" and len(doc.parts) > 2
    for match in path_in_backticks.finditer(text):
        path = match.group(1).rstrip("/.")
        named += 1
        if in_crate and not path.startswith(("crates/", "packaging/", "docs/")):
            candidates = [doc.parent / path]
        elif in_crate:
            # A crate can have a `packaging/` of its own (the unit files
            # the split repository ships) as well as meaning the root's.
            candidates = [doc.parent / path, Path(path)]
        else:
            candidates = [Path(path)] + [crate / path for crate in crates]
        if not any(c.exists() for c in candidates):
            line = text[: match.start()].count("\n") + 1
            problems.append(f"{doc}:{line} names `{path}`, which does not exist")

# --- the README's repository table is the set of standalone crates --
#
# "Standalone" by shape, the way the dependency-pin step finds them: a
# manifest with no workspace inheritance left. A crate prepared for a
# split is then held to this without anybody adding it to a list.
def inherits(obj):
    if isinstance(obj, dict):
        return obj.get("workspace") is True or any(inherits(v) for v in obj.values())
    if isinstance(obj, list):
        return any(inherits(v) for v in obj)
    return False

standalone = sorted(
    m.parent.name
    for m in Path("crates").glob("*/Cargo.toml")
    if not inherits(tomllib.loads(m.read_text()))
)
readme = Path("README.md").read_text()
listed = sorted(
    set(re.findall(r"^\| \[(hyprforge-[a-z0-9-]+)\]\(https://github\.com/", readme, re.M))
)
for crate in sorted(set(standalone) - set(listed)):
    problems.append(f"README.md's repository table is missing {crate}, which is standalone")
for crate in sorted(set(listed) - set(standalone)):
    problems.append(f"README.md's repository table lists {crate}, which is not standalone")

# The counts in prose, spelled out: "## Nine repositories" is the
# components plus this one, "Eight of these directories" the components.
WORDS = "zero one two three four five six seven eight nine ten eleven twelve thirteen fourteen fifteen sixteen seventeen eighteen nineteen twenty".split()
WORDS += [f"twenty-{w}" for w in WORDS[1:10]] + ["thirty"]
def spelled(n):
    return WORDS[n] if n < len(WORDS) else str(n)

heading = re.search(r"^## (\w+) repositories, one workspace", readme, re.M | re.I)
if not heading:
    problems.append("README.md has no '## <N> repositories, one workspace' heading to check")
elif heading.group(1).lower() != spelled(len(standalone) + 1):
    problems.append(
        f"README.md says '{heading.group(1)} repositories'; there are "
        f"{spelled(len(standalone) + 1)} ({len(standalone)} components and this one)"
    )
# Required, not optional: this sentence moved from the README to
# docs/architecture.md with the workspace layout, and a check written
# as "if it is there" would have stopped checking without a word.
architecture = Path("docs/architecture.md").read_text()
directories = re.search(r"^(\w+) of these directories", architecture, re.M)
if not directories:
    problems.append("docs/architecture.md no longer says how many of its directories are repositories")
elif directories.group(1).lower() != spelled(len(standalone)):
    problems.append(
        f"docs/architecture.md says '{directories.group(1)} of these directories' are repositories; "
        f"there are {spelled(len(standalone))}"
    )

# CLAUDE.md's count of live gates: each `Gate(` in tools/hflib/tiers.py
# is one question a live check waits behind. When this counted check.sh's
# indented `step` lines instead, CLAUDE.md said twelve while there were
# fourteen, and named neither of the two it was missing.
tiers = re.findall(r'^    Gate\(\s*\n?\s*"([^"]+)"', Path("tools/hflib/tiers.py").read_text(), re.M)
# `[\w-]`, so "Twenty-one" is read whole rather than as "one".
claimed = re.search(r"([\w-]+)\s+live\s+gates", Path("CLAUDE.md").read_text(), re.I)
if not claimed:
    problems.append("CLAUDE.md no longer says how many live gates there are")
elif claimed.group(1).lower() != spelled(len(tiers)):
    problems.append(
        f"CLAUDE.md says '{claimed.group(1)} live gates'; tools/hflib/tiers.py has "
        f"{spelled(len(tiers))}: {', '.join(tiers)}"
    )

# --- the Settings README names every page the sidebar has -----------
#
# The published README opened with "ten screens" and a list of them
# while the app had thirteen, and the regroup that made them eighteen
# would have left it saying ten. Titles come from `Screen::title`, the
# strings the sidebar actually draws.
settings_main = Path("crates/hyprforge-settings/src/main.rs").read_text()
title_fn = re.search(r"fn title\(self\) -> &'static str \{(.*?)\n    \}", settings_main, re.S)
page_titles = re.findall(r'Screen::\w+ => "([^"]+)"', title_fn.group(1)) if title_fn else []
if not page_titles:
    problems.append("couldn't find Settings' page titles in Screen::title to check the README against")
# Whitespace collapsed, because prose wraps: "Idle &" ending one line
# and "lock" starting the next is still the page's name.
settings_readme = " ".join(Path("crates/hyprforge-settings/README.md").read_text().split())
for title in page_titles:
    if title not in settings_readme:
        problems.append(f"crates/hyprforge-settings/README.md does not name the {title!r} page")
page_words = {n: w for n, w in enumerate(
    "zero one two three four five six seven eight nine ten eleven twelve thirteen "
    "fourteen fifteen sixteen seventeen eighteen nineteen twenty".split())}
said = re.search(r"\b(\w+)\s+pages\b", settings_readme)
if not said:
    problems.append("crates/hyprforge-settings/README.md no longer says how many pages there are")
elif said.group(1).lower() != page_words.get(len(page_titles), str(len(page_titles))):
    problems.append(
        f"crates/hyprforge-settings/README.md says {said.group(1)} pages; "
        f"Screen::title has {len(page_titles)}"
    )

# --- the design-system catalogue names everything hyprforge-ui exports -
#
# The catalogue exists so the next widget is found rather than written
# again, and a catalogue missing the newest widget sends its author to
# write a second one. Names, not prose: every public widget, size and
# mark must appear in backticks somewhere in it.
ui = Path("crates/hyprforge-ui/src")
catalogue = Path("crates/hyprforge-ui/DESIGN-SYSTEM.md").read_text()
exported = set()
widgets_rs = (ui / "widgets.rs").read_text()
for group in re.findall(r"^pub use \w+::\{([^}]*)\};", widgets_rs, re.M | re.S):
    exported.update(n.strip() for n in group.split(",") if n.strip())
exported.update(re.findall(r"^pub use \w+::(\w+);", widgets_rs, re.M))
for source in (widgets_rs, (ui / "density.rs").read_text(), (ui / "glyph.rs").read_text()):
    exported.update(re.findall(r"^pub (?:fn|const) (\w+)", source, re.M))
if not exported:
    problems.append("couldn't find what hyprforge-ui exports to check its catalogue against")
for name in sorted(exported):
    if f"`{name}`" not in catalogue:
        problems.append(f"crates/hyprforge-ui/DESIGN-SYSTEM.md does not name `{name}`")

print(f"CHECKED {named} {len(standalone)} {len(tiers)} {len(page_titles)} {len(exported)}")
for problem in problems:
    print(f"PROBLEM {problem}")
