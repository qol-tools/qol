#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
LAUNCHER="$SCRIPT_DIR/launcher"

if [[ ! -x "$LAUNCHER" ]]; then
    if command -v launcher >/dev/null 2>&1; then
        LAUNCHER="launcher"
    else
        echo "launcher binary not found" >&2
        exit 1
    fi
fi

if [[ "${1:-}" == "open" ]]; then
    setsid "$LAUNCHER" --show >/dev/null 2>&1 &
    exit 0
fi

exec "$LAUNCHER" "$@"
