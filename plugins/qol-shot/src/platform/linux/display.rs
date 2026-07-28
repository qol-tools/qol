use anyhow::{anyhow, Context, Result};
use std::process::Command;

use crate::Monitor;

pub fn get_monitors() -> Result<Vec<Monitor>> {
    let output = Command::new("xrandr")
        .args(["--query"])
        .output()
        .context("failed to run xrandr")?;
    if !output.status.success() {
        return Err(anyhow!("xrandr failed"));
    }
    let stdout = String::from_utf8_lossy(&output.stdout);
    let monitors: Vec<Monitor> = qol_runtime::display::x11::parse_monitors(&stdout)
        .into_iter()
        .map(monitor_from_xrandr)
        .collect();
    if monitors.is_empty() {
        return Err(anyhow!("no monitors found from xrandr"));
    }
    Ok(monitors)
}

pub fn full_screen_bounds() -> Result<Monitor> {
    let output = Command::new("xdpyinfo")
        .output()
        .context("failed to run xdpyinfo")?;
    if !output.status.success() {
        return Err(anyhow!("xdpyinfo failed"));
    }
    let stdout = String::from_utf8_lossy(&output.stdout);
    let dimensions = stdout
        .lines()
        .find_map(|line| {
            if !line.contains("dimensions:") {
                return None;
            }
            line.split_whitespace().find(|token| {
                token.contains('x') && token.chars().all(|c| c.is_ascii_digit() || c == 'x')
            })
        })
        .ok_or_else(|| anyhow!("could not read dimensions from xdpyinfo"))?;
    let split = dimensions
        .find('x')
        .ok_or_else(|| anyhow!("invalid dimensions"))?;
    let w = dimensions[..split]
        .parse::<i32>()
        .context("invalid width from xdpyinfo")?;
    let h = dimensions[split + 1..]
        .parse::<i32>()
        .context("invalid height from xdpyinfo")?;
    Ok(Monitor { x: 0, y: 0, w, h })
}

fn monitor_from_xrandr(monitor: qol_runtime::display::x11::XrandrMonitor) -> Monitor {
    Monitor {
        x: monitor.bounds.x as i32,
        y: monitor.bounds.y as i32,
        w: monitor.bounds.width as i32,
        h: monitor.bounds.height as i32,
    }
}
