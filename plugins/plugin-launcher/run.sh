#!/bin/bash
set -e

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
BINARY="$SCRIPT_DIR/launcher"

if [[ -x "$BINARY" ]]; then
    exec "$BINARY"
elif command -v launcher &> /dev/null; then
    exec launcher
else
    echo "launcher binary not found" >&2
    echo "Install from: https://github.com/qol-tools/plugin-launcher/releases" >&2
    exit 1
fi
