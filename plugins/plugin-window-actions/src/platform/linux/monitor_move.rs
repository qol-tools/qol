use std::process::Command;
use std::thread;
use std::time::Duration;

use super::system::{run_cinnamon_eval, run_status};

pub fn move_monitor(script: &str, reveal_taskbar_after_move: bool) -> Result<(), String> {
    let output = run_cinnamon_eval(script)?;
    if !reveal_taskbar_after_move {
        return Ok(());
    }
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
        .map_err(|error| format!("Failed to run xdotool: {error}"))?;

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
        &[
            "mousemove",
            "--sync",
            &edge_x.to_string(),
            &edge_y.to_string(),
        ],
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
        .map_err(|error| format!("Failed to run xdotool: {error}"))?;

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
    fn from_xrandr(monitor: qol_runtime::xrandr::XrandrMonitor) -> Self {
        Self {
            x: monitor.bounds.x as i32,
            y: monitor.bounds.y as i32,
            width: monitor.bounds.width as i32,
            height: monitor.bounds.height as i32,
        }
    }

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

    let stdout = String::from_utf8_lossy(&output.stdout);
    qol_runtime::xrandr::parse_monitors(&stdout)
        .into_iter()
        .map(MonitorBounds::from_xrandr)
        .collect()
}
