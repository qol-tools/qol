#!/bin/bash
set -euo pipefail

query="$1"
limit=50

if [[ -z "$query" ]]; then
    exit 0
fi

mdfind -name "$query" 2>/dev/null | head -n "$limit"
