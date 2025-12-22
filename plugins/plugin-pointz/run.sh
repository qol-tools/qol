#!/bin/bash
set -e

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
BINARY="$SCRIPT_DIR/target/release/pointzerver"

if [[ -x "$BINARY" ]]; then
    exec "$BINARY"
else
    echo "pointzerver not built. Run 'make release' in plugin directory." >&2
    exit 1
fi
