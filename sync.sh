#!/usr/bin/env bash
# Reports whether each split-out component's published repository still
# matches what this monorepo would produce right now, and — with
# --push — brings the ones that have drifted back into sync.
#
# ## Why this exists
#
# `split.sh` produces a branch. Publishing it was always a manual
# afterthought: `git push <url> <crate>-split:main`, five times, once per
# component, typed by hand. Nothing records that it happened and nothing
# checks it stayed true. A component repository that has fallen behind
# does not error — it just quietly serves stale code to whoever clones
# it, forever, until someone happens to diff it by eye. That is exactly
# the class of failure CLAUDE.md keeps naming: "a test that cannot run
# must not report as one that passed." A check that was never written is
# the same failure with no test to even fail.
#
# ## How "in sync" is decided, without pushing anything to find out
#
# `git subtree split --prefix=crates/<crate>` — with no `-b` and no
# `--rejoin` — computes the commit a real split would produce and prints
# its hash. Called this way it is read-only: no branch is created, no ref
# in this repository moves, HEAD does not change, and it works fine on a
# dirty tree, because it operates on committed history, not the working
# directory or index. (`--rejoin` is what makes `split.sh` want a clean
# tree — the merge commit it leaves would otherwise record a half-finished
# state. Plain `split` leaves nothing here to record.)
#
# That commit's tree is compared against the tree at the tip of the
# published repository's `main`, fetched — never pushed — into this
# repository's object database. Comparing trees rather than commit hashes
# matters: the split commit's *parent* differs from run to run (it is
# whatever this repository's history looks like today), so the hashes
# never match even when the content is identical. The tree is the actual
# claim being made — "this is what the component looks like" — and it is
# what two different histories can still agree on.
#
# If the trees differ, which side moved decides what happens next.
# Ordinary drift is the remote commit being an ancestor of the new split:
# development happened here and hasn't been pushed out yet, so pushing is
# safe and is exactly `split.sh`'s own "Publish it" step, automated.
# Genuine divergence — the remote commit is not an ancestor, meaning
# something landed in the component repository that never came back
# through here — is a different problem with a different fix
# (`git subtree pull`, a merge, a human looking at both sides), and this
# script refuses to guess. Resolving it silently would mean deciding
# whose history wins without anyone deciding that on purpose.
#
# ## Why the default is read-only
#
# Every other script here (`split.sh`, `check.sh`) either changes nothing
# outward-facing or only reports. This is the first one that *can* push,
# and pushing here means to a separate GitHub repository per component,
# each one something other people might have cloned. So `./sync.sh` alone only ever reads: local
# object database writes from `git subtree split` and `git fetch`, never
# a ref update anywhere that matters, never a push. `--push` is the one
# way to make it act, same as `split.sh` leaves the actual `git push` to
# a human by default and only automates it here because the comparison
# above makes "is this safe" a checkable question instead of a guess.
#
# Usage:
#   ./sync.sh          report drift for every component; never pushes
#   ./sync.sh --push   push every component that has drifted; refuses on
#                       a dirty tree or a failing `./check.sh --quick`
set -uo pipefail
cd "$(dirname "${BASH_SOURCE[0]}")"

BOLD=$'\e[1m'; DIM=$'\e[2m'; RED=$'\e[31m'; GREEN=$'\e[32m'; YELLOW=$'\e[33m'; OFF=$'\e[0m'

die()  { printf '%s✗%s %s\n' "$RED" "$OFF" "$1" >&2; exit 1; }
ok()   { printf '  %s✓%s %s\n' "$GREEN" "$OFF" "$1"; }
bad()  { printf '  %s✗%s %s\n' "$RED" "$OFF" "$1"; FAILURES+=("$1"); }
note() { printf '  %s–%s %s %s(%s)%s\n' "$YELLOW" "$OFF" "$1" "$DIM" "$2" "$OFF"; }
step() { printf '\n%s==> %s%s\n' "$BOLD" "$1" "$OFF"; }

GITHUB_OWNER="hyprforge-suite"
NET_TIMEOUT=20

PUSH=false
case "${1:-}" in
    "") ;;
    --push) PUSH=true ;;
    *) die "usage: ./sync.sh [--push]" ;;
