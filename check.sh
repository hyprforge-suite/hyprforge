#!/usr/bin/env bash
# Runs every check this project has, in the order that fails fastest.
#
# Three tiers, and they answer different questions:
#
#   1. clippy + unit tests  — does the code do what *this project* thinks?
#                             No system needed; safe anywhere. Run twice:
#                             once for this workspace, once for the nested
#                             `notif/` one, which cargo will not reach from
#                             here (it is `exclude`d, and for a reason —
#                             see Cargo.toml).
#   2. live tests           — does Hyprland agree? Every claim the option
#                             catalogues make is checked against the running
#                             compositor. Needs Hyprland.
#   3. parse tests          — do the ecosystem daemons agree? The generated
#                             config files are handed to hyprpaper/hypridle
#                             themselves. Needs those installed, not running.
#   3b. services            — do NetworkManager, BlueZ, logind and whatever
#                             bar is running agree? Read-only checks
#                             that its interface is the shape
#                             hyprforge-network claims. Needs it running.
#   3c. BlueZ               — does BlueZ agree? Read-only checks that its
#                             interface is the shape hyprforge-bluetooth
#                             claims. Needs bluetooth.service running.
#   3d. UPower              — does UPower agree? Read-only checks that
#                             battery status is the shape hyprforge-power
#                             claims. Needs upower.service running.
#   3e. power-profiles-daemon — does it agree? Read-only checks that the
#                             offered profiles and the active one are the
#                             shape hyprforge-power claims. Never sets the
#                             active profile. Needs power-profiles-daemon
#                             running.
#   3f. fprintd             — does fprintd agree? Read-only checks that the
#                             reader, the enrolled list and VerifyStatus are
#                             the shape the lock's fingerprint path claims.
#                             Never claims the reader. Needs fprintd
#                             installed (it is bus-activated).
#
# Tiers 2, 3, 3b and 3c each gate on the thing they actually ask, rather
# than sharing one --ignored run: a check that silently never runs is
# worse than one that fails.
#
# Tier 1 catches a mistake in the code. Tiers 2 and 3 catch the far nastier
# kind: code that is internally consistent and wrong about the system it is
# talking to — a renamed option, a type that changed, a misspelled key that
# the daemon silently ignores rather than rejecting.
#
# Not covered here: the lock screen's live behaviour. Its unit tests run
# in tier 1 like everything else, but proving it actually locks, draws
# and unlocks needs a nested compositor, and must never be pointed at the
# session you are using. See "The lock screen" in the README and
# crates/hyprforge-lock/testing/nested.sh.
#
# Usage:
#   ./check.sh          everything available on this machine
#   ./check.sh --quick  tier 1 only (no compositor, no daemons)
set -uo pipefail
cd "$(dirname "${BASH_SOURCE[0]}")"

QUICK=false
[[ "${1:-}" == "--quick" ]] && QUICK=true

BOLD=$'\e[1m'; DIM=$'\e[2m'; RED=$'\e[31m'; GREEN=$'\e[32m'; YELLOW=$'\e[33m'; OFF=$'\e[0m'
FAILURES=()

step() { printf '\n%s==> %s%s\n' "$BOLD" "$1" "$OFF"; }
ok()   { printf '  %s✓%s %s\n' "$GREEN" "$OFF" "$1"; }
bad()  { printf '  %s✗%s %s\n' "$RED" "$OFF" "$1"; FAILURES+=("$1"); }
skip() { printf '  %s–%s %s %s(%s)%s\n' "$YELLOW" "$OFF" "$1" "$DIM" "$2" "$OFF"; }

# Counts the tests reported across every binary in a `cargo test` run.
count_tests() { grep 'test result' | awk -F'[.;] ' '{s+=$2} END {print s+0}'; }

step "Lint"
if warnings=$(cargo clippy --workspace --all-targets 2>&1 | grep -cE '^(error|warning)'); then :; fi
if [[ "$warnings" -eq 0 ]]; then
    ok "clippy: no warnings"
else
    bad "clippy: $warnings warning(s) — run: cargo clippy --workspace --all-targets"
fi

# A dependency-feature fact, so no Rust test can see it — and the damage
# it does is invisible to every test we have, because both renderers
# report the same `Color` and only the pixels differ.
step "Renderer colour space"
if cargo tree --workspace -e features 2>/dev/null | grep -q 'web-colors'; then
    bad "iced's \`web-colors\` is enabled — the wgpu hosts will draw the theme"
    bad "  lighter than the tiny-skia ones. See the iced entry in Cargo.toml."
else
    ok "iced web-colors: off, so every host draws the same theme the same way"
fi

