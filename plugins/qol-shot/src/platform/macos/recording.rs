use anyhow::{anyhow, Context, Result};
use std::fs::{self, File, OpenOptions};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::thread;
use std::time::{Duration, Instant};

use crate::platform::{CaptureProcess, CaptureSegment, CaptureSession};
use crate::{Config, Rect};

use super::conversion::{convert_recording, run_conversion_command};
use super::display::{active_displays, rect_intersection, DisplayInfo};
use super::labels::{monitor_label, path_label, rect_label};
use super::overlay::{show_status_overlay, StatusOverlayLifecycle};
use super::swift::{
    ensure_swift_helper, prewarm_swift_helper, STATUS_OVERLAY_HELPER, STATUS_OVERLAY_SWIFT,
    VIDEO_COMPOSER_HELPER, VIDEO_COMPOSER_SWIFT,
};
use super::system::{
    capture_work_dir, ensure_capture_work_dir, move_file, open_path, output_format_label,
    path_extension_is, paths_match, show_notification, signal_process, videos_dir,
    wait_for_process_exit,
};

#[derive(Debug, Clone)]
struct DisplayCaptureSegment {
    display_index: u32,
    rect: Rect,
    offset_x: i32,
    offset_y: i32,
}

pub fn start_capture(rect: &Rect, config: &Config, output_file: &Path) -> Result<CaptureSession> {
    let capture_file = native_capture_file_path(output_file);
    ensure_capture_work_dir()?;
    let segments = capture_segments(rect)?;

    qol_runtime::probe!(
        "SHOT_RECORD_START_PLAN",
        "plan=full-display-native-crop segments={} rect={} output={} capture={} same_file={} audio={}",
        segments.len(),
        rect_label(*rect),
        path_label(output_file),
        path_label(&capture_file),
        paths_match(output_file, &capture_file),
        config.audio.enabled,
    );

    start_multi_display_capture(*rect, config, output_file, &capture_file, &segments)
}

pub fn capture_screenshot(rect: &Rect, output_file: &Path) -> Result<()> {
    let (stdout_log, stderr_log) = capture_log_files(CaptureLogMode::Truncate)?;
    let region = format!("{},{},{},{}", rect.x, rect.y, rect.w, rect.h);
    let status = Command::new("screencapture")
        .args(["-R", region.as_str(), "-x"])
        .arg(output_file)
        .stdin(Stdio::null())
        .stdout(Stdio::from(stdout_log))
        .stderr(Stdio::from(stderr_log))
        .status()
        .context("failed to run screencapture screenshot capture")?;

    if status.success() {
        return Ok(());
    }

    Err(anyhow!(
        "screencapture screenshot capture exited with {status}"
    ))
}

pub fn recording_format(format: &str) -> String {
    match format.to_ascii_lowercase().as_str() {
        "mkv" | "mp4" | "mov" | "webm" => format.to_ascii_lowercase(),
        _ => "mov".to_string(),
    }
}

