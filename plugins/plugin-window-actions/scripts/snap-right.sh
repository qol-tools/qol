#!/usr/bin/env bash
set -euo pipefail
source "$(dirname "$(readlink -f "$0")")/lib.sh"

win=$(get_active_window) || exit 0
read -r win_x win_y win_w win_h < <(get_window_geometry "$win")
[ -z "$win_x" ] && exit 0

read -r mon_w mon_h mon_x mon_y < <(find_window_monitor "$win_x" "$win_y" "$win_w" "$win_h") || exit 0

unmaximize_if_needed "$win"
move_resize_window "$win" "$((mon_x + mon_w / 2))" "$mon_y" "$((mon_w / 2))" "$mon_h"
