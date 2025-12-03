#!/usr/bin/env bash

get_active_window() {
    local win
    win=$(xdotool getactivewindow 2>/dev/null || echo "")
    [ -z "$win" ] && return 1
    
    if ! xprop -id "$win" _NET_WM_WINDOW_TYPE 2>/dev/null | grep -qE "_NET_WM_WINDOW_TYPE_(NORMAL|DIALOG|UTILITY)"; then
        return 1
    fi
    
    echo "$win"
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

unmaximize_window() {
    local win="$1"
    wmctrl -ir "$win" -b remove,maximized_vert,maximized_horz 2>/dev/null
}

maximize_window() {
    local win="$1"
    wmctrl -ir "$win" -b add,maximized_vert,maximized_horz 2>/dev/null
}

move_resize_window() {
    local win="$1" x="$2" y="$3" w="$4" h="$5"
    wmctrl -ir "$win" -e "0,$x,$y,$w,$h" 2>/dev/null
}

