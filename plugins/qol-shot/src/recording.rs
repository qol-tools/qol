use anyhow::{anyhow, Context, Result};
use serde::{Deserialize, Serialize};
use std::env;
use std::fs;
use std::path::{Path, PathBuf};

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
    trace_record_config("cli", config);
    qol_runtime::probe!("SHOT_RECORD_TOGGLE", "source=cli");
    if stop_active_recording_if_any(config)? {
        qol_runtime::probe!("SHOT_RECORD_TOGGLE", "source=cli result=stopped");
        return Ok(());
    }

    let Some(_capture) = crate::capture_gate::try_acquire("cli-record") else {
        qol_runtime::probe!("SHOT_RECORD_TOGGLE", "source=cli result=busy");
        return Ok(());
    };

    qol_runtime::probe!("SHOT_SELECT_REQUEST", "source=cli");
    let Some(selected) = platform::select_region(crate::space::CaptureKind::Recording)? else {
        qol_runtime::probe!("SHOT_RECORD_TOGGLE", "source=cli result=select-cancel");
        return Ok(());
    };
    qol_runtime::probe!(
        "SHOT_SELECT_RESULT",
        "source=cli rect={}x{}+{},{}",
        selected.w,
        selected.h,
        selected.x,
        selected.y
    );

    start_recording_from_selection(selected, config)
}

pub enum StopOutcome {
    Idle,
    Stopped(Box<FinalizeJob>),
}

pub struct FinalizeJob {
    session: platform::CaptureSession,
    config: Config,
}

impl FinalizeJob {
    pub fn run(self) {
        platform::recording_stopped(&self.session, &self.config);
    }
}

pub fn stop_active_recording_if_any(config: &Config) -> Result<bool> {
    match begin_stop_active_recording(config)? {
        StopOutcome::Idle => Ok(false),
        StopOutcome::Stopped(job) => {
            job.run();
            Ok(true)
        }
    }
}

pub fn begin_stop_active_recording(config: &Config) -> Result<StopOutcome> {
    let Some(state) = read_capture_state() else {
        qol_runtime::probe!("SHOT_RECORD_STATE", "read=none state=idle");
        return Ok(StopOutcome::Idle);
    };

    trace_capture_session("read", &state);
    if platform::session_alive(&state) {
        qol_runtime::probe!("SHOT_RECORD_STOP", "pids={} state=active", state.pid_list());
        stop_capture_processes(&state, config)?;
        return Ok(StopOutcome::Stopped(Box::new(FinalizeJob {
            session: state,
            config: config.clone(),
        })));
    }

    qol_runtime::probe!("SHOT_RECORD_STATE", "pids={} state=stale", state.pid_list());
    remove_state_file();
    Ok(StopOutcome::Idle)
}

