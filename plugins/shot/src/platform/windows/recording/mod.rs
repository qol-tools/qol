use anyhow::{anyhow, Context, Result};
use qol_plugin_daemon::notification::send_notification;
use std::fs::File;
use std::os::windows::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};
use windows_sys::Win32::System::Threading::CREATE_NO_WINDOW;

use crate::capture::geometry::{even_dimensions, rect_label};
use crate::platform::{recorder, CaptureProcess, CaptureSession, SavedRecording};
use crate::{Config, Rect};

use super::display::{native_displays, physical_rect};
use super::ffmpeg;

mod audio;
mod helper;
mod mixer;
mod plan;

pub use helper::run_internal_capture_helper;
pub use recorder::recording_format;

const READY_TIMEOUT: Duration = Duration::from_secs(8);
const READY_POLL_INTERVAL: Duration = Duration::from_millis(50);

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
    let mut child = Command::new(executable)
        .env(helper::HELPER_ENV, request)
        .stdin(Stdio::null())
        .stdout(Stdio::from(stdout_log))
        .stderr(Stdio::from(log_file))
        .creation_flags(CREATE_NO_WINDOW)
        .spawn()
        .context("failed to start the Windows capture helper")?;
    if let Err(error) = wait_for_ready(&mut child, output_file) {
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
        processes: vec![CaptureProcess { pid: child.id() }],
        segments: Vec::new(),
    })
}

fn wait_for_ready(child: &mut Child, output_file: &Path) -> Result<()> {
    let deadline = Instant::now() + READY_TIMEOUT;
    while Instant::now() < deadline {
        if let Some(status) = child
            .try_wait()
            .context("failed to inspect the capture helper")?
        {
            return Err(anyhow!("capture helper exited early with {status}"));
        }
        if let Some(len) = output_file
            .metadata()
            .ok()
            .filter(|metadata| metadata.is_file() && metadata.len() > 0)
            .map(|metadata| metadata.len())
        {
            qol_runtime::probe!("SHOT_RECORD_CAPTURE_READY", "backend=gdigrab len={len}");
            return Ok(());
        }
        std::thread::sleep(READY_POLL_INTERVAL);
    }
    qol_runtime::probe!(
        "SHOT_RECORD_CAPTURE_READY",
        "backend=gdigrab len=0 reason=timeout helper=alive"
    );
    Ok(())
}

pub fn recording_started(_session: &CaptureSession) {}

pub fn recording_stopped(session: &CaptureSession, _config: &Config) -> Option<SavedRecording> {
    send_notification("Recording stopped", "Saving recording");
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
        qol_process::signal_term_pid(process.pid)
            .with_context(|| format!("failed to stop capture helper {}", process.pid))?;
    }
    Ok(())
}
