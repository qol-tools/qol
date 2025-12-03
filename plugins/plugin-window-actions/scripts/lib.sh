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

