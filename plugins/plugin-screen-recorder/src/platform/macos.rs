use anyhow::{anyhow, Context, Result};
use std::env;
use std::fs::{self, File, OpenOptions};
use std::io::Write;
use std::os::unix::fs::{MetadataExt, PermissionsExt};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Output, Stdio};
use std::sync::Mutex;
use std::thread;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use crate::{Config, Monitor, Rect};

const SETTINGS_URL: &str = "http://127.0.0.1:42700/plugins/plugin-screen-recorder/";
const MAX_DISPLAYS: u32 = 16;
const SWIFT_HELPER_CACHE_DIR: &str = "qol-screen-recorder-swift";
const FNV_OFFSET_BASIS: u64 = 0xcbf29ce484222325;
const FNV_PRIME: u64 = 0x100000001b3;
static STATUS_OVERLAY_PID: Mutex<Option<u32>> = Mutex::new(None);
const SWIFT_PRELUDE: &str = include_str!("macos_swift/prelude.swift");
const REGION_SELECTOR_SWIFT: &str = include_str!("macos_swift/region_selector.swift");
const STATUS_OVERLAY_SWIFT: &str = include_str!("macos_swift/status_overlay.swift");
const REGION_SELECTOR_HELPER: &[u8] = include_bytes!(concat!(env!("OUT_DIR"), "/region-selector"));
const STATUS_OVERLAY_HELPER: &[u8] = include_bytes!(concat!(env!("OUT_DIR"), "/status-overlay"));

type CGDirectDisplayID = u32;

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
}

extern "C" {
    fn getuid() -> u32;
}

pub fn select_region() -> Result<Option<Rect>> {
    select_region_with_overlay()
}

pub fn get_monitors() -> Result<Vec<Monitor>> {
    active_display_bounds()
}

pub fn full_screen_bounds() -> Result<Monitor> {
    let monitors = active_display_bounds()?;
    union_bounds(&monitors)
}

pub fn start_capture(rect: &Rect, config: &Config, output_file: &Path) -> Result<u32> {
    let log_file =
        File::create(super::CAPTURE_LOG).context("failed to create recording log file")?;
    let stdout_log = log_file
        .try_clone()
        .context("failed to clone recording log file")?;

    let mut command = Command::new("screencapture");
    let region = format!("{},{},{},{}", rect.x, rect.y, rect.w, rect.h);
    command.args(["-v", "-R", region.as_str(), "-k", "-x"]);

    if config.audio.enabled {
        command.arg("-g");
    }

    let child = command
        .arg(output_file)
        .stdin(Stdio::null())
        .stdout(Stdio::from(stdout_log))
        .stderr(Stdio::from(log_file))
        .spawn()
        .context("failed to start screencapture")?;

    Ok(child.id())
}

pub fn recording_format(format: &str) -> String {
    match format.to_ascii_lowercase().as_str() {
        "mkv" | "mp4" | "mov" | "webm" => format.to_ascii_lowercase(),
        _ => "mov".to_string(),
    }
}

pub fn capture_file_path(output_file: &Path) -> PathBuf {
    if path_extension_is(output_file, "mov") {
        return output_file.to_path_buf();
    }
    output_file.with_extension("mov")
}

pub fn recording_started() {
    show_notification("Recording started", "Press your hotkey to stop", 1200);
    prewarm_recording_helpers();
}

