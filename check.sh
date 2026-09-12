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
#   3b. services            — do NetworkManager, BlueZ, logind and whatever
#                             bar is running agree? Read-only checks
#                             that its interface is the shape
#                             hyprforge-network claims. Needs it running.
#   3c. BlueZ               — does BlueZ agree? Read-only checks that its
#                             interface is the shape hyprforge-bluetooth
#                             claims. Needs bluetooth.service running.
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
        #
        # Two invocations rather than one --workspace run, because three
        # crates hold live tests that answer to something other than the
        # compositor: the ecosystem crate's parse tests need the daemons
        # installed and *not* running, and the network and bluetooth
        # crates' need NetworkManager and BlueZ respectively. Each gets
        # its own step below. Running them here as well would report a
        # daemon rejecting a generated file, or disagreeing about an
        # interface that has nothing to do with Hyprland, under the
        # heading "the code disagrees with the running system", which is
        # a different thing to go and look at.
        #
        # cargo cannot exclude a single test target, so the live targets
        # are named. That also survives a test being renamed, which
        # `--skip` on the test names would not.
        output=$(
            cargo test --workspace --exclude hyprforge-ecosystem --exclude hyprforge-network --exclude hyprforge-bluetooth --exclude hyprforge-tray --exclude hyprforge-power -- --ignored --test-threads=1 2>&1
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
