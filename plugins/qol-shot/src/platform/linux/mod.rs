use anyhow::{anyhow, Context, Result};
use gpui::{Bounds, Pixels, Point, WindowDecorations, WindowKind};
use qol_gpui::monitor::{ActiveMonitor, MonitorTracker};
use std::fs::File;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::rc::Rc;
use std::sync::mpsc;
use std::time::{Duration, Instant};

use crate::frozen_frame::FrozenFrame;
use crate::platform::{CaptureProcess, CaptureSession};
use crate::space::CaptureKind;
use crate::{Config, Monitor, Rect};

mod clipboard;
mod display;
mod preview;
mod status;
mod system;
mod window;

pub use clipboard::{copy_image_to_clipboard, copy_path_to_clipboard};
pub use display::{full_screen_bounds, get_monitors};
pub use preview::{capture_frozen_frame, configure_preview_window, grab_preview_rgba};
pub use status::{hide_capture_status, show_capture_status};
use system::resolve_command;
pub use system::{
    open_url, platform_supported_check, required_binaries_check, show_notification,
    show_saved_notification,
};
pub use window::{
    configure_pin_window, pin_focus, pin_release_focus, pin_resize_session, prepare_pin_window,
    PinResizeSession,
};
use window::{configure_selector_window, prepare_selector_window};

const MIN_DETECTED_TARGET_PX: i32 = 24;

thread_local! {
    static SELECTOR_CACHE: crate::region_selector::SelectorCache =
        crate::region_selector::SelectorCache::default();
}

pub fn pre_create_selector(cx: &mut gpui::App) {
    let bounds = selector_bounds(None);
    let selector = selector_window(bounds, None, None, None, None, None);
    SELECTOR_CACHE.with(|cache| {
        let Some(title) =
            crate::region_selector::pre_create_cached(cache, selector, CaptureKind::Screenshot, cx)
        else {
            return;
        };
        prepare_selector_window(&title, rect_from_bounds(bounds));
    });
}

pub fn select_region(kind: CaptureKind, frozen_frame: Option<FrozenFrame>) -> Result<Option<Rect>> {
    crate::region_selector::select_region_blocking_with(move |tx, cx| {
        let tracker = MonitorTracker::start(cx);
        let cursor = tracker.snapshot_cursor();
        let active_bounds = Some(tracker_active_bounds_source(tracker));
        open_region_selector_with_sender(tx, true, kind, cursor, active_bounds, frozen_frame, cx);
    })
}

pub fn select_region_in_app(
    cx: &mut gpui::App,
    kind: CaptureKind,
    cursor: Option<(ActiveMonitor, Option<Point<Pixels>>)>,
    _monitors: Vec<ActiveMonitor>,
    frozen_frame: Option<FrozenFrame>,
) -> Option<mpsc::Receiver<Option<Rect>>> {
    let active_bounds = Some(tracker_active_bounds_source(MonitorTracker::start(cx)));
    Some(open_region_selector(
        cx,
        kind,
        cursor,
        active_bounds,
        frozen_frame,
    ))
}

fn open_region_selector(
    cx: &mut gpui::App,
    kind: CaptureKind,
    cursor: Option<(ActiveMonitor, Option<Point<Pixels>>)>,
    active_bounds: Option<crate::region_selector::ActiveBoundsSource>,
    frozen_frame: Option<FrozenFrame>,
) -> mpsc::Receiver<Option<Rect>> {
    let (tx, rx) = mpsc::channel();
    open_region_selector_with_sender(tx, false, kind, cursor, active_bounds, frozen_frame, cx);
    rx
}