pub fn recording_stopped(output_file: Option<&Path>, capture_file: Option<&Path>, config: &Config) {
    if let Some(output_file) = output_file {
        let capture_file = capture_file.unwrap_or(output_file);
        let conversion_needed = !paths_match(output_file, capture_file);
        show_recording_ended(output_file, conversion_needed);
        let reveal_file = match finalize_recording(output_file, capture_file, config) {
            Ok(reveal_file) => {
                show_recording_saved(&reveal_file, conversion_needed);
                reveal_file
            }
            Err(error) => conversion_fallback(error, capture_file),
        };
        if reveal_recording(&reveal_file) {
            return;
        }
    }

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

pub fn stop_capture(pid: u32) -> Result<()> {
    signal_capture_process(pid, "INT")?;
    if wait_for_process_exit(pid, Duration::from_secs(8)) {
        return Ok(());
    }

    signal_capture_process(pid, "TERM")?;
    if wait_for_process_exit(pid, Duration::from_secs(2)) {
        return Ok(());
    }

    signal_capture_process(pid, "KILL")?;
    if wait_for_process_exit(pid, Duration::from_secs(2)) {
        return Ok(());
    }

    Err(anyhow!("capture process pid {} did not stop", pid))
}

pub fn process_alive(pid: u32) -> bool {
    let pid_arg = pid.to_string();
    Command::new("kill")
        .args(["-0", pid_arg.as_str()])
        .status()
        .map(|status| status.success())
        .unwrap_or(false)
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

pub fn open_settings() -> Result<()> {
    Command::new("open")
        .arg(SETTINGS_URL)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .context("failed to open settings URL")?;
    Ok(())
}

fn active_display_bounds() -> Result<Vec<Monitor>> {
    let mut displays = [0u32; MAX_DISPLAYS as usize];
    let mut count = 0u32;
    let result = unsafe { CGGetActiveDisplayList(MAX_DISPLAYS, displays.as_mut_ptr(), &mut count) };

    if result != 0 {
        return Err(anyhow!("CGGetActiveDisplayList failed: {}", result));
    }

    let monitors = displays
        .iter()
        .take(count as usize)
        .map(|display| {
            let bounds = unsafe { CGDisplayBounds(*display) };
            monitor_from_cg_bounds(bounds)
        })
        .collect::<Vec<_>>();

    if monitors.is_empty() {
        return Err(anyhow!("no active displays found"));
    }

    Ok(monitors)
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

fn select_region_with_overlay() -> Result<Option<Rect>> {
    let output = run_region_selector().context("failed to run macOS region selector")?;

    if output.status.code() == Some(2) {
        return Ok(None);
    }
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(anyhow!("macOS region selector failed: {}", stderr.trim()));
    }

    let raw = String::from_utf8_lossy(&output.stdout).trim().to_string();
    if raw.is_empty() {
        return Ok(None);
    }

    parse_selection_geometry(&raw).map(Some)
}

fn run_region_selector() -> Result<Output> {
    let helper = ensure_swift_helper(
        "region-selector",
        REGION_SELECTOR_SWIFT,
        REGION_SELECTOR_HELPER,
    )
    .context("failed to install embedded region selector helper")?;

    run_compiled_region_selector(&helper).inspect_err(|_| {
        let _ = fs::remove_file(&helper);
    })
}

fn run_compiled_region_selector(helper: &Path) -> Result<Output> {
    Command::new(helper)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()
        .context("failed to start compiled macOS region selector")
}

fn parse_selection_geometry(raw: &str) -> Result<Rect> {
    let values = raw
        .split(',')
        .map(str::trim)
        .map(str::parse::<i32>)
        .collect::<std::result::Result<Vec<_>, _>>()
        .context("invalid selection geometry")?;
    if values.len() != 4 {
        return Err(anyhow!(
            "expected 4 values in geometry, got {}",
            values.len()
        ));
    }

    Ok(Rect {
        x: values[0],
        y: values[1],
        w: values[2],
        h: values[3],
    })
}

fn finalize_recording(output_file: &Path, capture_file: &Path, config: &Config) -> Result<PathBuf> {
    wait_for_stable_file(capture_file)?;
    if paths_match(output_file, capture_file) {
        return Ok(output_file.to_path_buf());
    }

    convert_recording(capture_file, output_file, config)?;
    let _ = std::fs::remove_file(capture_file);
    Ok(output_file.to_path_buf())
}

fn conversion_fallback(error: anyhow::Error, capture_file: &Path) -> PathBuf {
    show_status_overlay(
        "Conversion failed",
        "Saved native MOV instead",
        3000,
        StatusOverlayLifecycle::ExitAfterHide,
    );
    show_notification(
        "Recording conversion failed",
        &format!("Saved native MOV instead. {}", error),
        2500,
    );
    capture_file.to_path_buf()
}

fn show_recording_ended(output_file: &Path, conversion_needed: bool) {
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
}

fn show_status_overlay(
    title: &str,
    message: &str,
    timeout_ms: u32,
    lifecycle: StatusOverlayLifecycle,
) {
    dismiss_status_overlay();
    let Ok(child) = spawn_status_overlay(title, message, timeout_ms, lifecycle) else {
        return;
    };
    let pid = child.id();
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
            match spawn_compiled_status_overlay(&helper, title, message, timeout_ms, lifecycle) {
                Ok(child) => Ok(child),
                Err(error) => {
                    let _ = fs::remove_file(&helper);
                    spawn_source_status_overlay(title, message, timeout_ms, lifecycle)
                        .with_context(|| format!("compiled status overlay failed first: {error:#}"))
                }
            }
        }
        Err(error) => spawn_source_status_overlay(title, message, timeout_ms, lifecycle)
            .with_context(|| format!("failed to build status overlay helper: {error:#}")),
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
            "QOL_STATUS_EXIT_AFTER_HIDE",
            if lifecycle.exit_after_hide() {
                "1"
            } else {
                "0"
            },
        );
}

