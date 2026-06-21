use anyhow::{anyhow, Context, Result};
use serde::{Deserialize, Serialize};
use std::env;
use std::fs;
use std::path::PathBuf;

use crate::geometry;
use crate::{platform, Config, Rect};

const STATE_FILE_NAME: &str = "record-region.pid";
const CAPTURE_STATE_VERSION: u32 = 1;

#[derive(Debug, Deserialize, Serialize)]
struct PersistedCaptureState {
    version: u32,
    session: platform::CaptureSession,
}

pub fn toggle_recording(config: &Config) -> Result<()> {
    if let Some(state) = read_capture_state() {
        if platform::session_alive(&state) {
            return stop_active_recording(&state, config);
        }
        remove_state_file();
    }

    let Some(rect) = select_recording_rect()? else {
        return Ok(());
    };

    let output_format = platform::recording_format(&config.video.format);
    let output_file = crate::output::recording_output_file_path(&output_format)?;
    let session = platform::start_capture(&rect, config, &output_file)?;

    if let Err(error) = write_capture_state(&session) {
        let _ = platform::stop_capture(&session);
        return Err(error);
    }

    if platform::session_started(&session) {
        platform::recording_started();
    } else {
        let _ = platform::stop_capture(&session);
        remove_state_file();
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

fn stop_active_recording(state: &platform::CaptureSession, config: &Config) -> Result<()> {
    if let Err(error) = platform::stop_capture(state) {
        eprintln!(
            "failed to stop recording pids {}: {error:#}",
            state.pid_list()
        );
        if platform::session_alive(state) {
            platform::show_notification("Recording failed", "Could not stop capture process", 1800);
            return Err(error).context("capture process is still running after stop request");
        }
    }

    remove_state_file();
    platform::recording_stopped(state, config);
    Ok(())
}

fn read_capture_state() -> Option<platform::CaptureSession> {
    let content = fs::read_to_string(state_file_path()).ok()?;
    parse_capture_state(&content)
}

fn parse_capture_state(content: &str) -> Option<platform::CaptureSession> {
    if let Ok(state) = serde_json::from_str::<PersistedCaptureState>(content) {
        return Some(state.session);
    }

    parse_legacy_capture_state(content)
}

fn parse_legacy_capture_state(content: &str) -> Option<platform::CaptureSession> {
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

    Some(platform::CaptureSession::legacy(
        pid,
        output_file,
        capture_file,
    ))
}

fn write_capture_state(session: &platform::CaptureSession) -> Result<()> {
    let state = PersistedCaptureState {
        version: CAPTURE_STATE_VERSION,
        session: session.clone(),
    };
    let content = serde_json::to_string_pretty(&state).context("failed to encode capture state")?;
    fs::write(state_file_path(), format!("{content}\n")).context("failed to write capture state")
}

fn remove_state_file() {
    let _ = fs::remove_file(state_file_path());
}

fn state_file_path() -> PathBuf {
    env::temp_dir().join(STATE_FILE_NAME)
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    #[test]
    fn capture_state_reads_output_and_capture_paths() {
        let state = super::parse_capture_state("123\n/a/final.webm\n/a/native.mov\n").unwrap();
        assert_eq!(state.processes[0].pid, 123);
        assert_eq!(state.output_file, Some(PathBuf::from("/a/final.webm")));
        assert_eq!(state.capture_file, Some(PathBuf::from("/a/native.mov")));
    }

    #[test]
    fn capture_state_uses_output_path_for_legacy_pidfiles() {
        let state = super::parse_capture_state("123\n/a/final.mov\n").unwrap();
        assert_eq!(state.processes[0].pid, 123);
        assert_eq!(state.output_file, Some(PathBuf::from("/a/final.mov")));
        assert_eq!(state.capture_file, Some(PathBuf::from("/a/final.mov")));
    }

    #[test]
    fn capture_state_round_trips_json_session() {
        let session = crate::platform::CaptureSession::single(
            123,
            crate::Rect {
                x: 10,
                y: 20,
                w: 300,
                h: 200,
            },
            PathBuf::from("/a/final.webm"),
            PathBuf::from("/a/native.mov"),
        );
        let state = super::PersistedCaptureState {
            version: super::CAPTURE_STATE_VERSION,
            session: session.clone(),
        };
        let content = serde_json::to_string(&state).unwrap();

        assert_eq!(super::parse_capture_state(&content), Some(session));
    }
}