pub(super) fn native_capture_file_path(output_file: &Path) -> PathBuf {
    let stem = output_file
        .file_stem()
        .map(|stem| stem.to_string_lossy())
        .unwrap_or_else(|| "recording".into());
    capture_work_dir().join(format!("{stem}.mov"))
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Finalization {
    MoveNative,
    Reencode,
}

pub(super) fn finalization_for(output_file: &Path) -> Finalization {
    if path_extension_is(output_file, "mov") {
        Finalization::MoveNative
    } else {
        Finalization::Reencode
    }
}

pub fn recording_started(_session: &CaptureSession, _countdown_completed: bool) {
    qol_runtime::probe!("SHOT_RECORD_NOTIFY", "stage=started");
    show_notification("Recording started", "Press your hotkey to stop", 1200);
    prewarm_recording_helpers();
}

pub fn recording_stopped(session: &CaptureSession, config: &Config) -> Option<PathBuf> {
    qol_runtime::probe!(
        "SHOT_RECORD_FINALIZE",
        "stage=stopped-entry pids={} segments={}",
        session.pid_list(),
        session.segments.len()
    );
    if let Some(output_file) = session.output_file.as_deref() {
        let capture_file = session.capture_file.as_deref().unwrap_or(output_file);
        let reencode_needed = finalization_for(output_file) == Finalization::Reencode;
        qol_runtime::probe!(
            "SHOT_RECORD_FINALIZE",
            "stage=begin output={} capture={} reencode_needed={} segments={}",
            path_label(output_file),
            path_label(capture_file),
            reencode_needed,
            session.segments.len()
        );
        show_recording_ended(output_file, reencode_needed);
        let (reveal_file, message) =
            match finalize_recording(session, output_file, capture_file, config) {
                Ok(reveal_file) => {
                    qol_runtime::probe!(
                        "SHOT_RECORD_FINALIZE",
                        "stage=ok reveal={} reencoded={}",
                        path_label(&reveal_file),
                        reencode_needed
                    );
                    let message = show_recording_saved(&reveal_file, reencode_needed);
                    (reveal_file, message)
                }
                Err(error) => {
                    qol_runtime::probe!("SHOT_RECORD_FINALIZE", "stage=error result=fallback");
                    let reveal_file =
                        finalization_fallback(error, session, output_file, capture_file);
                    let message = format!("Saved native recording as {}", path_label(&reveal_file));
                    (reveal_file, message)
                }
            };
        crate::capture::completion::background_saved(
            "Recording saved",
            &message,
            &reveal_file,
            config.capture.open_folder_after_save,
        );
        return Some(reveal_file);
    }

    qol_runtime::probe!("SHOT_RECORD_FINALIZE", "stage=open-videos");
    show_status_overlay(
        "Recording stopped",
        "Opening Videos\u{2026}",
        1800,
        StatusOverlayLifecycle::ExitAfterHide,
    );
    let videos_dir = config
        .capture
        .open_folder_after_save
        .then(videos_dir)
        .flatten();
    if let Some(videos_dir) = videos_dir {
        open_path(&videos_dir);
    }
    None
}

pub fn stop_capture(session: &CaptureSession) -> Result<()> {
    qol_runtime::probe!(
        "SHOT_RECORD_STOP_PLATFORM",
        "pids={} count={} segments={}",
        session.pid_list(),
        session.processes.len(),
        session.segments.len()
    );
    let mut failures = Vec::new();
    for process in &session.processes {
        if let Err(error) = stop_capture_process(process.pid) {
            failures.push(format!("{}: {error:#}", process.pid));
        }
    }

    if failures.is_empty() {
        qol_runtime::probe!("SHOT_RECORD_STOP_PLATFORM", "result=ok");
        return Ok(());
    }

    qol_runtime::probe!(
        "SHOT_RECORD_STOP_PLATFORM",
        "result=error failures={}",
        failures.len()
    );
    Err(anyhow!(
        "failed to stop capture processes: {}",
        failures.join("; ")
    ))
}

fn display_summary(displays: &[DisplayInfo]) -> (usize, String) {
    (
        displays.len(),
        displays
            .iter()
            .map(|display| {
                format!(
                    "d{}:{}",
                    display.display_index,
                    monitor_label(display.bounds)
                )
            })
            .collect::<Vec<_>>()
            .join("|"),
    )
}

fn segment_summary(segments: &[DisplayCaptureSegment]) -> String {
    if segments.is_empty() {
        return "none".to_string();
    }

    segments
        .iter()
        .enumerate()
        .map(|(index, segment)| {
            format!(
                "#{index}:d{}:{}@{},{}",
                segment.display_index,
                rect_label(segment.rect),
                segment.offset_x,
                segment.offset_y
            )
        })
        .collect::<Vec<_>>()
        .join("|")
}

fn capture_segments(rect: &Rect) -> Result<Vec<DisplayCaptureSegment>> {
    let displays = active_displays()?;
    let display_summary = display_summary(&displays);
    let segments = displays
        .into_iter()
        .filter_map(|display| {
            rect_intersection(*rect, display.bounds).map(|_intersection| {
                let capture_rect = Rect {
                    x: display.bounds.x,
                    y: display.bounds.y,
                    w: display.bounds.w,
                    h: display.bounds.h,
                };
                DisplayCaptureSegment {
                    display_index: display.display_index,
                    rect: capture_rect,
                    offset_x: capture_rect.x - rect.x,
                    offset_y: capture_rect.y - rect.y,
                }
            })
        })
        .collect::<Vec<_>>();
    qol_runtime::probe!(
        "SHOT_MAC_CAPTURE_SEGMENTS",
        "rect={} displays={} display_bounds={} segments={} segment_bounds={}",
        rect_label(*rect),
        display_summary.0,
        display_summary.1,
        segments.len(),
        segment_summary(&segments)
    );

    if segments.is_empty() {
        return Err(anyhow!(
            "selected recording area does not intersect an active display"
        ));
    }

    Ok(segments)
}

fn start_multi_display_capture(
    canvas: Rect,
    config: &Config,
    output_file: &Path,
    capture_file: &Path,
    segments: &[DisplayCaptureSegment],
) -> Result<CaptureSession> {
    let mut processes = Vec::new();
    let mut capture_segments = Vec::new();
    qol_runtime::probe!(
        "SHOT_MAC_CAPTURE_MULTI",
        "stage=begin canvas={} segments={} output={} capture={} audio={}",
        rect_label(canvas),
        segments.len(),
        path_label(output_file),
        path_label(capture_file),
        config.audio.enabled
    );

    for (index, segment) in segments.iter().enumerate() {
        let segment_file = segment_capture_file(capture_file, index);
        let log_mode = if index == 0 {
            CaptureLogMode::Truncate
        } else {
            CaptureLogMode::Append
        };
        qol_runtime::probe!(
            "SHOT_MAC_CAPTURE_MULTI",
            "stage=segment index={} display={} rect={} offset={},{} file={} audio={} log={}",
            index,
            segment.display_index,
            rect_label(segment.rect),
            segment.offset_x,
            segment.offset_y,
            path_label(&segment_file),
            config.audio.enabled && index == 0,
            log_mode.as_str()
        );
        let pid = match spawn_screencapture(
            None,
            Some(segment.display_index),
            config.audio.enabled && index == 0,
            &segment_file,
            log_mode,
        ) {
            Ok(pid) => pid,
            Err(error) => {
                qol_runtime::probe!(
                    "SHOT_MAC_CAPTURE_MULTI",
                    "stage=spawn-error index={} spawned={}",
                    index,
                    processes.len()
                );
                stop_spawned_processes(&processes);
                cleanup_segment_files(&capture_segments);
                return Err(error);
            }
        };

        processes.push(CaptureProcess { pid });
        capture_segments.push(CaptureSegment {
            file: segment_file,
            rect: segment.rect,
            offset_x: segment.offset_x,
            offset_y: segment.offset_y,
        });
    }

    qol_runtime::probe!(
        "SHOT_MAC_CAPTURE_MULTI",
        "stage=ok pids={} segments={}",
        processes
            .iter()
            .map(|process| process.pid.to_string())
            .collect::<Vec<_>>()
            .join(", "),
        capture_segments.len()
    );
    Ok(CaptureSession {
        output_file: Some(output_file.to_path_buf()),
        capture_file: Some(capture_file.to_path_buf()),
        canvas: Some(canvas),
        processes,
        segments: capture_segments,
    })
}

#[derive(Clone, Copy)]
enum CaptureLogMode {
    Truncate,
    Append,
}

impl CaptureLogMode {
    fn as_str(self) -> &'static str {
        match self {
            Self::Truncate => "truncate",
            Self::Append => "append",
        }
    }
}

