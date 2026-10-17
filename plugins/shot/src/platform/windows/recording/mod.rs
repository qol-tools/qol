use anyhow::{anyhow, Context, Result};
use qol_plugin_daemon::notification::send_notification;
use std::fs::File;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, ExitStatus, Stdio};
use std::time::{Duration, Instant};

use crate::capture::geometry::{even_dimensions, rect_label};
use crate::platform::{recorder, CaptureProcess, CaptureSession, SavedRecording};
use crate::{Config, Rect};

use super::display::{native_displays, physical_rect};
use super::ffmpeg;

mod audio;
mod helper;
mod mixer;
mod peer;
mod plan;

pub use helper::run_internal_capture_helper;
pub use recorder::recording_format;

const READY_TIMEOUT: Duration = Duration::from_secs(8);
const READY_POLL_INTERVAL: Duration = Duration::from_millis(50);
const HELPER_STOP_GRACE: Duration = Duration::from_secs(2);

#[derive(Debug, Eq, PartialEq)]
enum Readiness {
    Waiting,
    Ready(u64),
    Exited(ExitStatus),
    TimedOut,
}

pub fn capture_log_path() -> PathBuf {
    let name = Path::new(crate::platform::CAPTURE_LOG)
        .file_name()
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(crate::platform::RECORD_REGION_BASE));
    std::env::temp_dir().join(name)
}

pub fn start_capture(rect: &Rect, config: &Config, output_file: &Path) -> Result<CaptureSession> {
    let Some(ffmpeg) = ffmpeg::resolve_ffmpeg() else {
        send_notification(
            "Recording needs ffmpeg",
            "Install ffmpeg with: winget install Gyan.FFmpeg",
        );
        return Err(anyhow!("ffmpeg was not found. {}", ffmpeg::INSTALL_HINT));
    };
    let physical = even_dimensions(physical_rect(*rect, &native_displays()));
    let plan = plan::CapturePlan::new(ffmpeg, physical, config, output_file);
    qol_runtime::probe!(
        "SHOT_RECORD_START_BACKEND",
        "backend=gdigrab outcome=ready logical={} physical={} audio_sources={}",
        rect_label(*rect),
        rect_label(physical),
        plan.audio.len()
    );
    let request = serde_json::to_string(&plan).context("failed to encode capture request")?;
    let log_file = File::create(capture_log_path()).context("failed to create recording log")?;
    let stdout_log = log_file
        .try_clone()
        .context("failed to clone recording log")?;
    let executable = std::env::current_exe().context("failed to resolve qol-shot executable")?;
    let mut command = Command::new(executable);
    let mut child = qol_process::hide_console_window(&mut command)
        .env(helper::HELPER_ENV, request)
        .stdin(Stdio::null())
        .stdout(Stdio::from(stdout_log))
        .stderr(Stdio::from(log_file))
        .spawn()
        .context("failed to start the Windows capture helper")?;
    if let Err(error) = qol_process::bind_to_host_lifetime(child.id()) {
        log::warn!("the capture helper will not stop with qol-shot: {error}");
    }
    if let Err(error) = wait_for_ready(&mut child, output_file) {
        if let Err(stop_error) = qol_process::terminate_owned(&mut child, HELPER_STOP_GRACE) {
            log::warn!("failed to stop the capture helper: {stop_error}");
        }
        send_notification(
            "Recording failed",
            &format!("Check {}", capture_log_path().display()),
        );
        return Err(error);
    }
    Ok(CaptureSession {
        output_file: Some(output_file.to_path_buf()),
        capture_file: Some(output_file.to_path_buf()),
        canvas: Some(physical),
        processes: vec![CaptureProcess::spawned(child.id())],
        segments: Vec::new(),
    })
}

