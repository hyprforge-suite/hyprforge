#!/usr/bin/env bash
# Extracts one component into a branch that can become its own
# repository, with its history intact.
#
# Usage:
#   ./split.sh <crate-name> [branch]
#   ./split.sh hyprforge-clipboard
#
# What comes out is a branch whose root IS the crate — no `crates/`
# prefix, no workspace above it — carrying every commit that ever
# touched that directory. Push it to an empty repository and it is a
# project, not a directory that only builds inside a monorepo.
#
# ## Why `--rejoin`, and what it costs
#
# `git subtree split` on its own produces the branch and records nothing
# here. That is fine in one direction and silently broken in the other:
# changes made in the component repository cannot come back, because
# `git subtree pull` looks for a common ancestor, finds none, and stops
# with "refusing to merge unrelated histories". The split commits have
# the same trees as ours but different hashes, and nothing connects them.
#
# `--rejoin` is what connects them. It leaves a merge commit here —
# "Split 'crates/<crate>/' into commit '<sha>'" — recording which commit
# the split produced, so a later `git subtree pull` has an ancestor to
# find. That merge commit is the price, and it is the whole price: this
# is the one thing that makes the arrangement bidirectional rather than
# a one-way export.
#
# Both directions were verified before this script was written, and the
# failure above is what a plain split actually did.
#
# ## Afterwards
#
#   git push <new-repo-url> <branch>:main
#
#   # send later changes out (development happens here):
#   git subtree push --prefix=crates/<crate> <new-repo-url> main
#
#   # bring changes made over there back in:
#   git subtree pull --prefix=crates/<crate> <new-repo-url> main
#
# ## Undoing it
#
# Nothing here is destructive: delete the branch, and `git revert` the
# rejoin commit if you want it gone. The component keeps living in this
# repository either way — a subtree is not a move.
set -uo pipefail
cd "$(dirname "${BASH_SOURCE[0]}")"

BOLD=$'\e[1m'; DIM=$'\e[2m'; RED=$'\e[31m'; GREEN=$'\e[32m'; YELLOW=$'\e[33m'; OFF=$'\e[0m'

die() { printf '%s✗%s %s\n' "$RED" "$OFF" "$1" >&2; exit 1; }
ok()  { printf '  %s✓%s %s\n' "$GREEN" "$OFF" "$1"; }

CRATE="${1:-}"
[[ -n "$CRATE" ]] || die "usage: ./split.sh <crate-name> [branch]"
BRANCH="${2:-${CRATE#hyprforge-}-split}"
PREFIX="crates/$CRATE"

[[ -d "$PREFIX" ]] || die "no such crate: $PREFIX"

# A dirty tree would be split in whatever half-finished state it is in,
# and the rejoin commit would record that as the component's history.
[[ -z "$(git status --porcelain)" ]] || die "working tree is dirty; commit or stash first"

# A component that cannot be built as its own repository is not ready to
# be one. The specific thing this catches is workspace inheritance:
# `version.workspace = true` and friends resolve to nothing once the
# crate is its own root, and the failure is at manifest-parse time.
printf '\n%s==> Checking the manifest stands alone%s\n' "$BOLD" "$OFF"
if grep -q 'workspace = true\|workspace.dependencies' "$PREFIX/Cargo.toml"; then
    die "$PREFIX/Cargo.toml still inherits from the workspace.
     A standalone crate has no workspace to inherit from: spell out
     version, edition, license and every dependency version. See
     hyprforge-clipboard's manifest for the shape, and the [patch]
     section in the root Cargo.toml for how its Hyprforge dependencies
     stay local while naming a git URL."
fi
ok "no workspace inheritance left"

for f in LICENSE README.md; do
    if [[ ! -f "$PREFIX/$f" ]]; then
        printf '  %s–%s no %s %s(a repository wants one)%s\n' "$YELLOW" "$OFF" "$f" "$DIM" "$OFF"
    elif [[ -L "$PREFIX/$f" ]]; then
        # cargo dereferences a symlink when it packages a tarball, so
        # this is fine for a crate that is only ever published — and
        # broken for one that becomes a repository, because git carries
        # a symlink as a symlink and `../../LICENSE` is above the root.
        die "$PREFIX/$f is a symlink; it will dangle in the split repository. Replace it with a real file."
    else
        ok "$f is a real file"
    fi
done

printf '\n%s==> Splitting %s%s\n' "$BOLD" "$PREFIX" "$OFF"
git branch -D "$BRANCH" >/dev/null 2>&1 && printf '  %s–%s replaced the existing %s branch\n' "$YELLOW" "$OFF" "$BRANCH"

git subtree split --prefix="$PREFIX" --rejoin -b "$BRANCH" >/dev/null 2>&1 \
    || die "git subtree split failed"
ok "branch $BRANCH created, $(git log --oneline "$BRANCH" | wc -l) commits of history"
ok "rejoin commit recorded here, so changes can come back"

printf '\n%s==> Next%s\n' "$BOLD" "$OFF"
cat <<NEXT
  Inspect it without disturbing anything:
    git clone -b $BRANCH . /tmp/$CRATE && cd /tmp/$CRATE && cargo test

  Publish it as its own repository:
    git push <new-repo-url> $BRANCH:main

  Nothing has been pushed. That is deliberate — it is outward-facing and
  yours to trigger.
NEXT