fn dismiss_status_overlay() {
    let Some(pid) = take_status_overlay_pid() else {
        return;
    };

    let _ = signal_process(pid, "TERM");
}

fn write_status_overlay_pid(pid: u32) {
    if let Ok(mut current_pid) = STATUS_OVERLAY_PID.lock() {
        *current_pid = Some(pid);
    }
}

fn clear_status_overlay_pid(pid: u32) {
    let Ok(mut current_pid) = STATUS_OVERLAY_PID.lock() else {
        return;
    };

    if *current_pid == Some(pid) {
        *current_pid = None;
    }
}

fn take_status_overlay_pid() -> Option<u32> {
    STATUS_OVERLAY_PID.lock().ok()?.take()
}

fn prewarm_recording_helpers() {
    prewarm_swift_helper(
        "status-overlay",
        STATUS_OVERLAY_SWIFT,
        STATUS_OVERLAY_HELPER,
    );
    prewarm_swift_helper(
        "region-selector",
        REGION_SELECTOR_SWIFT,
        REGION_SELECTOR_HELPER,
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
        return Ok(());
    }

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
    let deadline = Instant::now() + Duration::from_secs(12);
    let mut last_len = None;
    let mut stable_count = 0;

    while Instant::now() < deadline {
        if let Ok(metadata) = output_file.symlink_metadata() {
            let len = metadata.len();
            if len > 0 && Some(len) == last_len {
                stable_count += 1;
                if stable_count >= 3 {
                    return Ok(());
                }
            } else {
                stable_count = 0;
                last_len = Some(len);
            }
        }
        thread::sleep(Duration::from_millis(200));
    }

    Err(anyhow!(
        "recording file did not finish writing: {}",
        output_file.display()
    ))
}

fn signal_capture_process(pid: u32, signal: &str) -> Result<()> {
    signal_process(pid, signal)
}

fn signal_process(pid: u32, signal: &str) -> Result<()> {
    let signal_arg = format!("-{signal}");
    let pid_arg = pid.to_string();
    let status = Command::new("kill")
        .args([signal_arg.as_str(), pid_arg.as_str()])
        .status()
        .with_context(|| format!("failed to send SIG{signal} to process"))?;

    if status.success() || !process_alive(pid) {
        return Ok(());
    }

    Err(anyhow!(
        "failed to send SIG{} to process pid {}",
        signal,
        pid
    ))
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
            capture_file_path(output),
            PathBuf::from("/tmp/recording.mov")
        );
    }

    #[test]
    fn mov_format_captures_directly_to_output() {
        let output = Path::new("/tmp/recording.mov");
        assert_eq!(capture_file_path(output), output);
    }

    #[test]
    fn parses_overlay_selection_geometry() {
        let rect = parse_selection_geometry("10,20,300,200").unwrap();
        assert_eq!(rect.x, 10);
        assert_eq!(rect.y, 20);
        assert_eq!(rect.w, 300);
        assert_eq!(rect.h, 200);
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
            swift_source_hash(REGION_SELECTOR_SWIFT),
            swift_source_hash_with_prelude(SWIFT_PRELUDE, REGION_SELECTOR_SWIFT),
            "helper hash should use the shared Swift prelude"
        );
        assert_ne!(
            swift_source_hash(REGION_SELECTOR_SWIFT),
            swift_source_hash(STATUS_OVERLAY_SWIFT),
            "different helper bodies should use different cache keys"
        );
        assert_ne!(
            swift_source_hash(REGION_SELECTOR_SWIFT),
            swift_source_hash_with_prelude("changed prelude", REGION_SELECTOR_SWIFT),
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
