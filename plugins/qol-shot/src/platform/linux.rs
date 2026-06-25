use anyhow::{anyhow, Context, Result};
use gpui::{Bounds, Pixels, WindowDecorations, WindowKind};
use qol_gpui::monitor::{ActiveMonitor, MonitorTracker};
use qol_headless::DoctorCheckResult;
use std::env;
use std::fs::File;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::mpsc;

use crate::platform::{CaptureProcess, CaptureSession};
use crate::{Config, Monitor, Rect};

pub fn select_region() -> Result<Option<Rect>> {
    crate::region_selector::select_region_blocking_with(|tx, cx| {
        let monitor = MonitorTracker::start(cx).snapshot_monitor();
        open_region_selector_with_sender(tx, true, monitor, cx);
    })
}

pub fn select_region_in_app(
    cx: &mut gpui::App,
    monitor: Option<ActiveMonitor>,
    _monitors: Vec<ActiveMonitor>,
) -> Option<mpsc::Receiver<Option<Rect>>> {
    Some(open_region_selector(cx, monitor))
}

fn open_region_selector(
    cx: &mut gpui::App,
    monitor: Option<ActiveMonitor>,
) -> mpsc::Receiver<Option<Rect>> {
    let (tx, rx) = mpsc::channel();
    open_region_selector_with_sender(tx, false, monitor, cx);
    rx
}

fn open_region_selector_with_sender(
    tx: mpsc::Sender<Option<Rect>>,
    quit_on_finish: bool,
    monitor: Option<ActiveMonitor>,
    cx: &mut gpui::App,
) {
    let selector = selector_window(monitor);
    let title = selector.title().to_string();
    if crate::region_selector::open_all(tx, quit_on_finish, vec![selector], cx) {
        configure_selector_window(title);
        cx.activate(true);
    }
}

fn selector_window(monitor: Option<ActiveMonitor>) -> crate::region_selector::SelectorWindow {
    crate::region_selector::SelectorWindow::new(
        selector_bounds(),
        monitor.map(|monitor| monitor.bounds()),
        crate::region_selector::SelectorWindowOptions {
            display_id: None,
            kind: WindowKind::PopUp,
            decorations: WindowDecorations::Client,
            focus: true,
        },
        crate::region_selector::identity_rect_mapper(),
        None,
    )
}

fn selector_bounds() -> Bounds<Pixels> {
    full_screen_bounds()
        .map(crate::region_selector::bounds_from_monitor)
        .unwrap_or_else(|_| crate::region_selector::fallback_bounds())
}

pub fn get_monitors() -> Result<Vec<Monitor>> {
    let output = Command::new("xrandr")
        .args(["--query"])
        .output()
        .context("failed to run xrandr")?;
    if !output.status.success() {
        return Err(anyhow!("xrandr failed"));
    }
    let stdout = String::from_utf8_lossy(&output.stdout);
    let monitors: Vec<Monitor> = qol_runtime::xrandr::parse_monitors(&stdout)
        .into_iter()
        .map(monitor_from_xrandr)
        .collect();
    if monitors.is_empty() {
        return Err(anyhow!("no monitors found from xrandr"));
    }
    Ok(monitors)
}

