use anyhow::{anyhow, Context, Result};
use std::fs::File;
use std::path::Path;
use std::process::{Command, Stdio};

use crate::{Config, Monitor, Rect};

const SETTINGS_URL: &str = "http://127.0.0.1:42700/plugins/plugin-screen-recorder/";
const MAX_DISPLAYS: u32 = 16;

type CGDirectDisplayID = u32;

#[repr(C)]
#[derive(Clone, Copy)]
struct CGPoint {
    x: f64,
    y: f64,
}

#[repr(C)]
#[derive(Clone, Copy)]
struct CGSize {
    width: f64,
    height: f64,
}

#[repr(C)]
#[derive(Clone, Copy)]
struct CGRect {
    origin: CGPoint,
    size: CGSize,
}

#[link(name = "CoreGraphics", kind = "framework")]
extern "C" {
    fn CGGetActiveDisplayList(
        max_displays: u32,
        active_displays: *mut CGDirectDisplayID,
        display_count: *mut u32,
    ) -> i32;
    fn CGDisplayBounds(display: CGDirectDisplayID) -> CGRect;
}

pub fn select_region() -> Result<Option<Rect>> {
    let bounds = full_screen_bounds()?;
    Ok(Some(Rect {
        x: bounds.x,
        y: bounds.y,
        w: bounds.w,
        h: bounds.h,
    }))
}

pub fn get_monitors() -> Result<Vec<Monitor>> {
    active_display_bounds()
}

pub fn full_screen_bounds() -> Result<Monitor> {
    let monitors = active_display_bounds()?;
    union_bounds(&monitors)
}

pub fn start_capture(_rect: &Rect, config: &Config, output_file: &Path) -> Result<u32> {
    let log_file =
        File::create(super::CAPTURE_LOG).context("failed to create recording log file")?;
    let stdout_log = log_file
        .try_clone()
        .context("failed to clone recording log file")?;

    let mut command = Command::new("screencapture");
    command.args(["-v", "-i", "-s", "-Jvideo", "-k", "-x"]);

    if config.audio.enabled {
        command.arg("-g");
    }

    let child = command
        .arg(output_file)
        .stdin(Stdio::null())
        .stdout(Stdio::from(stdout_log))
        .stderr(Stdio::from(log_file))
        .spawn()
        .context("failed to start screencapture")?;

    Ok(child.id())
}

pub fn recording_format(_format: &str) -> String {
    "mov".to_string()
}

pub fn stop_capture(pid: u32) -> Result<()> {
    Command::new("kill")
        .args(["-INT", &pid.to_string()])
        .status()
        .context("failed to send SIGINT to screencapture")?;
    Ok(())
}

pub fn process_alive(pid: u32) -> bool {
    let pid_arg = pid.to_string();
    Command::new("kill")
        .args(["-0", pid_arg.as_str()])
        .status()
        .map(|status| status.success())
        .unwrap_or(false)
}

pub fn show_notification(title: &str, message: &str, _timeout_ms: u32) {
    let escaped_title = escape_applescript(title);
    let escaped_message = escape_applescript(message);
    let script = format!(
        "display notification \"{}\" with title \"{}\"",
        escaped_message, escaped_title
    );

    let _ = Command::new("osascript")
        .arg("-e")
        .arg(script)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status();
}

pub fn open_settings() -> Result<()> {
    Command::new("open")
        .arg(SETTINGS_URL)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .context("failed to open settings URL")?;
    Ok(())
}

fn active_display_bounds() -> Result<Vec<Monitor>> {
    let mut displays = [0u32; MAX_DISPLAYS as usize];
    let mut count = 0u32;
    let result = unsafe { CGGetActiveDisplayList(MAX_DISPLAYS, displays.as_mut_ptr(), &mut count) };

    if result != 0 {
        return Err(anyhow!("CGGetActiveDisplayList failed: {}", result));
    }

    let monitors = displays
        .iter()
        .take(count as usize)
        .map(|display| {
            let bounds = unsafe { CGDisplayBounds(*display) };
            monitor_from_cg_bounds(bounds)
        })
        .collect::<Vec<_>>();

    if monitors.is_empty() {
        return Err(anyhow!("no active displays found"));
    }

    Ok(monitors)
}

fn union_bounds(monitors: &[Monitor]) -> Result<Monitor> {
    let first = monitors
        .first()
        .copied()
        .ok_or_else(|| anyhow!("no active displays found"))?;

    let mut left = first.x;
    let mut top = first.y;
    let mut right = first.x + first.w;
    let mut bottom = first.y + first.h;

    for monitor in monitors.iter().skip(1) {
        left = left.min(monitor.x);
        top = top.min(monitor.y);
        right = right.max(monitor.x + monitor.w);
        bottom = bottom.max(monitor.y + monitor.h);
    }

    Ok(Monitor {
        x: left,
        y: top,
        w: right - left,
        h: bottom - top,
    })
}

fn monitor_from_cg_bounds(bounds: CGRect) -> Monitor {
    Monitor {
        x: round_i32(bounds.origin.x),
        y: round_i32(bounds.origin.y),
        w: round_i32(bounds.size.width),
        h: round_i32(bounds.size.height),
    }
}

fn round_i32(value: f64) -> i32 {
    value.round() as i32
}

fn escape_applescript(input: &str) -> String {
    input.replace('\\', "\\\\").replace('"', "\\\"")
}
