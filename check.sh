#!/usr/bin/env bash
# Runs every check this project has, in the order that fails fastest.
#
# Three tiers, and they answer different questions:
#
#   1. clippy + unit tests  — does the code do what *this project* thinks?
#                             No system needed; safe anywhere.
#   2. live tests           — does Hyprland agree? Every claim the option
#                             catalogues make is checked against the running
#                             compositor. Needs Hyprland.
#   3. parse tests          — do the ecosystem daemons agree? The generated
#                             config files are handed to hyprpaper/hypridle
#                             themselves. Needs those installed, not running.
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

step "Unit and integration tests"
output=$(cargo test --workspace 2>&1)
if grep -q "test result: FAILED" <<<"$output"; then
    bad "$(grep -c 'test result: FAILED' <<<"$output") test binary(ies) failed"
    grep -E '^(test .* FAILED|failures:)' <<<"$output" | head -20
else
    ok "$(count_tests <<<"$output") tests passed"
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
        output=$(cargo test --workspace -- --ignored --test-threads=1 2>&1)
        if grep -q "test result: FAILED" <<<"$output"; then
            bad "live tests failed — the code disagrees with the running system"
            grep -E '^test .* FAILED' <<<"$output" | head -20
        else
            ok "$(count_tests <<<"$output") live tests passed against $(hyprctl version | head -1)"
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