fn open_region_selector_with_sender(
    tx: mpsc::Sender<Option<Rect>>,
    quit_on_finish: bool,
    kind: CaptureKind,
    cursor: Option<(ActiveMonitor, Option<Point<Pixels>>)>,
    active_bounds: Option<crate::region_selector::ActiveBoundsSource>,
    frozen_frame: Option<FrozenFrame>,
    cx: &mut gpui::App,
) {
    let (monitor, pointer) = match cursor {
        Some((monitor, pointer)) => (Some(monitor), pointer),
        None => (None, None),
    };
    let bounds = selector_bounds(frozen_frame.as_ref());
    let hover_target = snapshot_hover_target(bounds);
    let default_target = initial_default_target(
        pointer,
        hover_target.as_deref(),
        default_window_target(bounds),
        monitor
            .as_ref()
            .map(|monitor| rect_from_bounds(monitor.bounds())),
    );
    trace_default_target(default_target, bounds);
    let selector = selector_window(
        bounds,
        monitor,
        active_bounds,
        hover_target,
        default_target,
        frozen_frame,
    );
    if !quit_on_finish {
        let mut tx = Some(tx);
        let mut selector = Some(selector);
        let reveal_bounds = rect_from_bounds(bounds);
        let reveal: crate::region_selector::SelectorReveal = Rc::new(move |title| {
            configure_selector_window(title, reveal_bounds);
        });
        let title = SELECTOR_CACHE.with(|cache| {
            crate::region_selector::open_cached(cache, &mut tx, &mut selector, kind, reveal, cx)
        });
        if title.is_some() {
            cx.activate(true);
            return;
        }
        let (Some(tx), Some(selector)) = (tx, selector) else {
            return;
        };
        let title = selector.title().to_string();
        if crate::region_selector::open_all(tx, false, vec![selector], kind, cx) {
            configure_selector_window(title, rect_from_bounds(bounds));
            cx.activate(true);
        }
        return;
    }
    let title = selector.title().to_string();
    if crate::region_selector::open_all(tx, true, vec![selector], kind, cx) {
        configure_selector_window(title, rect_from_bounds(bounds));
        cx.activate(true);
    }
}

fn initial_default_target(
    pointer: Option<Point<Pixels>>,
    hover_target: Option<&dyn crate::region_selector::HoverTarget>,
    focused_window: Option<Rect>,
    monitor: Option<Rect>,
) -> Option<crate::region_selector::DetectedTarget> {
    let pointer_target = pointer.and_then(|point| hover_target?.target_at(point));
    let pointer_monitor = pointer
        .zip(monitor)
        .map(|(_, rect)| crate::region_selector::DetectedTarget::Monitor(rect));
    pointer_target
        .or(pointer_monitor)
        .or_else(|| focused_window.map(crate::region_selector::DetectedTarget::Window))
        .or_else(|| monitor.map(crate::region_selector::DetectedTarget::Monitor))
}

fn selector_window(
    bounds: Bounds<Pixels>,
    monitor: Option<ActiveMonitor>,
    active_bounds: Option<crate::region_selector::ActiveBoundsSource>,
    hover_target: Option<crate::region_selector::HoverTargetSource>,
    default_target: Option<crate::region_selector::DetectedTarget>,
    frozen_frame: Option<FrozenFrame>,
) -> crate::region_selector::SelectorWindow {
    crate::region_selector::SelectorWindow::new(
        bounds,
        monitor.map(|monitor| monitor.bounds()),
        default_target,
        crate::region_selector::SelectorWindowOptions {
            display_id: None,
            kind: WindowKind::PopUp,
            decorations: WindowDecorations::Client,
            focus: true,
        },
        crate::region_selector::SelectorWindowSources {
            map_rect: crate::region_selector::identity_rect_mapper(),
            global_pointer: None,
            cancel_signal: Some(Rc::new(qol_gpui::platform::is_escape_held)),
            active_bounds,
            hover_target,
            frozen_frame,
        },
    )
}

fn snapshot_hover_target(
    selector_bounds: Bounds<Pixels>,
) -> Option<crate::region_selector::HoverTargetSource> {
    let include_frame = crate::config::load().capture.include_window_frame;
    let windows: Vec<Rect> = x11_stacked_window_rects(include_frame)
        .into_iter()
        .filter(|rect| {
            rect.w >= MIN_DETECTED_TARGET_PX
                && rect.h >= MIN_DETECTED_TARGET_PX
                && rect_intersects_bounds(*rect, selector_bounds)
        })
        .collect();
    let monitors = runtime_monitor_rects();
    qol_runtime::probe!(
        "SHOT_SELECT_TARGET",
        "snapshot_windows={} monitors={} frame={include_frame}",
        windows.len(),
        monitors.len()
    );
    if windows.is_empty() && monitors.is_empty() {
        return None;
    }
    Some(Rc::new(SnapshotHoverTarget { windows, monitors }))
}