# hyprforge-clipboard, hyprforge-lock and hyprforge-greet are prepared to
# become their own repositories (see repo-plan.md), which means each of
# them now spells out every third-party dependency's version and feature
# set in full — a crate that is its own repository root has no
# `[workspace.dependencies]` to inherit from, so `serde.workspace = true`
# becomes `serde = { version = "1", features = ["derive"] }`, copied by
# hand.
#
# That is required, and it is a silent drift hazard: if the root table
# bumps a version or a feature and one of these three manifests is not
# updated to match, cargo resolves both happily. Clippy stays silent,
# every test stays green, and the suite quietly builds two different
# versions of the same dependency — the "one place to fix a shared
# thing" property (see CLAUDE.md and repo-plan.md) failing with no
# symptom at all. Nothing else here would ever notice, because nothing
# else compares a standalone crate's manifest against the root one.
#
# Which crates to check is discovered, not hardcoded: a crate counts as
# standalone-ready when nothing in its manifest inherits from the
# workspace at all — no `version.workspace = true`, no
# `foo.workspace = true` dependency. That is deliberately stronger than
# "has its own version number": hyprforge-paths, hyprforge-look,
# hyprforge-secret, hyprforge-process and hyprforge-ui also carry their
# own version (repo-plan.md step 1, for publishing to crates.io) while
# every one of their dependencies is still `.workspace = true` — they
# are publish-ready, not standalone-ready, and have nothing to drift.
# Discovering the list this way means the next crate someone prepares
# for a split is covered automatically, instead of silently not being
# covered — which is the exact failure this project keeps naming.
#
# Only dependencies present in BOTH the crate and the root table are
# compared. A crate-exclusive dependency (`xkbcommon`,
# `wayland-protocols`) has no root value to drift from. And
# `hyprforge-paths`, `hyprforge-look`, `hyprforge-authui` and
# `hyprforge-secret` are exempt by name: those four are deliberately
# spelled as a git dependency in the standalone crate and a local path in
# the root table — that mismatch in *form* is the split mechanism
# working (see the `[patch]` section in Cargo.toml), not drift, and
# there is no version there to compare in the first place.
#
# A dropped feature is flagged; an added one is not. Extending a shared
# dependency with extra features on top of the workspace base is the
# normal pattern here (`hyprforge-settings` adds `tokio` to `iced`,
# `hyprforge-clipmenu` adds `image`/`wayland` to `iced_tiny_skia`) — a
# standalone manifest has to spell that same addition out in full, since
# it has no workspace entry left to extend. A feature the root table
# turns on that the standalone copy has silently lost is the actual
# hazard: a matching version number gives no hint that anything changed.
step "Standalone crate dependency pins"
if ! command -v python3 >/dev/null; then
    skip "dependency pin check" "python3 not available to parse Cargo.toml"
else
    output=$(python3 - <<'PYEOF'
import glob
import sys
import tomllib

ROOT = "Cargo.toml"


def has_workspace_inheritance(obj):
    if isinstance(obj, dict):
        if obj.get("workspace") is True:
            return True
        return any(has_workspace_inheritance(v) for v in obj.values())
    if isinstance(obj, list):
        return any(has_workspace_inheritance(v) for v in obj)
    return False


def normalize(spec):
    """(version, features:set, default_features:bool), or None if there
    is no version to compare (a path/git-only dependency)."""
    if isinstance(spec, str):
        return (spec, frozenset(), True)
    if isinstance(spec, dict):
        version = spec.get("version")
        if version is None:
            return None
        return (
            version,
            frozenset(spec.get("features", [])),
            spec.get("default-features", True),
        )
    return None


with open(ROOT, "rb") as f:
    root = tomllib.load(f)
root_deps = root.get("workspace", {}).get("dependencies", {})

mismatches = []
checked_crates = 0
checked_deps = 0

for manifest_path in sorted(glob.glob("crates/*/Cargo.toml")):
    with open(manifest_path, "rb") as f:
        doc = tomllib.load(f)

    if has_workspace_inheritance(doc):
        continue  # still inherits from the workspace — not standalone-ready

    crate_name = doc.get("package", {}).get("name", manifest_path)
    checked_crates += 1
    crate_deps = doc.get("dependencies", {})

    for dep_name, crate_spec in crate_deps.items():
        if dep_name.startswith("hyprforge-"):
            continue  # exempt: git dep here, local path in the root table, by design
        if dep_name not in root_deps:
            continue  # crate-exclusive dependency — no root value to drift from

        root_norm = normalize(root_deps[dep_name])
        crate_norm = normalize(crate_spec)
        if root_norm is None or crate_norm is None:
            continue

        checked_deps += 1
        root_version, root_features, root_default = root_norm
        crate_version, crate_features, crate_default = crate_norm

        if root_version != crate_version:
            mismatches.append(
                f"{crate_name}: {dep_name} version — root={root_version!r} crate={crate_version!r}"
            )
        if root_default != crate_default:
            mismatches.append(
                f"{crate_name}: {dep_name} default-features — root={root_default!r} crate={crate_default!r}"
            )
        missing = root_features - crate_features
        if missing:
            mismatches.append(
                f"{crate_name}: {dep_name} features — root requires {sorted(missing)} "
                f"which is missing from the crate's {sorted(crate_features)}"
            )

if mismatches:
    for m in mismatches:
        print(m)
    sys.exit(1)

print(f"{checked_crates} standalone-ready crate(s), {checked_deps} shared dependency pin(s) checked")
PYEOF
    )
    if [[ $? -ne 0 ]]; then
        bad "a standalone-ready crate's pinned dependency has drifted from the root table"
        while IFS= read -r line; do
            [[ -n "$line" ]] && bad "  $line"
        done <<<"$output"
    else
        ok "$output"
    fi
fi

# The rule this exists for is in repo-plan.md under "The shape a package
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
step "Siblings build from this checkout"
# A standalone manifest names its Hyprforge siblings by git URL, and the
# root `[patch]` table is what points each one back at `crates/`. A name
# missing from that table does not fail to build — cargo fetches the
# sibling from GitHub at whatever commit the lockfile holds and builds
# against that. Seven were missing at once after the files, displayd and
# emojimenu split: Files built its archive, file-operation and
# browser crates from a days-old GitHub snapshot, the tray and clipboard
# their popup shell, and every local fix to those crates — and every test
# of the apps using them — silently ran against the old copy.
if ! command -v python3 >/dev/null; then
    skip "sibling patch check" "python3 not available to parse Cargo.toml"
