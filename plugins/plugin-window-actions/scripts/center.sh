#!/usr/bin/env bash
set -euo pipefail
source "$(dirname "$(readlink -f "$0")")/lib.sh"

CENTER_W=1152
CENTER_H=892

win=$(get_active_window) || exit 0
read -r win_x win_y win_w win_h < <(get_window_geometry "$win")
[ -z "$win_x" ] && exit 0

read -r mon_w mon_h mon_x mon_y < <(find_window_monitor "$win_x" "$win_y" "$win_w" "$win_h") || exit 0

new_x=$(( mon_x + (mon_w - CENTER_W) / 2 ))
new_y=$(( mon_y + (mon_h - CENTER_H) / 2 ))

unmaximize_window "$win"
move_resize_window "$win" "$new_x" "$new_y" "$CENTER_W" "$CENTER_H"