fn runtime_monitor_rects() -> Vec<Rect> {
    let Some(state) = qol_runtime::PlatformStateClient::from_env().get_state() else {
        return Vec::new();
    };
    state
        .monitors
        .iter()
        .map(|monitor| Rect {
            x: monitor.x.round() as i32,
            y: monitor.y.round() as i32,
            w: monitor.width.round() as i32,
            h: monitor.height.round() as i32,
        })
        .collect()
}

struct SnapshotHoverTarget {
    windows: Vec<Rect>,
    monitors: Vec<Rect>,
}

impl crate::region_selector::HoverTarget for SnapshotHoverTarget {
    fn target_at(
        &self,
        point: gpui::Point<Pixels>,
    ) -> Option<crate::region_selector::DetectedTarget> {
        let x = f32::from(point.x).round() as i32;
        let y = f32::from(point.y).round() as i32;
        let hit =
            |rect: &Rect| x >= rect.x && x < rect.x + rect.w && y >= rect.y && y < rect.y + rect.h;
        if let Some(window) = self.windows.iter().copied().find(hit) {
            return Some(crate::region_selector::DetectedTarget::Window(window));
        }
        self.monitors
            .iter()
            .copied()
            .find(hit)
            .map(crate::region_selector::DetectedTarget::Monitor)
    }
}

fn x11_stacked_window_rects(include_frame: bool) -> Vec<Rect> {
    x11_stacked_window_rects_impl(include_frame).unwrap_or_default()
}

fn x11_stacked_window_rects_impl(include_frame: bool) -> Option<Vec<Rect>> {
    use x11rb::connection::Connection;
    use x11rb::protocol::xproto::{AtomEnum, ConnectionExt};

    let (conn, screen_num) = x11rb::connect(None).ok()?;
    let root = conn.setup().roots.get(screen_num)?.root;
    let atom = |name: &str| {
        conn.intern_atom(false, name.as_bytes())
            .ok()
            .and_then(|cookie| cookie.reply().ok())
            .map(|reply| reply.atom)
    };
    let stacking = atom("_NET_CLIENT_LIST_STACKING")?;
    let wm_type = atom("_NET_WM_WINDOW_TYPE")?;
    let type_normal = atom("_NET_WM_WINDOW_TYPE_NORMAL")?;
    let type_dialog = atom("_NET_WM_WINDOW_TYPE_DIALOG")?;
    let type_utility = atom("_NET_WM_WINDOW_TYPE_UTILITY")?;
    let wm_state = atom("_NET_WM_STATE")?;
    let state_hidden = atom("_NET_WM_STATE_HIDDEN")?;
    let wm_desktop = atom("_NET_WM_DESKTOP")?;
    let current_desktop = atom("_NET_CURRENT_DESKTOP")?;
    let net_frame = atom("_NET_FRAME_EXTENTS")?;
    let gtk_frame = atom("_GTK_FRAME_EXTENTS")?;

    let cardinal = |window: u32, prop: u32| {
        conn.get_property(false, window, prop, AtomEnum::CARDINAL, 0, 1)
            .ok()
            .and_then(|cookie| cookie.reply().ok())
            .and_then(|reply| reply.value32().and_then(|mut values| values.next()))
    };
    let atom_list = |window: u32, prop: u32| -> Vec<u32> {
        conn.get_property(false, window, prop, AtomEnum::ATOM, 0, 32)
            .ok()
            .and_then(|cookie| cookie.reply().ok())
            .and_then(|reply| reply.value32().map(|values| values.collect()))
            .unwrap_or_default()
    };
    let extents = |window: u32, prop: u32| -> Option<[i32; 4]> {
        let values: Vec<u32> = conn
            .get_property(false, window, prop, AtomEnum::CARDINAL, 0, 4)
            .ok()?
            .reply()
            .ok()?
            .value32()?
            .collect();
        let [left, right, top, bottom] = values.as_slice() else {
            return None;
        };
        Some([*left as i32, *right as i32, *top as i32, *bottom as i32])
    };

    let ids: Vec<u32> = conn
        .get_property(false, root, stacking, AtomEnum::WINDOW, 0, 1024)
        .ok()?
        .reply()
        .ok()?
        .value32()?
        .collect();
    let desktop = cardinal(root, current_desktop);
    let capturable_types = [type_normal, type_dialog, type_utility];
    let total = ids.len();
    let mut type_rejected = 0;
    let mut hidden = 0;
    let mut off_desktop = 0;
    let mut unresolved = 0;

    let mut rects = Vec::new();
    for &id in ids.iter().rev() {
        let types = atom_list(id, wm_type);
        if !capturable_window_type(&types, &capturable_types) {
            type_rejected += 1;
            continue;
        }
        if atom_list(id, wm_state).contains(&state_hidden) {
            hidden += 1;
            continue;
        }
        if let (Some(current), Some(window_desktop)) = (desktop, cardinal(id, wm_desktop)) {
            if window_desktop != current && window_desktop != u32::MAX {
                off_desktop += 1;
                continue;
            }
        }
        let Some(geometry) = conn.get_geometry(id).ok().and_then(|c| c.reply().ok()) else {
            unresolved += 1;
            continue;
        };
        let Some(origin) = conn
            .translate_coordinates(id, root, 0, 0)
            .ok()
            .and_then(|c| c.reply().ok())
        else {
            unresolved += 1;
            continue;
        };
        let mut rect = Rect {
            x: origin.dst_x as i32,
            y: origin.dst_y as i32,
            w: geometry.width as i32,
            h: geometry.height as i32,
        };
        if include_frame {
            rect = framed_rect(rect, extents(id, net_frame), extents(id, gtk_frame));
        }
        rects.push(rect);
    }
    qol_runtime::probe!(
        "SHOT_SELECT_DISCOVERY",
        "total={total} accepted={} type_rejected={type_rejected} hidden={hidden} off_desktop={off_desktop} unresolved={unresolved}",
        rects.len()
    );
    Some(rects)
}

