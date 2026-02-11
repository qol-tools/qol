use std::env;
use std::fs;
use std::io::ErrorKind;
use std::path::PathBuf;
use std::process::{Command, ExitCode};
use std::thread;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

const LAST_MINIMIZED_WINDOW_FILE_NAME: &str = "qol-window-actions-last-minimized";
const LAST_MINIMIZED_MAX_AGE_SECS: u64 = 60 * 60 * 8;
const LAUNCHER_MATCH_MARKERS: [&str; 4] = [
    "qol-tray-launcher",
    "plugin-launcher",
    "qol-launcher",
    "qol launcher",
];

struct MinimizedWindowRecord {
    window_id: String,
    pid: u32,
    process_start_ticks: u64,
    saved_at_unix_secs: u64,
}

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
        track_last_minimized_window(&window_id);
        Ok(())
    } else {
        Err("Failed to minimize window".to_string())
    }
}

fn restore_window() -> Result<(), String> {
    if restore_last_minimized_window()? {
        return Ok(());
    }

    restore_hidden_window_from_stacking()
}

fn restore_last_minimized_window() -> Result<bool, String> {
    let Some(record) = read_last_minimized_window_record()? else {
        return Ok(false);
    };

    if is_record_expired(&record) {
        clear_last_minimized_window_id();
        return Ok(false);
    }

    if !is_record_current(&record)? {
        clear_last_minimized_window_id();
        return Ok(false);
    }

    if try_restore_window(&record.window_id)? {
        clear_last_minimized_window_id();
        return Ok(true);
    }

    clear_last_minimized_window_id();
    Ok(false)
}

fn restore_hidden_window_from_stacking() -> Result<(), String> {
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
        if try_restore_window(&window_id)? {
            break;
        }
    }

    Ok(())
}

fn try_restore_window(window_id: &str) -> Result<bool, String> {
    if !is_window_id(window_id) {
        return Ok(false);
    }
    if window_query_or(is_excluded_window_type(window_id), false) {
        return Ok(false);
    }
    if is_launcher_window(window_id) {
        return Ok(false);
    }
    if !window_query_or(is_hidden_window(window_id), false) {
        return Ok(false);
    }
    activate_window(window_id)
}

fn window_query_or(value: Result<bool, String>, fallback: bool) -> bool {
    value.unwrap_or(fallback)
}

fn move_monitor(script: &str) -> Result<(), String> {
    let output = run_cinnamon_eval(script)?;
    if monitor_changed(&output) && !output.contains("fullscreen=true") {
        reveal_taskbar()?;
    }
    Ok(())
}

fn monitor_changed(output: &str) -> bool {
    let Some((from, to)) = parse_monitor_move(output) else {
        return false;
    };
    from != to
}

fn parse_monitor_move(output: &str) -> Option<(i32, i32)> {
    let section = output.split("Moved from monitor ").nth(1)?;
    let (from_raw, tail) = section.split_once(" to ")?;
    let (to_raw, _) = tail.split_once(" |")?;
    let from = from_raw.trim().parse::<i32>().ok()?;
    let to = to_raw.trim().parse::<i32>().ok()?;
    Some((from, to))
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

    let (edge_x, edge_y) = display_edge_point(x, y)?;

    run_status(
        "xdotool",
        &["mousemove", "--sync", &edge_x.to_string(), &edge_y.to_string()],
    )?;
    thread::sleep(Duration::from_millis(100));
    run_status("xdotool", &["mousemove", &x.to_string(), &y.to_string()])?;
    Ok(())
}

fn display_edge_point(pointer_x: i32, pointer_y: i32) -> Result<(i32, i32), String> {
    if let Some(bounds) = xrandr_monitor_bounds()
        .into_iter()
        .find(|bounds| bounds.contains(pointer_x, pointer_y))
    {
        let edge_x = pointer_x.clamp(bounds.left(), bounds.right());
        let edge_y = bounds.bottom();
        return Ok((edge_x, edge_y));
    }

    display_edge_point_from_root_geometry()
}

fn display_edge_point_from_root_geometry() -> Result<(i32, i32), String> {
    let output = Command::new("xdotool")
        .arg("getdisplaygeometry")
        .output()
        .map_err(|e| format!("Failed to run xdotool: {e}"))?;

    if !output.status.success() {
        return Err("Failed to read display geometry".to_string());
    }

    let raw = String::from_utf8_lossy(&output.stdout);
    let mut parts = raw.split_whitespace();
    let width = parts
        .next()
        .and_then(|value| value.parse::<i32>().ok())
        .ok_or_else(|| "Missing display width".to_string())?;
    let height = parts
        .next()
        .and_then(|value| value.parse::<i32>().ok())
        .ok_or_else(|| "Missing display height".to_string())?;

    Ok((width.saturating_sub(1), height.saturating_sub(1)))
}

