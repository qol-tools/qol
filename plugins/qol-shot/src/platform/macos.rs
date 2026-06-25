use anyhow::{anyhow, Context, Result};
use gpui::{Bounds, Pixels};
use qol_gpui::monitor::{ActiveMonitor, MonitorTracker};
use qol_gpui::platform::{ghost_window_decorations, ghost_window_kind};
use qol_gpui::window::centered_window_placement;
use qol_headless::DoctorCheckResult;
use std::env;
use std::ffi::c_void;
use std::fs::{self, File, OpenOptions};
use std::io::Write;
use std::os::unix::fs::{MetadataExt, PermissionsExt};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::{mpsc, Mutex};
use std::thread;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use crate::platform::{CaptureProcess, CaptureSegment, CaptureSession};
use crate::{Config, Monitor, Rect};

const MAX_DISPLAYS: u32 = 16;
const SWIFT_HELPER_CACHE_DIR: &str = "qol-shot-swift";
const STATUS_OVERLAY_PID_FILE_NAME: &str = "qol-shot-status-overlay.pid";
const STATUS_OVERLAY_MAX_LIFETIME_MS: u32 = 120_000;
const RECORDING_OVERLAY_PID_FILE_NAME: &str = "qol-shot-recording-overlay.pid";
const RECORDING_OVERLAY_MAX_LIFETIME_MS: u32 = 43_200_000;
const FNV_OFFSET_BASIS: u64 = 0xcbf29ce484222325;
const FNV_PRIME: u64 = 0x100000001b3;
static STATUS_OVERLAY_PID: Mutex<Option<u32>> = Mutex::new(None);
static RECORDING_OVERLAY_PID: Mutex<Option<u32>> = Mutex::new(None);
const SWIFT_PRELUDE: &str = include_str!("macos_swift/prelude.swift");
const STATUS_OVERLAY_SWIFT: &str = include_str!("macos_swift/status_overlay.swift");
const RECORDING_OVERLAY_SWIFT: &str = include_str!("macos_swift/recording_overlay.swift");
const CLIPBOARD_WRITER_SWIFT: &str = include_str!("macos_swift/clipboard_writer.swift");
const VIDEO_COMPOSER_SWIFT: &str = include_str!("macos_swift/video_composer.swift");
const STATUS_OVERLAY_HELPER: &[u8] = include_bytes!(concat!(env!("OUT_DIR"), "/status-overlay"));
const RECORDING_OVERLAY_HELPER: &[u8] =
    include_bytes!(concat!(env!("OUT_DIR"), "/recording-overlay"));
const CLIPBOARD_WRITER_HELPER: &[u8] =
    include_bytes!(concat!(env!("OUT_DIR"), "/clipboard-writer"));
const VIDEO_COMPOSER_HELPER: &[u8] = include_bytes!(concat!(env!("OUT_DIR"), "/video-composer"));

type CGDirectDisplayID = u32;
type CGEventRef = *const c_void;
const CG_EVENT_SOURCE_STATE_HID_SYSTEM: i32 = 1;
const CG_MOUSE_BUTTON_LEFT: i32 = 0;

#[repr(C)]
#[derive(Clone, Copy)]
struct CGPoint {
    x: f64,
    y: f64,
}

#[repr(C)]
#[derive(Clone, Copy)]
struct CGSize {
    width: f64,
    height: f64,
}

#[repr(C)]
#[derive(Clone, Copy)]
struct CGRect {
    origin: CGPoint,
    size: CGSize,
}

#[link(name = "CoreGraphics", kind = "framework")]
extern "C" {
    fn CGGetActiveDisplayList(
        max_displays: u32,
        active_displays: *mut CGDirectDisplayID,
        display_count: *mut u32,
    ) -> i32;
    fn CGDisplayBounds(display: CGDirectDisplayID) -> CGRect;
    fn CGEventCreate(source: *const c_void) -> CGEventRef;
    fn CGEventGetLocation(event: CGEventRef) -> CGPoint;
    fn CGEventSourceButtonState(state_id: i32, button: i32) -> bool;
}

#[link(name = "CoreFoundation", kind = "framework")]
extern "C" {
    fn CFRelease(cf: *const c_void);
}

extern "C" {
    fn getuid() -> u32;
}

extern "C" {
    fn CGMainDisplayID() -> CGDirectDisplayID;
}

#[derive(Debug, Clone, Copy)]
struct DisplayInfo {
    display_index: u32,
    bounds: Monitor,
}

#[derive(Debug, Clone)]
struct DisplayCaptureSegment {
    display_index: u32,
    rect: Rect,
    offset_x: i32,
    offset_y: i32,
}

pub fn select_region() -> Result<Option<Rect>> {
    crate::region_selector::select_region_blocking_with(|tx, cx| {
        let tracker = MonitorTracker::start(cx);
        let monitor = tracker.snapshot_monitor();
        let monitors = selector_monitors(&tracker);
        qol_runtime::probe!(
            "SHOT_SELECT_PLATFORM",
            "mode=blocking monitor={} monitors={}",
            monitor.is_some(),
            monitors.len()
        );
        open_region_selector_with_sender(tx, true, monitor, monitors, cx);
    })
}

pub fn select_region_in_app(
    cx: &mut gpui::App,
    monitor: Option<ActiveMonitor>,
    monitors: Vec<ActiveMonitor>,
) -> Option<mpsc::Receiver<Option<Rect>>> {
    qol_runtime::probe!(
        "SHOT_SELECT_PLATFORM",
        "mode=in-app monitor={} monitors={}",
        monitor.is_some(),
        monitors.len()
    );
    Some(open_region_selector(cx, monitor, monitors))
}

fn open_region_selector(
    cx: &mut gpui::App,
    monitor: Option<ActiveMonitor>,
    monitors: Vec<ActiveMonitor>,
) -> mpsc::Receiver<Option<Rect>> {
    let (tx, rx) = mpsc::channel();
    open_region_selector_with_sender(tx, false, monitor, monitors, cx);
    rx
}

fn open_region_selector_with_sender(
    tx: mpsc::Sender<Option<Rect>>,
    quit_on_finish: bool,
    monitor: Option<ActiveMonitor>,
    monitors: Vec<ActiveMonitor>,
    cx: &mut gpui::App,
) {
    let selectors = selector_windows(monitor.as_ref(), monitors, cx);
    let titles = selectors
        .iter()
        .map(|selector| selector.title().to_string())
        .collect::<Vec<_>>();
    qol_runtime::probe!(
        "SHOT_SELECT_PLATFORM",
        "mode=open selectors={} quit_on_finish={quit_on_finish}",
        titles.len()
    );
    if crate::region_selector::open_all(tx, quit_on_finish, selectors, cx) {
        for title in titles {
            configure_selector_window(title, cx);
        }
        cx.activate(true);
    }
}