fn capturable_window_type(types: &[u32], accepted: &[u32]) -> bool {
    types.is_empty()
        || types
            .iter()
            .any(|window_type| accepted.contains(window_type))
}

fn framed_rect(client: Rect, wm_extents: Option<[i32; 4]>, csd_shadow: Option<[i32; 4]>) -> Rect {
    let mut rect = client;
    if let Some([left, right, top, bottom]) = wm_extents {
        rect.x -= left;
        rect.y -= top;
        rect.w += left + right;
        rect.h += top + bottom;
    }
    if let Some([left, right, top, bottom]) = csd_shadow {
        rect.x += left;
        rect.y += top;
        rect.w -= left + right;
        rect.h -= top + bottom;
    }
    rect
}

fn default_window_target(selector_bounds: Bounds<Pixels>) -> Option<Rect> {
    runtime_focused_window_target(selector_bounds)
        .or_else(|| xdotool_active_window_target(selector_bounds))
}

fn runtime_focused_window_target(selector_bounds: Bounds<Pixels>) -> Option<Rect> {
    let window = qol_runtime::PlatformStateClient::from_env()
        .get_state()?
        .focused_window?;
    let rect = Rect {
        x: window.x.round() as i32,
        y: window.y.round() as i32,
        w: window.width.round() as i32,
        h: window.height.round() as i32,
    };
    usable_target("runtime", rect, selector_bounds)
}

fn xdotool_active_window_target(selector_bounds: Bounds<Pixels>) -> Option<Rect> {
    let rect = parse_xdotool_geometry(&run_xdotool(&[
        "getactivewindow",
        "getwindowgeometry",
        "--shell",
    ])?)?;
    usable_target("xdotool", rect, selector_bounds)
}

fn run_xdotool(args: &[&str]) -> Option<String> {
    let program = resolve_command("xdotool")?;
    let output = Command::new(program)
        .args(args)
        .stdin(Stdio::null())
        .stderr(Stdio::null())
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    String::from_utf8(output.stdout).ok()
}

