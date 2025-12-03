#!/usr/bin/env bash
set -euo pipefail
source "$(dirname "$(readlink -f "$0")")/lib.sh"

win=$(get_active_window) || exit 0
maximize_window "$win"

