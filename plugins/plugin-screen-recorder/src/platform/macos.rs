use anyhow::{anyhow, Context, Result};
use std::env;
use std::fs::{File, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::thread;
use std::time::{Duration, Instant};

use crate::{Config, Monitor, Rect};

const SETTINGS_URL: &str = "http://127.0.0.1:42700/plugins/plugin-screen-recorder/";
const MAX_DISPLAYS: u32 = 16;
const REGION_SELECTOR_SWIFT: &str = r#"
import AppKit
import CoreGraphics
import Foundation

struct OverlayText {
    let title: String
    let subtitle: String?
    let titleSize: CGFloat
    let subtitleSize: CGFloat

    func drawPanel(in panel: NSRect) {
        let panelPath = NSBezierPath(roundedRect: panel, xRadius: 14, yRadius: 14)
        NSColor.black.withAlphaComponent(0.78).setFill()
        panelPath.fill()
        NSColor.white.withAlphaComponent(0.86).setStroke()
        panelPath.lineWidth = 1.5
        panelPath.stroke()

        drawLine(
            title,
            in: NSRect(x: panel.minX + 18, y: panel.minY + 37, width: panel.width - 36, height: 28),
            size: titleSize,
            weight: .semibold,
            alpha: 1.0
        )

        guard let subtitle else {
            return
        }

        drawLine(
            subtitle,
            in: NSRect(x: panel.minX + 18, y: panel.minY + 15, width: panel.width - 36, height: 20),
            size: subtitleSize,
            weight: .regular,
            alpha: 0.78
        )
    }

    func drawLabel(in rect: NSRect) {
        drawLine(title, in: rect, size: titleSize, weight: .semibold, alpha: 0.96)
    }

    private func drawLine(_ text: String, in rect: NSRect, size: CGFloat, weight: NSFont.Weight, alpha: CGFloat) {
        let paragraph = NSMutableParagraphStyle()
        paragraph.alignment = .center
        let attributes: [NSAttributedString.Key: Any] = [
            .font: NSFont.systemFont(ofSize: size, weight: weight),
            .foregroundColor: NSColor.white.withAlphaComponent(alpha),
            .paragraphStyle: paragraph
        ]
        text.draw(in: rect, withAttributes: attributes)
    }
}

final class SelectionView: NSView {
    let displayBounds: CGRect
    var startPoint: NSPoint?
    var currentPoint: NSPoint?

    init(frame: NSRect, displayBounds: CGRect) {
        self.displayBounds = displayBounds
        super.init(frame: NSRect(origin: .zero, size: frame.size))
    }

    required init?(coder: NSCoder) {
        fatalError("init(coder:) has not been implemented")
    }

    override var acceptsFirstResponder: Bool {
        true
    }

    override func draw(_ dirtyRect: NSRect) {
        NSColor.black.withAlphaComponent(0.42).setFill()
        bounds.fill()
        drawGuide()

        guard let rect = selectionRect() else {
            return
        }

        NSColor.systemRed.withAlphaComponent(0.34).setFill()
        rect.fill()
        NSColor.white.setStroke()
        let outerPath = NSBezierPath(rect: rect)
        outerPath.lineWidth = 7
        outerPath.stroke()
        NSColor.systemRed.setStroke()
        let path = NSBezierPath(rect: rect)
        path.lineWidth = 4
        path.stroke()
        drawSelectionLabel(rect)
    }

    override func mouseDown(with event: NSEvent) {
        let point = convert(event.locationInWindow, from: nil)
        startPoint = point
        currentPoint = point
        needsDisplay = true
    }

    override func mouseDragged(with event: NSEvent) {
        currentPoint = convert(event.locationInWindow, from: nil)
        needsDisplay = true
    }

    override func mouseUp(with event: NSEvent) {
        currentPoint = convert(event.locationInWindow, from: nil)
        finishSelection()
    }

    override func keyDown(with event: NSEvent) {
        if event.keyCode == 53 {
            exit(2)
        }
    }

    private func selectionRect() -> NSRect? {
        guard let start = startPoint, let current = currentPoint else {
            return nil
        }

        let x = min(start.x, current.x)
        let y = min(start.y, current.y)
        return NSRect(
            x: x,
            y: y,
            width: abs(start.x - current.x),
            height: abs(start.y - current.y)
        )
    }

    private func drawGuide() {
        let title = startPoint == nil ? "Drag to select recording area" : "Release mouse to start recording"
        let width = min(bounds.width - 48, 520)
        let panel = NSRect(x: bounds.midX - width / 2, y: bounds.maxY - 126, width: width, height: 78)
        OverlayText(title: title, subtitle: "Press Esc to cancel", titleSize: 22, subtitleSize: 14)
            .drawPanel(in: panel)
    }

    private func drawSelectionLabel(_ rect: NSRect) {
        guard rect.width >= 180, rect.height >= 80 else {
            return
        }

        let labelRect = NSRect(x: rect.minX + 12, y: rect.midY - 13, width: rect.width - 24, height: 26)
        OverlayText(title: "Recording area", subtitle: nil, titleSize: 18, subtitleSize: 14)
            .drawLabel(in: labelRect)
    }

    private func finishSelection() {
        guard let rect = selectionRect(), rect.width >= 4, rect.height >= 4 else {
            exit(2)
        }

        let x = displayBounds.origin.x + rect.minX
        let y = displayBounds.origin.y + bounds.height - rect.maxY
        let line = "\(Int(x.rounded())),\(Int(y.rounded())),\(Int(rect.width.rounded())),\(Int(rect.height.rounded()))\n"
        FileHandle.standardOutput.write(Data(line.utf8))
        NSApp.terminate(nil)
    }
}

let app = NSApplication.shared
app.setActivationPolicy(.accessory)

var windows: [NSWindow] = []
for screen in NSScreen.screens {
    let displayID = screen.deviceDescription[NSDeviceDescriptionKey("NSScreenNumber")] as? CGDirectDisplayID ?? CGMainDisplayID()
    let view = SelectionView(frame: screen.frame, displayBounds: CGDisplayBounds(displayID))
    let window = NSWindow(
        contentRect: screen.frame,
        styleMask: [.borderless],
        backing: .buffered,
        defer: false,
        screen: screen
    )
    window.level = .screenSaver
    window.backgroundColor = .clear
    window.isOpaque = false
    window.hasShadow = false
    window.collectionBehavior = [.canJoinAllSpaces, .fullScreenAuxiliary, .stationary]
    window.contentView = view
    window.makeKeyAndOrderFront(nil)
    window.makeFirstResponder(view)
    windows.append(window)
}

app.activate(ignoringOtherApps: true)
app.run()
"#;

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
}

