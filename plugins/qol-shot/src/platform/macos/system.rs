use anyhow::{anyhow, Context, Result};
use qol_headless::DoctorCheckResult;
use qol_runtime::protocol::NotificationLevel;
use std::env;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};
use std::thread;
use std::time::{Duration, Instant};

use crate::capture::frozen_frame::FrozenFrame;
use crate::{Monitor, Rect};

use super::display::{active_displays, DisplayInfo};
use super::native_capture;

static FROZEN_CAPTURE_SEQUENCE: AtomicU64 = AtomicU64::new(0);

#[link(name = "CoreGraphics", kind = "framework")]
extern "C" {
    fn CGPreflightScreenCaptureAccess() -> bool;
}

pub fn process_alive(pid: u32) -> bool {
    super::super::unix::process_alive(pid)
}

pub fn show_notification(title: &str, message: &str, _timeout_ms: u32) {
    let client = qol_runtime::PlatformStateClient::from_env();
    if client.send_notification(title, message, NotificationLevel::Info) {
        return;
    }
    let script = format!(
        "display notification \"{}\" with title \"{}\"",
        escape_applescript(message),
        escape_applescript(title)
    );

    let _ = Command::new("osascript")
        .arg("-e")
        .arg(script)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status();
}

pub fn show_saved_notification(
    title: &str,
    message: &str,
    timeout_ms: u32,
    _target: crate::capture::completion::RevealTarget,
) {
    let client = qol_runtime::PlatformStateClient::from_env();
    if client.send_notification_with_layout(
        title,
        message,
        NotificationLevel::Info,
        None,
        Some(crate::capture::completion::corner_toast_layout()),
    ) {
        return;
    }
    show_notification(title, message, timeout_ms);
}

pub fn open_url(url: &str) -> Result<()> {
    qol_apps::desktop_integration::open_with_default_app(url).context("failed to open URL")
}

pub fn grab_preview_rgba(_rect: &Rect) -> Option<(Vec<u8>, u32, u32)> {
    None
}

pub fn capture_frozen_frame() -> Result<Option<FrozenFrame>> {
    let displays = active_displays()?;
    let sequence = FROZEN_CAPTURE_SEQUENCE.fetch_add(1, Ordering::Relaxed);
    let mut files = FrozenCaptureFiles::default();
    let jobs = displays
        .into_iter()
        .map(|display| FrozenCaptureJob {
            display,
            path: files.path(sequence, display.display_index),
        })
        .collect();
    let segments = capture_frozen_displays(jobs)?;
    FrozenFrame::from_bgra_segments(segments)
        .map(Some)
        .context("captured macOS displays could not form a frozen frame")
}

type FrozenSegmentData = (Rect, Vec<u8>, u32, u32);

struct FrozenCaptureJob {
    display: DisplayInfo,
    path: PathBuf,
}

fn capture_frozen_displays(jobs: Vec<FrozenCaptureJob>) -> Result<Vec<FrozenSegmentData>> {
    thread::scope(|scope| {
        let handles = jobs
            .into_iter()
            .map(|job| scope.spawn(move || capture_frozen_display(job)))
            .collect::<Vec<_>>();
        handles
            .into_iter()
            .map(|handle| {
                handle
                    .join()
                    .map_err(|_| anyhow!("frozen display capture worker panicked"))?
            })
            .collect()
    })
}

fn capture_frozen_display(job: FrozenCaptureJob) -> Result<FrozenSegmentData> {
    match native_capture::capture_display(job.display.bounds) {
        Ok(Some(frame)) => {
            let bounds = rect_from_monitor(job.display.bounds);
            qol_runtime::probe!(
                "SHOT_FREEZE",
                "stage=display backend=sck index={} logical={}x{} pixels={}x{} capture_ms={} copy_ms={} total_ms={}",
                job.display.display_index,
                bounds.w,
                bounds.h,
                frame.width,
                frame.height,
                frame.capture_ms,
                frame.copy_ms,
                frame.total_ms
            );
            return Ok((bounds, frame.pixels, frame.width, frame.height));
        }
        Ok(None) => {
            qol_runtime::probe!(
                "SHOT_FREEZE",
                "stage=fallback index={} reason=sck-unavailable",
                job.display.display_index
            );
        }
        Err(error) => {
            eprintln!(
                "[qol-shot] native frozen display {} capture failed: {error:#}",
                job.display.display_index
            );
            qol_runtime::probe!(
                "SHOT_FREEZE",
                "stage=fallback index={} reason=sck-error",
                job.display.display_index
            );
        }
    }
    capture_frozen_display_fallback(job)
}

