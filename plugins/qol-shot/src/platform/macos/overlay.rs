use anyhow::{Context, Result};
use std::env;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::Mutex;
use std::thread;
use std::time::Duration;

use crate::platform::CaptureSession;
use crate::Rect;

use super::display::active_display_bounds;
use super::labels::rect_label;
use super::swift::{
    ensure_swift_helper, spawn_source_swift, RECORDING_OVERLAY_HELPER, RECORDING_OVERLAY_SWIFT,
    STATUS_OVERLAY_HELPER, STATUS_OVERLAY_SWIFT,
};
use super::system::{process_alive, signal_process, wait_for_process_exit};

const STATUS_OVERLAY_PID_FILE_NAME: &str = "qol-shot-status-overlay.pid";
const STATUS_OVERLAY_MAX_LIFETIME_MS: u32 = 120_000;
const RECORDING_OVERLAY_PID_FILE_NAME: &str = "qol-shot-recording-overlay.pid";
const RECORDING_OVERLAY_MAX_LIFETIME_MS: u32 = 43_200_000;
static STATUS_OVERLAY_PID: Mutex<Option<u32>> = Mutex::new(None);
static RECORDING_OVERLAY_PIDS: Mutex<Vec<u32>> = Mutex::new(Vec::new());

pub(super) enum StatusOverlayLifecycle {
    KeepAlive,
    ExitAfterHide,
}

impl StatusOverlayLifecycle {
    fn exit_after_hide(self) -> bool {
        matches!(self, StatusOverlayLifecycle::ExitAfterHide)
    }

    fn as_str(self) -> &'static str {
        match self {
            Self::KeepAlive => "keep-alive",
            Self::ExitAfterHide => "exit-after-hide",
        }
    }
}

pub(super) fn show_status_overlay(
    title: &str,
    message: &str,
    timeout_ms: u32,
    lifecycle: StatusOverlayLifecycle,
) {
    qol_runtime::probe!(
        "SHOT_STATUS_OVERLAY",
        "event=show title={title:?} timeout_ms={timeout_ms} lifecycle={}",
        lifecycle.as_str()
    );
    dismiss_status_overlay();
    let Ok(child) = spawn_status_overlay(title, message, timeout_ms, lifecycle) else {
        qol_runtime::probe!("SHOT_STATUS_OVERLAY", "event=spawn result=error");
        return;
    };
    let pid = child.id();
    qol_runtime::probe!("SHOT_STATUS_OVERLAY", "event=spawn result=ok pid={pid}");
    write_status_overlay_pid(pid);

    thread::spawn(move || {
        let mut child = child;
        let _ = child.wait();
        clear_status_overlay_pid(pid);
    });
}

fn spawn_status_overlay(
    title: &str,
    message: &str,
    timeout_ms: u32,
    lifecycle: StatusOverlayLifecycle,
) -> Result<Child> {
    match ensure_swift_helper(
        "status-overlay",
        STATUS_OVERLAY_SWIFT,
        STATUS_OVERLAY_HELPER,
    ) {
        Ok(helper) => {
            qol_runtime::probe!("SHOT_STATUS_OVERLAY", "event=helper result=compiled");
            match spawn_compiled_status_overlay(&helper, title, message, timeout_ms, lifecycle) {
                Ok(child) => Ok(child),
                Err(error) => {
                    qol_runtime::probe!(
                        "SHOT_STATUS_OVERLAY",
                        "event=compiled-spawn result=error fallback=source"
                    );
                    let _ = fs::remove_file(&helper);
                    spawn_source_status_overlay(title, message, timeout_ms, lifecycle)
                        .with_context(|| format!("compiled status overlay failed first: {error:#}"))
                }
            }
        }
        Err(error) => {
            qol_runtime::probe!(
                "SHOT_STATUS_OVERLAY",
                "event=helper result=error fallback=source"
            );
            spawn_source_status_overlay(title, message, timeout_ms, lifecycle)
                .with_context(|| format!("failed to build status overlay helper: {error:#}"))
        }
    }
}

fn spawn_compiled_status_overlay(
    helper: &Path,
    title: &str,
    message: &str,
    timeout_ms: u32,
    lifecycle: StatusOverlayLifecycle,
) -> Result<Child> {
    let mut command = Command::new(helper);
    configure_status_overlay(&mut command, title, message, timeout_ms, lifecycle);
    command
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .context("failed to start compiled macOS status overlay")
}

fn spawn_source_status_overlay(
    title: &str,
    message: &str,
    timeout_ms: u32,
    lifecycle: StatusOverlayLifecycle,
) -> Result<Child> {
    spawn_source_swift(STATUS_OVERLAY_SWIFT, |command| {
        configure_status_overlay(command, title, message, timeout_ms, lifecycle);
        command.stdout(Stdio::null()).stderr(Stdio::null());
    })
    .context("failed to start source macOS status overlay")
}