fn spawn_screencapture(
    rect: Option<Rect>,
    display_index: Option<u32>,
    include_audio: bool,
    output_file: &Path,
    log_mode: CaptureLogMode,
) -> Result<u32> {
    let (stdout_log, stderr_log) = capture_log_files(log_mode)?;

    let mut command = Command::new("screencapture");
    let args = screencapture_recording_args(rect, display_index, include_audio);
    qol_runtime::probe!(
        "SHOT_MAC_CAPTURE_SPAWN",
        "display={:?} rect={} audio={include_audio} log={} output={} args={}",
        display_index,
        rect.map(rect_label)
            .unwrap_or_else(|| "full-display".to_string()),
        log_mode.as_str(),
        path_label(output_file),
        args.join(" ")
    );
    command.args(&args);

    let child = command
        .arg(output_file)
        .stdin(Stdio::null())
        .stdout(Stdio::from(stdout_log))
        .stderr(Stdio::from(stderr_log))
        .spawn()
        .context("failed to start screencapture")?;

    qol_runtime::probe!(
        "SHOT_MAC_CAPTURE_PID",
        "pid={} display={:?} output={}",
        child.id(),
        display_index,
        path_label(output_file)
    );
    Ok(child.id())
}

pub(super) fn screencapture_recording_args(
    rect: Option<Rect>,
    display_index: Option<u32>,
    include_audio: bool,
) -> Vec<String> {
    let mut args = vec!["-v".to_string()];
    if let Some(display_index) = display_index {
        args.extend(["-D".to_string(), display_index.to_string()]);
    }
    if let Some(rect) = rect {
        args.extend([
            "-R".to_string(),
            format!("{},{},{},{}", rect.x, rect.y, rect.w, rect.h),
        ]);
    }
    args.push("-x".to_string());

    if include_audio {
        args.push("-g".to_string());
    }

    args
}