fn capture_frozen_display_fallback(job: FrozenCaptureJob) -> Result<FrozenSegmentData> {
    let started = Instant::now();
    let capture_started = Instant::now();
    let output = run_frozen_screencapture(&job)?;
    let capture_ms = capture_started.elapsed().as_millis();
    ensure_frozen_capture_succeeded(&job, &output)?;
    let decode_started = Instant::now();
    let mut pixels = image::open(&job.path)
        .with_context(|| {
            format!(
                "failed to decode frozen display {}",
                job.display.display_index
            )
        })?
        .to_rgba8();
    let decode_ms = decode_started.elapsed().as_millis();
    let (pixel_width, pixel_height) = pixels.dimensions();
    rgba_to_bgra(pixels.as_mut());
    let bounds = rect_from_monitor(job.display.bounds);
    qol_runtime::probe!(
        "SHOT_FREEZE",
        "stage=display backend=screencapture index={} logical={}x{} pixels={}x{} capture_ms={} decode_ms={} total_ms={}",
        job.display.display_index,
        bounds.w,
        bounds.h,
        pixel_width,
        pixel_height,
        capture_ms,
        decode_ms,
        started.elapsed().as_millis()
    );
    Ok((bounds, pixels.into_raw(), pixel_width, pixel_height))
}

fn run_frozen_screencapture(job: &FrozenCaptureJob) -> Result<std::process::Output> {
    ensure_capture_work_dir()?;
    Command::new("screencapture")
        .args(screencapture_frozen_args(job.display.display_index))
        .arg(&job.path)
        .stdin(Stdio::null())
        .output()
        .context("failed to run screencapture frozen-frame capture")
}

fn ensure_frozen_capture_succeeded(
    job: &FrozenCaptureJob,
    output: &std::process::Output,
) -> Result<()> {
    if output.status.success() {
        return Ok(());
    }
    Err(anyhow!(
        "screencapture display {} exited with {}: {}",
        job.display.display_index,
        output.status,
        String::from_utf8_lossy(&output.stderr).trim()
    ))
}

#[derive(Default)]
struct FrozenCaptureFiles(Vec<PathBuf>);

impl FrozenCaptureFiles {
    fn path(&mut self, sequence: u64, display_index: u32) -> PathBuf {
        let path = capture_work_dir().join(format!(
            "qol-shot-freeze-{}-{sequence}-{display_index}.png",
            std::process::id()
        ));
        self.0.push(path.clone());
        path
    }
}

impl Drop for FrozenCaptureFiles {
    fn drop(&mut self) {
        for path in &self.0 {
            let _ = fs::remove_file(path);
        }
    }
}

pub(super) fn screencapture_frozen_args(display_index: u32) -> Vec<String> {
    vec![
        "-D".into(),
        display_index.to_string(),
        "-x".into(),
        "-t".into(),
        "png".into(),
    ]
}

fn rect_from_monitor(monitor: Monitor) -> Rect {
    Rect {
        x: monitor.x,
        y: monitor.y,
        w: monitor.w,
        h: monitor.h,
    }
}

fn rgba_to_bgra(pixels: &mut [u8]) {
    for pixel in pixels.chunks_exact_mut(4) {
        pixel.swap(0, 2);
    }
}

pub fn configure_preview_window(_title: String) {}

#[derive(Clone)]
pub struct PinResizeSession;

pub fn pin_resize_session(_title: &str) -> Option<PinResizeSession> {
    None
}

impl PinResizeSession {
    pub fn apply(&self, _x: f32, _y: f32, _width: f32, _height: f32) {}

    pub fn move_to(&self, _x: f32, _y: f32) {}

    pub fn pointer(&self) -> Option<(f32, f32)> {
        None
    }

    pub fn bounds(&self) -> Option<(f32, f32, f32, f32)> {
        None
    }

    pub fn anchor(&self, _right: bool, _bottom: bool) {}
}

pub fn pin_focus(_title: &str) {}

pub fn pin_release_focus(_title: &str) {}

pub fn prepare_pin_window(_title: &str, _origin: (f64, f64)) -> bool {
    false
}

pub fn configure_pin_window(title: String, _origin: (f64, f64), source_preview: Option<String>) {
    qol_gpui::popup_window::configure_pinned_window(&title);
    if let Some(source_preview) = source_preview {
        qol_gpui::popup_window::hide_invisible(&source_preview);
        qol_gpui::popup_window::restore_composite(&source_preview);
    }
}

pub fn platform_supported_check() -> DoctorCheckResult {
    DoctorCheckResult::ok(
        "platform_supported",
        "macOS capture is supported through screencapture.",
    )
}

pub fn required_binaries_check() -> DoctorCheckResult {
    let required = ["screencapture", "open", "osascript"];
    let missing = required
        .iter()
        .copied()
        .filter(|name| resolve_command(name).is_none())
        .collect::<Vec<_>>();

    if !missing.is_empty() {
        return DoctorCheckResult::fail(
            "required_binaries",
            format!("Missing required binaries: {}.", missing.join(", ")),
        )
        .with_fix("Restore the missing macOS command line tools.");
    }

    if resolve_command("ffmpeg").is_none() && resolve_command("avconvert").is_none() {
        return DoctorCheckResult::warn(
            "required_binaries",
            "Native MOV recording is available, including multi-display composition, but non-MOV conversion is limited without ffmpeg or avconvert.",
        )
        .with_fix("Install ffmpeg for MP4, MKV, or WebM output, or avconvert for MP4 output.");
    }

    if resolve_command("ffmpeg").is_none() {
        return DoctorCheckResult::warn(
            "required_binaries",
            "Native MOV recording and MP4 conversion through avconvert are available, including multi-display composition, but WebM/MKV require ffmpeg.",
        )
        .with_fix("Install ffmpeg for WebM or MKV output.");
    }

    DoctorCheckResult::ok(
        "required_binaries",
        "Required macOS capture, composition, and conversion tools are available.",
    )
}