fn configure_status_overlay(
    command: &mut Command,
    title: &str,
    message: &str,
    timeout_ms: u32,
    lifecycle: StatusOverlayLifecycle,
) {
    command
        .env("QOL_STATUS_TITLE", title)
        .env("QOL_STATUS_SUBTITLE", message)
        .env("QOL_STATUS_DURATION_MS", timeout_ms.to_string())
        .env(
            "QOL_STATUS_MAX_LIFETIME_MS",
            if lifecycle.exit_after_hide() {
                "0".to_string()
            } else {
                STATUS_OVERLAY_MAX_LIFETIME_MS.to_string()
            },
        )
        .env(
            "QOL_STATUS_EXIT_AFTER_HIDE",
            if lifecycle.exit_after_hide() {
                "1"
            } else {
                "0"
            },
        );
    configure_active_monitor_env(command);
}

fn configure_active_monitor_env(command: &mut Command) {
    let Some(monitor) = qol_runtime::PlatformStateClient::from_env()
        .get_state()
        .and_then(|state| state.active_monitor())
    else {
        qol_runtime::probe!("SHOT_STATUS_OVERLAY", "event=active-monitor result=none");
        return;
    };

    qol_runtime::probe!(
        "SHOT_STATUS_OVERLAY",
        "event=active-monitor result=some rect={}x{}+{},{}",
        monitor.width,
        monitor.height,
        monitor.x,
        monitor.y
    );
    command
        .env("QOL_ACTIVE_MONITOR_X", monitor.x.to_string())
        .env("QOL_ACTIVE_MONITOR_Y", monitor.y.to_string())
        .env("QOL_ACTIVE_MONITOR_WIDTH", monitor.width.to_string())
        .env("QOL_ACTIVE_MONITOR_HEIGHT", monitor.height.to_string());
}

fn dismiss_status_overlay() {
    let pids = take_status_overlay_pids();
    qol_runtime::probe!("SHOT_STATUS_OVERLAY", "event=dismiss count={}", pids.len());
    for entry in pids {
        stop_status_overlay_pid(entry);
    }
}

fn write_status_overlay_pid(pid: u32) {
    if let Ok(mut current_pid) = STATUS_OVERLAY_PID.lock() {
        *current_pid = Some(pid);
    }
    let _ = fs::write(status_overlay_pid_file_path(), format!("{pid}\n"));
}

fn clear_status_overlay_pid(pid: u32) {
    if let Ok(mut current_pid) = STATUS_OVERLAY_PID.lock() {
        if *current_pid == Some(pid) {
            *current_pid = None;
        }
    }
    if read_status_overlay_pid_file() == Some(pid) {
        let _ = fs::remove_file(status_overlay_pid_file_path());
    }
}

fn take_status_overlay_pids() -> Vec<StatusOverlayPid> {
    let mut pids = Vec::new();
    if let Ok(mut current_pid) = STATUS_OVERLAY_PID.lock() {
        if let Some(pid) = current_pid.take() {
            pids.push(StatusOverlayPid { pid, trusted: true });
        }
    }

    if let Some(pid) = read_status_overlay_pid_file() {
        if pids.iter().all(|entry| entry.pid != pid) {
            pids.push(StatusOverlayPid {
                pid,
                trusted: false,
            });
        }
    }

    let _ = fs::remove_file(status_overlay_pid_file_path());
    pids
}

#[derive(Debug, Clone, Copy)]
struct StatusOverlayPid {
    pid: u32,
    trusted: bool,
}

fn read_status_overlay_pid_file() -> Option<u32> {
    let content = fs::read_to_string(status_overlay_pid_file_path()).ok()?;
    content.lines().next()?.trim().parse().ok()
}

fn status_overlay_pid_file_path() -> PathBuf {
    env::temp_dir().join(STATUS_OVERLAY_PID_FILE_NAME)
}

fn stop_status_overlay_pid(entry: StatusOverlayPid) {
    if !entry.trusted && !status_overlay_process_matches(entry.pid) {
        qol_runtime::probe!("SHOT_STATUS_OVERLAY", "pid={} result=skip-stale", entry.pid);
        return;
    }
    if !process_alive(entry.pid) {
        return;
    }

    qol_runtime::probe!("SHOT_STATUS_OVERLAY", "pid={} signal=term", entry.pid);
    let _ = signal_process(entry.pid, libc::SIGTERM);
    if wait_for_process_exit(entry.pid, Duration::from_millis(500)) {
        return;
    }

    qol_runtime::probe!("SHOT_STATUS_OVERLAY", "pid={} signal=kill", entry.pid);
    let _ = signal_process(entry.pid, libc::SIGKILL);
}

