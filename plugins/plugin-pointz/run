#!/bin/bash
set -e

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
BINARY="$SCRIPT_DIR/pointzerver"

if [[ -x "$BINARY" ]]; then
    exec "$BINARY"
elif command -v pointzerver &> /dev/null; then
    exec pointzerver
else
    echo "pointzerver not found" >&2
    echo "Install from: https://github.com/qol-tools/pointZ/releases" >&2
    exit 1
fi