else
    output=$(python3 - <<'PYEOF'
import tomllib
from pathlib import Path

root = tomllib.loads(Path("Cargo.toml").read_text())
# The URL comes from the [patch] table's own key, not from a literal
# here. A literal would have to be updated when the repository moves,
# and if it were not, this check would match no manifest and report
# "all 0 siblings build from crates/" in green — a check that silently
# never runs, wearing a tick. The table is the one place the URL must
# be right for anything to build locally, so it is the one to trust.
patches = root.get("patch", {})
if len(patches) != 1:
    print(f"PROBLEM the root [patch] table has {len(patches)} source(s) ({', '.join(patches) or 'none'}); exactly one — the suite's git URL — is expected")
    raise SystemExit
URL = next(iter(patches))
patched = set(patches[URL])

named = {}
for manifest in sorted(Path("crates").glob("*/Cargo.toml")):
    data = tomllib.loads(manifest.read_text())
    tables = [data.get("dependencies", {}), data.get("dev-dependencies", {}), data.get("build-dependencies", {})]
    for target in data.get("target", {}).values():
        tables += [target.get(k, {}) for k in ("dependencies", "dev-dependencies", "build-dependencies")]
    for table in tables:
        for name, spec in table.items():
            if isinstance(spec, dict) and spec.get("git") == URL:
                named.setdefault(name, set()).add(manifest.parent.name)

print(f"CHECKED {len(named)}")
if not named:
    # Nine standalone manifests name the URL today; zero means the
    # manifests and the table have stopped agreeing on what it is, and
    # every sibling is being fetched from wherever the manifests point.
    print(f"PROBLEM no manifest under crates/ names {URL}, the URL the root [patch] table redirects — the two have diverged, and nothing is being redirected")
for name in sorted(set(named) - patched):
    print(f"PROBLEM {name} is named by git in {', '.join(sorted(named[name]))} but not redirected by [patch] — it builds from GitHub, not from crates/")
PYEOF
    )
    if grep -q "^PROBLEM" <<<"$output"; then
        bad "$(grep -c '^PROBLEM' <<<"$output") sibling(s) build from GitHub instead of this checkout"
        sed -n 's/^PROBLEM /    • /p' <<<"$output"
    else
        ok "all $(sed -n 's/^CHECKED //p' <<<"$output") git-named siblings build from crates/"
    fi
fi

step "Every package is a repository"
if ! command -v python3 >/dev/null; then
    skip "package/repository check" "python3 not available to parse the PKGBUILD"
else
    output=$(python3 - <<'PYEOF'
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
    binaries = sorted(
        set(re.findall(r"^\s*_bin\s+(hyprforge-[a-z0-9-]+)", body.group(1), re.M))
        | set(re.findall(r"usr/bin/(hyprforge-[a-z0-9-]+)", body.group(1)))
    )
    if not binaries:
        problems.append(f"{package}: installs no hyprforge binary this check can see")
        continue

    checked += 1

    # Which crate directory declares each binary. One directory for the
    # whole package is the rule; several means the split cannot produce
    # a repository that is the package.
    owners = {}
    for manifest in sorted(Path("crates").glob("*/Cargo.toml")):
        data = tomllib.loads(manifest.read_text())
        declared = {b.get("name") for b in data.get("bin", [])}
        if not declared:
            # A crate with no [[bin]] but a src/main.rs builds one named
            # after the package.
            if (manifest.parent / "src" / "main.rs").is_file():
                declared = {data.get("package", {}).get("name")}
        for binary in binaries:
            if binary in declared:
                owners.setdefault(binary, manifest.parent.name)

    missing = [b for b in binaries if b not in owners]
    if missing:
        problems.append(f"{package}: no crate declares {', '.join(missing)}")
        continue

    directories = sorted(set(owners.values()))
    if len(directories) > 1:
        problems.append(
            f"{package}: installs {', '.join(binaries)} from {len(directories)} crate "
            f"directories ({', '.join(directories)}) — a subtree split takes one "
            f"prefix, so this package cannot become one repository"
        )

print(f"CHECKED {checked}")
for problem in problems:
    print(f"PROBLEM {problem}")
PYEOF
    )
    if grep -q "^SKIP" <<<"$output"; then
        skip "package/repository check" "$(sed -n 's/^SKIP //p' <<<"$output")"
    elif grep -q "^PROBLEM" <<<"$output"; then
        bad "$(grep -c '^PROBLEM' <<<"$output") package(s) cannot become a repository as they stand"
        sed -n 's/^PROBLEM /    • /p' <<<"$output"
    else
        ok "$(sed -n 's/^CHECKED //p' <<<"$output") package(s) each build from one crate directory"
    fi
fi

step "The installer can plan every component"
# `./hyprforge --install --all --dry-run` builds nothing and asks for no
# sudo, but still runs every guard the real install does — among them
# that each `build` names a crate that exists. That guard was written for
# exactly this and still let a real `--install --all` die halfway: two
# popups had moved into their parent crates as second binaries, the
# installer kept building them by their old crate names, and nobody had
# run the dry run since. Running it here is what makes the guard a check.
if output=$(./hyprforge --install --all --dry-run </dev/null 2>&1); then
    ok "every component's install plan resolves (dry run)"