fn status_overlay_process_matches(pid: u32) -> bool {
    let pid_arg = pid.to_string();
    let Ok(output) = Command::new("ps")
        .args(["-p", pid_arg.as_str(), "-o", "command="])
        .output()
    else {
        return false;
    };
    if !output.status.success() {
        return false;
    }
    let command = String::from_utf8_lossy(&output.stdout);
    command.contains("/qol-shot-swift/status-overlay-")
}

pub(super) fn show_recording_region_overlay(session: &CaptureSession) {
    let Some(rect) = session.canvas else {
        qol_runtime::probe!("SHOT_RECORD_OVERLAY", "event=show result=no-canvas");
        return;
    };

    let targets = recording_overlay_targets(session, rect);
    let display_count = active_display_bounds()
        .map(|displays| displays.len())
        .unwrap_or(0);
    qol_runtime::probe!(
        "SHOT_RECORD_OVERLAY",
        "event=show rect={} displays={display_count} targets={}",
        rect_label(rect),
        targets.len()
    );
    dismiss_recording_region_overlay();

    let mut spawned = 0usize;
    for (index, target) in targets.into_iter().enumerate() {
        let target_label = target
            .display
            .map(rect_label)
            .unwrap_or_else(|| "all-displays".to_string());
        let Ok(child) = spawn_recording_region_overlay(target) else {
            qol_runtime::probe!(
                "SHOT_RECORD_OVERLAY",
                "event=spawn result=error index={index} target={target_label}"
            );
            continue;
        };

        let pid = child.id();
        qol_runtime::probe!(
            "SHOT_RECORD_OVERLAY",
            "event=spawn result=ok index={index} pid={pid} target={target_label}"
        );
        write_recording_overlay_pid(pid);
        spawned += 1;

        thread::spawn(move || {
            let mut child = child;
            let _ = child.wait();
            clear_recording_overlay_pid(pid);
        });
    }

    if spawned == 0 {
        qol_runtime::probe!("SHOT_RECORD_OVERLAY", "event=spawn result=none");
    }
}

#[derive(Clone, Copy)]
struct RecordingOverlayTarget {
    capture: Rect,
    display: Option<Rect>,
}

fn recording_overlay_targets(session: &CaptureSession, capture: Rect) -> Vec<RecordingOverlayTarget> {
    let targets = session
        .segments
        .iter()
        .map(|segment| RecordingOverlayTarget {
            capture,
            display: Some(segment.rect),
        })
        .collect::<Vec<_>>();

    if !targets.is_empty() {
        return targets;
    }

    vec![RecordingOverlayTarget {
        capture,
        display: None,
    }]
}

fn spawn_recording_region_overlay(target: RecordingOverlayTarget) -> Result<Child> {
    match ensure_swift_helper(
        "recording-overlay",
        RECORDING_OVERLAY_SWIFT,
        RECORDING_OVERLAY_HELPER,
    ) {
        Ok(helper) => {
            qol_runtime::probe!("SHOT_RECORD_OVERLAY", "event=helper result=compiled");
            match spawn_compiled_recording_region_overlay(&helper, target) {
                Ok(child) => Ok(child),
                Err(error) => {
                    qol_runtime::probe!(
                        "SHOT_RECORD_OVERLAY",
                        "event=compiled-spawn result=error fallback=source"
                    );
                    let _ = fs::remove_file(&helper);
                    spawn_source_recording_region_overlay(target).with_context(|| {
                        format!("compiled recording overlay failed first: {error:#}")
                    })
                }
            }
        }
        Err(error) => {
            qol_runtime::probe!(
                "SHOT_RECORD_OVERLAY",
                "event=helper result=error fallback=source"
            );
            spawn_source_recording_region_overlay(target)
                .with_context(|| format!("failed to build recording overlay helper: {error:#}"))
        }
    }
}

fn spawn_compiled_recording_region_overlay(
    helper: &Path,
    target: RecordingOverlayTarget,
) -> Result<Child> {
    let mut command = Command::new(helper);
    configure_recording_region_overlay(&mut command, target);
    command
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .context("failed to start compiled macOS recording overlay")
}

fn spawn_source_recording_region_overlay(target: RecordingOverlayTarget) -> Result<Child> {
    spawn_source_swift(RECORDING_OVERLAY_SWIFT, |command| {
        configure_recording_region_overlay(command, target);
        command.stdout(Stdio::null()).stderr(Stdio::null());
    })
    .context("failed to start source macOS recording overlay")
}