fn capture_log_files(mode: CaptureLogMode) -> Result<(File, File)> {
    let log_file = match mode {
        CaptureLogMode::Truncate => File::create(crate::platform::CAPTURE_LOG),
        CaptureLogMode::Append => OpenOptions::new()
            .create(true)
            .append(true)
            .open(crate::platform::CAPTURE_LOG),
    }
    .context("failed to open recording log file")?;
    let stdout_log = log_file
        .try_clone()
        .context("failed to clone recording log file")?;
    Ok((stdout_log, log_file))
}

fn segment_capture_file(capture_file: &Path, index: usize) -> PathBuf {
    let stem = capture_file
        .file_stem()
        .map(|stem| stem.to_string_lossy())
        .unwrap_or_else(|| "recording".into());
    capture_file.with_file_name(format!("{stem}.segment-{:02}.mov", index + 1))
}

fn stop_spawned_processes(processes: &[CaptureProcess]) {
    for process in processes {
        let _ = stop_capture_process(process.pid);
    }
}

fn cleanup_segment_files(segments: &[CaptureSegment]) {
    for segment in segments {
        let _ = fs::remove_file(&segment.file);
    }
}

fn finalize_recording(
    session: &CaptureSession,
    output_file: &Path,
    capture_file: &Path,
    config: &Config,
) -> Result<PathBuf> {
    qol_runtime::probe!(
        "SHOT_RECORD_FINALIZE",
        "stage=prepare-native capture={} segments={}",
        path_label(capture_file),
        session.segments.len()
    );
    prepare_native_capture_file(session, capture_file, config)?;
    match finalization_for(output_file) {
        Finalization::MoveNative => {
            qol_runtime::probe!(
                "SHOT_RECORD_FINALIZE",
                "stage=move-native capture={} output={}",
                path_label(capture_file),
                path_label(output_file)
            );
            move_file(capture_file, output_file)?;
        }
        Finalization::Reencode => {
            qol_runtime::probe!(
                "SHOT_RECORD_FINALIZE",
                "stage=reencode capture={} output={}",
                path_label(capture_file),
                path_label(output_file)
            );
            convert_recording(capture_file, output_file, config)?;
            let _ = std::fs::remove_file(capture_file);
        }
    }
    Ok(output_file.to_path_buf())
}

