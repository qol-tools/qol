#!/bin/bash
set -e

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
SETTINGS_URL="http://127.0.0.1:42700/plugins/plugin-pointz/"

resolve_binary() {
    local candidates=(
        "$SCRIPT_DIR/pointzerver"
        "$SCRIPT_DIR/target/release/pointzerver"
        "$SCRIPT_DIR/target/debug/pointzerver"
    )

    local candidate
    for candidate in "${candidates[@]}"; do
        if [[ -x "$candidate" ]]; then
            echo "$candidate"
            return 0
        fi
    done

    if command -v pointzerver >/dev/null 2>&1; then
        command -v pointzerver
        return 0
    fi

    return 1
}

if [[ "${1:-}" == "settings" ]]; then
    if command -v xdg-open >/dev/null 2>&1; then
        xdg-open "$SETTINGS_URL" >/dev/null 2>&1 &
        exit 0
    fi
    if command -v open >/dev/null 2>&1; then
        open "$SETTINGS_URL" >/dev/null 2>&1 &
        exit 0
    fi
    if command -v cmd.exe >/dev/null 2>&1; then
        cmd.exe /C start "$SETTINGS_URL" >/dev/null 2>&1 &
        exit 0
    fi
    exit 1
fi

BINARY="$(resolve_binary || true)"
if [[ -z "$BINARY" ]]; then
    echo "pointzerver binary not found (checked ./pointzerver, ./target/release/pointzerver, ./target/debug/pointzerver, PATH)." >&2
    exit 1
fi

exec "$BINARY"