fn parse_xdotool_geometry(output: &str) -> Option<Rect> {
    let x = parse_shell_i32(output, "X")?;
    let y = parse_shell_i32(output, "Y")?;
    let w = parse_shell_i32(output, "WIDTH")?;
    let h = parse_shell_i32(output, "HEIGHT")?;
    positive_rect(Rect { x, y, w, h })
}

fn parse_shell_i32(output: &str, key: &str) -> Option<i32> {
    output.lines().find_map(|line| {
        let (candidate, value) = line.split_once('=')?;
        (candidate == key).then(|| value.parse().ok()).flatten()
    })
}

fn positive_rect(rect: Rect) -> Option<Rect> {
    (rect.w > 0 && rect.h > 0).then_some(rect)
}

fn usable_target(
    source: &'static str,
    rect: Rect,
    selector_bounds: Bounds<Pixels>,
) -> Option<Rect> {
    let rect = positive_rect(rect)?;
    let usable = rect.w >= MIN_DETECTED_TARGET_PX
        && rect.h >= MIN_DETECTED_TARGET_PX
        && rect_intersects_bounds(rect, selector_bounds);
    trace_target_candidate(source, rect, selector_bounds, usable);
    usable.then_some(rect)
}

fn rect_intersects_bounds(rect: Rect, bounds: Bounds<Pixels>) -> bool {
    let left = rect.x as f32;
    let top = rect.y as f32;
    let right = left + rect.w as f32;
    let bottom = top + rect.h as f32;
    let bounds_left = f32::from(bounds.origin.x);
    let bounds_top = f32::from(bounds.origin.y);
    let bounds_right = bounds_left + f32::from(bounds.size.width);
    let bounds_bottom = bounds_top + f32::from(bounds.size.height);
    left < bounds_right && right > bounds_left && top < bounds_bottom && bottom > bounds_top
}

fn trace_target_candidate(
    source: &'static str,
    rect: Rect,
    selector_bounds: Bounds<Pixels>,
    usable: bool,
) {
    let rect = crate::geometry::rect_label(rect);
    let bounds = rect_from_bounds(selector_bounds);
    let bounds = crate::geometry::rect_label(bounds);
    qol_runtime::probe!(
        "SHOT_SELECT_TARGET",
        "source={source} rect={rect} selector={bounds} usable={usable}"
    );
}

fn trace_default_target(
    target: Option<crate::region_selector::DetectedTarget>,
    selector_bounds: Bounds<Pixels>,
) {
    let target = match target {
        Some(crate::region_selector::DetectedTarget::Window(rect)) => {
            format!("window:{}", crate::geometry::rect_label(rect))
        }
        Some(crate::region_selector::DetectedTarget::Monitor(rect)) => {
            format!("monitor:{}", crate::geometry::rect_label(rect))
        }
        None => "none".to_string(),
    };
    let bounds = rect_from_bounds(selector_bounds);
    let bounds = crate::geometry::rect_label(bounds);
    qol_runtime::probe!("SHOT_SELECT_TARGET", "default={target} selector={bounds}");
}

fn rect_from_bounds(bounds: Bounds<Pixels>) -> Rect {
    Rect {
        x: f32::from(bounds.origin.x).round() as i32,
        y: f32::from(bounds.origin.y).round() as i32,
        w: f32::from(bounds.size.width).round() as i32,
        h: f32::from(bounds.size.height).round() as i32,
    }
}

const ACTIVE_BOUNDS_SAMPLE_TTL: std::time::Duration = std::time::Duration::from_millis(200);

fn tracker_active_bounds_source(
    tracker: MonitorTracker,
) -> crate::region_selector::ActiveBoundsSource {
    Rc::new(TrackerActiveBounds {
        tracker,
        cache: std::cell::RefCell::new(None),
    })
}

struct TrackerActiveBounds {
    tracker: MonitorTracker,
    cache: std::cell::RefCell<Option<(std::time::Instant, Option<Bounds<Pixels>>)>>,
}

impl crate::region_selector::ActiveBounds for TrackerActiveBounds {
    fn active_bounds(&self) -> Option<Bounds<Pixels>> {
        if let Some((sampled_at, bounds)) = *self.cache.borrow() {
            if sampled_at.elapsed() < ACTIVE_BOUNDS_SAMPLE_TTL {
                return bounds;
            }
        }
        let bounds = self
            .tracker
            .snapshot_cursor()
            .map(|(monitor, _)| monitor.bounds());
        *self.cache.borrow_mut() = Some((std::time::Instant::now(), bounds));
        bounds
    }
}

