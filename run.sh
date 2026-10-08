#!/usr/bin/env bash
# Builds (if needed) and launches hyprforge-displayd + hyprforge-settings
# for local development/testing. Not a replacement for the systemd unit —
# see docs/displays.md for installing hyprforge-displayd as a proper user service.
set -euo pipefail
cd "$(dirname "${BASH_SOURCE[0]}")"

MODE=debug
MOCK=false

usage() {
    cat <<EOF
Usage: $0 [options]

Options:
  --release   Build and run release binaries instead of debug (default: debug)
  --mock      Run hyprforge-displayd against the mock backend instead of a
              real compositor connection (safe to use even outside Hyprland)
  -h, --help  Show this help
EOF
}

for arg in "$@"; do
    case "$arg" in
        --release) MODE=release ;;
        --mock) MOCK=true ;;
        -h|--help) usage; exit 0 ;;
        *) echo "Unknown option: $arg" >&2; usage >&2; exit 1 ;;
    esac
done

if [ "$MODE" = release ]; then
    CARGO_FLAGS=(--release)
    BIN_DIR=target/release
else
    CARGO_FLAGS=()
    BIN_DIR=target/debug
fi

echo "Building workspace ($MODE)..."
cargo build --workspace "${CARGO_FLAGS[@]}"

DISPLAYD_BIN="$BIN_DIR/hyprforge-displayd"
SETTINGS_BIN="$BIN_DIR/hyprforge-settings"

DAEMON_PID=""
STARTED_DAEMON=false

cleanup() {
    if [ "$STARTED_DAEMON" = true ] && [ -n "$DAEMON_PID" ]; then
        echo "Stopping hyprforge-displayd (pid $DAEMON_PID)..."
        kill "$DAEMON_PID" 2>/dev/null || true
        wait "$DAEMON_PID" 2>/dev/null || true
    fi
}
trap cleanup EXIT INT TERM

if systemctl --user is-active --quiet hyprforge-displayd 2>/dev/null; then
    echo "hyprforge-displayd is already running as a systemd service — using that."
elif pgrep -f "$DISPLAYD_BIN run" >/dev/null 2>&1; then
    echo "hyprforge-displayd is already running — using that."
else
    if [ "$MOCK" = true ]; then
        echo "Starting hyprforge-displayd (mock backend)..."
        "$DISPLAYD_BIN" run --mock &
    else
        echo "Starting hyprforge-displayd..."
        "$DISPLAYD_BIN" run &
    fi
    DAEMON_PID=$!
    STARTED_DAEMON=true
    sleep 1
fi

echo "Starting hyprforge-settings..."
"$SETTINGS_BIN"
