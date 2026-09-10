#!/usr/bin/env bash
#
# Start a clean nested Hyprland on wayland-2 for lock-screen testing.
#
# NEVER test the lock screen against the session you are using. A lock
# screen that misbehaves there is a machine you power-cycle. Everything
# below exists to make the nested compositor the path of least
# resistance.
#
# Run this before EVERY lock test. A lock client that is killed while
# holding the lock leaves the session locked with nothing left to unlock
# it — by design, since a lock screen that released on a signal would be
# useless — and Hyprland then paints its "lockscreen app died" recovery
# screen over the nested window. Restarting is the clean way back; the
# other way is:
#
#   hyprctl --instance <N> eval 'hl.clear_crashed_lockscreen()'
#
# Usage, from the repo root:
#   ./crates/hyprforge-lock/testing/nested.sh
#   ./target/debug/hyprforge-lock --display wayland-2 \
#       --fake-password hunter2 --type-in hunter2
#
# Do NOT also export WAYLAND_DISPLAY=wayland-2: --display already sets
# it, and --fake-password refuses when the display it is given is the one
# the process would have used anyway. That check is what stops
# `--display $WAYLAND_DISPLAY --fake-password x` from locking the real
# session with a known password.
set -euo pipefail

HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"

# Kill only nested instances, matched on the config path.
#
# Deliberately not `pkill -f`: a -f pattern also matches the shell
# running it, because the pattern appears in that shell's own command
# line. That kills the script itself and exits 144, which looks like
# anything except what it is.
for pid in $(pgrep -x Hyprland); do
    if grep -qz 'nested.lua' "/proc/$pid/cmdline" 2>/dev/null; then kill "$pid"; fi
done
sleep 2

# WAYLAND_DISPLAY must point at the *host* session. Without it Hyprland
# tries to drive the hardware directly and dies with
# `CBackend::create() failed!`.
#
# start-hyprland rather than the Hyprland binary: launching the binary
# directly earns a red "started without start-hyprland" banner across
# the nested window.
WAYLAND_DISPLAY="${WAYLAND_DISPLAY:-wayland-1}" \
    setsid start-hyprland -- --config "$HERE/nested.lua" \
    > "${TMPDIR:-/tmp}/hyprforge-nested.log" 2>&1 < /dev/null &
sleep 7

# `hyprctl instances` maps sockets to instances. Do NOT pick the newest
# directory under /run/user/*/hypr: the real session writes its log
# continuously, so mtime order can hand you the session you must not
# touch.
SIG=$(hyprctl instances -j | python3 -c \
    "import json,sys;print([i['instance'] for i in json.load(sys.stdin) if i['wl_socket']=='wayland-2'][0])")
echo "nested instance: $SIG"
HYPRLAND_INSTANCE_SIGNATURE=$SIG hyprctl configerrors
echo "ready on wayland-2"
