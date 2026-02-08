#!/bin/bash
set -e

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
BINARY="$SCRIPT_DIR/target/release/pointzerver"
SETTINGS_URL="http://127.0.0.1:42700/plugins/plugin-pointz/"

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

if [[ -x "$BINARY" ]]; then
    exec "$BINARY"
else
    echo "pointzerver not built. Run 'make release' in plugin directory." >&2
    exit 1
fi