esac

FAILURES=()

step "Checking gh is usable"
command -v gh >/dev/null || die "gh is not installed. sync.sh needs it: git is configured to
     authenticate to github.com through 'gh auth git-credential' (see
     'git config --get credential.https://github.com.helper'), so without
     gh, every fetch below would fail with an auth error, not a clean one."
gh auth status >/dev/null 2>&1 || die "gh is not authenticated. Run: gh auth login"
ok "gh is installed and authenticated"

# Which crates are components at all — discovered, not hardcoded, and
# discovered the same way check.sh's "Standalone crate dependency pins"
# step does: a crate counts once its manifest inherits nothing from the
# workspace (no version.workspace = true, no foo.workspace = true
# dependency). That is exactly the set that can be `split.sh`'d, because
# it is exactly the set with no workspace above it to lean on. Using the
# same discovery as check.sh means a crate someone newly prepares for a
# split is picked up here automatically, the same day it starts being
# checked for dependency drift — instead of silently not being covered,
# which is the failure this whole file exists to stop.
step "Discovering standalone-ready crates"
command -v python3 >/dev/null || die "python3 is not available to parse Cargo.toml manifests"
mapfile -t CRATES < <(python3 - <<'PYEOF'
import glob
import tomllib


def has_workspace_inheritance(obj):
    if isinstance(obj, dict):
        if obj.get("workspace") is True:
            return True
        return any(has_workspace_inheritance(v) for v in obj.values())
    if isinstance(obj, list):
        return any(has_workspace_inheritance(v) for v in obj)
    return False


for manifest_path in sorted(glob.glob("crates/*/Cargo.toml")):
    with open(manifest_path, "rb") as f:
        doc = tomllib.load(f)
    if has_workspace_inheritance(doc):
        continue
    name = doc.get("package", {}).get("name")
    if name:
        print(name)
PYEOF
)
[[ ${#CRATES[@]} -gt 0 ]] || die "no standalone-ready crates found — nothing to sync against"
ok "${#CRATES[@]} standalone-ready crate(s): ${CRATES[*]}"

# The URL is derived, not looked up in a table — crates/hyprforge-tray
# maps to hyprforge-tray, every time, by stripping the shared prefix.
# There is deliberately no hardcoded name -> URL table to fall out of
# date: the one thing that could go silently wrong here is a crate whose
# derived URL is not actually the right repository, and there is no way
# to detect that automatically. So the derivation is named here loudly
# instead of hidden in a lookup, and every crate the discovery step above
# finds gets a URL — none can silently fall through with no mapping,
# because there is no separate mapping step to fall through.
url_for() {
    local crate="$1" suffix="${1#hyprforge-}"
    [[ "$suffix" != "$crate" ]] || die "crate '$crate' doesn't start with hyprforge- — the URL derivation
     (strip the prefix, prepend https://github.com/$GITHUB_OWNER/hyprforge-)
     does not hold for it. Fix the derivation or name its repository by
     hand before trusting this script with it."
    printf 'https://github.com/%s/hyprforge-%s' "$GITHUB_OWNER" "$suffix"
}

if $PUSH; then
    step "Working tree"
    # Same reason split.sh refuses on a dirty tree: a push here re-splits
    # first, and a dirty tree would be split in whatever half-finished
    # state it is in — except now that half-finished state goes straight
    # out to a repository other people clone, with no rejoin commit
    # between it and them.
    [[ -z "$(git status --porcelain)" ]] || die "working tree is dirty; commit or stash first"
    ok "clean"

    step "Running ./check.sh --quick (tier 1: clippy + unit tests — this takes a minute)"
    if ./check.sh --quick; then
        ok "tier 1 passed"
    else
        die "./check.sh --quick failed. Publishing code that doesn't pass tier 1 to every
     component repository other people clone is worse than not publishing it.
     Fix it, then re-run ./sync.sh --push."
    fi
fi

# The monorepo comes first, and it is not optional.
#
# Every standalone manifest names https://github.com/<owner>/hyprforge for
# every sibling crate it needs — hyprforge-settings alone pulls seventeen of
# them that way. So a component pushed while this repository is behind
# gets built by its own CI against whatever siblings were published days
# ago, and the failure names the *component* (`cannot find function
# `update` in module `hyprforge_tray::prefs``) rather than the stale
# dependency it actually resolved. That happened: five components checked,
# three pushed, and not one word about the repository all five depend on.
#
# Pushed before any component, for the same reason: a component's CI
# starts the moment its push lands, and resolves siblings from here as it
# runs.
step "The monorepo every component's manifest points at"
MONOREPO_URL="https://github.com/$GITHUB_OWNER/hyprforge"
# The branch this checkout is on, not a hardcoded name: the URL in the
# manifests carries no branch, so cargo fetches the repository's default,
# and pushing some other branch would leave the default stale while this
# script reported success.
MONOREPO_BRANCH="$(git symbolic-ref --quiet --short HEAD || true)"
MONOREPO_BEHIND=false
if [[ -z "$MONOREPO_BRANCH" ]]; then
    bad "HEAD is detached; cannot tell which branch should be published"
    FAILURES+=("monorepo")
elif ! timeout "$NET_TIMEOUT" git fetch --no-tags "$MONOREPO_URL.git" "$MONOREPO_BRANCH" >/dev/null 2>&1; then
    bad "$MONOREPO_URL is unreachable — every component's manifest depends on it"
    FAILURES+=("monorepo")
else
    monorepo_local=$(git rev-parse HEAD)
    monorepo_remote=$(git rev-parse FETCH_HEAD)
    if [[ "$monorepo_local" == "$monorepo_remote" ]]; then
        ok "in sync ($MONOREPO_BRANCH at ${monorepo_local:0:7})"
    elif git merge-base --is-ancestor "$monorepo_remote" "$monorepo_local"; then
        ahead=$(git rev-list --count "$monorepo_remote..$monorepo_local")
        note "monorepo is behind by $ahead commit(s)" \
             "every component's CI resolves its siblings from here, so this goes first"
        MONOREPO_BEHIND=true
    else
        bad "$MONOREPO_BRANCH has diverged from $MONOREPO_URL — resolve by hand, never with --force"
        # Deliberately not added to DIVERGED: that array is declared
        # (and so emptied) further down, alongside the component pass it
        # belongs to. FAILURES is declared at the top and is what the
        # exit status actually reads.
        FAILURES+=("monorepo")
    fi
fi

if $PUSH && $MONOREPO_BEHIND; then
    if git push "$MONOREPO_URL.git" "HEAD:$MONOREPO_BRANCH"; then
        ok "pushed $MONOREPO_BRANCH to $MONOREPO_URL"
    else
        bad "pushing the monorepo failed — not pushing any component against a stale dependency"
        die "Fix the push above, then re-run ./sync.sh --push."
    fi
fi

# One pass over every component, computing what a split would produce
# right now and what is actually published, in read-only mode always —
# --push only decides what happens with the answer, not how it's found.
declare -A LOCAL_SHA REMOTE_SHA STATUS
IN_SYNC=() DRIFTED=() UNPUBLISHED=() UNREACHABLE=() DIVERGED=()

step "Comparing each component against its published repository"
for crate in "${CRATES[@]}"; do
    url="$(url_for "$crate")"

    # Read-only: no -b, no --rejoin, so nothing here moves a ref or
    # touches the working tree — see the comment block at the top for
    # why that is true and not just hoped-for.
    local_sha=$(git subtree split --prefix="crates/$crate" 2>/dev/null)
    if [[ -z "$local_sha" ]]; then
        bad "$crate: 'git subtree split' produced nothing — see the manifest guards in split.sh"
        STATUS[$crate]=error
        continue
    fi
    LOCAL_SHA[$crate]="$local_sha"

    fetch_err=$(mktemp)
    if ! timeout "$NET_TIMEOUT" git fetch --no-tags "$url.git" main >"$fetch_err" 2>&1; then
        errtext=$(cat "$fetch_err"); rm -f "$fetch_err"
        if grep -qi 'not found' <<<"$errtext"; then
            note "$crate: no published repository yet at $url" "run with --push to create the first sync"
            UNPUBLISHED+=("$crate")
            STATUS[$crate]=unpublished
        elif [[ -z "$errtext" ]] || grep -qiE 'timed out|timeout' <<<"$errtext"; then
            bad "$crate: timed out reaching $url after ${NET_TIMEOUT}s — network unreachable?"
            UNREACHABLE+=("$crate")
            STATUS[$crate]=unreachable
        else
            bad "$crate: could not reach $url — $errtext"
            UNREACHABLE+=("$crate")
            STATUS[$crate]=unreachable
        fi
        continue
    fi
    rm -f "$fetch_err"
    remote_sha=$(git rev-parse FETCH_HEAD)
    REMOTE_SHA[$crate]="$remote_sha"

    local_tree=$(git rev-parse "${local_sha}^{tree}")
    remote_tree=$(git rev-parse "${remote_sha}^{tree}")

    if [[ "$local_tree" == "$remote_tree" ]]; then
        ok "$crate: in sync ($local_sha)"
        IN_SYNC+=("$crate")
        STATUS[$crate]=in_sync
    elif git merge-base --is-ancestor "$remote_sha" "$local_sha" 2>/dev/null; then
        note "$crate: published repository is behind" "local $local_sha, published $remote_sha"
        DRIFTED+=("$crate")
        STATUS[$crate]=drifted
    else
        # The published commit is not an ancestor of what a split would
        # produce now: something reached the component repository that
        # never came back through here. Pushing would silently discard
        # it — force or not — so this is the one state the script
        # refuses to resolve on its own. See the comment block at top.
        bad "$crate: DIVERGED — published history is not an ancestor of the current split.
     Someone pushed to $url directly, or a rejoin was skipped. This needs
     a human: git subtree pull --prefix=crates/$crate $url main"
        DIVERGED+=("$crate")
        STATUS[$crate]=diverged
    fi
done

step "Summary"
printf '  %d in sync, %d behind, %d unpublished, %d unreachable, %d diverged (of %d)\n' \
    "${#IN_SYNC[@]}" "${#DRIFTED[@]}" "${#UNPUBLISHED[@]}" "${#UNREACHABLE[@]}" "${#DIVERGED[@]}" "${#CRATES[@]}"

if ! $PUSH; then
    # `$MONOREPO_BEHIND` counts here even when every component is in
    # sync, which is exactly the state that hid this problem: five green
    # ticks and a stale repository underneath all five.
    if [[ ${#DRIFTED[@]} -gt 0 || ${#DIVERGED[@]} -gt 0 ]] || $MONOREPO_BEHIND; then
        printf '  Re-run with %s--push%s to publish what has drifted.\n' "$BOLD" "$OFF"
    fi
    [[ ${#FAILURES[@]} -eq 0 && ${#DIVERGED[@]} -eq 0 ]]
    exit $?
fi

# --push from here on: act on what the pass above found.
step "Pushing what has drifted"
if [[ ${#DRIFTED[@]} -eq 0 ]]; then
    ok "nothing to push — every reachable component already matches its repository"
else
    for crate in "${DRIFTED[@]}"; do
        url="$(url_for "$crate")"
        sha="${LOCAL_SHA[$crate]}"
        # NEVER --force: a component repository can carry commits nobody
        # pushed from here (an outside contributor, a hotfix on GitHub
        # directly). The divergence check above is what stands between
        # this push and overwriting that; a force flag here would remove
        # that protection entirely rather than merely skip it once.
        if git push "$url.git" "$sha:main"; then
            ok "$crate: pushed $sha to $url"
        else
            bad "$crate: push to $url failed — see git's message above"
        fi
    done
fi

if [[ ${#UNPUBLISHED[@]} -gt 0 ]]; then
    step "Not published at all"
    for crate in "${UNPUBLISHED[@]}"; do
        note "$crate" "$(url_for "$crate") doesn't exist yet — create it empty, then re-run --push"
    done
fi

if [[ ${#DIVERGED[@]} -gt 0 ]]; then
    printf '\n%s%d component(s) diverged and were left untouched — see above for the fix.%s\n' \
        "$RED" "${#DIVERGED[@]}" "$OFF"
fi

[[ ${#FAILURES[@]} -eq 0 && ${#DIVERGED[@]} -eq 0 ]]
exit $?
