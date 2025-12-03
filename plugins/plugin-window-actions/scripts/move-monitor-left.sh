#!/usr/bin/env bash
set -euo pipefail

CACHE="/tmp/qol-monitors.cache"
CACHE_TTL=60

win=$(xdotool getactivewindow 2>/dev/null) || exit 0

# Get Frame extents
read -r left right top bottom < <(xprop -id "$win" _NET_FRAME_EXTENTS 2>/dev/null | grep -oP '[0-9]+' | xargs)
frame_top=${top:-0}

# Get client window geometry (content only)
eval "$(xwininfo -id "$win" 2>/dev/null | awk '
    /Absolute upper-left X:/ {print "win_x="$4}
    /Absolute upper-left Y:/ {print "win_y="$4}
    /Width:/ {print "win_w="$2}
    /Height:/ {print "win_h="$2}
')"
[ -z "${win_x:-}" ] && exit 0

# Actual visual top-left including titlebar
visual_x=$((win_x - ${left:-0}))
visual_y=$((win_y - frame_top))

if [[ -f "$CACHE" ]] && (( $(date +%s) - $(stat -c %Y "$CACHE") < CACHE_TTL )); then
    mapfile -t monitors < "$CACHE"
else
    mapfile -t monitors < <(xrandr --query 2>/dev/null | awk '
        / connected/ {
            for(i=1; i<=NF; i++) {
                if ($i ~ /^[0-9]+x[0-9]+\+[0-9]+\+[0-9]+$/) print $i
            }
        }
    ' | sort -t'+' -k3 -n)
    printf '%s\n' "${monitors[@]}" > "$CACHE"
fi

[ ${#monitors[@]} -lt 2 ] && exit 0

# Center point for monitor detection
win_cx=$((visual_x + win_w / 2))
win_cy=$((visual_y + win_h / 2))

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

target_idx=$(( (current_idx - 1 + ${#monitors[@]}) % ${#monitors[@]} ))
IFS='x+' read -r tgt_w tgt_h tgt_x tgt_y <<< "${monitors[$target_idx]}"

# Calculate relative center position
cur_cx_rel=$(( win_cx - cur_x ))
cur_cy_rel=$(( win_cy - cur_y ))

tgt_cx_rel=$(( cur_cx_rel * tgt_w / cur_w ))
tgt_cy_rel=$(( cur_cy_rel * tgt_h / cur_h ))

# New center -> New top-left
new_cx=$(( tgt_x + tgt_cx_rel ))
new_cy=$(( tgt_y + tgt_cy_rel ))

new_x=$(( new_cx - win_w / 2 ))
new_y=$(( new_cy - win_h / 2 ))

# wmctrl moves the client area, but we calculated the visual top-left?
# Wait, if we use wmctrl -e, it expects coordinates. EWMH spec says:
# "The x, y coordinates... are those of the root window..."
# BUT most WMs interpret this as the top-left of the FRAME.
# EXCEPT when they don't.

# Let's try compensating for the frame:
# If wmctrl places the CONTENT at (x,y), we need to subtract frame_top
# BUT usually wmctrl -e places the FRAME at (x,y).

# Let's assume wmctrl places the FRAME.
# Our new_x/new_y are calculated for the visual top-left (frame).
# So we should pass them directly?

# Let's use xdotool windowmove, it's usually more consistent with decoration handling
xdotool windowmove "$win" "$new_x" "$new_y"