else
    bad "./hyprforge --install --all --dry-run fails"
    grep -E 'error|die' <<<"$output" | head -5 | sed 's/^/    • /'
fi

step "Docs name things that exist"
# The half of keeping docs current that a machine can do. The other half
# — "four icons" when there are five — is judgement, and lives in the
# `keep-docs-current` skill; this step catches what needs none.
#
# Both checks here were written after the drift they catch shipped: a
# published README listing `src/dbusmenu.rs` long after the file was
# deleted, and a README saying five components were split when there
# were eight.
if ! command -v python3 >/dev/null; then
    skip "docs check" "python3 not available"
else
    output=$(python3 - <<'PYEOF'
import re
import subprocess
import tomllib
from pathlib import Path

problems = []

# --- every source path a doc names is one that exists ----------------
#
# A crate's own docs resolve `src/`, `tests/` and the like against that
# crate, strictly: it is what someone reading the published repository
# will look for. `crates/` and `packaging/` in those same docs may point
# back at the monorepo ("this repository is a split of `crates/x`"), so
# they also resolve from the root. The root docs name a crate in prose and then
# give a path inside it, so a path there passes if any crate has it —
# which still catches a file deleted outright, the case that actually
# happened.
docs = subprocess.run(
    ["git", "ls-files", "*.md"], capture_output=True, text=True, check=True
).stdout.split()
docs = [Path(d) for d in docs if not d.startswith("notif/")]
crates = sorted(p for p in Path("crates").iterdir() if p.is_dir())
path_in_backticks = re.compile(
    r"`((?:crates/|src/|tests/|testing/|config/|packaging/)[A-Za-z0-9_./-]+)`"
)
named = 0
for doc in docs:
    text = doc.read_text()
    in_crate = doc.parts[0] == "crates" and len(doc.parts) > 2
    for match in path_in_backticks.finditer(text):
        path = match.group(1).rstrip("/.")
        named += 1
        if in_crate and not path.startswith(("crates/", "packaging/")):
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
directories = re.search(r"^(\w+) of these directories", readme, re.M)
if directories and directories.group(1).lower() != spelled(len(standalone)):
    problems.append(
        f"README.md says '{directories.group(1)} of these directories' are repositories; "
        f"there are {spelled(len(standalone))}"
    )

# CLAUDE.md's "Fifteen gated tiers": the indented `step` lines below
# are the gated tiers — `--quick` skips the block they sit in — less the
# one that only announces the skip. It said twelve while there were
# fourteen, and named neither of the two it was missing.
tiers = [
    t
    for t in re.findall(r'^[ \t]+step "([^"]+)"', Path("check.sh").read_text(), re.M)
    if not t.startswith("Skipping")
]
claimed = re.search(r"(\w+)\s+gated\s+tiers", Path("CLAUDE.md").read_text(), re.I)
if not claimed:
    problems.append("CLAUDE.md no longer says how many gated tiers there are")
elif claimed.group(1).lower() != spelled(len(tiers)):
    problems.append(
        f"CLAUDE.md says '{claimed.group(1)} gated tiers'; check.sh has "
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

print(f"CHECKED {named} {len(standalone)} {len(tiers)} {len(page_titles)}")
for problem in problems:
    print(f"PROBLEM {problem}")
PYEOF
    )
    if grep -q "^PROBLEM" <<<"$output"; then
        bad "$(grep -c '^PROBLEM' <<<"$output") doc claim(s) no longer true"
        sed -n 's/^PROBLEM /    • /p' <<<"$output"
    else
        read -r named repos tiers pages < <(sed -n 's/^CHECKED //p' <<<"$output")
        ok "$named source path(s) named in docs all exist; README lists all $repos standalone repositories; CLAUDE.md counts all $tiers gated tiers; Settings' README names all $pages pages"
    fi
fi

step "Unit and integration tests"
output=$(cargo test --workspace 2>&1)
if grep -q "test result: FAILED" <<<"$output"; then
    bad "$(grep -c 'test result: FAILED' <<<"$output") test binary(ies) failed"
    grep -E '^(test .* FAILED|failures:)' <<<"$output" | head -20
else
    ok "$(count_tests <<<"$output") tests passed"
fi

# Tier 1, and a separate invocation because `notif/` is a separate cargo
# workspace — excluded from this one on purpose (see `exclude` in
# Cargo.toml: the suite needs zbus/tokio and notif needs zbus/async-io,
# and unified they would panic at runtime).
#
# The cost of that seam is that `cargo test --workspace` above does not
# reach a single notif test, and would not say so. A component whose
# tests silently never run is exactly the failure the HYPRFORGE-SKIP
# convention exists for, one level up: here there is no marker to grep,
# because libtest is never asked in the first place. So it gets its own
# step, and its own count, and the count is what proves it ran.
#
# It needs no compositor, no daemon and no bus, so it belongs here with
# the rest of tier 1 rather than behind the --quick gate.
step "The notif workspace (its own cargo workspace)"
if warnings=$(cd notif && cargo clippy --all-targets 2>&1 | grep -cE '^(error|warning)'); then :; fi
if [[ "$warnings" -eq 0 ]]; then
    ok "notif clippy: no warnings"
else
    bad "notif clippy: $warnings warning(s) — run: (cd notif && cargo clippy --all-targets)"
fi
output=$(cd notif && cargo test 2>&1)
if grep -q "test result: FAILED" <<<"$output"; then
    bad "$(grep -c 'test result: FAILED' <<<"$output") notif test binary(ies) failed"
    grep -E '^(test .* FAILED|failures:)' <<<"$output" | head -20
else
    ok "$(count_tests <<<"$output") notif tests passed"
fi

if $QUICK; then
    step "Skipping system checks (--quick)"
else
    step "Live tests against Hyprland"
    if ! command -v hyprctl >/dev/null; then
        skip "live tests" "hyprctl not installed"
    elif ! hyprctl version >/dev/null 2>&1; then
        skip "live tests" "Hyprland isn't running"
    else
        # --test-threads=1 because these share one compositor: each loads
        # probe values and reloads to drop them, so two at once would undo
        # each other mid-assertion.
        #
        # Two invocations rather than one --workspace run, because
        # several crates hold live tests that answer to something other
        # than the compositor: the ecosystem crate's parse tests need the
        # daemons installed and *not* running, the network and bluetooth
        # crates' need NetworkManager and BlueZ respectively, and the
        # archive crate's need `unzip`, `tar` and `7z` — which have
        # nothing to do with a compositor at all. Each gets its own step
        # below. Running them here as well would report a
        # daemon rejecting a generated file, or disagreeing about an
        # interface that has nothing to do with Hyprland, under the
        # heading "the code disagrees with the running system", which is
        # a different thing to go and look at.
        #
        # cargo cannot exclude a single test target, so the live targets
        # are named. That also survives a test being renamed, which
        # `--skip` on the test names would not.
        output=$(
            cargo test --workspace --exclude hyprforge-ecosystem --exclude hyprforge-network --exclude hyprforge-bluetooth --exclude hyprforge-tray --exclude hyprforge-power --exclude hyprforge-clipboard --exclude hyprforge-archive -- --ignored --test-threads=1 2>&1
            cargo test -p hyprforge-ecosystem --lib --test live_ecosystem -- --ignored --test-threads=1 2>&1
        )
        if grep -q "test result: FAILED" <<<"$output"; then
            bad "live tests failed — the code disagrees with the running system"
            grep -E '^test .* FAILED' <<<"$output" | head -20
        else
            ok "$(count_tests <<<"$output") live tests passed against $(hyprctl version | head -1)"
        fi
    fi

    # Tier 3, and deliberately not gated on Hyprland: `--config` is read
    # before a daemon does anything else, so these need hyprpaper and
    # hypridle *installed*, not running — and not a compositor at all.
    # Folded into the tier 2 run they inherited its `hyprctl version`
    # gate, so on a machine with the daemons installed and no compositor
    # they silently never ran, which is the one arrangement where they
    # are the only check left.
    step "Parse tests against the ecosystem daemons"
    installed=()
    for daemon in hyprpaper hypridle; do
        command -v "$daemon" >/dev/null && installed+=("$daemon")
    done
    if [[ ${#installed[@]} -eq 0 ]]; then
        skip "parse tests" "neither hyprpaper nor hypridle is installed"
    else
        # --nocapture so the skip markers reach this script. libtest has no
        # skipped state: a check that could not run returns early and
        # prints `ok`, exactly like one that passed. With hyprpaper
        # running that made three of these four report success having
        # verified nothing — so the tests announce a skip and this reads
        # it. See SKIP_MARKER in the test file.
        output=$(cargo test -p hyprforge-ecosystem --test generated_configs_parse \
            -- --ignored --test-threads=1 --nocapture 2>&1)
        if grep -q "test result: FAILED" <<<"$output"; then
            bad "parse tests failed — a daemon rejected a file this project generates"
            grep -E '^test .* FAILED' <<<"$output" | head -20
        else
            ok "$(count_tests <<<"$output") parse tests passed"
            # Shown even though nothing failed: a skipped check is not a
            # passing one, and the count above cannot tell them apart.
            while IFS= read -r reason; do
                [[ -n "$reason" ]] && skip "  a check inside them was skipped" "$reason"
            done < <(sed -n 's/.*HYPRFORGE-SKIP: \([^.]*\).*/\1/p' <<<"$output" | sort -u)
        fi
    fi

    # Answers to `unzip`, `tar` and `7z`, so it gets its own gate for the
    # same reason every other step here does. Folded into the tier 2 run
    # these would have needed a compositor to ask a question about three
    # command-line tools.
    #
    # What they add over the round-trip tests in tier 1: those prove this
    # crate can read what it wrote, which a writer emitting something
    # only its own reader accepts would also pass. These hand the file to
    # an implementation nobody here wrote, and read back what one of them
    # produced.
    #
    # Not gated on any one tool being present — each test says for itself
    # which it needs and prints HYPRFORGE-SKIP when it is missing, so a
    # machine with `tar` and no `7z` still gets the half it can run, and
    # the summary says which half that was.
    step "Live tests against the system's archive tools"
    if ! command -v tar >/dev/null && ! command -v unzip >/dev/null && ! command -v 7z >/dev/null; then
        skip "archive tool tests" "none of tar, unzip or 7z is installed"
    else
        output=$(cargo test -p hyprforge-archive --test live_system_tools \
            -- --ignored --test-threads=1 --nocapture 2>&1)
        if grep -q "test result: FAILED" <<<"$output"; then
            bad "archive tool tests failed — something else on this machine disagrees with what this suite writes"
            grep -E '^test .* FAILED' <<<"$output" | head -20
        else
            ok "$(count_tests <<<"$output") archive tool tests passed"
            while IFS= read -r reason; do
                [[ -n "$reason" ]] && skip "  a check inside them was skipped" "$reason"
            done < <(sed -n 's/.*HYPRFORGE-SKIP: \([^(]*\).*/\1/p' <<<"$output" | sort -u)
        fi
    fi

    # Answers to NetworkManager, not to Hyprland, so it gets its own gate
    # for the same reason the parse tests did: folded into the tier 2 run
    # these would need a compositor to check a service that has nothing to
    # do with one.
    #
    # Every test behind this step is read-only. They run on a machine
    # somebody is using, quite possibly over the connection they inspect.
    step "Live tests against NetworkManager"
    if ! command -v systemctl >/dev/null; then
        skip "NetworkManager tests" "systemctl not available to ask"
    elif ! systemctl is-active --quiet NetworkManager; then
        skip "NetworkManager tests" "NetworkManager isn't running"
    else
        output=$(cargo test -p hyprforge-network --test live_networkmanager \
            -- --ignored --test-threads=1 --nocapture 2>&1)
        if grep -q "test result: FAILED" <<<"$output"; then
            bad "NetworkManager tests failed — the code disagrees with the running service"
            grep -E '^test .* FAILED' <<<"$output" | head -20
        else
            ok "$(count_tests <<<"$output") NetworkManager tests passed"
            while IFS= read -r reason; do
                [[ -n "$reason" ]] && skip "  a check inside them was skipped" "$reason"
            done < <(sed -n 's/.*HYPRFORGE-SKIP: \([^(]*\).*/\1/p' <<<"$output" | sort -u)
        fi
    fi

    # Answers to BlueZ, not to Hyprland, so it gets its own gate for the
    # same reason the NetworkManager step did: folded into the tier 2 run
    # this would need a compositor to check a service that has nothing to
    # do with one.
    #
    # Every test behind this step is read-only. They run on a machine
    # somebody is using — starting discovery from an unattended test
    # would cost battery on every device in range, not just this one.
    step "Live tests against BlueZ"
    if ! command -v systemctl >/dev/null; then
        skip "BlueZ tests" "systemctl not available to ask"
    elif ! systemctl is-active --quiet bluetooth; then
        skip "BlueZ tests" "bluetooth isn't running"
    else
        output=$(cargo test -p hyprforge-bluetooth --test live_bluez \
            -- --ignored --test-threads=1 --nocapture 2>&1)
        if grep -q "test result: FAILED" <<<"$output"; then
            bad "BlueZ tests failed — the code disagrees with the running service"
            grep -E '^test .* FAILED' <<<"$output" | head -20
        else
            ok "$(count_tests <<<"$output") BlueZ tests passed"
            while IFS= read -r reason; do
                [[ -n "$reason" ]] && skip "  a check inside them was skipped" "$reason"
            done < <(sed -n 's/.*HYPRFORGE-SKIP: \([^(]*\).*/\1/p' <<<"$output" | sort -u)
        fi
    fi
    # Answers to whichever bar is offering a tray, which is neither the
    # compositor nor a system service. Registering really does put an icon
    # in the user's bar for a fraction of a second — that is the smallest
    # observable form of "a host accepted it", and no unit test can reach it.
    # Gated on the bus name rather than on `systemctl is-active
    # systemd-logind`, because what these tests need is something
    # answering on `org.freedesktop.login1` — which is the thing they
    # actually ask, and is true on a system where logind is socket
    # -activated and not yet started as a unit.
    # Gated on hyprsunset actually running, which is what these ask.
    # They are read-only: hyprsunset sets the colour of the screen
    # somebody is looking at, and a test may not change that unless it
    # can restore it.
    step "Live tests against hyprsunset"
    if ! pgrep -x hyprsunset >/dev/null 2>&1; then
        skip "hyprsunset tests" "hyprsunset isn't running"
    else
        output=$(cargo test -p hyprforge-ecosystem --test live_hyprsunset \
            -- --ignored --test-threads=1 --nocapture 2>&1)
        if grep -q "test result: FAILED" <<<"$output"; then
            bad "hyprsunset tests failed — the code disagrees with the running daemon"
            grep -E '^test .* FAILED' <<<"$output" | head -20
        else
            ok "$(count_tests <<<"$output") hyprsunset tests passed"
            while IFS= read -r reason; do
                [[ -n "$reason" ]] && skip "  a check inside them was skipped" "$reason"
            done < <(sed -n 's/.*HYPRFORGE-SKIP: \([^(]*\).*/\1/p' <<<"$output" | sort -u)
        fi
    fi

    step "Live tests against systemd-logind"
    if ! command -v busctl >/dev/null; then
        skip "logind tests" "busctl not available to ask"
    elif ! busctl --system status org.freedesktop.login1 >/dev/null 2>&1; then
        skip "logind tests" "nothing is answering on org.freedesktop.login1"
    else
        output=$(cargo test -p hyprforge-power --test live_logind \
            -- --ignored --test-threads=1 --nocapture 2>&1)
        if grep -q "test result: FAILED" <<<"$output"; then
            bad "logind tests failed — the code disagrees with the running service"
            grep -E '^test .* FAILED' <<<"$output" | head -20
        else
            ok "$(count_tests <<<"$output") logind tests passed"
            while IFS= read -r reason; do
                [[ -n "$reason" ]] && skip "  a check inside them was skipped" "$reason"
            done < <(sed -n 's/.*HYPRFORGE-SKIP: \([^(]*\).*/\1/p' <<<"$output" | sort -u)
        fi
    fi

    # Gated on a home trash directory existing, and on nothing else.
    # Not on a compositor, not on a bus: this asks whether the
    # `.trashinfo` files *another implementation already wrote* are ones
    # this crate can read, and the only thing that has to be true for
    # that question to mean anything is that such files exist. A trash
    # that is empty reports a skip rather than a false green, because
    # "parsed every one of zero entries" is the shape of check that
    # CLAUDE.md already caught passing while verifying nothing.
    step "Trash entries written by another implementation"
    if [[ ! -d "${XDG_DATA_HOME:-$HOME/.local/share}/Trash/info" ]]; then
        skip "trash tests" "no home trash directory on this machine"
    else
        output=$(cargo test -p hyprforge-fileops --test live_trash \
            -- --ignored --test-threads=1 --nocapture 2>&1)
        if grep -q "test result: FAILED" <<<"$output"; then
            bad "trash tests failed — this crate disagrees with the trash already on disk"
            grep -E '^test .* FAILED' <<<"$output" | head -20
        else
            ok "$(count_tests <<<"$output") trash tests passed"
            while IFS= read -r reason; do
                [[ -n "$reason" ]] && skip "  a check inside them was skipped" "$reason"
            done < <(sed -n 's/.*HYPRFORGE-SKIP: \([^(]*\).*/\1/p' <<<"$output" | sort -u)
        fi
    fi

    # Answers to UPower, not to Hyprland or to logind — its own gate, for
    # the same reason NetworkManager and BlueZ got their own steps: folded
    # into any of the runs above, this would need something that has
    # nothing to do with UPower to be up before it ever ran. UPower and
    # power-profiles-daemon are two separate services with independent
    # lifetimes (one can be down while the other answers fine), so they
    # get two separate gates below rather than one combined "power" step
    # — a machine with UPower masked but power-profiles-daemon running
    # should still get the second step's coverage, and vice versa.
    #
    # Every test behind this step is read-only: it only reads properties
    # UPower already exposes read-only in the first place.
    step "Live tests against UPower"
    if ! command -v systemctl >/dev/null; then
        skip "UPower tests" "systemctl not available to ask"
    elif ! systemctl is-active --quiet upower; then
        skip "UPower tests" "UPower isn't running"
    else
        output=$(cargo test -p hyprforge-power --test live_upower \
            -- --ignored --test-threads=1 --nocapture 2>&1)
        if grep -q "test result: FAILED" <<<"$output"; then
            bad "UPower tests failed — the code disagrees with the running service"
            grep -E '^test .* FAILED' <<<"$output" | head -20
        else
            ok "$(count_tests <<<"$output") UPower tests passed"
            while IFS= read -r reason; do
                [[ -n "$reason" ]] && skip "  a check inside them was skipped" "$reason"
            done < <(sed -n 's/.*HYPRFORGE-SKIP: \([^(]*\).*/\1/p' <<<"$output" | sort -u)
        fi
    fi

    # Answers to power-profiles-daemon, not to Hyprland or to UPower — see
    # the note above for why this is a separate gate rather than folded
    # into the UPower step.
    #
    # Every test behind this step is read-only, and deliberately never
    # calls SetActiveProfile: this runs on a machine somebody is using,
    # and changing the active profile changes how it performs and how
    # loud its fans are out from under them. See
    # `tests/live_power_profiles.rs`'s module doc for the full reasoning.
    step "Live tests against power-profiles-daemon"
    if ! command -v systemctl >/dev/null; then
        skip "power-profiles-daemon tests" "systemctl not available to ask"
    elif ! systemctl is-active --quiet power-profiles-daemon; then
        skip "power-profiles-daemon tests" "power-profiles-daemon isn't running"
    else
        output=$(cargo test -p hyprforge-power --test live_power_profiles \
            -- --ignored --test-threads=1 --nocapture 2>&1)
        if grep -q "test result: FAILED" <<<"$output"; then
            bad "power-profiles-daemon tests failed — the code disagrees with the running service"
            grep -E '^test .* FAILED' <<<"$output" | head -20
        else
            ok "$(count_tests <<<"$output") power-profiles-daemon tests passed"
            while IFS= read -r reason; do
                [[ -n "$reason" ]] && skip "  a check inside them was skipped" "$reason"
            done < <(sed -n 's/.*HYPRFORGE-SKIP: \([^(]*\).*/\1/p' <<<"$output" | sort -u)
        fi
    fi

    # Answers to fprintd, the lock screen's fingerprint path. Gated on the
    # system bus knowing fprintd's name — running or activatable — and not
    # on `systemctl is-active`: fprintd is bus-activated and exits when
    # idle, so it is "inactive" on nearly every machine that has it.
    #
    # Read-only: it asks for the default reader, lists this user's
    # enrolled fingers and reads the device's introspection. It never
    # claims the reader or starts a verification, so the sensor does not
    # even light. The first version of the lock's fingerprint code named
    # the wrong manager path and passed every unit test; this is the step
    # that would have said so.
    step "Live tests against fprintd"
    if ! command -v busctl >/dev/null; then
        skip "fprintd tests" "busctl not available to ask"
    elif ! busctl --system list --acquired --activatable --no-legend 2>/dev/null \
            | awk '{print $1}' | grep -qx 'net.reactivated.Fprint'; then
        skip "fprintd tests" "fprintd isn't installed"
    else
        output=$(cargo test -p hyprforge-lock --bin hyprforge-lock live_fprintd \
            -- --ignored --test-threads=1 --nocapture 2>&1)
        if grep -q "test result: FAILED" <<<"$output"; then
            bad "fprintd tests failed — the lock disagrees with the running fprintd"
            grep -E '^test .* FAILED' <<<"$output" | head -20
        else
            ok "$(count_tests <<<"$output") fprintd tests passed"
            while IFS= read -r reason; do
                [[ -n "$reason" ]] && skip "  a check inside them was skipped" "$reason"
            done < <(sed -n 's/.*HYPRFORGE-SKIP: \([^(]*\).*/\1/p' <<<"$output" | sort -u)
        fi
    fi

    # Gated on an icon theme being installed to resolve against, which
    # is what this asks — not on a bar, a bus or a compositor. An icon
    # name that no theme carries draws a blank gap and logs nothing
    # anywhere, so a list of "standard" names cannot catch it and only a
    # real theme can.
    # Gated on a Wayland session, which is what these ask — the
    # clipboard is the compositor's, and data-control is how it is read.
    # Read-only: they enumerate what is offered and never set or clear a
    # selection. The clipboard belongs to whoever is using the machine.
    step "Live tests against the Wayland clipboard"
    if [[ -z "${WAYLAND_DISPLAY:-}" ]]; then
        skip "clipboard tests" "no WAYLAND_DISPLAY — not in a Wayland session"
    else
        output=$(cargo test -p hyprforge-clipboard \
            -- --ignored --test-threads=1 --nocapture 2>&1)
        if grep -q "test result: FAILED" <<<"$output"; then
            bad "clipboard tests failed — the code disagrees with the running compositor"
            grep -E '^test .* FAILED' <<<"$output" | head -20
        else
            ok "$(count_tests <<<"$output") clipboard tests passed"
            while IFS= read -r reason; do
                [[ -n "$reason" ]] && skip "  a check inside them was skipped" "$reason"
            done < <(sed -n 's/.*HYPRFORGE-SKIP: \([^(]*\).*/\1/p' <<<"$output" | sort -u)
        fi
    fi

    step "Icon names against the installed theme"
    if ! command -v gsettings >/dev/null; then
        skip "icon resolution" "gsettings not available to ask which theme is set"
    else
        output=$(cargo test -p hyprforge-tray --test live_icons \
            -- --ignored --test-threads=1 --nocapture 2>&1)
        if grep -q "test result: FAILED" <<<"$output"; then
            bad "an icon name is missing, non-symbolic among symbolic siblings, or resolves through a different theme than the rest"
            grep -E 'do not resolve in|break the one-family rule|not carried by any installed theme|resolve through different themes' <<<"$output" | head -10
        else
            ok "$(count_tests <<<"$output") icon check passed"
            while IFS= read -r reason; do
                [[ -n "$reason" ]] && skip "  a check inside it was skipped" "$reason"
            done < <(sed -n 's/.*HYPRFORGE-SKIP: \([^(]*\).*/\1/p' <<<"$output" | sort -u)
        fi
    fi

    step "The installed shared MIME database"
    # Gated on the database itself, not on a compositor or a daemon:
    # what these ask is whether this machine's own globs2 and desktop
    # entries say what the parsers here expect.
    if [[ ! -r /usr/share/mime/globs2 && ! -r "${XDG_DATA_HOME:-$HOME/.local/share}/mime/globs2" ]]; then
        skip "mime database tests" "no shared MIME database installed (shared-mime-info)"
    else
        output=$(cargo test -p hyprforge-mime --test live_mime_database \
            -- --ignored --test-threads=1 --nocapture 2>&1)
        if grep -q "test result: FAILED" <<<"$output"; then
            bad "mime tests failed — this machine's database disagrees with how these files are read"
            grep -E '^test .* FAILED|should be|gio says' <<<"$output" | head -20
        else
            ok "$(count_tests <<<"$output") mime database tests passed"
            while IFS= read -r reason; do
                [[ -n "$reason" ]] && skip "  a check inside them was skipped" "$reason"
            done < <(sed -n 's/.*HYPRFORGE-SKIP: \([^(]*\).*/\1/p' <<<"$output" | sort -u)
        fi
    fi

    step "Live tests against a tray host"
    if ! command -v busctl >/dev/null; then
        skip "tray tests" "busctl not available to ask"
    elif ! busctl --user list --no-legend 2>/dev/null | grep -q '^org.kde.StatusNotifierWatcher'; then
        skip "tray tests" "no StatusNotifierWatcher is running (no bar with a tray)"
    else
        output=$(cargo test -p hyprforge-tray --test live_tray \
            -- --ignored --test-threads=1 --nocapture 2>&1)
        if grep -q "test result: FAILED" <<<"$output"; then
            bad "tray tests failed — a real tray host would not show these icons"
            grep -E '^test .* FAILED' <<<"$output" | head -20
        else
            ok "$(count_tests <<<"$output") tray tests passed"
            while IFS= read -r reason; do
                [[ -n "$reason" ]] && skip "  a check inside them was skipped" "$reason"
            done < <(sed -n 's/.*HYPRFORGE-SKIP: \([^(]*\).*/\1/p' <<<"$output" | sort -u)
        fi
    fi
fi

step "Summary"
if [[ ${#FAILURES[@]} -eq 0 ]]; then
    printf '  %sEverything passed.%s\n\n' "$GREEN" "$OFF"
    exit 0
fi
printf '  %s%d check(s) failed:%s\n' "$RED" "${#FAILURES[@]}" "$OFF"
printf '    • %s\n' "${FAILURES[@]}"
printf '\n'
exit 1