fn selector_bounds(frozen_frame: Option<&FrozenFrame>) -> Bounds<Pixels> {
    if let Some(frame) = frozen_frame {
        let bounds = frame.bounds();
        return crate::region_selector::bounds_from_monitor(Monitor {
            x: bounds.x,
            y: bounds.y,
            w: bounds.w,
            h: bounds.h,
        });
    }
    full_screen_bounds()
        .map(crate::region_selector::bounds_from_monitor)
        .unwrap_or_else(|_| crate::region_selector::fallback_bounds())
}

pub fn start_capture(rect: &Rect, config: &Config, output_file: &Path) -> Result<CaptureSession> {
    let mut args = vec![
        "-thread_queue_size".to_string(),
        "512".to_string(),
        "-f".to_string(),
        "x11grab".to_string(),
        "-video_size".to_string(),
        format!("{}x{}", rect.w, rect.h),
        "-framerate".to_string(),
        config.video.framerate.to_string(),
        "-i".to_string(),
        format!(":0.0+{},{}", rect.x, rect.y),
    ];

    if config.audio.enabled {
        let has_mic = config.audio.inputs.iter().any(|input| input == "mic");
        let has_system = config.audio.inputs.iter().any(|input| input == "system");

        let mut audio_inputs = Vec::new();
        if has_mic {
            audio_inputs.push(config.audio.mic_device.clone());
        }
        if has_system {
            audio_inputs.push(format!("{}.monitor", config.audio.system_device));
        }

        for input in &audio_inputs {
            args.extend(["-thread_queue_size", "128", "-f", "pulse", "-i"].map(str::to_string));
            args.push(input.clone());
        }
        if audio_inputs.len() == 2 {
            args.extend(
                [
                    "-filter_complex",
                    "[1:a][2:a]amerge=inputs=2[aout]",
                    "-map",
                    "0:v",
                    "-map",
                    "[aout]",
                ]
                .map(str::to_string),
            );
        }
        if !audio_inputs.is_empty() {
            args.extend(["-c:a", "aac", "-b:a", "192k"].map(str::to_string));
        }
    }

    args.extend_from_slice(&[
        "-c:v".to_string(),
        "libx264".to_string(),
        "-r".to_string(),
        config.video.framerate.to_string(),
        "-crf".to_string(),
        config.video.crf.to_string(),
        "-preset".to_string(),
        config.video.preset.clone(),
        "-pix_fmt".to_string(),
        "yuv420p".to_string(),
        output_file.to_string_lossy().to_string(),
    ]);

    let log_file =
        File::create(super::CAPTURE_LOG).context("failed to create recording log file")?;
    let stdout_log = log_file
        .try_clone()
        .context("failed to clone recording log file")?;

    let child = Command::new("ffmpeg")
        .args(&args)
        .stdin(Stdio::null())
        .stdout(Stdio::from(stdout_log))
        .stderr(Stdio::from(log_file))
        .spawn()
        .context("failed to start ffmpeg")?;

    Ok(CaptureSession {
        output_file: Some(output_file.to_path_buf()),
        capture_file: Some(output_file.to_path_buf()),
        canvas: Some(*rect),
        processes: vec![CaptureProcess { pid: child.id() }],
        segments: Vec::new(),
    })
}

pub fn capture_screenshot(rect: &Rect, output_file: &Path) -> Result<()> {
    let log_file = File::create(super::CAPTURE_LOG).context("failed to create capture log file")?;
    let stdout_log = log_file
        .try_clone()
        .context("failed to clone capture log file")?;
    let video_size = format!("{}x{}", rect.w, rect.h);
    let input = format!(":0.0+{},{}", rect.x, rect.y);
    let status = Command::new("ffmpeg")
        .args([
            "-y",
            "-f",
            "x11grab",
            "-video_size",
            video_size.as_str(),
            "-i",
            input.as_str(),
            "-frames:v",
            "1",
        ])
        .arg(output_file)
        .stdin(Stdio::null())
        .stdout(Stdio::from(stdout_log))
        .stderr(Stdio::from(log_file))
        .status()
        .context("failed to run ffmpeg screenshot capture")?;

    if status.success() {
        return Ok(());
    }

    Err(anyhow!("ffmpeg screenshot capture exited with {status}"))
}