#[derive(Clone, Copy)]
struct MonitorBounds {
    x: i32,
    y: i32,
    width: i32,
    height: i32,
}

impl MonitorBounds {
    fn left(self) -> i32 {
        self.x
    }

    fn right(self) -> i32 {
        self.x.saturating_add(self.width).saturating_sub(1)
    }

    fn bottom(self) -> i32 {
        self.y.saturating_add(self.height).saturating_sub(1)
    }

    fn contains(self, px: i32, py: i32) -> bool {
        px >= self.x
            && px < self.x.saturating_add(self.width)
            && py >= self.y
            && py < self.y.saturating_add(self.height)
    }
}

fn xrandr_monitor_bounds() -> Vec<MonitorBounds> {
    let output = match Command::new("xrandr").arg("--current").output() {
        Ok(output) if output.status.success() => output,
        _ => return Vec::new(),
    };

    String::from_utf8_lossy(&output.stdout)
        .lines()
        .filter(|line| line.contains(" connected"))
        .filter_map(parse_monitor_bounds_from_xrandr_line)
        .collect()
}

fn parse_monitor_bounds_from_xrandr_line(line: &str) -> Option<MonitorBounds> {
    line.split_whitespace().find_map(parse_xrandr_geometry_token)
}

fn parse_xrandr_geometry_token(token: &str) -> Option<MonitorBounds> {
    let (width_raw, rest) = token.split_once('x')?;
    let width = width_raw.parse::<i32>().ok()?;

    let x_offset_start = rest.find(|ch| ch == '+' || ch == '-')?;
    let height = rest.get(..x_offset_start)?.parse::<i32>().ok()?;
    if width <= 0 || height <= 0 {
        return None;
    }

    let offsets = rest.get(x_offset_start..)?;
    let y_offset_start = offsets
        .char_indices()
        .skip(1)
        .find_map(|(idx, ch)| ((ch == '+') || (ch == '-')).then_some(idx))?;
    let x_offset = offsets.get(..y_offset_start)?.parse::<i32>().ok()?;
    let y_offset = offsets.get(y_offset_start..)?.parse::<i32>().ok()?;

    Some(MonitorBounds {
        x: x_offset,
        y: y_offset,
        width,
        height,
    })
}

fn is_window_id(id: &str) -> bool {
    id.starts_with("0x") && id.chars().skip(2).all(|c| c.is_ascii_hexdigit())
}

fn is_excluded_window_type(window_id: &str) -> Result<bool, String> {
    let output = run_output("xprop", &["-id", window_id, "_NET_WM_WINDOW_TYPE"])?;
    Ok(
        output.contains("_NET_WM_WINDOW_TYPE_DESKTOP")
            || output.contains("_NET_WM_WINDOW_TYPE_DOCK"),
    )
}

fn is_hidden_window(window_id: &str) -> Result<bool, String> {
    let output = run_output("xprop", &["-id", window_id, "_NET_WM_STATE"])?;
    Ok(output.contains("_NET_WM_STATE_HIDDEN"))
}

fn is_launcher_window(window_id: &str) -> bool {
    if launcher_pid_matches(window_pid(window_id).ok().flatten()) {
        return true;
    }

    launcher_window_metadata_matches(window_id)
}

fn launcher_pid_matches(pid: Option<u32>) -> bool {
    let Some(pid) = pid else {
        return false;
    };

    launcher_process_cmdline_matches(pid) || launcher_process_name_matches(pid)
}

fn launcher_process_cmdline_matches(pid: u32) -> bool {
    let Some(cmdline) = process_cmdline(pid) else {
        return false;
    };

    launcher_text_matches(&cmdline)
}

fn launcher_process_name_matches(pid: u32) -> bool {
    let Some(name) = process_name(pid) else {
        return false;
    };

    launcher_text_matches(&name)
}

fn launcher_window_metadata_matches(window_id: &str) -> bool {
    let class = run_output_optional("xprop", &["-id", window_id, "WM_CLASS"]);
    let name = run_output_optional("xprop", &["-id", window_id, "_NET_WM_NAME"]);
    let app_id = run_output_optional("xprop", &["-id", window_id, "_GTK_APPLICATION_ID"]);
    let haystack = format!("{class}\n{name}\n{app_id}");
    launcher_text_matches(&haystack)
}

fn launcher_text_matches(value: &str) -> bool {
    let lower = value.to_ascii_lowercase();
    LAUNCHER_MATCH_MARKERS.iter().any(|marker| lower.contains(marker))
}

fn window_pid(window_id: &str) -> Result<Option<u32>, String> {
    let output = run_output("xprop", &["-id", window_id, "_NET_WM_PID"])?;
    let pid = output
        .split('=')
        .nth(1)
        .map(str::trim)
        .and_then(|value| value.split_whitespace().next())
        .and_then(|value| value.parse::<u32>().ok());
    Ok(pid)
}

