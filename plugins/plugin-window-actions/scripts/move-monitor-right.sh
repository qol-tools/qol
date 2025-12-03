#!/usr/bin/env bash
set -euo pipefail
source "$(dirname "$(readlink -f "$0")")/lib.sh"

win=$(get_active_window) || exit 0
read -r win_x win_y win_w win_h < <(get_window_geometry "$win")
[ -z "$win_x" ] && exit 0

win_cx=$((win_x + win_w / 2))
win_cy=$((win_y + win_h / 2))

mapfile -t monitors < <(get_monitors | sort -t'+' -k3 -n)
[ ${#monitors[@]} -lt 2 ] && exit 0

current_idx=-1
for i in "${!monitors[@]}"; do
    mon="${monitors[$i]}"
    IFS='x+' read -r mon_w mon_h mon_x mon_y <<< "$mon"
    
    if (( win_cx >= mon_x && win_cx < mon_x + mon_w && win_cy >= mon_y && win_cy < mon_y + mon_h )); then
        current_idx=$i
        break
    fi
done

[ "$current_idx" -lt 0 ] && exit 0

target_idx=$(( (current_idx + 1) % ${#monitors[@]} ))
target="${monitors[$target_idx]}"

IFS='x+' read -r tgt_w tgt_h tgt_x tgt_y <<< "$target"

IFS='x+' read -r cur_w cur_h cur_x cur_y <<< "${monitors[$current_idx]}"

rel_x=$(( win_x - cur_x ))
rel_y=$(( win_y - cur_y ))

new_x=$(( tgt_x + rel_x * tgt_w / cur_w ))
new_y=$(( tgt_y + rel_y * tgt_h / cur_h ))
new_w=$(( win_w * tgt_w / cur_w ))
new_h=$(( win_h * tgt_h / cur_h ))

unmaximize_window "$win"
move_resize_window "$win" "$new_x" "$new_y" "$new_w" "$new_h"

