#!/usr/bin/env bash
set -euo pipefail

CACHE="/tmp/qol-monitors.cache"
CACHE_TTL=60

win=$(xdotool getactivewindow 2>/dev/null) || exit 0

# Clear tiled/snapped state BEFORE reading geometry
wmctrl -ir "$win" -b remove,maximized_vert,maximized_horz 2>/dev/null
xprop -id "$win" -remove _NET_WM_STATE 2>/dev/null

# Small delay to let WM process state change
sleep 0.02

# NOW read geometry (after state is cleared)
read -r left right top bottom < <(xprop -id "$win" _NET_FRAME_EXTENTS 2>/dev/null | grep -oP '[0-9]+' | xargs)
frame_left=${left:-0}
frame_right=${right:-0}
frame_top=${top:-0}
frame_bottom=${bottom:-0}

eval "$(xwininfo -id "$win" 2>/dev/null | awk '
    /Absolute upper-left X:/ {print "win_x="$4}
    /Absolute upper-left Y:/ {print "win_y="$4}
    /Width:/ {print "win_w="$2}
    /Height:/ {print "win_h="$2}
')"
[ -z "${win_x:-}" ] && exit 0

visual_x=$((win_x - frame_left))
visual_y=$((win_y - frame_top))
visual_w=$((win_w + frame_left + frame_right))
visual_h=$((win_h + frame_top + frame_bottom))

if [[ -f "$CACHE" ]] && (( $(date +%s) - $(stat -c %Y "$CACHE") < CACHE_TTL )); then
    mapfile -t monitors < "$CACHE"
else
    mapfile -t monitors < <(xrandr --query 2>/dev/null | awk '
        / connected/ {
            for(i=1; i<=NF; i++) {
                if ($i ~ /^[0-9]+x[0-9]+\+[0-9]+\+[0-9]+$/) print $i
            }
        }
    ' | sort -t'+' -k2 -n)
    printf '%s\n' "${monitors[@]}" > "$CACHE"
fi

[ ${#monitors[@]} -lt 2 ] && exit 0

win_cx=$((visual_x + visual_w / 2))
win_cy=$((visual_y + visual_h / 2))

current_idx=-1
for i in "${!monitors[@]}"; do
    IFS='x+' read -r mon_w mon_h mon_x mon_y <<< "${monitors[$i]}"
    if (( win_cx >= mon_x && win_cx < mon_x + mon_w && win_cy >= mon_y && win_cy < mon_y + mon_h )); then
        current_idx=$i
        cur_w=$mon_w cur_h=$mon_h cur_x=$mon_x cur_y=$mon_y
        break
    fi
done
[ "$current_idx" -lt 0 ] && exit 0

target_idx=$(( (current_idx + 1) % ${#monitors[@]} ))
IFS='x+' read -r tgt_w tgt_h tgt_x tgt_y <<< "${monitors[$target_idx]}"

cur_cx_rel=$(( win_cx - cur_x ))
cur_cy_rel=$(( win_cy - cur_y ))

tgt_cx_rel=$(( cur_cx_rel * tgt_w / cur_w ))
tgt_cy_rel=$(( cur_cy_rel * tgt_h / cur_h ))

new_w=$(( win_w * tgt_w / cur_w ))
new_h=$(( win_h * tgt_h / cur_h ))

new_visual_w=$((new_w + frame_left + frame_right))
new_visual_h=$((new_h + frame_top + frame_bottom))

new_cx=$(( tgt_x + tgt_cx_rel ))
new_cy=$(( tgt_y + tgt_cy_rel ))

new_x=$(( new_cx - new_visual_w / 2 ))
new_y=$(( new_cy - new_visual_h / 2 ))

xdotool windowsize "$win" "$new_w" "$new_h"
xdotool windowmove "$win" "$new_x" "$new_y"