pub fn full_screen_bounds() -> Result<Monitor> {
    let output = Command::new("xdpyinfo")
        .output()
        .context("failed to run xdpyinfo")?;
    if !output.status.success() {
        return Err(anyhow!("xdpyinfo failed"));
    }
    let stdout = String::from_utf8_lossy(&output.stdout);
    let dimensions = stdout
        .lines()
        .find_map(|line| {
            if !line.contains("dimensions:") {
                return None;
            }
            line.split_whitespace().find(|token| {
                token.contains('x') && token.chars().all(|c| c.is_ascii_digit() || c == 'x')
            })
        })
        .ok_or_else(|| anyhow!("could not read dimensions from xdpyinfo"))?;
    let split = dimensions
        .find('x')
        .ok_or_else(|| anyhow!("invalid dimensions"))?;
    let w = dimensions[..split]
        .parse::<i32>()
        .context("invalid width from xdpyinfo")?;
    let h = dimensions[split + 1..]
        .parse::<i32>()
        .context("invalid height from xdpyinfo")?;
    Ok(Monitor { x: 0, y: 0, w, h })
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
        if has_mic && has_system {
            args.extend_from_slice(&[
                "-thread_queue_size".to_string(),
                "128".to_string(),
                "-f".to_string(),
                "pulse".to_string(),
                "-i".to_string(),
                config.audio.mic_device.clone(),
                "-thread_queue_size".to_string(),
                "128".to_string(),
                "-f".to_string(),
                "pulse".to_string(),
                "-i".to_string(),
                format!("{}.monitor", config.audio.system_device),
                "-filter_complex".to_string(),
                "[1:a][2:a]amerge=inputs=2[aout]".to_string(),
                "-map".to_string(),
                "0:v".to_string(),
                "-map".to_string(),
                "[aout]".to_string(),
                "-c:a".to_string(),
                "aac".to_string(),
                "-b:a".to_string(),
                "192k".to_string(),
            ]);
        } else if has_mic {
            args.extend_from_slice(&[
                "-thread_queue_size".to_string(),
                "128".to_string(),
                "-f".to_string(),
                "pulse".to_string(),
                "-i".to_string(),
                config.audio.mic_device.clone(),
                "-c:a".to_string(),
                "aac".to_string(),
                "-b:a".to_string(),
                "192k".to_string(),
            ]);
        } else if has_system {
            args.extend_from_slice(&[
                "-thread_queue_size".to_string(),
                "128".to_string(),
                "-f".to_string(),
                "pulse".to_string(),
                "-i".to_string(),
                format!("{}.monitor", config.audio.system_device),
                "-c:a".to_string(),
                "aac".to_string(),
                "-b:a".to_string(),
                "192k".to_string(),
            ]);
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

pub fn copy_image_to_clipboard(path: &Path) -> Result<()> {
    let wl_copy_error = match copy_image_with("wl-copy", &["--type", "image/png"], path) {
        Ok(()) => return Ok(()),
        Err(error) => error,
    };
    let xclip_error = match copy_image_with(
        "xclip",
        &["-selection", "clipboard", "-t", "image/png", "-i"],
        path,
    ) {
        Ok(()) => return Ok(()),
        Err(error) => error,
    };

    Err(anyhow!(
        "failed to copy image to clipboard; wl-copy: {wl_copy_error:#}; xclip: {xclip_error:#}"
    ))
}

pub fn copy_path_to_clipboard(path: &Path) -> Result<()> {
    let text = path.to_string_lossy();
    let wl_copy_error = match copy_text_with("wl-copy", &[], &text) {
        Ok(()) => return Ok(()),
        Err(error) => error,
    };
    let xclip_error = match copy_text_with("xclip", &["-selection", "clipboard"], &text) {
        Ok(()) => return Ok(()),
        Err(error) => error,
    };

    Err(anyhow!(
        "failed to copy path to clipboard; wl-copy: {wl_copy_error:#}; xclip: {xclip_error:#}"
    ))
}

fn copy_text_with(program: &str, args: &[&str], text: &str) -> Result<()> {
    use std::io::Write;

    let mut child = Command::new(program)
        .args(args)
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .with_context(|| format!("failed to run {program}"))?;

    child
        .stdin
        .take()
        .ok_or_else(|| anyhow!("failed to open {program} stdin"))?
        .write_all(text.as_bytes())
        .with_context(|| format!("failed to write to {program}"))?;

    let status = child
        .wait()
        .with_context(|| format!("failed to wait for {program}"))?;
    if status.success() {
        return Ok(());
    }

    Err(anyhow!(
        "{program} text clipboard copy exited with {status}"
    ))
}

fn copy_image_with(program: &str, args: &[&str], path: &Path) -> Result<()> {
    let file = File::open(path).with_context(|| {
        format!(
            "failed to open screenshot for clipboard: {}",
            path.display()
        )
    })?;
    let status = Command::new(program)
        .args(args)
        .stdin(Stdio::from(file))
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .with_context(|| format!("failed to run {program}"))?;

    if status.success() {
        return Ok(());
    }

    Err(anyhow!(
        "{program} image clipboard copy exited with {status}"
    ))
}

pub fn recording_format(format: &str) -> String {
    format.to_string()
}

pub fn recording_started(_session: &CaptureSession) {
    show_notification("Recording started", "Press your hotkey to stop", 1200);
}

pub fn recording_stopped(_session: &CaptureSession, _config: &Config) {
    show_notification("Recording stopped", "Saved to ~/Videos", 2000);
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

pub fn show_notification(title: &str, message: &str, timeout_ms: u32) {
    let _ = Command::new("notify-send")
        .args([
            "-u",
            "normal",
            "-t",
            &timeout_ms.to_string(),
            title,
            message,
        ])
        .status();
}

pub fn open_url(url: &str) -> Result<()> {
    Command::new("xdg-open")
        .arg(url)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .context("failed to open URL")?;
    Ok(())
}

pub fn grab_preview_rgba(rect: &Rect) -> Option<(Vec<u8>, u32, u32)> {
    use x11rb::connection::Connection;
    use x11rb::protocol::xproto::{ConnectionExt, ImageFormat};

    if rect.w <= 0 || rect.h <= 0 {
        return None;
    }
    let (conn, screen_num) = x11rb::connect(None).ok()?;
    let root = conn.setup().roots.get(screen_num)?.root;
    let (w, h) = (rect.w as u16, rect.h as u16);
    let reply = conn
        .get_image(
            ImageFormat::Z_PIXMAP,
            root,
            rect.x as i16,
            rect.y as i16,
            w,
            h,
            u32::MAX,
        )
        .ok()?
        .reply()
        .ok()?;

    let mut data = reply.data;
    if data.len() != w as usize * h as usize * 4 {
        return None;
    }
    for pixel in data.chunks_exact_mut(4) {
        pixel.swap(0, 2);
        pixel[3] = 255;
    }
    Some((data, w as u32, h as u32))
}

pub fn configure_preview_window(title: String) {
    configure_overlay_window_async(title, "SHOT_OVERLAY");
}

fn configure_selector_window(title: String) {
    configure_overlay_window_async(title, "SHOT_SELECT_OVERLAY");
}

fn configure_overlay_window_async(title: String, probe: &'static str) {
    std::thread::spawn(move || {
        let started = std::time::Instant::now();
        for attempt in 1..=30 {
            if qol_gpui::popup_window::configure_overlay_window(&title) {
                qol_runtime::probe!(
                    probe,
                    "ms={} attempt={attempt} result=mapped",
                    started.elapsed().as_millis()
                );
                return;
            }
            std::thread::sleep(std::time::Duration::from_millis(40));
        }
        qol_runtime::probe!(probe, "ms={} result=timeout", started.elapsed().as_millis());
    });
}

pub fn platform_supported_check() -> DoctorCheckResult {
    DoctorCheckResult::ok(
        "platform_supported",
        "Linux capture is supported through ffmpeg/x11grab.",
    )
}

pub fn required_binaries_check() -> DoctorCheckResult {
    let required = ["ffmpeg", "xrandr", "xdpyinfo"];
    let missing = required
        .iter()
        .copied()
        .filter(|name| resolve_command(name).is_none())
        .collect::<Vec<_>>();

    if missing.is_empty() {
        return DoctorCheckResult::ok(
            "required_binaries",
            "Required Linux capture tools are available.",
        );
    }

    DoctorCheckResult::fail(
        "required_binaries",
        format!("Missing required binaries: {}.", missing.join(", ")),
    )
    .with_fix("Install ffmpeg, xrandr, and xdpyinfo.")
}

fn monitor_from_xrandr(monitor: qol_runtime::xrandr::XrandrMonitor) -> Monitor {
    Monitor {
        x: monitor.bounds.x as i32,
        y: monitor.bounds.y as i32,
        w: monitor.bounds.width as i32,
        h: monitor.bounds.height as i32,
    }
}

fn resolve_command(command: &str) -> Option<PathBuf> {
    command_search_dirs()
        .into_iter()
        .map(|dir| dir.join(command))
        .find(|path| is_executable_file(path))
}

fn command_search_dirs() -> Vec<PathBuf> {
    let mut dirs = env::var_os("PATH")
        .map(|paths| env::split_paths(&paths).collect::<Vec<_>>())
        .unwrap_or_default();
    dirs.extend([
        PathBuf::from("/usr/bin"),
        PathBuf::from("/bin"),
        PathBuf::from("/usr/local/bin"),
    ]);
    dirs
}

fn is_executable_file(path: &Path) -> bool {
    let Ok(metadata) = path.metadata() else {
        return false;
    };
    metadata.is_file() && metadata.permissions().mode() & 0o111 != 0
}
