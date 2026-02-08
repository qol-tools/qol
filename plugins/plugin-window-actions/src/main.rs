use std::env;
use std::process::{Command, ExitCode};
use std::thread;
use std::time::Duration;

const SNAP_LEFT_SCRIPT: &str = r#"
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
"#;

const SNAP_RIGHT_SCRIPT: &str = r#"
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
        const newX = workArea.x + Math.floor(workArea.width / 2);
        const newY = workArea.y;

        win.move_resize_frame(true, newX, newY, newWidth, newHeight);
        'Snapped to right half';
    }
"#;

const SNAP_BOTTOM_SCRIPT: &str = r#"
    const win = global.display.focus_window;
    if (!win) {
        'ERROR: No focused window';
    } else {
        if (win.maximized_horizontally || win.maximized_vertically) {
            win.unmaximize(3);
        }

        const workArea = win.get_work_area_current_monitor();
        const newWidth = workArea.width;
        const newHeight = Math.floor(workArea.height / 2);
        const newX = workArea.x;
        const newY = workArea.y + Math.floor(workArea.height / 2);

        win.move_resize_frame(true, newX, newY, newWidth, newHeight);
        'Snapped to bottom half';
    }
"#;

const MAXIMIZE_SCRIPT: &str = r#"
    const win = global.display.focus_window;
    if (!win) {
        'ERROR: No focused window';
    } else {
        win.maximize(3);
        'Maximized window';
    }
"#;

const CENTER_SCRIPT: &str = r#"
    const win = global.display.focus_window;
    if (!win) {
        'ERROR: No focused window';
    } else {
        if (win.maximized_horizontally || win.maximized_vertically) {
            win.unmaximize(3);
        }

        const workArea = win.get_work_area_current_monitor();
        const newWidth = 1152;
        const newHeight = 892;
        const newX = workArea.x + Math.floor((workArea.width - newWidth) / 2);
        const newY = workArea.y + Math.floor((workArea.height - newHeight) / 2);

        win.move_resize_frame(true, newX, newY, newWidth, newHeight);
        'Centered window';
    }
"#;

const MOVE_MONITOR_LEFT_SCRIPT: &str = r#"
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

const MOVE_MONITOR_RIGHT_SCRIPT: &str = r#"
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

fn main() -> ExitCode {
    let action = match env::args().nth(1) {
        Some(action) => action,
        None => {
            eprintln!("Usage: window-actions <action>");
            return ExitCode::from(1);
        }
    };

    if let Err(error) = execute_action(&action) {
        eprintln!("{error}");
        return ExitCode::from(1);
    }

    ExitCode::SUCCESS
}

fn execute_action(action: &str) -> Result<(), String> {
    match action {
        "snap-left" => run_cinnamon_eval(SNAP_LEFT_SCRIPT).map(|_| ()),
        "snap-right" => run_cinnamon_eval(SNAP_RIGHT_SCRIPT).map(|_| ()),
        "snap-bottom" => run_cinnamon_eval(SNAP_BOTTOM_SCRIPT).map(|_| ()),
        "maximize" => run_cinnamon_eval(MAXIMIZE_SCRIPT).map(|_| ()),
        "minimize" => minimize_window(),
        "restore" => restore_window(),
        "center" => run_cinnamon_eval(CENTER_SCRIPT).map(|_| ()),
        "move-monitor-left" => move_monitor(MOVE_MONITOR_LEFT_SCRIPT),
        "move-monitor-right" => move_monitor(MOVE_MONITOR_RIGHT_SCRIPT),
        _ => Err(format!("Unknown action: {action}")),
    }
}

