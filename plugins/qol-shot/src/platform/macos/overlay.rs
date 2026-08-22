use anyhow::{Context, Result};
use std::env;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::Mutex;
use std::thread;
use std::time::Duration;

use super::swift::{
    ensure_swift_helper, spawn_source_swift, STATUS_OVERLAY_HELPER, STATUS_OVERLAY_SWIFT,
};
use super::system::{process_alive, signal_process, wait_for_process_exit};

const STATUS_OVERLAY_PID_FILE_NAME: &str = "qol-shot-status-overlay.pid";
const STATUS_OVERLAY_MAX_LIFETIME_MS: u32 = 120_000;
static STATUS_OVERLAY_PID: Mutex<Option<u32>> = Mutex::new(None);

#[derive(Clone, Copy)]
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
        stop_overlay_pid(
            entry,
            "SHOT_STATUS_OVERLAY",
            "/qol-shot-swift/status-overlay-",
        );
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

fn take_status_overlay_pids() -> Vec<OverlayPid> {
    let mut pids = Vec::new();
    if let Ok(mut current_pid) = STATUS_OVERLAY_PID.lock() {
        if let Some(pid) = current_pid.take() {
            pids.push(OverlayPid { pid, trusted: true });
        }
    }

    if let Some(pid) = read_status_overlay_pid_file() {
        if pids.iter().all(|entry| entry.pid != pid) {
            pids.push(OverlayPid {
                pid,
                trusted: false,
            });
        }
    }

    let _ = fs::remove_file(status_overlay_pid_file_path());
    pids
}

#[derive(Debug, Clone, Copy)]
struct OverlayPid {
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

fn stop_overlay_pid(entry: OverlayPid, probe: &str, needle: &str) {
    if !entry.trusted && !overlay_process_matches(entry.pid, needle) {
        qol_runtime::probe!(probe, "pid={} result=skip-stale", entry.pid);
        return;
    }
    if !process_alive(entry.pid) {
        return;
    }

    qol_runtime::probe!(probe, "pid={} signal=term", entry.pid);
    let _ = signal_process(entry.pid, libc::SIGTERM);
    if wait_for_process_exit(entry.pid, Duration::from_millis(500)) {
        return;
    }

    qol_runtime::probe!(probe, "pid={} signal=kill", entry.pid);
    let _ = signal_process(entry.pid, libc::SIGKILL);
}

fn overlay_process_matches(pid: u32, needle: &str) -> bool {
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
    command.contains(needle)
}
