pub fn snap_left_script(fraction: f64) -> String {
    format!(
        r#"
    const win = global.display.focus_window;
    if (!win) {{
        'ERROR: No focused window';
    }}
    if (win.maximized_horizontally || win.maximized_vertically) {{
        win.unmaximize(3);
    }}

    const workArea = win.get_work_area_current_monitor();
    const fraction = {fraction};
    const newWidth = Math.floor(workArea.width * fraction);
    const newHeight = workArea.height;
    const newX = workArea.x;
    const newY = workArea.y;

    win.move_resize_frame(true, newX, newY, newWidth, newHeight);
    'Snapped to left region';
"#
    )
}

pub fn snap_right_script(fraction: f64) -> String {
    format!(
        r#"
    const win = global.display.focus_window;
    if (!win) {{
        'ERROR: No focused window';
    }}
    if (win.maximized_horizontally || win.maximized_vertically) {{
        win.unmaximize(3);
    }}

    const workArea = win.get_work_area_current_monitor();
    const fraction = {fraction};
    const newWidth = Math.floor(workArea.width * fraction);
    const newHeight = workArea.height;
    const newX = workArea.x + (workArea.width - newWidth);
    const newY = workArea.y;

    win.move_resize_frame(true, newX, newY, newWidth, newHeight);
    'Snapped to right region';
"#
    )
}

pub fn snap_bottom_script(fraction: f64) -> String {
    format!(
        r#"
    const win = global.display.focus_window;
    if (!win) {{
        'ERROR: No focused window';
    }}
    if (win.maximized_horizontally || win.maximized_vertically) {{
        win.unmaximize(3);
    }}

    const workArea = win.get_work_area_current_monitor();
    const fraction = {fraction};
    const newWidth = workArea.width;
    const newHeight = Math.floor(workArea.height * fraction);
    const newX = workArea.x;
    const newY = workArea.y + (workArea.height - newHeight);

    win.move_resize_frame(true, newX, newY, newWidth, newHeight);
    'Snapped to bottom region';
"#
    )
}

pub const MAXIMIZE_SCRIPT: &str = r#"
    const win = global.display.focus_window;
    if (!win) {
        'ERROR: No focused window';
    } else {
        win.maximize(3);
        'Maximized window';
    }
"#;

pub fn center_script(config: &crate::config::WindowActionsConfig) -> String {
    let width = config.center_width_px.round().max(1.0);
    let height = config.center_height_px.round().max(1.0);
    let width_percent = config.center_width_percent.clamp(0.1, 1.0);
    let height_percent = config.center_height_percent.clamp(0.1, 1.0);
    let use_percent = if config.center_mode == crate::config::CenterMode::Percent {
        "true"
    } else {
        "false"
    };
    format!(
        r#"
    const win = global.display.focus_window;
    if (!win) {{
        'ERROR: No focused window';
    }}
    if (win.maximized_horizontally || win.maximized_vertically) {{
        win.unmaximize(3);
    }}

    const workArea = win.get_work_area_current_monitor();
    const usePercent = {use_percent};
    const newWidth = usePercent
        ? Math.min(Math.floor(workArea.width * {width_percent}), workArea.width)
        : Math.min({width}, workArea.width);
    const newHeight = usePercent
        ? Math.min(Math.floor(workArea.height * {height_percent}), workArea.height)
        : Math.min({height}, workArea.height);
    const newX = workArea.x + Math.floor((workArea.width - newWidth) / 2);
    const newY = workArea.y + Math.floor((workArea.height - newHeight) / 2);

    win.move_resize_frame(true, newX, newY, newWidth, newHeight);
    'Centered window';
"#
    )
}

pub const MOVE_MONITOR_LEFT_SCRIPT: &str = r#"
    const win = global.display.focus_window;
    if (!win) {
        'ERROR: No focused window';
    } else {
        const beforeRect = win.get_frame_rect();
        const beforeWorkArea = win.get_work_area_current_monitor();
        const beforeMonitor = win.get_monitor();

        const widthRatio = beforeRect.width / beforeWorkArea.width;
        const heightRatio = beforeRect.height / beforeWorkArea.height;
        const xRatio = (beforeRect.x - beforeWorkArea.x) / beforeWorkArea.width;
        const yRatio = (beforeRect.y - beforeWorkArea.y) / beforeWorkArea.height;

        const numMonitors = global.display.get_n_monitors();
        const prevMonitor = (beforeMonitor - 1 + numMonitors) % numMonitors;

        win.move_to_monitor(prevMonitor);

        const afterWorkArea = win.get_work_area_current_monitor();
        const newWidth = Math.floor(afterWorkArea.width * widthRatio);
        const newHeight = Math.floor(afterWorkArea.height * heightRatio);
        const newX = afterWorkArea.x + Math.floor(afterWorkArea.width * xRatio);
        const newY = afterWorkArea.y + Math.floor(afterWorkArea.height * yRatio);

        win.move_resize_frame(true, newX, newY, newWidth, newHeight);

        'Moved from monitor ' + beforeMonitor + ' to ' + prevMonitor + ' | fullscreen=' + win.is_fullscreen();
    }
"#;

pub const MOVE_MONITOR_RIGHT_SCRIPT: &str = r#"
    const win = global.display.focus_window;
    if (!win) {
        'ERROR: No focused window';
    } else {
        const beforeRect = win.get_frame_rect();
        const beforeWorkArea = win.get_work_area_current_monitor();
        const beforeMonitor = win.get_monitor();

        const widthRatio = beforeRect.width / beforeWorkArea.width;
        const heightRatio = beforeRect.height / beforeWorkArea.height;
        const xRatio = (beforeRect.x - beforeWorkArea.x) / beforeWorkArea.width;
        const yRatio = (beforeRect.y - beforeWorkArea.y) / beforeWorkArea.height;

        const numMonitors = global.display.get_n_monitors();
        const nextMonitor = (beforeMonitor + 1) % numMonitors;

        win.move_to_monitor(nextMonitor);

        const afterWorkArea = win.get_work_area_current_monitor();
        const newWidth = Math.floor(afterWorkArea.width * widthRatio);
        const newHeight = Math.floor(afterWorkArea.height * heightRatio);
        const newX = afterWorkArea.x + Math.floor(afterWorkArea.width * xRatio);
        const newY = afterWorkArea.y + Math.floor(afterWorkArea.height * yRatio);

        win.move_resize_frame(true, newX, newY, newWidth, newHeight);

        'Moved from monitor ' + beforeMonitor + ' to ' + nextMonitor + ' | fullscreen=' + win.is_fullscreen();
    }
"#;
