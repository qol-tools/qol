#!/bin/bash
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
BINARY="$SCRIPT_DIR/launcher"

[[ -x "$BINARY" ]] && "$BINARY" --preload &
