#!/usr/bin/env bash
set -euo pipefail

stacking_raw=$(xprop -root _NET_CLIENT_LIST_STACKING 2>/dev/null || echo "")
[ -z "$stacking_raw" ] && exit 0

printf '%s\n' "$stacking_raw" \
    | sed -E 's/^.*#[[:space:]]*//' \
    | tr ',' '\n' \
    | awk '{$1=$1; if($1 ~ /^0x[0-9a-fA-F]+$/) print $1}' \
    | tac \
    | while read -r win_id; do
        [ -z "$win_id" ] && continue

        if xprop -id "$win_id" _NET_WM_WINDOW_TYPE 2>/dev/null | grep -q "_NET_WM_WINDOW_TYPE_DESKTOP"; then
            continue
        fi

        if xprop -id "$win_id" _NET_WM_STATE 2>/dev/null | grep -q "_NET_WM_STATE_HIDDEN"; then
            wmctrl -ia "$win_id" 2>/dev/null && exit 0
        fi
    done

exit 0