pub fn recording_format(format: &str) -> String {
    format.to_string()
}

pub fn recording_started(_session: &CaptureSession) {
    show_notification("Recording started", "Press your hotkey to stop", 1200);
}

pub fn recording_stopped(session: &CaptureSession, config: &Config) -> Option<PathBuf> {
    show_notification("Recording stopped", "Saving recording...", 1800);
    let output_file = session.output_file.as_deref()?;
    if let Err(error) = wait_for_recording_file(session, output_file) {
        eprintln!("[qol-shot] recording finalization failed: {error:#}");
        show_notification(
            "Recording save delayed",
            "The recorder is still finalizing the file",
            3000,
        );
        return None;
    }
    let message = output_file
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("Saved in Videos");
    crate::completion::background_saved(
        "Recording saved",
        message,
        output_file,
        config.capture.open_folder_after_save,
    );
    Some(output_file.to_path_buf())
}

fn wait_for_recording_file(session: &CaptureSession, output_file: &Path) -> Result<()> {
    let deadline = Instant::now() + Duration::from_secs(12);
    let mut previous_len = None;
    let mut stable_samples = 0;
    while Instant::now() < deadline {
        let recording = session
            .processes
            .iter()
            .any(|process| process_alive(process.pid));
        let len = output_file.metadata().ok().map(|metadata| metadata.len());
        if !recording && len.is_some_and(|len| len > 0) {
            if len == previous_len {
                stable_samples += 1;
                if stable_samples >= 2 {
                    qol_runtime::probe!(
                        "SHOT_RECORD_FINALIZE",
                        "stage=file-ready len={}",
                        len.unwrap_or_default()
                    );
                    return Ok(());
                }
            } else {
                previous_len = len;
                stable_samples = 0;
            }
        }
        std::thread::sleep(Duration::from_millis(100));
    }
    Err(anyhow!(
        "recording file did not finish writing: {}",
        output_file.display()
    ))
}

pub fn stop_capture(session: &CaptureSession) -> Result<()> {
    for process in &session.processes {
        super::unix_signal_process(process.pid, libc::SIGINT)
            .context("failed to send SIGINT to ffmpeg")?;
    }
    Ok(())
}

pub fn process_alive(pid: u32) -> bool {
    super::unix_process_alive(pid)
}

#[cfg(test)]
mod tests {
    use super::{
        capturable_window_type, framed_rect, initial_default_target, parse_xdotool_geometry,
        usable_target, SnapshotHoverTarget,
    };
    use crate::region_selector::{DetectedTarget, HoverTarget};
    use crate::Rect;
    use gpui::{point, px, size, Bounds};

    #[test]
    fn window_type_filter_accepts_capture_surfaces() {
        let normal = 1;
        let dialog = 2;
        let utility = 3;
        let dock = 4;
        let menu = 5;
        let tooltip = 6;
        let accepted = [normal, dialog, utility];
        let cases: &[(&[u32], bool)] = &[
            (&[], true),
            (&[normal], true),
            (&[dialog], true),
            (&[utility], true),
            (&[99, dialog], true),
            (&[dock], false),
            (&[menu], false),
            (&[tooltip], false),
            (&[dock, tooltip], false),
        ];

        for (types, expected) in cases {
            assert_eq!(
                capturable_window_type(types, &accepted),
                *expected,
                "types: {types:?}"
            );
        }
    }