fn wait_for_ready(child: &mut Child, output_file: &Path) -> Result<()> {
    let deadline = Instant::now() + READY_TIMEOUT;
    loop {
        let exited = child
            .try_wait()
            .context("failed to inspect the capture helper")?;
        let len = output_file
            .metadata()
            .ok()
            .filter(|metadata| metadata.is_file())
            .map(|metadata| metadata.len());
        match readiness(exited, len, Instant::now() >= deadline) {
            Readiness::Waiting => std::thread::sleep(READY_POLL_INTERVAL),
            Readiness::Ready(len) => {
                qol_runtime::probe!("SHOT_RECORD_CAPTURE_READY", "backend=gdigrab len={len}");
                return Ok(());
            }
            Readiness::Exited(status) => {
                return Err(anyhow!("capture helper exited early with {status}"));
            }
            Readiness::TimedOut => {
                qol_runtime::probe!(
                    "SHOT_RECORD_CAPTURE_READY",
                    "backend=gdigrab len=0 reason=timeout helper=alive"
                );
                return Err(anyhow!(
                    "ffmpeg wrote nothing to {} within {} s",
                    output_file.display(),
                    READY_TIMEOUT.as_secs()
                ));
            }
        }
    }
}

fn readiness(exited: Option<ExitStatus>, output_len: Option<u64>, timed_out: bool) -> Readiness {
    if let Some(status) = exited {
        return Readiness::Exited(status);
    }
    if let Some(len) = output_len.filter(|len| *len > 0) {
        return Readiness::Ready(len);
    }
    if timed_out {
        return Readiness::TimedOut;
    }
    Readiness::Waiting
}

pub fn recording_started(_session: &CaptureSession) {}

pub fn recording_stopped(session: &CaptureSession, _config: &Config) -> Option<SavedRecording> {
    qol_plugin_daemon::notification::send_banner("Recording stopped", "Saving recording", None);
    let output_file = session.output_file.as_deref()?;
    recorder::await_capture_file(session, output_file)?;
    let message = output_file
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("Saved in Videos")
        .to_string();
    Some(SavedRecording {
        path: output_file.to_path_buf(),
        message,
    })
}

pub fn stop_capture(session: &CaptureSession) -> Result<()> {
    for process in &session.processes {
        let Some(identity) = process.identity.as_deref() else {
            continue;
        };
        let requested = qol_process::request_stop_if_identity(process.pid, identity)
            .with_context(|| format!("failed to stop capture helper {}", process.pid))?;
        qol_runtime::probe!(
            "SHOT_RECORD_HELPER_STOP",
            "pid={} requested={requested}",
            process.pid
        );
        if !requested && process.alive() {
            return Err(anyhow!(
                "capture helper {} is not listening for a stop request",
                process.pid
            ));
        }
    }
    Ok(())
}

pub fn watch_daemon_stop(on_stop: fn()) {
    if let Err(error) = qol_process::on_stop_request(on_stop) {
        log::warn!("qol-shot cannot hear daemon stop requests: {error}");
    }
}

#[cfg(test)]
mod tests {
    use super::{readiness, Readiness};

    #[test]
    fn readiness_waits_for_output_and_fails_on_exit_or_timeout() {
        let exited = qol_process::system_command("cmd.exe")
            .args(["/C", "exit 3"])
            .status()
            .unwrap();
        let cases = [
            (None, None, false, Readiness::Waiting),
            (None, Some(0), false, Readiness::Waiting),
            (None, Some(42), false, Readiness::Ready(42)),
            (None, Some(42), true, Readiness::Ready(42)),
            (None, None, true, Readiness::TimedOut),
            (None, Some(0), true, Readiness::TimedOut),
            (Some(exited), Some(42), false, Readiness::Exited(exited)),
            (Some(exited), None, true, Readiness::Exited(exited)),
        ];
        for (status, len, timed_out, expected) in cases {
            assert_eq!(
                readiness(status, len, timed_out),
                expected,
                "{status:?} {len:?} {timed_out}"
            );
        }
    }
}