pub fn recording_stopped(output_file: Option<&Path>, capture_file: Option<&Path>) {
    if let Some(output_file) = output_file {
        let capture_file = capture_file.unwrap_or(output_file);
        let conversion_needed = !paths_match(output_file, capture_file);
        show_recording_ended(output_file, conversion_needed);
        let reveal_file = match finalize_recording(output_file, capture_file) {
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
    Ok(())
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
    let mut child = Command::new("swift")
        .arg("-")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .context("failed to start macOS region selector")?;

    child
        .stdin
        .take()
        .context("failed to open region selector stdin")?
        .write_all(REGION_SELECTOR_SWIFT.as_bytes())
        .context("failed to write region selector source")?;

    let output = child
        .wait_with_output()
        .context("failed to wait for macOS region selector")?;

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

fn finalize_recording(output_file: &Path, capture_file: &Path) -> Result<PathBuf> {
    wait_for_stable_file(capture_file)?;
    if paths_match(output_file, capture_file) {
        return Ok(output_file.to_path_buf());
    }

    convert_recording(capture_file, output_file)?;
    let _ = std::fs::remove_file(capture_file);
    Ok(output_file.to_path_buf())
}

fn conversion_fallback(error: anyhow::Error, capture_file: &Path) -> PathBuf {
    show_notification(
        "Recording conversion failed",
        &format!("Saved native MOV instead. {}", error),
        2500,
    );
    capture_file.to_path_buf()
}

fn show_recording_ended(output_file: &Path, conversion_needed: bool) {
    if !conversion_needed {
        show_notification("Recording ended", "Saving recording...", 1600);
        return;
    }

    show_notification(
        "Recording ended",
        &format!("Converting to {}...", output_format_label(output_file)),
        2200,
    );
}

fn show_recording_saved(output_file: &Path, converted: bool) {
    let format = output_format_label(output_file);
    let message = if converted {
        format!("Converted to {} in Videos", format)
    } else {
        format!("Saved as {} in Videos", format)
    };
    show_notification("Recording saved", &message, 2200);
}

fn convert_recording(capture_file: &Path, output_file: &Path) -> Result<()> {
    match converter_for(
        output_file,
        resolve_command("ffmpeg").is_some(),
        resolve_command("avconvert").is_some(),
    )? {
        Converter::Ffmpeg => convert_with_ffmpeg(capture_file, output_file),
        Converter::Avconvert => convert_with_avconvert(capture_file, output_file),
    }
}

fn convert_with_ffmpeg(capture_file: &Path, output_file: &Path) -> Result<()> {
    let mut command = Command::new(resolve_command("ffmpeg").unwrap_or_else(|| "ffmpeg".into()));
    command.args(conversion_args(capture_file, output_file));
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

fn conversion_args(capture_file: &Path, output_file: &Path) -> Vec<String> {
    let mut args = vec![
        "-hide_banner".to_string(),
        "-loglevel".to_string(),
        "error".to_string(),
        "-y".to_string(),
        "-i".to_string(),
        capture_file.to_string_lossy().to_string(),
    ];

    match output_extension(output_file).as_deref() {
        Some("webm") => args.extend(webm_conversion_args()),
        Some("mp4") => args.extend(mp4_conversion_args()),
        Some("mkv") => args.extend(mkv_conversion_args()),
        _ => {}
    }

    args.push(output_file.to_string_lossy().to_string());
    args
}

fn webm_conversion_args() -> Vec<String> {
    [
        "-c:v",
        "libvpx-vp9",
        "-b:v",
        "0",
        "-crf",
        "32",
        "-c:a",
        "libopus",
    ]
    .into_iter()
    .map(str::to_string)
    .collect()
}

fn mp4_conversion_args() -> Vec<String> {
    [
        "-c:v",
        "libx264",
        "-pix_fmt",
        "yuv420p",
        "-c:a",
        "aac",
        "-movflags",
        "+faststart",
    ]
    .into_iter()
    .map(str::to_string)
    .collect()
}

fn mkv_conversion_args() -> Vec<String> {
    ["-c:v", "libx264", "-pix_fmt", "yuv420p", "-c:a", "aac"]
        .into_iter()
        .map(str::to_string)
        .collect()
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
    let signal_arg = format!("-{signal}");
    let pid_arg = pid.to_string();
    let status = Command::new("kill")
        .args([signal_arg.as_str(), pid_arg.as_str()])
        .status()
        .with_context(|| format!("failed to send SIG{signal} to screencapture"))?;

    if status.success() || !process_alive(pid) {
        return Ok(());
    }

    Err(anyhow!(
        "failed to send SIG{} to screencapture pid {}",
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
        let args = conversion_args(Path::new("/tmp/native.mov"), Path::new("/tmp/out.webm"));
        assert!(args.iter().any(|arg| arg == "libvpx-vp9"));
        assert!(args.iter().any(|arg| arg == "libopus"));
        assert_eq!(args.last().map(String::as_str), Some("/tmp/out.webm"));
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
}