fn configure_recording_region_overlay(command: &mut Command, target: RecordingOverlayTarget) {
    let rect = target.capture;
    command
        .env("QOL_RECORDING_RECT_X", rect.x.to_string())
        .env("QOL_RECORDING_RECT_Y", rect.y.to_string())
        .env("QOL_RECORDING_RECT_WIDTH", rect.w.to_string())
        .env("QOL_RECORDING_RECT_HEIGHT", rect.h.to_string())
        .env(
            "QOL_RECORDING_OVERLAY_MAX_LIFETIME_MS",
            RECORDING_OVERLAY_MAX_LIFETIME_MS.to_string(),
        );

    if let Some(display) = target.display {
        command
            .env("QOL_RECORDING_DISPLAY_X", display.x.to_string())
            .env("QOL_RECORDING_DISPLAY_Y", display.y.to_string())
            .env("QOL_RECORDING_DISPLAY_WIDTH", display.w.to_string())
            .env("QOL_RECORDING_DISPLAY_HEIGHT", display.h.to_string());
    }
}

pub(super) fn dismiss_recording_region_overlay() {
    let pids = take_recording_overlay_pids();
    qol_runtime::probe!("SHOT_RECORD_OVERLAY", "event=dismiss count={}", pids.len());
    for entry in pids {
        stop_recording_overlay_pid(entry);
    }
}

fn write_recording_overlay_pid(pid: u32) {
    let mut pids = vec![pid];
    if let Ok(mut current_pids) = RECORDING_OVERLAY_PIDS.lock() {
        if !current_pids.contains(&pid) {
            current_pids.push(pid);
        }
        pids = current_pids.clone();
    }
    write_recording_overlay_pid_file(&pids);
}

fn clear_recording_overlay_pid(pid: u32) {
    let mut pids = read_recording_overlay_pid_file();
    pids.retain(|entry| *entry != pid);
    if let Ok(mut current_pids) = RECORDING_OVERLAY_PIDS.lock() {
        current_pids.retain(|entry| *entry != pid);
        pids = current_pids.clone();
    }
    write_recording_overlay_pid_file(&pids);
}

fn take_recording_overlay_pids() -> Vec<RecordingOverlayPid> {
    let mut pids = Vec::new();
    if let Ok(mut current_pids) = RECORDING_OVERLAY_PIDS.lock() {
        for pid in current_pids.drain(..) {
            pids.push(RecordingOverlayPid { pid, trusted: true });
        }
    }

    for pid in read_recording_overlay_pid_file() {
        if pids.iter().all(|entry| entry.pid != pid) {
            pids.push(RecordingOverlayPid {
                pid,
                trusted: false,
            });
        }
    }

    let _ = fs::remove_file(recording_overlay_pid_file_path());
    pids
}

#[derive(Debug, Clone, Copy)]
struct RecordingOverlayPid {
    pid: u32,
    trusted: bool,
}

fn read_recording_overlay_pid_file() -> Vec<u32> {
    let Ok(content) = fs::read_to_string(recording_overlay_pid_file_path()) else {
        return Vec::new();
    };
    content
        .lines()
        .filter_map(|line| line.trim().parse().ok())
        .collect()
}

fn write_recording_overlay_pid_file(pids: &[u32]) {
    if pids.is_empty() {
        let _ = fs::remove_file(recording_overlay_pid_file_path());
        return;
    }

    let content = pids
        .iter()
        .map(u32::to_string)
        .collect::<Vec<_>>()
        .join("\n");
    let _ = fs::write(recording_overlay_pid_file_path(), format!("{content}\n"));
}

fn recording_overlay_pid_file_path() -> PathBuf {
    env::temp_dir().join(RECORDING_OVERLAY_PID_FILE_NAME)
}

fn stop_recording_overlay_pid(entry: RecordingOverlayPid) {
    if !entry.trusted && !recording_overlay_process_matches(entry.pid) {
        qol_runtime::probe!("SHOT_RECORD_OVERLAY", "pid={} result=skip-stale", entry.pid);
        return;
    }
    if !process_alive(entry.pid) {
        return;
    }

    qol_runtime::probe!("SHOT_RECORD_OVERLAY", "pid={} signal=term", entry.pid);
    let _ = signal_process(entry.pid, libc::SIGTERM);
    if wait_for_process_exit(entry.pid, Duration::from_millis(500)) {
        return;
    }

    qol_runtime::probe!("SHOT_RECORD_OVERLAY", "pid={} signal=kill", entry.pid);
    let _ = signal_process(entry.pid, libc::SIGKILL);
}

fn recording_overlay_process_matches(pid: u32) -> bool {
    let pid_arg = pid.to_string();
    let Ok(output) = Command::new("ps")
        .args(["-p", pid_arg.as_str(), "-o", "command="])
        .output()
    else {
        return false;
    };
    if !output.status.success() {
        return false;
    }
    let command = String::from_utf8_lossy(&output.stdout);
    command.contains("/qol-shot-swift/recording-overlay-")
}