fn prepare_native_capture_file(
    session: &CaptureSession,
    capture_file: &Path,
    _config: &Config,
) -> Result<()> {
    if session.segments.is_empty() {
        qol_runtime::probe!(
            "SHOT_RECORD_NATIVE_FILE",
            "mode=legacy-single capture={}",
            path_label(capture_file)
        );
        return wait_for_stable_file(capture_file);
    }

    qol_runtime::probe!(
        "SHOT_RECORD_NATIVE_FILE",
        "mode=compose count={} capture={}",
        session.segments.len(),
        path_label(capture_file)
    );
    compose_segment_recordings(session, capture_file)
}

fn compose_segment_recordings(session: &CaptureSession, capture_file: &Path) -> Result<()> {
    let canvas = session
        .canvas
        .ok_or_else(|| anyhow!("multi-display capture session is missing canvas bounds"))?;

    for segment in &session.segments {
        wait_for_stable_file(&segment.file)?;
    }

    qol_runtime::probe!(
        "SHOT_RECORD_COMPOSE",
        "tool=native segments={} capture={}",
        session.segments.len(),
        path_label(capture_file)
    );
    compose_segment_recordings_with_native_helper(session, canvas, capture_file)?;
    cleanup_segment_files(&session.segments);
    Ok(())
}

fn compose_segment_recordings_with_native_helper(
    session: &CaptureSession,
    canvas: Rect,
    capture_file: &Path,
) -> Result<()> {
    let helper = ensure_swift_helper(
        "video-composer",
        VIDEO_COMPOSER_SWIFT,
        VIDEO_COMPOSER_HELPER,
    )
    .context("failed to install embedded video composer helper")?;
    let mut command = Command::new(&helper);
    command.args(native_segment_composition_args(
        session,
        canvas,
        capture_file,
    ));
    run_conversion_command(&mut command, "native segment composition")
}

pub(super) fn native_segment_composition_args(
    session: &CaptureSession,
    canvas: Rect,
    capture_file: &Path,
) -> Vec<String> {
    let mut args = vec![
        canvas.w.to_string(),
        canvas.h.to_string(),
        capture_file.to_string_lossy().to_string(),
    ];

    for segment in &session.segments {
        args.extend([
            segment.offset_x.to_string(),
            segment.offset_y.to_string(),
            segment.rect.w.to_string(),
            segment.rect.h.to_string(),
            segment.file.to_string_lossy().to_string(),
        ]);
    }

    args
}

fn finalization_fallback(
    error: anyhow::Error,
    session: &CaptureSession,
    output_file: &Path,
    capture_file: &Path,
) -> PathBuf {
    qol_runtime::probe!(
        "SHOT_RECORD_FINALIZE",
        "stage=fallback capture={} segments={}",
        path_label(capture_file),
        session.segments.len()
    );
    let native_file = fallback_recording_file(session, capture_file);
    let reveal_file = relocate_fallback_recording(&native_file, output_file);
    show_status_overlay(
        "Conversion failed",
        "Saved native recording instead",
        3000,
        StatusOverlayLifecycle::ExitAfterHide,
    );
    show_notification(
        "Recording conversion failed",
        &format!("Saved native recording instead. {}", error),
        2500,
    );
    reveal_file
}

fn relocate_fallback_recording(native_file: &Path, output_file: &Path) -> PathBuf {
    let destination = output_file.with_extension("mov");
    if native_file == destination {
        return destination;
    }
    match move_file(native_file, &destination) {
        Ok(()) => destination,
        Err(_) => native_file.to_path_buf(),
    }
}

fn fallback_recording_file(session: &CaptureSession, capture_file: &Path) -> PathBuf {
    if capture_file.symlink_metadata().is_ok() {
        return capture_file.to_path_buf();
    }

    session
        .segments
        .iter()
        .find(|segment| segment.file.symlink_metadata().is_ok())
        .map(|segment| segment.file.clone())
        .unwrap_or_else(|| capture_file.to_path_buf())
}