fn selector_windows(
    monitor: Option<&ActiveMonitor>,
    monitors: Vec<ActiveMonitor>,
    cx: &gpui::App,
) -> Vec<crate::region_selector::SelectorWindow> {
    let active_bounds = monitor.map(|monitor| monitor.bounds());
    let map_rect = selector_rect_mapper();
    let global_pointer: Option<crate::region_selector::GlobalPointerSource> =
        Some(std::rc::Rc::new(MacPointerSource));
    let monitors = if monitors.is_empty() {
        monitor.cloned().into_iter().collect()
    } else {
        monitors
    };
    if monitors.is_empty() {
        return vec![selector_window(
            selector_fallback_bounds(),
            active_bounds,
            None,
            true,
            map_rect,
            global_pointer,
        )];
    }
    monitors
        .into_iter()
        .map(|monitor| {
            let bounds = monitor.bounds();
            let placement = centered_window_placement(Some(&monitor), bounds.size, cx);
            selector_window(
                bounds,
                active_bounds,
                placement.display_id,
                selector_focus(bounds, active_bounds),
                map_rect.clone(),
                global_pointer.clone(),
            )
        })
        .collect()
}

fn selector_window(
    bounds: Bounds<Pixels>,
    active_bounds: Option<Bounds<Pixels>>,
    display_id: Option<gpui::DisplayId>,
    focus: bool,
    map_rect: crate::region_selector::RectMapper,
    global_pointer: Option<crate::region_selector::GlobalPointerSource>,
) -> crate::region_selector::SelectorWindow {
    crate::region_selector::SelectorWindow::new(
        bounds,
        active_bounds,
        crate::region_selector::SelectorWindowOptions {
            display_id,
            kind: ghost_window_kind(),
            decorations: ghost_window_decorations(false),
            focus,
        },
        map_rect,
        global_pointer,
    )
}

fn selector_focus(bounds: Bounds<Pixels>, active_bounds: Option<Bounds<Pixels>>) -> bool {
    match active_bounds {
        Some(active_bounds) => bounds == active_bounds,
        None => true,
    }
}

struct MacPointerSource;

impl crate::region_selector::GlobalPointer for MacPointerSource {
    fn position(&self) -> Option<gpui::Point<Pixels>> {
        let event = CfGuard::new(unsafe { CGEventCreate(std::ptr::null()) })?;
        let location = unsafe { CGEventGetLocation(event.as_ptr()) };
        Some(gpui::point(
            gpui::px(location.x as f32),
            gpui::px(location.y as f32),
        ))
    }

    fn primary_button_down(&self) -> bool {
        unsafe { CGEventSourceButtonState(CG_EVENT_SOURCE_STATE_HID_SYSTEM, CG_MOUSE_BUTTON_LEFT) }
    }
}

struct CfGuard(*const c_void);

impl CfGuard {
    fn new(ptr: *const c_void) -> Option<Self> {
        if ptr.is_null() {
            return None;
        }
        Some(Self(ptr))
    }

    fn as_ptr(&self) -> *const c_void {
        self.0
    }
}

impl Drop for CfGuard {
    fn drop(&mut self) {
        unsafe { CFRelease(self.0) }
    }
}

fn selector_monitors(tracker: &MonitorTracker) -> Vec<ActiveMonitor> {
    let monitors = tracker.all_monitors();
    if !monitors.is_empty() {
        return monitors;
    }
    tracker.snapshot_monitor().into_iter().collect()
}

fn selector_fallback_bounds() -> Bounds<Pixels> {
    let displays = active_display_bounds()
        .ok()
        .filter(|displays| !displays.is_empty())
        .and_then(|displays| displays.into_iter().next());
    displays
        .map(crate::region_selector::bounds_from_monitor)
        .unwrap_or_else(crate::region_selector::fallback_bounds)
}

fn selector_rect_mapper() -> crate::region_selector::RectMapper {
    let displays = active_display_bounds().unwrap_or_default();
    std::rc::Rc::new(move |rect| map_selector_rect_to_capture(rect, &displays))
}

fn map_selector_rect_to_capture(rect: Rect, displays: &[Monitor]) -> Option<Rect> {
    if displays.is_empty() {
        return Some(rect);
    }

    displays
        .iter()
        .filter_map(|display| rect_intersection(rect, *display))
        .reduce(union_rects)
}

fn union_rects(left: Rect, right: Rect) -> Rect {
    let x = left.x.min(right.x);
    let y = left.y.min(right.y);
    let right_edge = (left.x + left.w).max(right.x + right.w);
    let bottom_edge = (left.y + left.h).max(right.y + right.h);
    Rect {
        x,
        y,
        w: right_edge - x,
        h: bottom_edge - y,
    }
}

fn configure_selector_window(title: String, cx: &mut gpui::App) {
    cx.defer(move |_| configure_selector_window_now(&title));
}

fn configure_selector_window_now(title: &str) {
    let started = Instant::now();
    if qol_gpui::popup_window::configure_overlay_window(title) {
        qol_runtime::probe!(
            "SHOT_SELECT_OVERLAY",
            "ms={} result=mapped",
            started.elapsed().as_millis()
        );
        return;
    }
    qol_runtime::probe!(
        "SHOT_SELECT_OVERLAY",
        "ms={} result=missing",
        started.elapsed().as_millis()
    );
}

pub fn get_monitors() -> Result<Vec<Monitor>> {
    active_display_bounds()
}

pub fn full_screen_bounds() -> Result<Monitor> {
    let monitors = active_display_bounds()?;
    union_bounds(&monitors)
}

