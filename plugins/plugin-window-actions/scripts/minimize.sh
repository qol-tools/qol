#!/usr/bin/env bash
set -euo pipefail

win=$(xdotool getactivewindow 2>/dev/null || echo "")
[ -z "$win" ] && exit 0

xdotool windowminimize "$win" 2>/dev/null