fn run_cinnamon_eval(script: &str) -> Result<String, String> {
    let output = Command::new("gdbus")
        .args([
            "call",
            "--session",
            "--dest",
            "org.Cinnamon",
            "--object-path",
            "/org/Cinnamon",
            "--method",
            "org.Cinnamon.Eval",
            script,
        ])
        .output()
        .map_err(|e| format!("Failed to run gdbus: {e}"))?;

    if output.status.success() {
        Ok(String::from_utf8_lossy(&output.stdout).to_string())
    } else {
        Err(format!(
            "gdbus failed: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        ))
    }
}

fn minimize_window() -> Result<(), String> {
    let output = Command::new("xdotool")
        .arg("getactivewindow")
        .output()
        .map_err(|e| format!("Failed to run xdotool: {e}"))?;

    if !output.status.success() {
        return Ok(());
    }

    let window_id = String::from_utf8_lossy(&output.stdout).trim().to_string();
    if window_id.is_empty() {
        return Ok(());
    }

    let status = Command::new("xdotool")
        .args(["windowminimize", &window_id])
        .status()
        .map_err(|e| format!("Failed to run xdotool: {e}"))?;

    if status.success() {
        Ok(())
    } else {
        Err("Failed to minimize window".to_string())
    }
}

fn restore_window() -> Result<(), String> {
    let output = Command::new("xprop")
        .args(["-root", "_NET_CLIENT_LIST_STACKING"])
        .output()
        .map_err(|e| format!("Failed to run xprop: {e}"))?;

    if !output.status.success() {
        return Ok(());
    }

    let raw = String::from_utf8_lossy(&output.stdout);
    let Some(list) = raw.split('#').nth(1) else {
        return Ok(());
    };

    let mut window_ids: Vec<String> = list
        .split(',')
        .map(|id| id.trim().to_string())
        .filter(|id| is_window_id(id))
        .collect();
    window_ids.reverse();

    for window_id in window_ids {
        if is_desktop_window(&window_id)? {
            continue;
        }
        if is_hidden_window(&window_id)? && activate_window(&window_id)? {
            break;
        }
    }

    Ok(())
}

fn move_monitor(script: &str) -> Result<(), String> {
    let output = run_cinnamon_eval(script)?;
    if output.contains("to 1 |") && !output.contains("fullscreen=true") {
        reveal_taskbar()?;
    }
    Ok(())
}

fn reveal_taskbar() -> Result<(), String> {
    let output = Command::new("xdotool")
        .args(["getmouselocation", "--shell"])
        .output()
        .map_err(|e| format!("Failed to run xdotool: {e}"))?;

    if !output.status.success() {
        return Err("Failed to read mouse location".to_string());
    }

    let raw = String::from_utf8_lossy(&output.stdout);
    let mut x = None;
    let mut y = None;
    for line in raw.lines() {
        if let Some(value) = line.strip_prefix("X=") {
            x = value.parse::<i32>().ok();
        }
        if let Some(value) = line.strip_prefix("Y=") {
            y = value.parse::<i32>().ok();
        }
    }

    let x = x.ok_or_else(|| "Missing mouse X coordinate".to_string())?;
    let y = y.ok_or_else(|| "Missing mouse Y coordinate".to_string())?;

    run_status("xdotool", &["mousemove", "--sync", "3200", "1439"])?;
    thread::sleep(Duration::from_millis(100));
    run_status("xdotool", &["mousemove", &x.to_string(), &y.to_string()])?;
    Ok(())
}

fn is_window_id(id: &str) -> bool {
    id.starts_with("0x") && id.chars().skip(2).all(|c| c.is_ascii_hexdigit())
}

fn is_desktop_window(window_id: &str) -> Result<bool, String> {
    let output = run_output("xprop", &["-id", window_id, "_NET_WM_WINDOW_TYPE"])?;
    Ok(output.contains("_NET_WM_WINDOW_TYPE_DESKTOP"))
}

fn is_hidden_window(window_id: &str) -> Result<bool, String> {
    let output = run_output("xprop", &["-id", window_id, "_NET_WM_STATE"])?;
    Ok(output.contains("_NET_WM_STATE_HIDDEN"))
}

fn activate_window(window_id: &str) -> Result<bool, String> {
    let status = Command::new("wmctrl")
        .args(["-ia", window_id])
        .status()
        .map_err(|e| format!("Failed to run wmctrl: {e}"))?;
    Ok(status.success())
}

fn run_output(command: &str, args: &[&str]) -> Result<String, String> {
    let output = Command::new(command)
        .args(args)
        .output()
        .map_err(|e| format!("Failed to run {command}: {e}"))?;
    Ok(String::from_utf8_lossy(&output.stdout).to_string())
}

fn run_status(command: &str, args: &[&str]) -> Result<(), String> {
    let status = Command::new(command)
        .args(args)
        .status()
        .map_err(|e| format!("Failed to run {command}: {e}"))?;
    if status.success() {
        Ok(())
    } else {
        Err(format!("{command} exited with status {status}"))
    }
}