pub fn start_capture(rect: &Rect, config: &Config, output_file: &Path) -> Result<CaptureSession> {
    let capture_file = native_capture_file_path(output_file);
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

#[cfg(test)]
#[derive(Debug, Clone, Copy, Eq, PartialEq)]
enum CapturePlan {
    SingleDisplay,
    SegmentedFfmpeg,
    SegmentedNative,
}

#[cfg(test)]
fn capture_plan(segment_count: usize, ffmpeg_available: bool) -> CapturePlan {
    if segment_count <= 1 {
        return CapturePlan::SingleDisplay;
    }
    if ffmpeg_available {
        return CapturePlan::SegmentedFfmpeg;
    }
    CapturePlan::SegmentedNative
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

pub fn copy_image_to_clipboard(path: &Path) -> Result<()> {
    let helper = ensure_swift_helper(
        "clipboard-writer",
        CLIPBOARD_WRITER_SWIFT,
        CLIPBOARD_WRITER_HELPER,
    )
    .context("failed to install embedded clipboard writer helper")?;

    let output = Command::new(&helper)
        .arg(path)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .output()
        .inspect_err(|_| {
            let _ = fs::remove_file(&helper);
        })
        .context("failed to start compiled macOS clipboard writer")?;

    if output.status.success() {
        return Ok(());
    }

    let stderr = String::from_utf8_lossy(&output.stderr);
    Err(anyhow!(
        "macOS clipboard writer exited with {}: {}",
        output.status,
        stderr.trim()
    ))
}

pub fn copy_path_to_clipboard(path: &Path) -> Result<()> {
    let mut child = Command::new("pbcopy")
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .context("failed to run pbcopy")?;

    child
        .stdin
        .take()
        .ok_or_else(|| anyhow!("failed to open pbcopy stdin"))?
        .write_all(path.to_string_lossy().as_bytes())
        .context("failed to write to pbcopy")?;

    let status = child.wait().context("failed to wait for pbcopy")?;
    if status.success() {
        return Ok(());
    }

    Err(anyhow!("pbcopy exited with {status}"))
}

pub fn recording_format(format: &str) -> String {
    match format.to_ascii_lowercase().as_str() {
        "mkv" | "mp4" | "mov" | "webm" => format.to_ascii_lowercase(),
        _ => "mov".to_string(),
    }
}

fn native_capture_file_path(output_file: &Path) -> PathBuf {
    if path_extension_is(output_file, "mov") {
        return output_file.to_path_buf();
    }
    output_file.with_extension("mov")
}

pub fn recording_started(session: &CaptureSession) {
    qol_runtime::probe!("SHOT_RECORD_NOTIFY", "stage=started");
    show_recording_region_overlay(session);
    show_notification("Recording started", "Press your hotkey to stop", 1200);
    prewarm_recording_helpers();
}

pub fn recording_stopped(session: &CaptureSession, config: &Config) {
    dismiss_recording_region_overlay();
    qol_runtime::probe!(
        "SHOT_RECORD_FINALIZE",
        "stage=stopped-entry pids={} segments={}",
        session.pid_list(),
        session.segments.len()
    );
    if let Some(output_file) = session.output_file.as_deref() {
        let capture_file = session.capture_file.as_deref().unwrap_or(output_file);
        let conversion_needed = !paths_match(output_file, capture_file);
        qol_runtime::probe!(
            "SHOT_RECORD_FINALIZE",
            "stage=begin output={} capture={} conversion_needed={} segments={}",
            path_label(output_file),
            path_label(capture_file),
            conversion_needed,
            session.segments.len()
        );
        show_recording_ended(output_file, conversion_needed);
        let reveal_file = match finalize_recording(session, output_file, capture_file, config) {
            Ok(reveal_file) => {
                qol_runtime::probe!(
                    "SHOT_RECORD_FINALIZE",
                    "stage=ok reveal={} converted={}",
                    path_label(&reveal_file),
                    conversion_needed
                );
                show_recording_saved(&reveal_file, conversion_needed);
                reveal_file
            }
            Err(error) => {
                qol_runtime::probe!("SHOT_RECORD_FINALIZE", "stage=error result=fallback");
                finalization_fallback(error, session, capture_file)
            }
        };
        let revealed = reveal_recording(&reveal_file);
        qol_runtime::probe!(
            "SHOT_RECORD_FINALIZE",
            "stage=reveal file={} result={revealed}",
            path_label(&reveal_file)
        );
        if revealed {
            return;
        }
    }

    qol_runtime::probe!("SHOT_RECORD_FINALIZE", "stage=open-videos");
    show_status_overlay(
        "Recording stopped",
        "Opening Videos...",
        1800,
        StatusOverlayLifecycle::ExitAfterHide,
    );
    if let Some(videos_dir) = videos_dir() {
        open_path(&videos_dir);
    }
}

pub fn stop_capture(session: &CaptureSession) -> Result<()> {
    dismiss_recording_region_overlay();
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

pub fn process_alive(pid: u32) -> bool {
    super::unix_process_alive(pid)
}

pub fn show_notification(title: &str, message: &str, _timeout_ms: u32) {
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

pub fn open_url(url: &str) -> Result<()> {
    Command::new("open")
        .arg(url)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .context("failed to open URL")?;
    Ok(())
}

pub fn grab_preview_rgba(_rect: &Rect) -> Option<(Vec<u8>, u32, u32)> {
    None
}

pub fn configure_preview_window(_title: String) {}

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

fn active_display_bounds() -> Result<Vec<Monitor>> {
    Ok(active_displays()?
        .into_iter()
        .map(|display| display.bounds)
        .collect())
}

fn active_displays() -> Result<Vec<DisplayInfo>> {
    let mut displays = [0u32; MAX_DISPLAYS as usize];
    let mut count = 0u32;
    let result = unsafe { CGGetActiveDisplayList(MAX_DISPLAYS, displays.as_mut_ptr(), &mut count) };

    if result != 0 {
        return Err(anyhow!("CGGetActiveDisplayList failed: {}", result));
    }

    if count == 0 {
        return Err(anyhow!("no active displays found"));
    }

    let display_ids = screencapture_display_order(&displays[..count as usize]);
    Ok(display_ids
        .into_iter()
        .enumerate()
        .map(|(index, display_id)| {
            let bounds = unsafe { CGDisplayBounds(display_id) };
            DisplayInfo {
                display_index: index as u32 + 1,
                bounds: monitor_from_cg_bounds(bounds),
            }
        })
        .collect())
}

fn screencapture_display_order(display_ids: &[CGDirectDisplayID]) -> Vec<CGDirectDisplayID> {
    let main = unsafe { CGMainDisplayID() };
    let mut ordered = Vec::with_capacity(display_ids.len());

    if display_ids.contains(&main) {
        ordered.push(main);
    }

    ordered.extend(
        display_ids
            .iter()
            .copied()
            .filter(|display| *display != main),
    );
    ordered
}

fn union_bounds(monitors: &[Monitor]) -> Result<Monitor> {
    let first = monitors
        .first()
        .copied()
        .ok_or_else(|| anyhow!("no active displays found"))?;

    let mut left = first.x;
    let mut top = first.y;
    let mut right = first.x + first.w;
    let mut bottom = first.y + first.h;

    for monitor in monitors.iter().skip(1) {
        left = left.min(monitor.x);
        top = top.min(monitor.y);
        right = right.max(monitor.x + monitor.w);
        bottom = bottom.max(monitor.y + monitor.h);
    }

    Ok(Monitor {
        x: left,
        y: top,
        w: right - left,
        h: bottom - top,
    })
}

fn monitor_from_cg_bounds(bounds: CGRect) -> Monitor {
    Monitor {
        x: round_i32(bounds.origin.x),
        y: round_i32(bounds.origin.y),
        w: round_i32(bounds.size.width),
        h: round_i32(bounds.size.height),
    }
}

fn round_i32(value: f64) -> i32 {
    value.round() as i32
}

fn rect_label(rect: Rect) -> String {
    format!("{}x{}+{},{}", rect.w, rect.h, rect.x, rect.y)
}

fn monitor_label(monitor: Monitor) -> String {
    format!("{}x{}+{},{}", monitor.w, monitor.h, monitor.x, monitor.y)
}

fn path_label(path: &Path) -> String {
    path.file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("none")
        .to_string()
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

fn rect_intersection(left: Rect, right: Monitor) -> Option<Rect> {
    let x = left.x.max(right.x);
    let y = left.y.max(right.y);
    let right_edge = (left.x + left.w).min(right.x + right.w);
    let bottom_edge = (left.y + left.h).min(right.y + right.h);
    let w = right_edge - x;
    let h = bottom_edge - y;

    if w <= 0 || h <= 0 {
        return None;
    }

    Some(Rect { x, y, w, h })
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

fn screencapture_recording_args(
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
        CaptureLogMode::Truncate => File::create(super::CAPTURE_LOG),
        CaptureLogMode::Append => OpenOptions::new()
            .create(true)
            .append(true)
            .open(super::CAPTURE_LOG),
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
    if paths_match(output_file, capture_file) {
        qol_runtime::probe!("SHOT_RECORD_FINALIZE", "stage=no-conversion");
        return Ok(output_file.to_path_buf());
    }

    qol_runtime::probe!(
        "SHOT_RECORD_FINALIZE",
        "stage=convert capture={} output={}",
        path_label(capture_file),
        path_label(output_file)
    );
    convert_recording(capture_file, output_file, config)?;
    let _ = std::fs::remove_file(capture_file);
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

fn native_segment_composition_args(
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
            segment.file.to_string_lossy().to_string(),
        ]);
    }

    args
}

#[cfg(test)]
fn segment_composition_filter(session: &CaptureSession, canvas: Rect) -> String {
    let mut filters = Vec::new();
    let mut inputs = String::new();

    for index in 0..session.segments.len() {
        filters.push(format!("[{index}:v]setpts=PTS-STARTPTS[v{index}]"));
        inputs.push_str(&format!("[v{index}]"));
    }

    let layout = session
        .segments
        .iter()
        .map(|segment| format!("{}_{}", segment.offset_x, segment.offset_y))
        .collect::<Vec<_>>()
        .join("|");
    filters.push(format!(
        "{inputs}xstack=inputs={}:layout={layout}:fill=black:shortest=1[stacked]",
        session.segments.len()
    ));
    filters.push(format!(
        "[stacked]pad={}:{}:0:0:color=black,crop={}:{}:0:0[v]",
        canvas.w, canvas.h, canvas.w, canvas.h
    ));

    filters.join(";")
}

fn finalization_fallback(
    error: anyhow::Error,
    session: &CaptureSession,
    capture_file: &Path,
) -> PathBuf {
    qol_runtime::probe!(
        "SHOT_RECORD_FINALIZE",
        "stage=fallback capture={} segments={}",
        path_label(capture_file),
        session.segments.len()
    );
    let fallback_file = fallback_recording_file(session, capture_file);
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
    fallback_file
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
            "Saving recording...",
            1800,
            StatusOverlayLifecycle::KeepAlive,
        );
        show_notification("Recording stopped", "Saving recording...", 1600);
        return;
    }

    let message = format!("Converting to {}...", output_format_label(output_file));
    show_status_overlay(
        "Recording stopped",
        &message,
        2400,
        StatusOverlayLifecycle::KeepAlive,
    );
    show_notification("Recording stopped", &message, 2200);
}

fn show_recording_saved(output_file: &Path, converted: bool) {
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
    show_notification("Recording saved", &message, 2200);
}

#[derive(Clone, Copy)]
enum StatusOverlayLifecycle {
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

fn show_status_overlay(
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

fn show_recording_region_overlay(session: &CaptureSession) {
    let Some(rect) = session.canvas else {
        qol_runtime::probe!("SHOT_RECORD_OVERLAY", "event=show result=no-canvas");
        return;
    };

    qol_runtime::probe!(
        "SHOT_RECORD_OVERLAY",
        "event=show rect={}",
        rect_label(rect)
    );
    dismiss_recording_region_overlay();
    let Ok(child) = spawn_recording_region_overlay(rect) else {
        qol_runtime::probe!("SHOT_RECORD_OVERLAY", "event=spawn result=error");
        return;
    };

    let pid = child.id();
    qol_runtime::probe!("SHOT_RECORD_OVERLAY", "event=spawn result=ok pid={pid}");
    write_recording_overlay_pid(pid);

    thread::spawn(move || {
        let mut child = child;
        let _ = child.wait();
        clear_recording_overlay_pid(pid);
    });
}

fn spawn_recording_region_overlay(rect: Rect) -> Result<Child> {
    match ensure_swift_helper(
        "recording-overlay",
        RECORDING_OVERLAY_SWIFT,
        RECORDING_OVERLAY_HELPER,
    ) {
        Ok(helper) => {
            qol_runtime::probe!("SHOT_RECORD_OVERLAY", "event=helper result=compiled");
            match spawn_compiled_recording_region_overlay(&helper, rect) {
                Ok(child) => Ok(child),
                Err(error) => {
                    qol_runtime::probe!(
                        "SHOT_RECORD_OVERLAY",
                        "event=compiled-spawn result=error fallback=source"
                    );
                    let _ = fs::remove_file(&helper);
                    spawn_source_recording_region_overlay(rect).with_context(|| {
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
            spawn_source_recording_region_overlay(rect)
                .with_context(|| format!("failed to build recording overlay helper: {error:#}"))
        }
    }
}

fn spawn_compiled_recording_region_overlay(helper: &Path, rect: Rect) -> Result<Child> {
    let mut command = Command::new(helper);
    configure_recording_region_overlay(&mut command, rect);
    command
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .context("failed to start compiled macOS recording overlay")
}

fn spawn_source_recording_region_overlay(rect: Rect) -> Result<Child> {
    spawn_source_swift(RECORDING_OVERLAY_SWIFT, |command| {
        configure_recording_region_overlay(command, rect);
        command.stdout(Stdio::null()).stderr(Stdio::null());
    })
    .context("failed to start source macOS recording overlay")
}

fn configure_recording_region_overlay(command: &mut Command, rect: Rect) {
    command
        .env("QOL_RECORDING_RECT_X", rect.x.to_string())
        .env("QOL_RECORDING_RECT_Y", rect.y.to_string())
        .env("QOL_RECORDING_RECT_WIDTH", rect.w.to_string())
        .env("QOL_RECORDING_RECT_HEIGHT", rect.h.to_string())
        .env(
            "QOL_RECORDING_OVERLAY_MAX_LIFETIME_MS",
            RECORDING_OVERLAY_MAX_LIFETIME_MS.to_string(),
        );
}

fn dismiss_recording_region_overlay() {
    let pids = take_recording_overlay_pids();
    qol_runtime::probe!("SHOT_RECORD_OVERLAY", "event=dismiss count={}", pids.len());
    for entry in pids {
        stop_recording_overlay_pid(entry);
    }
}

fn write_recording_overlay_pid(pid: u32) {
    if let Ok(mut current_pid) = RECORDING_OVERLAY_PID.lock() {
        *current_pid = Some(pid);
    }
    let _ = fs::write(recording_overlay_pid_file_path(), format!("{pid}\n"));
}

fn clear_recording_overlay_pid(pid: u32) {
    if let Ok(mut current_pid) = RECORDING_OVERLAY_PID.lock() {
        if *current_pid == Some(pid) {
            *current_pid = None;
        }
    }
    if read_recording_overlay_pid_file() == Some(pid) {
        let _ = fs::remove_file(recording_overlay_pid_file_path());
    }
}

fn take_recording_overlay_pids() -> Vec<RecordingOverlayPid> {
    let mut pids = Vec::new();
    if let Ok(mut current_pid) = RECORDING_OVERLAY_PID.lock() {
        if let Some(pid) = current_pid.take() {
            pids.push(RecordingOverlayPid { pid, trusted: true });
        }
    }

    if let Some(pid) = read_recording_overlay_pid_file() {
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

fn read_recording_overlay_pid_file() -> Option<u32> {
    let content = fs::read_to_string(recording_overlay_pid_file_path()).ok()?;
    content.lines().next()?.trim().parse().ok()
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

fn prewarm_recording_helpers() {
    prewarm_swift_helper(
        "status-overlay",
        STATUS_OVERLAY_SWIFT,
        STATUS_OVERLAY_HELPER,
    );
    prewarm_swift_helper(
        "recording-overlay",
        RECORDING_OVERLAY_SWIFT,
        RECORDING_OVERLAY_HELPER,
    );
    prewarm_swift_helper(
        "video-composer",
        VIDEO_COMPOSER_SWIFT,
        VIDEO_COMPOSER_HELPER,
    );
}

fn prewarm_swift_helper(name: &'static str, body: &'static str, embedded_helper: &'static [u8]) {
    thread::spawn(move || {
        let _ = ensure_swift_helper(name, body, embedded_helper);
    });
}

fn spawn_source_swift(body: &str, configure: impl FnOnce(&mut Command)) -> Result<Child> {
    let mut command = Command::new("swift");
    command.arg("-").stdin(Stdio::piped());
    configure(&mut command);
    let mut child = command.spawn().context("failed to start Swift source")?;

    let Some(stdin) = child.stdin.take() else {
        return Err(anyhow!("failed to open Swift source stdin"));
    };

    write_swift_source(stdin, body).context("failed to write Swift source")?;
    Ok(child)
}

fn write_swift_source(mut writer: impl Write, body: &str) -> std::io::Result<()> {
    writer.write_all(SWIFT_PRELUDE.as_bytes())?;
    writer.write_all(body.as_bytes())
}

fn ensure_swift_helper(name: &str, body: &str, embedded_helper: &[u8]) -> Result<PathBuf> {
    let helper = swift_helper_path(name, body);
    if is_usable_swift_helper(&helper) {
        return Ok(helper);
    }

    let _ = fs::remove_file(&helper);
    install_embedded_swift_helper(name, embedded_helper, &helper)?;
    Ok(helper)
}

fn is_usable_swift_helper(path: &Path) -> bool {
    let Ok(metadata) = path.symlink_metadata() else {
        return false;
    };
    if !metadata.file_type().is_file() {
        return false;
    }
    metadata.permissions().mode() & 0o111 != 0 && metadata.uid() == current_uid()
}

fn install_embedded_swift_helper(name: &str, embedded_helper: &[u8], helper: &Path) -> Result<()> {
    let cache_dir = ensure_swift_helper_cache_dir()?;
    let token = format!("{}-{}", std::process::id(), unix_nanos());
    let temporary_helper = cache_dir.join(format!("{name}-{token}"));
    let mut file = File::create(&temporary_helper).context("failed to create Swift helper")?;
    file.write_all(embedded_helper)
        .context("failed to write embedded Swift helper")?;
    file.flush().context("failed to flush Swift helper")?;
    fs::set_permissions(&temporary_helper, fs::Permissions::from_mode(0o700))
        .context("failed to mark Swift helper executable")?;

    fs::rename(&temporary_helper, helper).context("failed to install embedded Swift helper")
}

fn ensure_swift_helper_cache_dir() -> Result<PathBuf> {
    let cache_dir = swift_helper_cache_dir();
    fs::create_dir_all(&cache_dir).context("failed to create Swift helper cache directory")?;
    let metadata = cache_dir
        .symlink_metadata()
        .context("failed to inspect Swift helper cache directory")?;
    if !metadata.file_type().is_dir() {
        return Err(anyhow!("Swift helper cache path is not a directory"));
    }
    fs::set_permissions(&cache_dir, fs::Permissions::from_mode(0o700))
        .context("failed to secure Swift helper cache directory")?;
    Ok(cache_dir)
}

fn swift_helper_cache_dir() -> PathBuf {
    if let Some(home) = env::var_os("HOME") {
        return PathBuf::from(home)
            .join("Library")
            .join("Caches")
            .join(SWIFT_HELPER_CACHE_DIR);
    }

    env::temp_dir().join(SWIFT_HELPER_CACHE_DIR)
}

fn swift_helper_path(name: &str, body: &str) -> PathBuf {
    swift_helper_cache_dir().join(format!("{name}-{:016x}", swift_source_hash(body)))
}

fn swift_source_hash(body: &str) -> u64 {
    swift_source_hash_with_prelude(SWIFT_PRELUDE, body)
}

fn swift_source_hash_with_prelude(prelude: &str, body: &str) -> u64 {
    let mut hash = FNV_OFFSET_BASIS;
    for byte in prelude.bytes().chain(body.bytes()) {
        hash ^= u64::from(byte);
        hash = hash.wrapping_mul(FNV_PRIME);
    }
    hash
}

fn current_uid() -> u32 {
    unsafe { getuid() }
}

fn unix_nanos() -> u128 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_nanos())
        .unwrap_or(0)
}

fn convert_recording(capture_file: &Path, output_file: &Path, config: &Config) -> Result<()> {
    match converter_for(
        output_file,
        resolve_command("ffmpeg").is_some(),
        resolve_command("avconvert").is_some(),
    )? {
        Converter::Ffmpeg => convert_with_ffmpeg(capture_file, output_file, config),
        Converter::Avconvert => convert_with_avconvert(capture_file, output_file),
    }
}

fn convert_with_ffmpeg(capture_file: &Path, output_file: &Path, config: &Config) -> Result<()> {
    let mut command = Command::new(resolve_command("ffmpeg").unwrap_or_else(|| "ffmpeg".into()));
    command.args(conversion_args(capture_file, output_file, config));
    run_conversion_command(&mut command, "ffmpeg")
}

fn convert_with_avconvert(capture_file: &Path, output_file: &Path) -> Result<()> {
    let mut command =
        Command::new(resolve_command("avconvert").unwrap_or_else(|| "avconvert".into()));
    command.args([
        "--source".to_string(),
        capture_file.to_string_lossy().to_string(),
        "--preset".to_string(),
        "PresetHighestQuality".to_string(),
        "--output".to_string(),
        output_file.to_string_lossy().to_string(),
        "--replace".to_string(),
    ]);
    run_conversion_command(&mut command, "avconvert")
}

fn run_conversion_command(command: &mut Command, name: &str) -> Result<()> {
    qol_runtime::probe!("SHOT_RECORD_CONVERT", "tool={name} stage=start");
    let log_file = OpenOptions::new()
        .create(true)
        .append(true)
        .open(super::CAPTURE_LOG)
        .context("failed to open recording log file")?;
    let stdout_log = log_file
        .try_clone()
        .context("failed to clone recording log file")?;

    let status = command
        .stdin(Stdio::null())
        .stdout(Stdio::from(stdout_log))
        .stderr(Stdio::from(log_file))
        .status()
        .with_context(|| format!("failed to run {name} conversion"))?;

    if status.success() {
        qol_runtime::probe!("SHOT_RECORD_CONVERT", "tool={name} stage=ok");
        return Ok(());
    }

    qol_runtime::probe!(
        "SHOT_RECORD_CONVERT",
        "tool={name} stage=error status={status}"
    );
    Err(anyhow!("{name} exited with {}", status))
}

fn conversion_args(capture_file: &Path, output_file: &Path, config: &Config) -> Vec<String> {
    let mut args = vec![
        "-hide_banner".to_string(),
        "-loglevel".to_string(),
        "error".to_string(),
        "-y".to_string(),
        "-i".to_string(),
        capture_file.to_string_lossy().to_string(),
    ];

    match output_extension(output_file).as_deref() {
        Some("webm") => args.extend(webm_conversion_args(config)),
        Some("mp4") => args.extend(mp4_conversion_args(config)),
        Some("mkv") => args.extend(mkv_conversion_args(config)),
        _ => {}
    }

    args.push(output_file.to_string_lossy().to_string());
    args
}

fn webm_conversion_args(config: &Config) -> Vec<String> {
    let mut args = ["-c:v", "libvpx-vp9", "-b:v", "0", "-crf"]
        .into_iter()
        .map(str::to_string)
        .collect::<Vec<_>>();
    args.push(clamped_crf(config.video.crf).to_string());
    args.extend(["-c:a", "libopus"].into_iter().map(str::to_string));
    args
}

fn mp4_conversion_args(config: &Config) -> Vec<String> {
    h264_conversion_args(config, true)
}

fn mkv_conversion_args(config: &Config) -> Vec<String> {
    h264_conversion_args(config, false)
}

fn h264_conversion_args(config: &Config, faststart: bool) -> Vec<String> {
    let mut args = vec![
        "-c:v".to_string(),
        "libx264".to_string(),
        "-crf".to_string(),
        clamped_crf(config.video.crf).to_string(),
        "-preset".to_string(),
        normalized_h264_preset(&config.video.preset).to_string(),
        "-pix_fmt".to_string(),
        "yuv420p".to_string(),
        "-c:a".to_string(),
        "aac".to_string(),
    ];
    if faststart {
        args.extend(["-movflags", "+faststart"].into_iter().map(str::to_string));
    }
    args
}

fn clamped_crf(crf: i32) -> i32 {
    crf.clamp(0, 51)
}

fn normalized_h264_preset(preset: &str) -> &'static str {
    match preset {
        "ultrafast" => "ultrafast",
        "superfast" => "superfast",
        "veryfast" => "veryfast",
        "faster" => "faster",
        "fast" => "fast",
        "medium" => "medium",
        "slow" => "slow",
        "slower" => "slower",
        "veryslow" => "veryslow",
        _ => "veryfast",
    }
}

fn reveal_recording(output_file: &Path) -> bool {
    wait_for_file(output_file);

    if reveal_path(output_file) {
        return true;
    }

    output_file.parent().map(open_path).unwrap_or(false)
}

fn wait_for_file(output_file: &Path) {
    for _ in 0..20 {
        if output_file.symlink_metadata().is_ok() {
            return;
        }
        thread::sleep(Duration::from_millis(100));
    }
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

fn stop_capture_process(pid: u32) -> Result<()> {
    qol_runtime::probe!("SHOT_RECORD_STOP_PID", "pid={} signal=int", pid);
    signal_process(pid, libc::SIGINT)?;
    if wait_for_process_exit(pid, Duration::from_secs(8)) {
        qol_runtime::probe!(
            "SHOT_RECORD_STOP_PID",
            "pid={} result=stopped signal=int",
            pid
        );
        return Ok(());
    }

    qol_runtime::probe!("SHOT_RECORD_STOP_PID", "pid={} signal=term", pid);
    signal_process(pid, libc::SIGTERM)?;
    if wait_for_process_exit(pid, Duration::from_secs(2)) {
        qol_runtime::probe!(
            "SHOT_RECORD_STOP_PID",
            "pid={} result=stopped signal=term",
            pid
        );
        return Ok(());
    }

    qol_runtime::probe!("SHOT_RECORD_STOP_PID", "pid={} signal=kill", pid);
    signal_process(pid, libc::SIGKILL)?;
    if wait_for_process_exit(pid, Duration::from_secs(2)) {
        qol_runtime::probe!(
            "SHOT_RECORD_STOP_PID",
            "pid={} result=stopped signal=kill",
            pid
        );
        return Ok(());
    }

    qol_runtime::probe!("SHOT_RECORD_STOP_PID", "pid={} result=still_alive", pid);
    Err(anyhow!("capture process pid {} did not stop", pid))
}

fn signal_process(pid: u32, signal: i32) -> Result<()> {
    super::unix_signal_process(pid, signal)
}

fn wait_for_process_exit(pid: u32, timeout: Duration) -> bool {
    let deadline = Instant::now() + timeout;
    while Instant::now() < deadline {
        if !process_alive(pid) {
            return true;
        }
        thread::sleep(Duration::from_millis(150));
    }
    !process_alive(pid)
}

fn reveal_path(path: &Path) -> bool {
    Command::new("open")
        .arg("-R")
        .arg(path)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .map(|status| status.success())
        .unwrap_or(false)
}

fn open_path(path: &Path) -> bool {
    Command::new("open")
        .arg(path)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .map(|status| status.success())
        .unwrap_or(false)
}

fn videos_dir() -> Option<PathBuf> {
    let home = env::var_os("HOME")?;
    Some(PathBuf::from(home).join("Videos"))
}

fn paths_match(left: &Path, right: &Path) -> bool {
    left == right
}

fn path_extension_is(path: &Path, expected: &str) -> bool {
    output_extension(path).as_deref() == Some(expected)
}

fn output_extension(path: &Path) -> Option<String> {
    path.extension()?
        .to_str()
        .map(|ext| ext.to_ascii_lowercase())
}

fn output_format_label(path: &Path) -> String {
    output_extension(path)
        .map(|ext| ext.to_ascii_uppercase())
        .unwrap_or_else(|| "recording".to_string())
}

fn escape_applescript(input: &str) -> String {
    input.replace('\\', "\\\\").replace('"', "\\\"")
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Converter {
    Ffmpeg,
    Avconvert,
}

fn converter_for(
    output_file: &Path,
    ffmpeg_available: bool,
    avconvert_available: bool,
) -> Result<Converter> {
    if ffmpeg_available {
        return Ok(Converter::Ffmpeg);
    }

    if output_extension(output_file).as_deref() == Some("mp4") && avconvert_available {
        return Ok(Converter::Avconvert);
    }

    Err(anyhow!(
        "ffmpeg is required to convert recordings to {}",
        output_extension(output_file).unwrap_or_else(|| "this format".to_string())
    ))
}

fn resolve_command(command: &str) -> Option<PathBuf> {
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn non_mov_formats_capture_to_temporary_mov() {
        let output = Path::new("/tmp/recording.webm");
        assert_eq!(
            native_capture_file_path(output),
            PathBuf::from("/tmp/recording.mov")
        );
    }

    #[test]
    fn mov_format_captures_directly_to_output() {
        let output = Path::new("/tmp/recording.mov");
        assert_eq!(native_capture_file_path(output), output);
    }

    #[test]
    fn screencapture_recording_args_do_not_enable_click_spotlight() {
        let args = screencapture_recording_args(
            Some(Rect {
                x: -1512,
                y: 458,
                w: 800,
                h: 600,
            }),
            Some(2),
            true,
        );

        assert_eq!(
            args,
            vec!["-v", "-D", "2", "-R", "-1512,458,800,600", "-x", "-g"]
        );
        assert!(!args.iter().any(|arg| arg == "-k"));
    }

    #[test]
    fn screencapture_full_display_recording_args_avoid_area_selection() {
        let args = screencapture_recording_args(None, Some(2), false);

        assert_eq!(args, vec!["-v", "-D", "2", "-x"]);
        assert!(!args.iter().any(|arg| arg == "-R"));
    }

    #[test]
    fn rect_intersection_returns_overlap() {
        let rect = Rect {
            x: 1800,
            y: 100,
            w: 500,
            h: 400,
        };
        let monitor = Monitor {
            x: 1920,
            y: 0,
            w: 1920,
            h: 1080,
        };

        assert_eq!(
            rect_intersection(rect, monitor),
            Some(Rect {
                x: 1920,
                y: 100,
                w: 380,
                h: 400,
            })
        );
    }

    #[test]
    fn selector_rect_mapper_clips_to_active_displays() {
        let rect = Rect {
            x: 1800,
            y: 100,
            w: 500,
            h: 400,
        };
        let displays = [
            Monitor {
                x: 0,
                y: 0,
                w: 1920,
                h: 1080,
            },
            Monitor {
                x: 1920,
                y: 0,
                w: 1920,
                h: 1080,
            },
        ];

        assert_eq!(map_selector_rect_to_capture(rect, &displays), Some(rect));
    }

    #[test]
    fn selector_rect_mapper_rejects_off_display_selection() {
        let rect = Rect {
            x: 4000,
            y: 100,
            w: 100,
            h: 100,
        };
        let displays = [Monitor {
            x: 0,
            y: 0,
            w: 1920,
            h: 1080,
        }];

        assert_eq!(map_selector_rect_to_capture(rect, &displays), None);
    }

    #[test]
    fn capture_plan_uses_single_display_for_single_segment() {
        assert_eq!(capture_plan(1, false), CapturePlan::SingleDisplay);
        assert_eq!(capture_plan(0, true), CapturePlan::SingleDisplay);
    }

    #[test]
    fn capture_plan_uses_segmented_capture_when_ffmpeg_is_available() {
        assert_eq!(capture_plan(2, true), CapturePlan::SegmentedFfmpeg);
    }

    #[test]
    fn capture_plan_uses_segmented_native_composition_without_ffmpeg() {
        assert_eq!(capture_plan(2, false), CapturePlan::SegmentedNative);
    }

    #[test]
    fn segment_composition_filter_preserves_canvas_offsets() {
        let session = CaptureSession {
            output_file: Some(PathBuf::from("/tmp/final.mov")),
            capture_file: Some(PathBuf::from("/tmp/final.mov")),
            canvas: Some(Rect {
                x: 1800,
                y: 100,
                w: 500,
                h: 400,
            }),
            processes: Vec::new(),
            segments: vec![
                CaptureSegment {
                    file: PathBuf::from("/tmp/left.mov"),
                    rect: Rect {
                        x: 1800,
                        y: 100,
                        w: 120,
                        h: 400,
                    },
                    offset_x: 0,
                    offset_y: 0,
                },
                CaptureSegment {
                    file: PathBuf::from("/tmp/right.mov"),
                    rect: Rect {
                        x: 1920,
                        y: 100,
                        w: 380,
                        h: 400,
                    },
                    offset_x: 120,
                    offset_y: 0,
                },
            ],
        };

        let filter = segment_composition_filter(
            &session,
            Rect {
                x: 1800,
                y: 100,
                w: 500,
                h: 400,
            },
        );

        assert!(filter.contains("layout=0_0|120_0"));
        assert!(filter.contains("pad=500:400:0:0:color=black,crop=500:400:0:0[v]"));
    }

    #[test]
    fn native_segment_composition_args_preserve_canvas_and_offsets() {
        let session = CaptureSession {
            output_file: Some(PathBuf::from("/tmp/final.mov")),
            capture_file: Some(PathBuf::from("/tmp/final.mov")),
            canvas: Some(Rect {
                x: 1800,
                y: 100,
                w: 500,
                h: 400,
            }),
            processes: Vec::new(),
            segments: vec![
                CaptureSegment {
                    file: PathBuf::from("/tmp/left.mov"),
                    rect: Rect {
                        x: 1800,
                        y: 100,
                        w: 120,
                        h: 400,
                    },
                    offset_x: 0,
                    offset_y: 0,
                },
                CaptureSegment {
                    file: PathBuf::from("/tmp/right.mov"),
                    rect: Rect {
                        x: 1920,
                        y: 100,
                        w: 380,
                        h: 400,
                    },
                    offset_x: 120,
                    offset_y: 0,
                },
            ],
        };

        assert_eq!(
            native_segment_composition_args(
                &session,
                session.canvas.unwrap(),
                Path::new("/tmp/final.mov")
            ),
            vec![
                "500",
                "400",
                "/tmp/final.mov",
                "0",
                "0",
                "/tmp/left.mov",
                "120",
                "0",
                "/tmp/right.mov",
            ]
        );
    }

    #[test]
    fn webm_conversion_uses_webm_codecs() {
        let args = conversion_args(
            Path::new("/tmp/native.mov"),
            Path::new("/tmp/out.webm"),
            &Config::default(),
        );
        assert!(args.iter().any(|arg| arg == "libvpx-vp9"));
        assert!(args.iter().any(|arg| arg == "libopus"));
        assert_eq!(args.last().map(String::as_str), Some("/tmp/out.webm"));
    }

    #[test]
    fn ffmpeg_conversion_uses_configured_encoding_settings() {
        let cases = [
            ("/tmp/out.mp4", 24, "slow", "24", "slow"),
            ("/tmp/out.mkv", 0, "ultrafast", "0", "ultrafast"),
            ("/tmp/out.webm", 41, "ignored", "41", ""),
            ("/tmp/out.mp4", -2, "invalid", "0", "veryfast"),
            ("/tmp/out.webm", 87, "ignored", "51", ""),
        ];

        for (output, crf, preset, expected_crf, expected_preset) in cases {
            let config = config_with_encoding(crf, preset);
            let args = conversion_args(Path::new("/tmp/native.mov"), Path::new(output), &config);
            assert_arg_value(&args, "-crf", expected_crf);
            if !expected_preset.is_empty() {
                assert_arg_value(&args, "-preset", expected_preset);
            }
        }
    }

    #[test]
    fn mp4_uses_avconvert_when_ffmpeg_is_missing() {
        let converter = converter_for(Path::new("/tmp/out.mp4"), false, true).unwrap();
        assert_eq!(converter, Converter::Avconvert);
    }

    #[test]
    fn ffmpeg_is_preferred_when_available() {
        let converter = converter_for(Path::new("/tmp/out.mp4"), true, true).unwrap();
        assert_eq!(converter, Converter::Ffmpeg);
    }

    #[test]
    fn webm_requires_ffmpeg() {
        assert!(converter_for(Path::new("/tmp/out.webm"), false, true).is_err());
    }

    #[test]
    fn format_label_uses_uppercase_extension() {
        assert_eq!(output_format_label(Path::new("/tmp/out.mp4")), "MP4");
    }

    #[test]
    fn swift_helper_hash_includes_prelude_and_body() {
        assert_eq!(
            swift_source_hash(STATUS_OVERLAY_SWIFT),
            swift_source_hash_with_prelude(SWIFT_PRELUDE, STATUS_OVERLAY_SWIFT),
            "helper hash should use the shared Swift prelude"
        );
        assert_ne!(
            swift_source_hash(STATUS_OVERLAY_SWIFT),
            swift_source_hash(CLIPBOARD_WRITER_SWIFT),
            "different helper bodies should use different cache keys"
        );
        assert_ne!(
            swift_source_hash(STATUS_OVERLAY_SWIFT),
            swift_source_hash_with_prelude("changed prelude", STATUS_OVERLAY_SWIFT),
            "prelude changes should invalidate cached helpers"
        );
    }

    fn config_with_encoding(crf: i32, preset: &str) -> Config {
        let mut config = Config::default();
        config.video.crf = crf;
        config.video.preset = preset.to_string();
        config
    }

    fn assert_arg_value(args: &[String], key: &str, expected: &str) {
        let Some(index) = args.iter().position(|arg| arg == key) else {
            panic!("missing arg {key} in {args:?}");
        };
        assert_eq!(args.get(index + 1).map(String::as_str), Some(expected));
    }
}
