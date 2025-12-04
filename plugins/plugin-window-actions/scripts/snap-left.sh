#!/usr/bin/env bash
set -euo pipefail

gdbus call --session --dest org.Cinnamon --object-path /org/Cinnamon --method org.Cinnamon.Eval "
    const win = global.display.focus_window;
    if (!win) {
        'ERROR: No focused window';
    } else {
        if (win.maximized_horizontally || win.maximized_vertically) {
            win.unmaximize(3);
        }

        const workArea = win.get_work_area_current_monitor();
        const newWidth = Math.floor(workArea.width / 2);
        const newHeight = workArea.height;
        const newX = workArea.x;
        const newY = workArea.y;

        win.move_resize_frame(true, newX, newY, newWidth, newHeight);

        'Snapped to left half';
    }
" 2>&1 > /dev/null