pub fn permissions_check() -> DoctorCheckResult {
    let trusted = unsafe { CGPreflightScreenCaptureAccess() };
    let details = serde_json::json!({
        "platform": "macos",
        "screen_recording_trusted": trusted,
        "prompted": false,
        "capture_attempted": false,
    });
    if trusted {
        return DoctorCheckResult::ok(
            "permissions",
            "macOS Screen Recording permission is granted.",
        )
        .with_details(details);
    }
    DoctorCheckResult::fail(
        "permissions",
        "macOS Screen Recording permission is not granted.",
    )
    .with_fix(
        "Enable QoL Shot in System Settings > Privacy & Security > Screen & System Audio Recording.",
    )
    .with_details(details)
}

pub fn external_services_check() -> DoctorCheckResult {
    match active_displays() {
        Ok(displays) => DoctorCheckResult::ok(
            "external_services",
            format!(
                "The macOS WindowServer display service reports {} active display(s).",
                displays.len()
            ),
        )
        .with_details(serde_json::json!({
            "platform": "macos",
            "service": "window_server",
            "display_count": displays.len(),
            "capture_attempted": false,
        })),
        Err(error) => DoctorCheckResult::fail(
            "external_services",
            format!("The macOS WindowServer display service is unavailable: {error}"),
        )
        .with_fix("Run QoL Shot in an active macOS graphical login session.")
        .with_details(serde_json::json!({
            "platform": "macos",
            "service": "window_server",
            "display_count": null,
            "capture_attempted": false,
        })),
    }
}

pub(super) fn signal_process(pid: u32, signal: i32) -> Result<()> {
    super::super::unix::signal_process(pid, signal)
}

pub(super) fn wait_for_process_exit(pid: u32, timeout: Duration) -> bool {
    let deadline = Instant::now() + timeout;
    while Instant::now() < deadline {
        if !process_alive(pid) {
            return true;
        }
        thread::sleep(Duration::from_millis(150));
    }
    !process_alive(pid)
}

pub(super) fn open_path(path: &Path) -> bool {
    qol_apps::desktop_integration::open_with_default_app(path).is_ok()
}

pub(super) fn videos_dir() -> Option<PathBuf> {
    let home = env::var_os("HOME")?;
    Some(PathBuf::from(home).join("Videos"))
}

pub(super) fn capture_work_dir() -> PathBuf {
    env::temp_dir().join("qol-shot")
}

pub(super) fn ensure_capture_work_dir() -> Result<PathBuf> {
    let dir = capture_work_dir();
    fs::create_dir_all(&dir).context("failed to create capture work directory")?;
    Ok(dir)
}

pub(super) fn move_file(source: &Path, destination: &Path) -> Result<()> {
    if fs::rename(source, destination).is_ok() {
        return Ok(());
    }

    fs::copy(source, destination).with_context(|| {
        format!(
            "failed to move {} to {}",
            source.display(),
            destination.display()
        )
    })?;
    let _ = fs::remove_file(source);
    Ok(())
}

pub(super) fn paths_match(left: &Path, right: &Path) -> bool {
    left == right
}

pub(super) fn path_extension_is(path: &Path, expected: &str) -> bool {
    output_extension(path).as_deref() == Some(expected)
}

pub(super) fn output_extension(path: &Path) -> Option<String> {
    path.extension()?
        .to_str()
        .map(|ext| ext.to_ascii_lowercase())
}

pub(super) fn output_format_label(path: &Path) -> String {
    output_extension(path)
        .map(|ext| ext.to_ascii_uppercase())
        .unwrap_or_else(|| "recording".to_string())
}

fn escape_applescript(input: &str) -> String {
    input.replace('\\', "\\\\").replace('"', "\\\"")
}

pub(super) fn resolve_command(command: &str) -> Option<PathBuf> {
    let mut dirs = env::var_os("PATH")
        .map(|paths| env::split_paths(&paths).collect::<Vec<_>>())
        .unwrap_or_default();
    dirs.extend([
        PathBuf::from("/usr/bin"),
        PathBuf::from("/usr/sbin"),
        PathBuf::from("/bin"),
        PathBuf::from("/opt/homebrew/bin"),
        PathBuf::from("/usr/local/bin"),
    ]);
    dirs.into_iter()
        .map(|dir| dir.join(command))
        .find(|path| path.is_file())
}

pub fn list_audio_sources() -> Vec<crate::platform::AudioDevice> {
    Vec::new()
}

pub fn list_audio_sinks() -> Vec<crate::platform::AudioDevice> {
    Vec::new()
}
