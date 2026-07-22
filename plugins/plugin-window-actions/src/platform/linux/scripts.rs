fn snap_script(
    edge: &str,
    fraction: f64,
    requested_width: &str,
    requested_height: &str,
    target_x: &str,
    target_y: &str,
) -> String {
    format!(
        r#"
    const win = global.display.focus_window;
    if (!win) {{
        'ERROR: No focused window';
    }} else {{
        if (win.maximized_horizontally || win.maximized_vertically) {{
            win.unmaximize(3);
        }}

        const workArea = win.get_work_area_current_monitor();
        const fraction = {fraction};
        const requestedWidth = {requested_width};
        const requestedHeight = {requested_height};
        const before = win.get_frame_rect();

        win.move_resize_frame(true, before.x, before.y, requestedWidth, requestedHeight);

        const resized = win.get_frame_rect();
        const targetX = {target_x};
        const targetY = {target_y};

        win.move_frame(true, targetX, targetY);

        const actual = win.get_frame_rect();
        if (actual.x !== targetX || actual.y !== targetY) {{
            throw new Error('Failed to snap {edge}');
        }}

        'snap edge={edge} requested=' + requestedWidth + 'x' + requestedHeight
            + ' actual=' + actual.width + 'x' + actual.height
            + ' position=' + actual.x + ',' + actual.y;
    }}
"#
    )
}

pub fn snap_left_script(fraction: f64) -> String {
    snap_script(
        "left",
        fraction,
        "Math.floor(workArea.width * fraction)",
        "workArea.height",
        "workArea.x",
        "workArea.y",
    )
}

pub fn snap_right_script(fraction: f64) -> String {
    snap_script(
        "right",
        fraction,
        "Math.floor(workArea.width * fraction)",
        "workArea.height",
        "workArea.x + workArea.width - resized.width",
        "workArea.y",
    )
}

pub fn snap_bottom_script(fraction: f64) -> String {
    snap_script(
        "bottom",
        fraction,
        "workArea.width",
        "Math.floor(workArea.height * fraction)",
        "workArea.x",
        "workArea.y + workArea.height - resized.height",
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
    }} else {{
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
        const before = win.get_frame_rect();
        win.move_resize_frame(true, before.x, before.y, newWidth, newHeight);

        const resized = win.get_frame_rect();
        const newX = workArea.x + Math.floor((workArea.width - resized.width) / 2);
        const newY = workArea.y + Math.floor((workArea.height - resized.height) / 2);

        win.move_frame(true, newX, newY);

        const actual = win.get_frame_rect();
        if (actual.x !== newX || actual.y !== newY) {{
            throw new Error('Failed to center window');
        }}

        'Centered window actual=' + actual.width + 'x' + actual.height
            + ' position=' + actual.x + ',' + actual.y;
    }}
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::{CenterMode, WindowActionsConfig};

    fn config() -> WindowActionsConfig {
        WindowActionsConfig {
            center_mode: CenterMode::Pixels,
            center_width_px: 1152.0,
            center_height_px: 892.0,
            center_width_percent: 0.64,
            center_height_percent: 0.79,
            snap_fraction: 0.5,
            reveal_taskbar_after_move: true,
            glide_speed_px_per_second: 1200.0,
        }
    }

    fn no_focused_window_is_guarded(script: &str) -> bool {
        script.contains("'ERROR: No focused window';\n    } else {")
    }

    #[test]
    fn no_focused_window_guards_every_win_dereference() {
        let config = config();
        let cases = [
            ("snap_left_script", snap_left_script(0.5)),
            ("snap_right_script", snap_right_script(0.5)),
            ("snap_bottom_script", snap_bottom_script(0.5)),
            ("center_script", center_script(&config)),
            ("MAXIMIZE_SCRIPT", MAXIMIZE_SCRIPT.to_string()),
            (
                "MOVE_MONITOR_LEFT_SCRIPT",
                MOVE_MONITOR_LEFT_SCRIPT.to_string(),
            ),
            (
                "MOVE_MONITOR_RIGHT_SCRIPT",
                MOVE_MONITOR_RIGHT_SCRIPT.to_string(),
            ),
        ];
        for (name, script) in cases {
            assert!(
                no_focused_window_is_guarded(&script),
                "{name}: `win` must not be dereferenced outside the `else` branch of the focus_window null check\n{script}",
            );
        }
    }

    #[test]
    fn snap_scripts_anchor_the_constrained_frame() {
        let cases = [
            (
                "left",
                snap_left_script(0.5),
                "const targetX = workArea.x;",
                "const targetY = workArea.y;",
            ),
            (
                "right",
                snap_right_script(0.5),
                "const targetX = workArea.x + workArea.width - resized.width;",
                "const targetY = workArea.y;",
            ),
            (
                "bottom",
                snap_bottom_script(0.5),
                "const targetX = workArea.x;",
                "const targetY = workArea.y + workArea.height - resized.height;",
            ),
        ];
        for (edge, script, target_x, target_y) in cases {
            assert!(
                script.contains("win.move_resize_frame(true, before.x, before.y"),
                "{edge}: resize must preserve position until Muffin applies size constraints\n{script}",
            );
            assert!(
                script.contains(target_x),
                "{edge}: constrained width must determine the horizontal anchor\n{script}",
            );
            assert!(
                script.contains(target_y),
                "{edge}: constrained height must determine the vertical anchor\n{script}",
            );
            assert!(
                script.contains("win.move_frame(true, targetX, targetY);"),
                "{edge}: constrained frame must be moved separately\n{script}",
            );
        }
    }

    #[test]
    fn center_script_anchors_the_constrained_frame() {
        let script = center_script(&config());

        assert!(script
            .contains("win.move_resize_frame(true, before.x, before.y, newWidth, newHeight);"));
        assert!(script.contains("const resized = win.get_frame_rect();"));
        assert!(script.contains("workArea.width - resized.width"));
        assert!(script.contains("workArea.height - resized.height"));
        assert!(script.contains("win.move_frame(true, newX, newY);"));
    }
}