fn process_name(pid: u32) -> Option<String> {
    let path = format!("/proc/{pid}/comm");
    fs::read_to_string(path).ok().map(|value| value.trim().to_string())
}

fn process_cmdline(pid: u32) -> Option<String> {
    let path = format!("/proc/{pid}/cmdline");
    let data = fs::read(path).ok()?;
    if data.is_empty() {
        return None;
    }

    Some(
        data.split(|byte| *byte == 0)
            .filter(|part| !part.is_empty())
            .map(|part| String::from_utf8_lossy(part).into_owned())
            .collect::<Vec<String>>()
            .join(" "),
    )
}

fn process_start_ticks(pid: u32) -> Option<u64> {
    let path = format!("/proc/{pid}/stat");
    let stat = fs::read_to_string(path).ok()?;
    let (_, rest) = stat.split_once(") ")?;
    let fields: Vec<&str> = rest.split_whitespace().collect();
    fields.get(19)?.parse::<u64>().ok()
}

fn normalize_window_id(window_id: &str) -> Option<String> {
    if is_window_id(window_id) {
        return Some(window_id.to_ascii_lowercase());
    }

    let numeric = window_id.trim().parse::<u64>().ok()?;
    Some(format!("0x{numeric:x}"))
}

fn track_last_minimized_window(window_id: &str) {
    let Some(window_id) = normalize_window_id(window_id) else {
        clear_last_minimized_window_id();
        return;
    };

    let Some(pid) = window_pid(&window_id).ok().flatten() else {
        clear_last_minimized_window_id();
        return;
    };

    let Some(process_start_ticks) = process_start_ticks(pid) else {
        clear_last_minimized_window_id();
        return;
    };

    let record = MinimizedWindowRecord {
        window_id,
        pid,
        process_start_ticks,
        saved_at_unix_secs: current_unix_secs(),
    };

    write_last_minimized_window_record(&record);
}

fn write_last_minimized_window_record(record: &MinimizedWindowRecord) {
    let line = format!(
        "{}|{}|{}|{}\n",
        record.window_id, record.pid, record.process_start_ticks, record.saved_at_unix_secs
    );
    let _ = fs::write(last_minimized_window_file(), line.as_bytes());
}

fn read_last_minimized_window_record() -> Result<Option<MinimizedWindowRecord>, String> {
    match fs::read_to_string(last_minimized_window_file()) {
        Ok(value) => {
            let parsed = parse_minimized_window_record(&value);
            if parsed.is_none() && !value.trim().is_empty() {
                clear_last_minimized_window_id();
            }
            Ok(parsed)
        }
        Err(error) if error.kind() == ErrorKind::NotFound => Ok(None),
        Err(error) => Err(format!("Failed to read minimized window state: {error}")),
    }
}

fn parse_minimized_window_record(raw: &str) -> Option<MinimizedWindowRecord> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return None;
    }

    let mut parts = trimmed.split('|');
    let window_id = normalize_window_id(parts.next()?.trim())?;
    let pid = parts.next()?.trim().parse::<u32>().ok()?;
    let process_start_ticks = parts.next()?.trim().parse::<u64>().ok()?;
    let saved_at_unix_secs = parts.next()?.trim().parse::<u64>().ok()?;

    if parts.next().is_some() {
        return None;
    }

    Some(MinimizedWindowRecord {
        window_id,
        pid,
        process_start_ticks,
        saved_at_unix_secs,
    })
}

fn is_record_expired(record: &MinimizedWindowRecord) -> bool {
    current_unix_secs().saturating_sub(record.saved_at_unix_secs) > LAST_MINIMIZED_MAX_AGE_SECS
}

fn is_record_current(record: &MinimizedWindowRecord) -> Result<bool, String> {
    let Some(pid) = window_pid(&record.window_id)? else {
        return Ok(false);
    };
    if pid != record.pid {
        return Ok(false);
    }

    let Some(start_ticks) = process_start_ticks(pid) else {
        return Ok(false);
    };
    Ok(start_ticks == record.process_start_ticks)
}

fn current_unix_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_secs())
        .unwrap_or(0)
}

fn clear_last_minimized_window_id() {
    let _ = fs::remove_file(last_minimized_window_file());
}

fn last_minimized_window_file() -> PathBuf {
    let base = env::var_os("XDG_RUNTIME_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("/tmp"));
    base.join(LAST_MINIMIZED_WINDOW_FILE_NAME)
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

    if output.status.success() {
        return Ok(String::from_utf8_lossy(&output.stdout).to_string());
    }

    let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
    if stderr.is_empty() {
        Err(format!("{command} exited with status {}", output.status))
    } else {
        Err(format!("{command} failed: {stderr}"))
    }
}

fn run_output_optional(command: &str, args: &[&str]) -> String {
    run_output(command, args).unwrap_or_default()
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