    #[test]
    fn hover_target_picks_topmost_window_containing_the_pointer() {
        let top = Rect {
            x: 100,
            y: 100,
            w: 400,
            h: 300,
        };
        let bottom = Rect {
            x: 0,
            y: 0,
            w: 1920,
            h: 1080,
        };
        let monitor = Rect {
            x: 1920,
            y: 0,
            w: 2560,
            h: 1440,
        };
        let source = SnapshotHoverTarget {
            windows: vec![top, bottom],
            monitors: vec![monitor],
        };
        let cases = [
            (
                point(px(150.0), px(150.0)),
                Some(DetectedTarget::Window(top)),
            ),
            (
                point(px(50.0), px(50.0)),
                Some(DetectedTarget::Window(bottom)),
            ),
            (
                point(px(499.0), px(399.0)),
                Some(DetectedTarget::Window(top)),
            ),
            (
                point(px(500.0), px(400.0)),
                Some(DetectedTarget::Window(bottom)),
            ),
            (
                point(px(3000.0), px(50.0)),
                Some(DetectedTarget::Monitor(monitor)),
            ),
            (point(px(5000.0), px(50.0)), None),
        ];
        for (pointer, expected) in cases {
            assert_eq!(source.target_at(pointer), expected, "pointer: {pointer:?}");
        }
    }

    #[test]
    fn initial_target_follows_cursor_before_focus_fallbacks() {
        let hovered = Rect {
            x: 2560,
            y: 0,
            w: 1920,
            h: 1080,
        };
        let focused = Rect {
            x: 0,
            y: 32,
            w: 2560,
            h: 1366,
        };
        let cursor_monitor = Rect {
            x: 2560,
            y: 0,
            w: 1920,
            h: 1080,
        };
        let source = SnapshotHoverTarget {
            windows: vec![hovered],
            monitors: vec![cursor_monitor],
        };
        let cases = [
            (
                Some(point(px(3500.0), px(500.0))),
                Some(&source as &dyn HoverTarget),
                Some(focused),
                Some(cursor_monitor),
                Some(DetectedTarget::Window(hovered)),
            ),
            (
                Some(point(px(5000.0), px(500.0))),
                Some(&source as &dyn HoverTarget),
                Some(focused),
                Some(cursor_monitor),
                Some(DetectedTarget::Monitor(cursor_monitor)),
            ),
            (
                None,
                Some(&source as &dyn HoverTarget),
                Some(focused),
                Some(cursor_monitor),
                Some(DetectedTarget::Window(focused)),
            ),
            (
                None,
                None,
                None,
                Some(cursor_monitor),
                Some(DetectedTarget::Monitor(cursor_monitor)),
            ),
            (None, None, None, None, None),
        ];
        for (pointer, hover, focused, monitor, expected) in cases {
            assert_eq!(
                initial_default_target(pointer, hover, focused, monitor),
                expected
            );
        }
    }

    #[test]
    fn parses_xdotool_shell_geometry() {
        assert_eq!(
            parse_xdotool_geometry("WINDOW=123\nX=1930\nY=72\nWIDTH=2560\nHEIGHT=1366\n"),
            Some(Rect {
                x: 1930,
                y: 72,
                w: 2560,
                h: 1366,
            })
        );
    }

    #[test]
    fn rejects_empty_xdotool_geometry() {
        assert_eq!(
            parse_xdotool_geometry("X=0\nY=0\nWIDTH=0\nHEIGHT=1366\n"),
            None
        );
    }

    #[test]
    fn rejects_tiny_detection_targets() {
        let bounds = Bounds::new(point(px(0.0), px(0.0)), size(px(4480.0), px(1440.0)));
        assert_eq!(
            usable_target(
                "test",
                Rect {
                    x: 3002,
                    y: 370,
                    w: 1,
                    h: 1,
                },
                bounds,
            ),
            None
        );
    }

    #[test]
    fn framed_rect_applies_wm_extents_and_csd_shadows() {
        let client = Rect {
            x: 100,
            y: 128,
            w: 800,
            h: 572,
        };
        let cases = [
            (None, None, client),
            (
                Some([2, 2, 28, 2]),
                None,
                Rect {
                    x: 98,
                    y: 100,
                    w: 804,
                    h: 602,
                },
            ),
            (
                None,
                Some([26, 26, 23, 29]),
                Rect {
                    x: 126,
                    y: 151,
                    w: 748,
                    h: 520,
                },
            ),
        ];
        for (wm, csd, expected) in cases {
            assert_eq!(
                framed_rect(client, wm, csd),
                expected,
                "wm: {wm:?} csd: {csd:?}"
            );
        }
    }
}