pub fn start_recording_from_selection(selected: Rect, config: &Config) -> Result<()> {
    qol_runtime::probe!("SHOT_RECORD_SELECTION", "selected={}", rect_label(selected));
    let rect = prepare_recording_rect(selected)?;
    qol_runtime::probe!(
        "SHOT_RECORD_START",
        "rect={},{} {}x{}",
        rect.x,
        rect.y,
        rect.w,
        rect.h
    );
    let output_format = platform::recording_format(&config.video.format);
    let output_file = crate::output::recording_output_file_path(&output_format)?;
    qol_runtime::probe!(
        "SHOT_RECORD_OUTPUT",
        "format={output_format} file={} ext={}",
        path_label(Some(&output_file)),
        output_file
            .extension()
            .and_then(|extension| extension.to_str())
            .unwrap_or("none")
    );
    let session = platform::start_capture(&rect, config, &output_file)?;
    trace_capture_session("spawned", &session);

    if let Err(error) = write_capture_state(&session) {
        let _ = platform::stop_capture(&session);
        qol_runtime::probe!(
            "SHOT_RECORD_STATE",
            "write=failed pids={}",
            session.pid_list()
        );
        return Err(error);
    }
    qol_runtime::probe!("SHOT_RECORD_STATE", "write=ok pids={}", session.pid_list());

    if platform::session_started(&session) {
        qol_runtime::probe!(
            "SHOT_RECORD_STARTED",
            "pids={} segments={}",
            session.pid_list(),
            session.segments.len()
        );
        platform::recording_started(&session);
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

fn prepare_recording_rect(selected: Rect) -> Result<Rect> {
    let monitors = platform::get_monitors().unwrap_or_default();
    let (fallback_bounds, fallback) = match geometry::monitor_for_selection(selected, &monitors) {
        Some(monitor) => (monitor, "selection-monitor"),
        None => (platform::full_screen_bounds()?, "full-screen"),
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

    let even = geometry::even_dimensions(rect);
    qol_runtime::probe!(
        "SHOT_RECORD_RECT",
        "selected={} monitors={} fallback={} fallback_rect={} prepared={} even={}",
        rect_label(selected),
        monitors.len(),
        fallback,
        monitor_label(fallback_bounds),
        rect_label(rect),
        rect_label(even)
    );
    Ok(even)
}

fn stop_capture_processes(state: &platform::CaptureSession, config: &Config) -> Result<()> {
    trace_record_config("stop", config);
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
    qol_runtime::probe!("SHOT_RECORD_STOPPED", "pids={}", state.pid_list());
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
    qol_runtime::probe!(
        "SHOT_RECORD_STATE_FILE",
        "action=write version={} pids={} segments={} file={}",
        CAPTURE_STATE_VERSION,
        session.pid_list(),
        session.segments.len(),
        STATE_FILE_NAME
    );
    fs::write(state_file_path(), format!("{content}\n")).context("failed to write capture state")
}

fn remove_state_file() {
    qol_runtime::probe!(
        "SHOT_RECORD_STATE_FILE",
        "action=remove file={STATE_FILE_NAME}"
    );
    let _ = fs::remove_file(state_file_path());
}

fn state_file_path() -> PathBuf {
    env::temp_dir().join(STATE_FILE_NAME)
}

fn trace_record_config(source: &'static str, config: &Config) {
    qol_runtime::probe!(
        "SHOT_RECORD_CONFIG",
        "source={source} format={} audio={} crf={} preset={}",
        config.video.format,
        config.audio.enabled,
        config.video.crf,
        config.video.preset
    );
}

fn trace_capture_session(stage: &'static str, session: &platform::CaptureSession) {
    qol_runtime::probe!(
        "SHOT_RECORD_SESSION",
        "stage={stage} pids={} segments={} canvas={} output={} capture={}",
        session.pid_list(),
        session.segments.len(),
        session
            .canvas
            .map(rect_label)
            .unwrap_or_else(|| "none".to_string()),
        path_label(session.output_file.as_deref()),
        path_label(session.capture_file.as_deref())
    );
}

fn rect_label(rect: Rect) -> String {
    format!("{}x{}+{},{}", rect.w, rect.h, rect.x, rect.y)
}

fn monitor_label(monitor: crate::Monitor) -> String {
    format!("{}x{}+{},{}", monitor.w, monitor.h, monitor.x, monitor.y)
}

fn path_label(path: Option<&Path>) -> String {
    path.and_then(Path::file_name)
        .and_then(|name| name.to_str())
        .unwrap_or("none")
        .to_string()
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
        let session = crate::platform::CaptureSession {
            output_file: Some(PathBuf::from("/a/final.webm")),
            capture_file: Some(PathBuf::from("/a/native.mov")),
            canvas: Some(crate::Rect {
                x: 10,
                y: 20,
                w: 300,
                h: 200,
            }),
            processes: vec![crate::platform::CaptureProcess { pid: 123 }],
            segments: Vec::new(),
        };
        let state = super::PersistedCaptureState {
            version: super::CAPTURE_STATE_VERSION,
            session: session.clone(),
        };
        let content = serde_json::to_string(&state).unwrap();

        assert_eq!(super::parse_capture_state(&content), Some(session));
    }
}
