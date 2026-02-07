#!/bin/bash
set -e

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
BINARY="$SCRIPT_DIR/launcher"

if [[ ! -x "$BINARY" ]]; then
    if command -v launcher &> /dev/null; then
        BINARY="launcher"
    else
        echo "launcher binary not found" >&2
        exit 1
    fi
fi

if [[ "$1" == "open" ]]; then
    setsid "$BINARY" --show &
    exit 0
else
    exec "$BINARY" "$@"
fi
