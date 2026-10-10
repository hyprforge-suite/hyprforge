#!/usr/bin/env bash
# Cut a Hyprforge release: one version for the libraries, the ten
# components, notif and the Arch packages (docs/design/repo-plan.md).
#
#   tools/release.sh 0.1.9              print every step, change nothing
#   tools/release.sh 0.1.9 --execute    do it
#
# Dry run is the default because most of this cannot be taken back: a
# crates.io version is permanent, a pushed tag is public, and an AUR
# package reaches users the moment it is pushed. Read the dry run first.
#
# The order is the rule CLAUDE.md gives for a component change, applied
# to all of them at once — a component commit is pushed to its
# repository's main *before* the pin naming it, because a pin on a
# commit nobody can fetch is a monorepo nobody can clone:
#
#   1. ./check.sh, in full: pins on main, components building against
#      crates.io, every tier this machine can run.
#   2. Every version moved (tools/versions.py), the changelogs filed
#      (tools/changelog.py), the AUR package re-rendered (tools/aur.py),
#      and ./check.sh --quick again over the result.
#   3. Each component: commit, push to its main.
#   4. The suite: the pins and everything above in one "Release vX.Y.Z"
#      commit on main, pushed.
#   5. Tag vX.Y.Z on the suite and push it: .github/workflows/publish.yml
#      publishes the libraries. Past 30 crates crates.io answers 429 and
#      the job needs re-running after the limit's stated wait; see the
#      Status list in docs/design/repo-plan.md.
#   6. Tag every component at its new pin, and create a GitHub Release
#      only for the components whose pin moved since the previous tag —
#      computed *before* step 3, because step 3 moves every pin.
#   7. The AUR push is printed, not done: it needs an SSH key registered
#      with aur.archlinux.org, and it stays a manual step until the
#      process has been trusted for a few releases.
#
# Nothing here retries or forces. A step that fails stops the script with
# what is done and what is not, because a release half-done by a script
# that pressed on is harder to finish than one that stopped.

set -euo pipefail
cd "$(dirname "$0")/.."

usage() { sed -n '2,8p' "$0" | sed 's/^# \{0,1\}//'; exit 2; }

VERSION=""
EXECUTE=false
for arg in "$@"; do
    case "$arg" in
        --execute) EXECUTE=true ;;
        -h|--help) usage ;;
        [0-9]*.[0-9]*.[0-9]*) VERSION="$arg" ;;
        *) echo "unknown argument: $arg" >&2; usage ;;
    esac
done
[[ -n "$VERSION" ]] || usage
TAG="v$VERSION"

BOLD=$'\e[1m'; DIM=$'\e[2m'; RED=$'\e[31m'; OFF=$'\e[0m'
phase() { printf '\n%s==> %s%s\n' "$BOLD" "$1" "$OFF"; }
die()   { printf '%s✗ %s%s\n' "$RED" "$1" "$OFF" >&2; exit 1; }

# run <command...>: print it; run it only under --execute. Every action
# with an effect outside the working tree goes through here, so the dry
# run is a complete list of them rather than a summary.
run() {
    printf '  %s+ %s%s\n' "$DIM" "$*" "$OFF"
    if $EXECUTE; then "$@"; fi
}

submodules=$(git config -f .gitmodules --get-regexp 'submodule\..*\.path' | awk '{print $2}')
current=$(python3 -c 'import tomllib;print(tomllib.load(open("Cargo.toml","rb"))["workspace"]["package"]["version"])')
previous_tag=$(git describe --tags --abbrev=0 --match 'v*')

phase "Preconditions"
# Refusals, not warnings — each is a state this script would otherwise
# turn into a half-release.
[[ "$(git branch --show-current)" == "main" ]] || {
    if $EXECUTE; then die "not on main (on $(git branch --show-current))"; fi
    echo "  (dry run: not on main — an --execute run would stop here)"
}
if git rev-parse -q --verify "refs/tags/$TAG" >/dev/null; then die "$TAG already exists"; fi
python3 - "$current" "$VERSION" <<'PY' || die "$VERSION is not newer than the workspace's $current"
import sys
old, new = (tuple(map(int, v.split("."))) for v in sys.argv[1:])
sys.exit(0 if new > old else 1)
PY
if [[ -n "$(git status --porcelain --ignore-submodules=none)" ]]; then
    if $EXECUTE; then die "the working tree is not clean — commit or stash first, including inside submodules"; fi
    echo "  (dry run: the working tree is not clean — an --execute run would stop here)"
fi
if $EXECUTE; then
    command -v gh >/dev/null || die "gh is needed for the GitHub Releases"
fi
echo "  $current -> $VERSION; previous tag $previous_tag"