fn show_recording_ended(output_file: &Path, conversion_needed: bool) {
    qol_runtime::probe!(
        "SHOT_RECORD_STATUS",
        "stage=ended output={} conversion_needed={conversion_needed}",
        path_label(output_file)
    );
    if !conversion_needed {
        show_status_overlay(
            "Recording stopped",
            "Saving recording\u{2026}",
            1800,
            StatusOverlayLifecycle::KeepAlive,
        );
        show_notification("Recording stopped", "Saving recording\u{2026}", 1600);
        return;
    }

    let message = format!("Converting to {}\u{2026}", output_format_label(output_file));
    show_status_overlay(
        "Recording stopped",
        &message,
        2400,
        StatusOverlayLifecycle::KeepAlive,
    );
    show_notification("Recording stopped", &message, 2200);
}

fn show_recording_saved(output_file: &Path, converted: bool) -> String {
    qol_runtime::probe!(
        "SHOT_RECORD_STATUS",
        "stage=saved output={} converted={converted}",
        path_label(output_file)
    );
    let format = output_format_label(output_file);
    let message = if converted {
        format!("Converted to {} in Videos", format)
    } else {
        format!("Saved as {} in Videos", format)
    };
    show_status_overlay(
        "Recording saved",
        &message,
        2400,
        StatusOverlayLifecycle::ExitAfterHide,
    );
    message
}

fn prewarm_recording_helpers() {
    prewarm_swift_helper(
        "status-overlay",
        STATUS_OVERLAY_SWIFT,
        STATUS_OVERLAY_HELPER,
    );
    prewarm_swift_helper(
        "video-composer",
        VIDEO_COMPOSER_SWIFT,
        VIDEO_COMPOSER_HELPER,
    );
}

fn wait_for_stable_file(output_file: &Path) -> Result<()> {
    qol_runtime::probe!(
        "SHOT_RECORD_FILE_STABLE",
        "stage=wait file={}",
        path_label(output_file)
    );
    let deadline = Instant::now() + Duration::from_secs(12);
    let mut last_len = None;
    let mut stable_count = 0;

    while Instant::now() < deadline {
        if let Ok(metadata) = output_file.symlink_metadata() {
            let len = metadata.len();
            if len > 0 && Some(len) == last_len {
                stable_count += 1;
                if stable_count >= 3 {
                    qol_runtime::probe!(
                        "SHOT_RECORD_FILE_STABLE",
                        "stage=ok file={} len={}",
                        path_label(output_file),
                        len
                    );
                    return Ok(());
                }
            } else {
                stable_count = 0;
                last_len = Some(len);
            }
        }
        thread::sleep(Duration::from_millis(200));
    }

    qol_runtime::probe!(
        "SHOT_RECORD_FILE_STABLE",
        "stage=timeout file={} last_len={}",
        path_label(output_file),
        last_len
            .map(|len| len.to_string())
            .unwrap_or_else(|| "none".to_string())
    );
    Err(anyhow!(
        "recording file did not finish writing: {}",
        output_file.display()
    ))
}

const STOP_ESCALATION: [(i32, &str, Duration); 3] = [
    (libc::SIGINT, "int", Duration::from_secs(8)),
    (libc::SIGTERM, "term", Duration::from_secs(2)),
    (libc::SIGKILL, "kill", Duration::from_secs(2)),
];

fn stop_capture_process(pid: u32) -> Result<()> {
    for (signal, label, timeout) in STOP_ESCALATION {
        qol_runtime::probe!("SHOT_RECORD_STOP_PID", "pid={} signal={label}", pid);
        signal_process(pid, signal)?;
        if wait_for_process_exit(pid, timeout) {
            qol_runtime::probe!(
                "SHOT_RECORD_STOP_PID",
                "pid={} result=stopped signal={label}",
                pid
            );
            return Ok(());
        }
    }

    qol_runtime::probe!("SHOT_RECORD_STOP_PID", "pid={} result=still_alive", pid);
    Err(anyhow!("capture process pid {} did not stop", pid))
}
