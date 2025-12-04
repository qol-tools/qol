#!/usr/bin/env bash
set -euo pipefail

CENTER_W=1152
CENTER_H=892

gdbus call --session --dest org.Cinnamon --object-path /org/Cinnamon --method org.Cinnamon.Eval "
    const win = global.display.focus_window;
    if (!win) {
        'ERROR: No focused window';
    } else {
        if (win.maximized_horizontally || win.maximized_vertically) {
            win.unmaximize(3);
        }

        const workArea = win.get_work_area_current_monitor();
        const newWidth = $CENTER_W;
        const newHeight = $CENTER_H;
        const newX = workArea.x + Math.floor((workArea.width - newWidth) / 2);
        const newY = workArea.y + Math.floor((workArea.height - newHeight) / 2);

        win.move_resize_frame(true, newX, newY, newWidth, newHeight);

        'Centered window';
    }
" 2>&1 > /dev/null
