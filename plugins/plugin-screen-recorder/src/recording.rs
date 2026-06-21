use anyhow::{anyhow, Context, Result};
use std::fs;
use std::path::{Path, PathBuf};

use crate::geometry;
use crate::{platform, Config, Rect};

const PIDFILE: &str = "/tmp/record-region.pid";

#[derive(Debug)]
struct CaptureState {
    pid: u32,
    output_file: Option<PathBuf>,
    capture_file: Option<PathBuf>,
}

pub fn toggle_recording(config: &Config) -> Result<()> {
    if let Some(state) = read_capture_state() {
        if platform::process_alive(state.pid) {
            return stop_active_recording(&state, config);
        }
        remove_pidfile();
    }

    let Some(rect) = select_recording_rect()? else {
        return Ok(());
    };

    let output_format = platform::recording_format(&config.video.format);
    let output_file = crate::output::recording_output_file_path(&output_format)?;
    let capture_file = platform::capture_file_path(&output_file);
    let pid = platform::start_capture(&rect, config, &capture_file)?;

    write_capture_state(pid, &output_file, &capture_file)?;

    if platform::process_alive(pid) {
        platform::recording_started();
    } else {
        remove_pidfile();
        platform::show_notification(
            "Recording failed",
            &format!("Check {}", platform::CAPTURE_LOG),
            1600,
        );
        return Err(anyhow!("capture process exited immediately"));
    }

    Ok(())
}

fn select_recording_rect() -> Result<Option<Rect>> {
    let Some(selected) = platform::select_region()? else {
        return Ok(None);
    };

    let monitors = platform::get_monitors().unwrap_or_default();
    let fallback_bounds = match geometry::monitor_for_selection(selected, &monitors) {
        Some(monitor) => monitor,
        None => platform::full_screen_bounds()?,
    };
    let rect = geometry::prepare_recording_rect(selected, &monitors, fallback_bounds);
    if rect.w <= 0 || rect.h <= 0 {
        platform::show_notification(
            "Recording failed",
            &format!("Invalid area: {}x{}", rect.w, rect.h),
            1200,
        );
        return Err(anyhow!("invalid recording area {}x{}", rect.w, rect.h));
    }

    Ok(Some(geometry::even_dimensions(rect)))
}

fn stop_active_recording(state: &CaptureState, config: &Config) -> Result<()> {
    if let Err(error) = platform::stop_capture(state.pid) {
        eprintln!("failed to stop recording pid {}: {error:#}", state.pid);
        if platform::process_alive(state.pid) {
            platform::show_notification("Recording failed", "Could not stop capture process", 1800);
            return Err(error).context("capture process is still running after stop request");
        }
    }

    remove_pidfile();
    platform::recording_stopped(
        state.output_file.as_deref(),
        state.capture_file.as_deref(),
        config,
    );
    Ok(())
}

fn read_capture_state() -> Option<CaptureState> {
    let content = fs::read_to_string(PIDFILE).ok()?;
    parse_capture_state(&content)
}

fn parse_capture_state(content: &str) -> Option<CaptureState> {
    let mut lines = content.lines();
    let pid = lines.next()?.trim().parse::<u32>().ok()?;
    let output_file = lines
        .next()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .map(PathBuf::from);
    let capture_file = lines
        .next()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .map(PathBuf::from)
        .or_else(|| output_file.clone());
    Some(CaptureState {
        pid,
        output_file,
        capture_file,
    })
}

fn write_capture_state(pid: u32, output_file: &Path, capture_file: &Path) -> Result<()> {
    let content = format!(
        "{}\n{}\n{}\n",
        pid,
        output_file.display(),
        capture_file.display()
    );
    fs::write(PIDFILE, content).context("failed to write pid file")
}

fn remove_pidfile() {
    let _ = fs::remove_file(PIDFILE);
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    #[test]
    fn capture_state_reads_output_and_capture_paths() {
        let state = super::parse_capture_state("123\n/a/final.webm\n/a/native.mov\n").unwrap();
        assert_eq!(state.pid, 123);
        assert_eq!(state.output_file, Some(PathBuf::from("/a/final.webm")));
        assert_eq!(state.capture_file, Some(PathBuf::from("/a/native.mov")));
    }

    #[test]
    fn capture_state_uses_output_path_for_legacy_pidfiles() {
        let state = super::parse_capture_state("123\n/a/final.mov\n").unwrap();
        assert_eq!(state.pid, 123);
        assert_eq!(state.output_file, Some(PathBuf::from("/a/final.mov")));
        assert_eq!(state.capture_file, Some(PathBuf::from("/a/final.mov")));
    }
}
