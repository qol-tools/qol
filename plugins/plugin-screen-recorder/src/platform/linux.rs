use anyhow::{anyhow, Context, Result};
use std::fs::File;
use std::path::Path;
use std::process::{Command, Stdio};

use crate::{Config, Monitor, Rect};

pub fn select_region() -> Result<Option<Rect>> {
    let output = Command::new("slop")
        .args([
            "--highlight",
            "--color=1,0,0,0.65",
            "-b",
            "0",
            "-f",
            "%x,%y,%w,%h",
        ])
        .output()
        .context("failed to run slop")?;

    if !output.status.success() {
        return Ok(None);
    }

    let raw = String::from_utf8_lossy(&output.stdout).trim().to_string();
    if raw.is_empty() {
        return Ok(None);
    }

    parse_selection_geometry(&raw).map(Some)
}

pub fn get_monitors() -> Result<Vec<Monitor>> {
    let output = Command::new("xrandr")
        .args(["--query"])
        .output()
        .context("failed to run xrandr")?;
    if !output.status.success() {
        return Err(anyhow!("xrandr failed"));
    }
    let stdout = String::from_utf8_lossy(&output.stdout);
    let monitors: Vec<Monitor> = stdout.lines().filter_map(parse_xrandr_line).collect();
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

pub fn start_capture(rect: &Rect, config: &Config, output_file: &Path) -> Result<u32> {
    let mut args = vec![
        "-thread_queue_size".to_string(),
        "512".to_string(),
        "-f".to_string(),
        "x11grab".to_string(),
        "-video_size".to_string(),
        format!("{}x{}", rect.w, rect.h),
        "-framerate".to_string(),
        config.video.framerate.to_string(),
        "-i".to_string(),
        format!(":0.0+{},{}", rect.x, rect.y),
    ];

    if config.audio.enabled {
        let has_mic = config.audio.inputs.iter().any(|input| input == "mic");
        let has_system = config.audio.inputs.iter().any(|input| input == "system");
        if has_mic && has_system {
            args.extend_from_slice(&[
                "-thread_queue_size".to_string(),
                "128".to_string(),
                "-f".to_string(),
                "pulse".to_string(),
                "-i".to_string(),
                config.audio.mic_device.clone(),
                "-thread_queue_size".to_string(),
                "128".to_string(),
                "-f".to_string(),
                "pulse".to_string(),
                "-i".to_string(),
                format!("{}.monitor", config.audio.system_device),
                "-filter_complex".to_string(),
                "[1:a][2:a]amerge=inputs=2[aout]".to_string(),
                "-map".to_string(),
                "0:v".to_string(),
                "-map".to_string(),
                "[aout]".to_string(),
                "-c:a".to_string(),
                "aac".to_string(),
                "-b:a".to_string(),
                "192k".to_string(),
            ]);
        } else if has_mic {
            args.extend_from_slice(&[
                "-thread_queue_size".to_string(),
                "128".to_string(),
                "-f".to_string(),
                "pulse".to_string(),
                "-i".to_string(),
                config.audio.mic_device.clone(),
                "-c:a".to_string(),
                "aac".to_string(),
                "-b:a".to_string(),
                "192k".to_string(),
            ]);
        } else if has_system {
            args.extend_from_slice(&[
                "-thread_queue_size".to_string(),
                "128".to_string(),
                "-f".to_string(),
                "pulse".to_string(),
                "-i".to_string(),
                format!("{}.monitor", config.audio.system_device),
                "-c:a".to_string(),
                "aac".to_string(),
                "-b:a".to_string(),
                "192k".to_string(),
            ]);
        }
    }

    args.extend_from_slice(&[
        "-c:v".to_string(),
        "libx264".to_string(),
        "-r".to_string(),
        config.video.framerate.to_string(),
        "-crf".to_string(),
        config.video.crf.to_string(),
        "-preset".to_string(),
        config.video.preset.clone(),
        "-pix_fmt".to_string(),
        "yuv420p".to_string(),
        output_file.to_string_lossy().to_string(),
    ]);

    let log_file =
        File::create(super::CAPTURE_LOG).context("failed to create recording log file")?;
    let stdout_log = log_file
        .try_clone()
        .context("failed to clone recording log file")?;

    let child = Command::new("ffmpeg")
        .args(&args)
        .stdin(Stdio::null())
        .stdout(Stdio::from(stdout_log))
        .stderr(Stdio::from(log_file))
        .spawn()
        .context("failed to start ffmpeg")?;

    Ok(child.id())
}

pub fn stop_capture(pid: u32) -> Result<()> {
    Command::new("kill")
        .args(["-INT", &pid.to_string()])
        .status()
        .context("failed to send SIGINT to ffmpeg")?;
    Ok(())
}

pub fn process_alive(pid: u32) -> bool {
    Path::new(&format!("/proc/{pid}")).exists()
}

pub fn show_notification(title: &str, message: &str, timeout_ms: u32) {
    let _ = Command::new("notify-send")
        .args([
            "-u",
            "normal",
            "-t",
            &timeout_ms.to_string(),
            title,
            message,
        ])
        .status();
}

pub fn open_settings() -> Result<()> {
    Command::new("xdg-open")
        .arg(super::SETTINGS_URL)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .context("failed to open settings URL")?;
    Ok(())
}

fn parse_selection_geometry(raw: &str) -> Result<Rect> {
    let values: Vec<i32> = raw
        .split(',')
        .map(str::trim)
        .map(str::parse::<i32>)
        .collect::<std::result::Result<Vec<_>, _>>()
        .context("invalid selection geometry")?;
    if values.len() != 4 {
        return Err(anyhow!(
            "expected 4 values in geometry, got {}",
            values.len()
        ));
    }
    Ok(Rect {
        x: values[0],
        y: values[1],
        w: values[2],
        h: values[3],
    })
}

fn parse_xrandr_line(line: &str) -> Option<Monitor> {
    if !line.contains(" connected") {
        return None;
    }
    let geometry = line
        .split_whitespace()
        .find(|token| token.contains('x') && token.contains('+'))?;
    parse_monitor_geometry(geometry)
}

fn parse_monitor_geometry(token: &str) -> Option<Monitor> {
    let x_split = token.find('x')?;
    let width = token[..x_split].parse::<i32>().ok()?;
    let after_x = &token[x_split + 1..];
    let first_sign = after_x.find(['+', '-'])?;
    let height = after_x[..first_sign].parse::<i32>().ok()?;
    let after_height = &after_x[first_sign..];
    let second_sign = after_height[1..].find(['+', '-'])? + 1;
    let x = after_height[..second_sign].parse::<i32>().ok()?;
    let y = after_height[second_sign..].parse::<i32>().ok()?;
    Some(Monitor {
        x,
        y,
        w: width,
        h: height,
    })
}