# Step 6 needs this, and step 3 is about to change every pin.
changed=$(python3 tools/changelog.py components-changed --since "$previous_tag")
echo "  components whose pin moved since $previous_tag: $(echo $changed | tr '\n' ' ')"
[[ -n "$changed" ]] || echo "  (none: every component gets a tag, none a GitHub Release)"

phase "1. Everything this machine can check"
run ./check.sh

phase "2. Versions, changelogs and the AUR render"
if $EXECUTE; then
    python3 tools/versions.py --set "$VERSION"
else
    python3 tools/versions.py --set "$VERSION" --dry-run | sed 's/^/  /'
fi
# The suite's Unreleased section has to have been edited by a person:
# tools/changelog.py refuses a section that is still its own draft.
if $EXECUTE; then
    python3 tools/changelog.py release "$VERSION" --since "$previous_tag" --write
else
    python3 tools/changelog.py release "$VERSION" --since "$previous_tag" 2>&1 | sed 's/^/  /' || true
fi
run cargo update --workspace --offline
run bash -c "cd crates/hyprforge-notif && cargo update --workspace --offline"
run python3 tools/aur.py --write
run ./check.sh --quick

phase "3. Each component, pushed to its main before any pin names it"
for path in $submodules; do
    # Every manifest whose version moved (notif has nine, in bin/ and
    # crates/), its lockfile, and a changelog — which only a component
    # whose pin moved has, so it is staged only if step 2 wrote one.
    #
    # The lockfile only where one is tracked — notif's, of the ten. Git
    # refuses a pathspec that matches nothing, so naming Cargo.lock for
    # every component stopped 0.1.10 at the first one, before anything
    # was committed.
    pathspecs=(':(glob)**/Cargo.toml')
    if [[ -n "$(git -C "$path" ls-files -- ':(glob)**/Cargo.lock')" ]]; then
        pathspecs+=(':(glob)**/Cargo.lock')
    fi
    run git -C "$path" add -u -- "${pathspecs[@]}"
    if ! $EXECUTE || [[ -e "$path/CHANGELOG.md" ]]; then
        run git -C "$path" add -- CHANGELOG.md
    fi
    run git -C "$path" commit -m "Release $TAG"
    run git -C "$path" push origin HEAD:main
done

phase "4. The suite: pins, versions, changelog, packaging"
run git add Cargo.toml Cargo.lock CHANGELOG.md packaging $submodules
run git commit -m "Release $TAG"
run git push origin main

phase "5. Tag the suite; publish.yml publishes the libraries"
run git tag -a "$TAG" -m "Hyprforge $VERSION"
run git push origin "$TAG"
echo "  then watch: gh run watch -R hyprforge-suite/hyprforge (a 429 past 30 crates means re-run after the wait)"

phase "6. Tag every component; release the ones that changed"
for path in $submodules; do
    run git -C "$path" tag -a "$TAG" -m "Hyprforge $VERSION"
    run git -C "$path" push origin "$TAG"
done
for path in $changed; do
    repo="hyprforge-suite/$(basename "$path")"
    notes="$(mktemp)"
    # The section changelog.py filed in step 2, from the heading to the
    # next one.
    if $EXECUTE; then
        awk -v v="## [$VERSION]" 'index($0,v)==1{f=1;next} /^## \[/{f=0} f' "$path/CHANGELOG.md" >"$notes"
    fi
    run gh release create "$TAG" -R "$repo" --title "$TAG" --notes-file "$notes"
    rm -f "$notes"
done
notes="$(mktemp)"
if $EXECUTE; then
    awk -v v="## [$VERSION]" 'index($0,v)==1{f=1;next} /^## \[/{f=0} f' CHANGELOG.md >"$notes"
fi
run gh release create "$TAG" -R hyprforge-suite/hyprforge --title "Hyprforge $VERSION" --notes-file "$notes"
rm -f "$notes"

phase "7. The AUR (by hand)"
cat <<EOF
  Once publish.yml is green and the tag is on GitHub:

    git clone ssh://aur@aur.archlinux.org/hyprforge.git /tmp/aur-hyprforge
    cp packaging/aur/hyprforge/{PKGBUILD,.SRCINFO,*.install} /tmp/aur-hyprforge/
    cd /tmp/aur-hyprforge && makepkg -o && git add -A && git commit -m "$VERSION" && git push

  makepkg -o fetches the tagged sources and runs prepare(): the cheap
  proof the tag and the render agree before anyone else downloads it.
EOF

if ! $EXECUTE; then
    printf '\n%sDry run: nothing above was done. Re-run with --execute.%s\n' "$BOLD" "$OFF"
fi
