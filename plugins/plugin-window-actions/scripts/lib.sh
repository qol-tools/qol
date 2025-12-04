#!/usr/bin/env bash

get_active_window() {
    xdotool getactivewindow 2>/dev/null
}

get_window_geometry() {
    local win="$1"
    xwininfo -id "$win" 2>/dev/null | awk '
        /Absolute upper-left X:/ {x=$4}
        /Absolute upper-left Y:/ {y=$4}
        /Width:/ {w=$2}
        /Height:/ {h=$2}
        END {print x, y, w, h}
    '
}

get_monitors() {
    xrandr --query 2>/dev/null | awk '
        / connected/ {
            for(i=1; i<=NF; i++) {
                if ($i ~ /^[0-9]+x[0-9]+\+[0-9]+\+[0-9]+$/) {
                    print $i
                }
            }
        }
    '
}

find_window_monitor() {
    local win_x="$1" win_y="$2" win_w="$3" win_h="$4"
    local win_cx=$((win_x + win_w / 2))
    local win_cy=$((win_y + win_h / 2))
    
    while read -r mon; do
        [ -z "$mon" ] && continue
        
        IFS='x+' read -r mon_w mon_h mon_x mon_y <<< "$mon"
        
        [ -z "$mon_w" ] || [ -z "$mon_h" ] || [ -z "$mon_x" ] || [ -z "$mon_y" ] && continue
        
        if (( win_cx >= mon_x && win_cx < mon_x + mon_w && win_cy >= mon_y && win_cy < mon_y + mon_h )); then
            echo "$mon_w $mon_h $mon_x $mon_y"
            return 0
        fi
    done < <(get_monitors)
    
    local fallback
    fallback=$(xdotool getdisplaygeometry 2>/dev/null || echo "")
    if [ -n "$fallback" ]; then
        read -r fw fh <<< "$fallback"
        echo "$fw $fh 0 0"
        return 0
    fi
    
    return 1
}

is_maximized() {
    local win="$1"
    xprop -id "$win" _NET_WM_STATE 2>/dev/null | grep -q "_NET_WM_STATE_MAXIMIZED"
}

unmaximize_window() {
    local win="$1"
    wmctrl -ir "$win" -b remove,maximized_vert,maximized_horz 2>/dev/null
}

unmaximize_if_needed() {
    local win="$1"
    if is_maximized "$win"; then
        unmaximize_window "$win"
    fi
}

maximize_window() {
    local win="$1"
    wmctrl -ir "$win" -b add,maximized_vert,maximized_horz 2>/dev/null
}

move_resize_window() {
    local win="$1" x="$2" y="$3" w="$4" h="$5"
    wmctrl -ir "$win" -e "0,$x,$y,$w,$h" 2>/dev/null
}

is_close() {
    local a="${1:-0}"
    local b="${2:-0}"
    local tol="${3:-0}"
    local diff=$((a - b))
    (( diff < 0 )) && diff=$(( -diff ))
    (( diff <= tol ))
}

unsnap_window_via_keyboard() {
    local win="$1"
    xdotool windowactivate --sync "$win" 2>/dev/null
    sleep 0.05
    xdotool key --window "$win" super+Down 2>/dev/null
    sleep 0.25
}

force_move_resize_window() {
    local win="$1" x="$2" y="$3" w="$4" h="$5"
    local log="${6:-/dev/null}"
    
    echo "force_move_resize: win=$win x=$x y=$y w=$w h=$h" >> "$log"
    
    unsnap_window_via_keyboard "$win"
    
    eval "$(xwininfo -id "$win" 2>/dev/null | awk '
        /Absolute upper-left X:/ {print "old_x="$4}
        /Absolute upper-left Y:/ {print "old_y="$4}
        /Width:/ {print "old_w="$2}
        /Height:/ {print "old_h="$2}
    ')"
    
    echo "After unsnap: old_x=$old_x old_y=$old_y old_w=$old_w old_h=$old_h" >> "$log"
    
    wmctrl -ir "$win" -b remove,maximized_vert,maximized_horz 2>/dev/null
    wmctrl -ir "$win" -b remove,fullscreen 2>/dev/null
    xprop -id "$win" -remove _GTK_EDGE_CONSTRAINTS 2>/dev/null
    sleep 0.1
    
    move_resize_window "$win" "$x" "$y" "$w" "$h"
    sleep 0.15
    
    eval "$(xwininfo -id "$win" 2>/dev/null | awk '
        /Absolute upper-left X:/ {print "new_x="$4}
        /Absolute upper-left Y:/ {print "new_y="$4}
        /Width:/ {print "new_w="$2}
        /Height:/ {print "new_h="$2}
    ')"
    
    echo "After move: new_x=$new_x new_y=$new_y new_w=$new_w new_h=$new_h" >> "$log"
    
    local x_diff=$((new_x - x))
    local y_diff=$((new_y - y))
    if (( x_diff < 0 )); then x_diff=$(( -x_diff )); fi
    if (( y_diff < 0 )); then y_diff=$(( -y_diff )); fi
    
    if (( x_diff > 50 || y_diff > 50 )); then
        echo "Move failed, trying xdotool directly" >> "$log"
        xdotool windowactivate --sync "$win" 2>/dev/null
        sleep 0.05
        xdotool windowsize "$win" "$w" "$h" 2>/dev/null
        xdotool windowmove --sync "$win" "$x" "$y" 2>/dev/null
        sleep 0.15
        
        eval "$(xwininfo -id "$win" 2>/dev/null | awk '
            /Absolute upper-left X:/ {print "final_x="$4}
            /Absolute upper-left Y:/ {print "final_y="$4}
        ')"
        echo "Final position: x=$final_x y=$final_y" >> "$log"
    fi
}
