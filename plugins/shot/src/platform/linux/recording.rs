use anyhow::{anyhow, Context, Result};
use serde::{Deserialize, Serialize};
use std::fs::File;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, ExitCode, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};
use x11rb::connection::Connection;
use x11rb::protocol::xfixes::ConnectionExt as XfixesExt;
use x11rb::rust_connection::RustConnection;

use crate::platform::{CaptureProcess, CaptureSession};
use crate::{Config, Rect};

use super::{process_alive, show_notification};

const CINNAMON_HELPER_ENV: &str = "QOL_SHOT_CINNAMON_CAPTURE_REQUEST";
const CINNAMON_READY_TIMEOUT: Duration = Duration::from_secs(3);
const CINNAMON_POLL_INTERVAL: Duration = Duration::from_millis(40);
static CINNAMON_STOP_REQUESTED: AtomicBool = AtomicBool::new(false);

#[derive(Debug, Deserialize, Serialize)]
struct CinnamonCaptureRequest {
    rect: Rect,
    framerate: u32,
    file_template: String,
    pipeline: String,
}

pub(super) fn run_internal_capture_helper() -> Option<ExitCode> {
    let request = std::env::var(CINNAMON_HELPER_ENV).ok()?;
    Some(match run_cinnamon_capture_helper(&request) {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("[qol-shot] Cinnamon capture helper failed: {error:#}");
            ExitCode::FAILURE
        }
    })
}

pub fn start_capture(rect: &Rect, config: &Config, output_file: &Path) -> Result<CaptureSession> {
    if cinnamon_desktop_session() {
        match start_cinnamon_capture(rect, config, output_file) {
            Ok(session) => {
                qol_runtime::probe!(
                    "SHOT_RECORD_START_BACKEND",
                    "backend=cinnamon_after_paint outcome=ready"
                );
                return Ok(session);
            }
            Err(error) => {
                qol_runtime::probe!(
                    "SHOT_RECORD_START_BACKEND",
                    "backend=cinnamon_after_paint outcome=fallback reason=start_failed"
                );
                eprintln!(
                    "[qol-shot] Cinnamon synchronized capture unavailable, using x11grab: {error:#}"
                );
            }
        }
    }

    qol_runtime::probe!(
        "SHOT_RECORD_START_BACKEND",
        "backend=x11grab outcome=ready reason={}",
        if cinnamon_desktop_session() {
            "cinnamon_start_failed"
        } else {
            "non_cinnamon_session"
        }
    );
    start_x11grab_capture(rect, config, output_file)
}

fn start_cinnamon_capture(
    rect: &Rect,
    config: &Config,
    output_file: &Path,
) -> Result<CaptureSession> {
    let capture_file = cinnamon_capture_file(output_file);
    if capture_file.symlink_metadata().is_ok() {
        return Err(anyhow!(
            "refusing to replace existing capture output {}",
            capture_file.display()
        ));
    }
    let output = capture_file
        .to_str()
        .ok_or_else(|| anyhow!("capture output path is not valid UTF-8"))?;
    let format = recording_format(&config.video.format);
    let request = CinnamonCaptureRequest {
        rect: *rect,
        framerate: config.video.framerate.clamp(1, 240),
        file_template: output.replace('%', "%%"),
        pipeline: cinnamon_pipeline(config),
    };
    let request_json =
        serde_json::to_string(&request).context("failed to encode capture request")?;

    qol_runtime::probe!(
        "SHOT_RECORD_START_PLAN",
        "backend=cinnamon_after_paint rect={}x{}+{},{} fps={} format={} audio_inputs={} cursor=xfixes_actor_fallback_builtin cursor_update=before_paint",
        rect.w,
        rect.h,
        rect.x,
        rect.y,
        request.framerate,
        format,
        audio_inputs(config).len()
    );

    let log_file =
        File::create(super::super::CAPTURE_LOG).context("failed to create recording log file")?;
    let stdout_log = log_file
        .try_clone()
        .context("failed to clone recording log file")?;
    let executable = std::env::current_exe().context("failed to resolve qol-shot executable")?;
    let mut child = Command::new(executable)
        .env(CINNAMON_HELPER_ENV, request_json)
        .stdin(Stdio::null())
        .stdout(Stdio::from(stdout_log))
        .stderr(Stdio::from(log_file))
        .spawn()
        .context("failed to start Cinnamon capture helper")?;

    if wait_for_cinnamon_ready(&mut child, &capture_file)? {
        return Ok(capture_session(
            child.id(),
            *rect,
            output_file,
            &capture_file,
        ));
    }

    stop_failed_cinnamon_start(&mut child);
    if capture_file.symlink_metadata().is_ok() {
        std::fs::remove_file(&capture_file).with_context(|| {
            format!(
                "failed to remove incomplete Cinnamon capture {}",
                capture_file.display()
            )
        })?;
    }
    Err(anyhow!("Cinnamon capture did not create its output file"))
}

fn wait_for_cinnamon_ready(child: &mut Child, output_file: &Path) -> Result<bool> {
    let deadline = Instant::now() + CINNAMON_READY_TIMEOUT;
    while Instant::now() < deadline {
        if child
            .try_wait()
            .context("failed to inspect Cinnamon capture helper")?
            .is_some()
        {
            return Ok(false);
        }
        if output_file.metadata().is_ok_and(|metadata| {
            if !metadata.is_file() || metadata.len() == 0 {
                return false;
            }
            qol_runtime::probe!(
                "SHOT_RECORD_CAPTURE_READY",
                "backend=cinnamon_after_paint len={}",
                metadata.len()
            );
            true
        }) {
            return Ok(true);
        }
        std::thread::sleep(CINNAMON_POLL_INTERVAL);
    }
    Ok(false)
}

fn stop_failed_cinnamon_start(child: &mut Child) {
    if child.try_wait().ok().flatten().is_some() {
        return;
    }
    let _ = super::super::unix::signal_process(child.id(), libc::SIGINT);
    let deadline = Instant::now() + Duration::from_secs(2);
    while Instant::now() < deadline {
        if child.try_wait().ok().flatten().is_some() {
            return;
        }
        std::thread::sleep(CINNAMON_POLL_INTERVAL);
    }
}

fn cinnamon_capture_file(output_file: &Path) -> PathBuf {
    let stem = output_file
        .file_stem()
        .map(|stem| stem.to_string_lossy())
        .unwrap_or_else(|| "recording".into());
    output_file.with_file_name(format!(".{stem}.cinnamon.webm"))
}

fn capture_session(
    pid: u32,
    rect: Rect,
    output_file: &Path,
    capture_file: &Path,
) -> CaptureSession {
    CaptureSession {
        output_file: Some(output_file.to_path_buf()),
        capture_file: Some(capture_file.to_path_buf()),
        canvas: Some(rect),
        processes: vec![CaptureProcess { pid }],
        segments: Vec::new(),
    }
}

fn run_cinnamon_capture_helper(request_json: &str) -> Result<()> {
    let request: CinnamonCaptureRequest =
        serde_json::from_str(request_json).context("invalid Cinnamon capture request")?;
    CINNAMON_STOP_REQUESTED.store(false, Ordering::Release);
    install_cinnamon_helper_signal_handlers()?;
    let cursor_guard = match XfixesCursorGuard::hide() {
        Ok(guard) => Some(guard),
        Err(error) => {
            eprintln!(
                "[qol-shot] XFixes cursor isolation unavailable, using Cinnamon's built-in cursor: {error:#}"
            );
            None
        }
    };
    let session = qol_cinnamon::Session::connect().map_err(anyhow::Error::msg)?;
    session
        .eval(&cinnamon_start_script(&request, cursor_guard.is_some())?)
        .map_err(anyhow::Error::msg)?;

    while !CINNAMON_STOP_REQUESTED.load(Ordering::Acquire) {
        std::thread::sleep(CINNAMON_POLL_INTERVAL);
    }

    let cursor_stats = session
        .eval(cinnamon_stop_script())
        .map_err(anyhow::Error::msg)?;
    drop(cursor_guard);
    let cursor_stats =
        serde_json::from_str::<String>(&cursor_stats).unwrap_or_else(|_| cursor_stats.clone());
    qol_runtime::probe!("SHOT_RECORD_CURSOR_STATS", "{cursor_stats}");
    Ok(())
}

struct XfixesCursorGuard {
    connection: RustConnection,
    root: u32,
}

impl XfixesCursorGuard {
    fn hide() -> Result<Self> {
        let (connection, screen_num) =
            x11rb::connect(None).context("failed to connect to X11 for cursor isolation")?;
        let root = connection
            .setup()
            .roots
            .get(screen_num)
            .context("X11 connection did not expose the selected screen")?
            .root;
        let version = connection
            .xfixes_query_version(6, 0)
            .context("failed to query XFixes")?
            .reply()
            .context("XFixes version query failed")?;
        if version.major_version < 4 {
            return Err(anyhow!(
                "XFixes {}.{} does not support cursor hiding",
                version.major_version,
                version.minor_version
            ));
        }
        connection
            .xfixes_hide_cursor(root)
            .context("failed to request XFixes cursor hiding")?
            .check()
            .context("XFixes rejected cursor hiding")?;
        connection
            .flush()
            .context("failed to flush XFixes cursor hiding")?;
        Ok(Self { connection, root })
    }

    fn show(&self) -> Result<()> {
        self.connection
            .xfixes_show_cursor(self.root)
            .context("failed to request XFixes cursor restore")?
            .check()
            .context("XFixes rejected cursor restore")?;
        self.connection
            .flush()
            .context("failed to flush XFixes cursor restore")
    }
}

impl Drop for XfixesCursorGuard {
    fn drop(&mut self) {
        if let Err(error) = self.show() {
            eprintln!("[qol-shot] failed to restore the X11 cursor: {error:#}");
        }
    }
}

fn install_cinnamon_helper_signal_handlers() -> Result<()> {
    for signal in [libc::SIGINT, libc::SIGTERM] {
        let previous = unsafe {
            libc::signal(
                signal,
                cinnamon_helper_signal as *const () as libc::sighandler_t,
            )
        };
        if previous == libc::SIG_ERR {
            return Err(std::io::Error::last_os_error())
                .context("failed to install Cinnamon helper signal handler");
        }
    }
    Ok(())
}

extern "C" fn cinnamon_helper_signal(_: libc::c_int) {
    CINNAMON_STOP_REQUESTED.store(true, Ordering::Release);
}

fn cinnamon_start_script(
    request: &CinnamonCaptureRequest,
    isolated_cursor: bool,
) -> Result<String> {
    let request =
        serde_json::to_string(request).context("failed to encode Cinnamon script data")?;
    Ok(format!(
        r#"(() => {{
const Cinnamon = imports.gi.Cinnamon;
const Clutter = imports.gi.Clutter;
const Meta = imports.gi.Meta;
const Main = imports.ui.main;
const Magnifier = imports.ui.magnifier;
const request = {request};
const isolatedCursor = {isolated_cursor};
if (global.__qolShotRecorder && global.__qolShotRecorder.is_recording())
    throw new Error("qol-shot recorder is already active");
if (global.__qolShotCursorOverlay) {{
    global.__qolShotCursorOverlay.destroy();
    global.__qolShotCursorOverlay = null;
}}
const tracker = Meta.CursorTracker.get_for_display(global.display);
const cursorContent = new Magnifier.MouseSpriteContent();
const cursorActor = new Clutter.Actor({{
    request_mode: Clutter.RequestMode.CONTENT_SIZE,
    reactive: false,
}});
cursorActor.content = cursorContent;
let cursorChangedId = 0;
let cursorMotionId = 0;
let cursorPaintId = 0;
let cursorDestroyed = false;
let cursorPositionChanges = 0;
let cursorMotionEvents = 0;
let cursorPaints = 0;
let cursorX = null;
let cursorY = null;
let cursorBaseScale = null;
const updateCursorSprite = () => {{
    cursorContent.texture = tracker.get_sprite();
    if (cursorBaseScale === null)
        cursorBaseScale = cursorContent._textureScale();
    cursorContent.monitorScale = cursorContent._textureScale() / cursorBaseScale;
    const [hotX, hotY] = tracker.get_hot();
    cursorActor.set_anchor_point(hotX, hotY);
}};
const setCursorPosition = (x, y) => {{
    if (x !== cursorX || y !== cursorY) {{
        cursorPositionChanges++;
        cursorX = x;
        cursorY = y;
    }}
    cursorActor.set_position(x, y);
}};
const updateCursorPosition = () => {{
    const [x, y] = global.get_pointer();
    setCursorPosition(x, y);
}};
const captureCursorPaint = () => {{
    cursorPaints++;
    updateCursorPosition();
}};
const captureCursorMotion = (_actor, event) => {{
    if (event.type() === Clutter.EventType.MOTION) {{
        const [x, y] = event.get_coords();
        cursorMotionEvents++;
        setCursorPosition(x, y);
    }}
    return Clutter.EVENT_PROPAGATE;
}};
const cursorOverlay = {{
    stats: () => `paints=${{cursorPaints}} events=${{cursorMotionEvents}} changes=${{cursorPositionChanges}}`,
    destroy: () => {{
        if (cursorDestroyed)
            return;
        cursorDestroyed = true;
        if (cursorPaintId)
            global.stage.disconnect(cursorPaintId);
        if (cursorMotionId)
            global.stage.disconnect(cursorMotionId);
        if (cursorChangedId)
            tracker.disconnect(cursorChangedId);
        cursorActor.destroy();
    }},
}};
try {{
    if (isolatedCursor) {{
        updateCursorSprite();
        updateCursorPosition();
        Main.uiGroup.add_child(cursorActor);
        Main.uiGroup.set_child_above_sibling(cursorActor, null);
        cursorChangedId = tracker.connect('cursor-changed', updateCursorSprite);
        cursorMotionId = global.stage.connect('captured-event', captureCursorMotion);
        cursorPaintId = global.stage.connect('before-paint', captureCursorPaint);
        global.__qolShotCursorOverlay = cursorOverlay;
    }}

    const recorder = new Cinnamon.Recorder({{stage: global.stage, display: global.display}});
    recorder.set_area(request.rect.x, request.rect.y, request.rect.w, request.rect.h);
    recorder.set_framerate(request.framerate);
    recorder.set_draw_cursor(!isolatedCursor);
    recorder.set_file_template(request.file_template);
    recorder.set_pipeline(request.pipeline);
    Meta.disable_unredirect_for_display(global.display);
    global.__qolShotUnredirectDisabled = true;
    const recordResult = recorder.record();
    const started = Array.isArray(recordResult) ? recordResult[0] : recordResult;
    if (!started)
        throw new Error("Cinnamon recorder rejected the capture pipeline");
    global.__qolShotRecorder = recorder;
    return true;
}} catch (error) {{
    cursorOverlay.destroy();
    global.__qolShotCursorOverlay = null;
    if (global.__qolShotUnredirectDisabled) {{
        Meta.enable_unredirect_for_display(global.display);
        global.__qolShotUnredirectDisabled = false;
    }}
    throw error;
}}
}})()"#
    ))
}

fn cinnamon_stop_script() -> &'static str {
    r#"(() => {
const Meta = imports.gi.Meta;
const recorder = global.__qolShotRecorder;
let cursorStats = "paints=0 events=0 changes=0";
try {
    if (recorder && recorder.is_recording())
        recorder.close();
} finally {
    global.__qolShotRecorder = null;
    if (global.__qolShotCursorOverlay) {
        cursorStats = global.__qolShotCursorOverlay.stats();
        global.__qolShotCursorOverlay.destroy();
        global.__qolShotCursorOverlay = null;
    }
    if (global.__qolShotUnredirectDisabled) {
        Meta.enable_unredirect_for_display(global.display);
        global.__qolShotUnredirectDisabled = false;
    }
}
return cursorStats;
})()"#
}

fn cinnamon_desktop_session() -> bool {
    ["XDG_CURRENT_DESKTOP", "DESKTOP_SESSION"]
        .into_iter()
        .filter_map(|name| std::env::var(name).ok())
        .any(|desktop| desktop.to_ascii_lowercase().contains("cinnamon"))
}

fn cinnamon_pipeline(config: &Config) -> String {
    let muxer = "webmmux";
    let video = format!(
        "vp8enc deadline=1 cpu-used=8 min-quantizer={quality} max-quantizer={quality} threads=%T ! queue",
        quality = config.video.crf.clamp(0, 63)
    );
    let audio = "opusenc bitrate=192000";
    let inputs = audio_inputs(config);
    if inputs.is_empty() {
        return format!("queue ! {video} ! {muxer}");
    }

    let mut pipeline = format!("{muxer} name=mux ! queue ");
    if inputs.len() == 1 {
        pipeline.push_str(&format!(
            "pulsesrc device={} do-timestamp=true ! audioconvert ! audioresample ! {audio} ! queue ! mux. ",
            gst_quote(&inputs[0])
        ));
    } else {
        pipeline.push_str(&format!(
            "audiomixer name=mix ! audioconvert ! audioresample ! {audio} ! queue ! mux. "
        ));
        for input in inputs {
            pipeline.push_str(&format!(
                "pulsesrc device={} do-timestamp=true ! audioconvert ! audioresample ! queue ! mix. ",
                gst_quote(&input)
            ));
        }
    }
    pipeline.push_str(&format!("queue ! {video} ! mux."));
    pipeline
}

fn normalized_h264_preset(preset: &str) -> &'static str {
    match preset {
        "ultrafast" => "ultrafast",
        "superfast" => "superfast",
        "faster" => "faster",
        "fast" => "fast",
        "medium" => "medium",
        "slow" => "slow",
        "slower" => "slower",
        "veryslow" => "veryslow",
        _ => "veryfast",
    }
}

fn audio_inputs(config: &Config) -> Vec<String> {
    if !config.audio.enabled {
        return Vec::new();
    }
    let mut inputs = Vec::new();
    if config.audio.inputs.iter().any(|input| input == "mic") {
        inputs.push(config.audio.mic_device.clone());
    }
    if config.audio.inputs.iter().any(|input| input == "system") {
        inputs.push(format!("{}.monitor", config.audio.system_device));
    }
    inputs
}

fn gst_quote(value: &str) -> String {
    format!(r#""{}""#, value.replace('\\', "\\\\").replace('"', "\\\""))
}

fn start_x11grab_capture(
    rect: &Rect,
    config: &Config,
    output_file: &Path,
) -> Result<CaptureSession> {
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

    let audio_inputs = audio_inputs(config);
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
        File::create(super::super::CAPTURE_LOG).context("failed to create recording log file")?;
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
    Ok(capture_session(child.id(), *rect, output_file, output_file))
}

pub fn capture_screenshot(rect: &Rect, output_file: &Path) -> Result<()> {
    let log_file =
        File::create(super::super::CAPTURE_LOG).context("failed to create capture log file")?;
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
    match format.to_ascii_lowercase().as_str() {
        "mkv" | "mp4" | "mov" | "webm" => format.to_ascii_lowercase(),
        _ => "mov".to_string(),
    }
}

pub fn recording_started(_session: &CaptureSession, countdown_completed: bool) {
    if countdown_completed {
        qol_runtime::probe!(
            "SHOT_RECORD_FEEDBACK",
            "stage=started surface=none reason=countdown-complete"
        );
        return;
    }
    show_notification("Recording started", "Press your hotkey to stop", 1200);
}

pub fn recording_stopped(session: &CaptureSession, config: &Config) -> Option<PathBuf> {
    show_notification("Recording stopped", "Saving recording", 1800);
    let output_file = session.output_file.as_deref()?;
    let capture_file = session.capture_file.as_deref().unwrap_or(output_file);
    if let Err(error) = wait_for_recording_file(session, capture_file) {
        eprintln!("[qol-shot] recording finalization failed: {error:#}");
        if discard_empty_capture(capture_file) {
            qol_runtime::probe!(
                "SHOT_RECORD_FINALIZE",
                "stage=failed reason=empty-capture removed=true"
            );
            show_notification("Recording failed", "No video frames were produced", 3000);
            return None;
        }
        show_notification(
            "Recording save delayed",
            "The recorder is still finalizing the file",
            3000,
        );
        return None;
    }
    let saved_file = if capture_file == output_file {
        output_file.to_path_buf()
    } else {
        match convert_cinnamon_recording(capture_file, output_file, config) {
            Ok(()) => output_file.to_path_buf(),
            Err(error) => {
                eprintln!("[qol-shot] Cinnamon recording conversion failed: {error:#}");
                show_notification(
                    "Recording conversion failed",
                    "Saved the synchronized WebM recording instead",
                    3000,
                );
                capture_file.to_path_buf()
            }
        }
    };
    let message = saved_file
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("Saved in Videos");
    crate::capture::completion::background_saved(
        "Recording saved",
        message,
        &saved_file,
        config.capture.open_folder_after_save,
    );
    Some(saved_file)
}

fn convert_cinnamon_recording(
    capture_file: &Path,
    output_file: &Path,
    config: &Config,
) -> Result<()> {
    let args = cinnamon_conversion_args(capture_file, output_file, config);

    let log_file = File::options()
        .create(true)
        .append(true)
        .open(super::super::CAPTURE_LOG)
        .context("failed to open recording conversion log")?;
    let stdout_log = log_file
        .try_clone()
        .context("failed to clone recording conversion log")?;
    let status = Command::new("ffmpeg")
        .args(&args)
        .stdin(Stdio::null())
        .stdout(Stdio::from(stdout_log))
        .stderr(Stdio::from(log_file))
        .status()
        .context("failed to convert synchronized recording")?;
    if !status.success() {
        return Err(anyhow!("ffmpeg recording conversion exited with {status}"));
    }
    std::fs::remove_file(capture_file).with_context(|| {
        format!(
            "failed to remove converted native recording {}",
            capture_file.display()
        )
    })?;
    qol_runtime::probe!(
        "SHOT_RECORD_FINALIZE",
        "stage=converted backend=cinnamon_after_paint format={}",
        recording_format(&config.video.format)
    );
    Ok(())
}

fn cinnamon_conversion_args(
    capture_file: &Path,
    output_file: &Path,
    config: &Config,
) -> Vec<String> {
    let format = recording_format(&config.video.format);
    let mut args = vec![
        "-y".to_string(),
        "-i".to_string(),
        capture_file.to_string_lossy().to_string(),
    ];
    if format == "webm" {
        args.extend(["-c:v", "libvpx-vp9", "-b:v", "0", "-crf"].map(str::to_string));
        args.push(config.video.crf.clamp(0, 63).to_string());
        args.extend(["-c:a", "libopus", "-b:a", "192k"].map(str::to_string));
    } else {
        args.extend(["-c:v", "libx264", "-crf"].map(str::to_string));
        args.push(config.video.crf.clamp(0, 51).to_string());
        args.extend(["-preset", normalized_h264_preset(&config.video.preset)].map(str::to_string));
        args.extend(["-pix_fmt", "yuv420p", "-c:a", "aac", "-b:a", "192k"].map(str::to_string));
        if matches!(format.as_str(), "mp4" | "mov") {
            args.extend(["-movflags", "+faststart"].map(str::to_string));
        }
    }
    args.extend(["-fps_mode", "passthrough"].map(str::to_string));
    args.push(output_file.to_string_lossy().to_string());
    args
}

fn discard_empty_capture(capture_file: &Path) -> bool {
    let Ok(metadata) = capture_file.symlink_metadata() else {
        return false;
    };
    if !metadata.file_type().is_file() || metadata.len() != 0 {
        return false;
    }
    match std::fs::remove_file(capture_file) {
        Ok(()) => true,
        Err(error) => {
            eprintln!(
                "[qol-shot] failed to remove empty capture {}: {error}",
                capture_file.display()
            );
            false
        }
    }
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
        super::super::unix::signal_process(process.pid, libc::SIGINT)
            .context("failed to send SIGINT to capture process")?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{
        cinnamon_capture_file, cinnamon_pipeline, cinnamon_start_script, gst_quote,
        recording_format, CinnamonCaptureRequest,
    };
    use crate::{Config, Rect};

    #[test]
    fn cinnamon_pipeline_uses_portable_webm_for_every_audio_shape() {
        let cases = [
            (false, &[][..], None),
            (true, &["mic"][..], Some("pulsesrc")),
            (true, &["mic", "system"][..], Some("audiomixer")),
            (true, &["system"][..], Some("opusenc")),
        ];
        for (enabled, inputs, audio) in cases {
            let mut config = Config::default();
            config.audio.enabled = enabled;
            config.audio.inputs = inputs.iter().map(|input| (*input).to_string()).collect();
            let pipeline = cinnamon_pipeline(&config);
            assert!(pipeline.contains("webmmux"), "{pipeline}");
            assert!(pipeline.contains("vp8enc"), "{pipeline}");
            if let Some(audio) = audio {
                assert!(pipeline.contains(audio), "{pipeline}");
            } else {
                assert!(!pipeline.contains("pulsesrc"), "{pipeline}");
            }
        }
    }

    #[test]
    fn cinnamon_capture_file_is_hidden_webm_beside_requested_output() {
        assert_eq!(
            cinnamon_capture_file(std::path::Path::new("/tmp/recording.mp4")),
            std::path::Path::new("/tmp/.recording.cinnamon.webm")
        );
    }

    #[test]
    fn gstreamer_device_quoting_escapes_pipeline_syntax() {
        assert_eq!(
            gst_quote(r#"pulse\device"name"#),
            r#""pulse\\device\"name""#
        );
    }

    #[test]
    fn cinnamon_script_embeds_request_as_json_data() {
        let request = CinnamonCaptureRequest {
            rect: Rect {
                x: -10,
                y: 20,
                w: 800,
                h: 600,
            },
            framerate: 60,
            file_template: r#"/tmp/a "quoted" recording.mov"#.to_string(),
            pipeline: r#"queue ! x264enc option-string="value" ! qtmux"#.to_string(),
        };
        let script = cinnamon_start_script(&request, true).unwrap();
        assert!(script.contains(r#""file_template":"/tmp/a \"quoted\" recording.mov""#));
        assert!(script.contains("recorder.set_pipeline(request.pipeline)"));
        assert!(script.contains("Array.isArray(recordResult)"));
        assert!(script.contains("const isolatedCursor = true"));
        assert!(script.contains("recorder.set_draw_cursor(!isolatedCursor)"));
        assert!(script.contains("new Magnifier.MouseSpriteContent()"));
        assert!(script.contains("global.stage.connect('captured-event', captureCursorMotion)"));
        assert!(script.contains("global.stage.connect('before-paint', captureCursorPaint)"));
        assert!(script.contains("cursorContent._textureScale() / cursorBaseScale"));
        assert!(!script.contains("tracker.set_pointer_visible"));
    }

    #[test]
    fn cinnamon_script_can_fall_back_to_builtin_cursor() {
        let request = CinnamonCaptureRequest {
            rect: Rect {
                x: 0,
                y: 0,
                w: 800,
                h: 600,
            },
            framerate: 60,
            file_template: "/tmp/fallback.webm".to_string(),
            pipeline: "queue ! vp8enc ! webmmux".to_string(),
        };

        let script = cinnamon_start_script(&request, false).unwrap();

        assert!(script.contains("const isolatedCursor = false"));
        assert!(script.contains("recorder.set_draw_cursor(!isolatedCursor)"));
    }

    #[test]
    fn cinnamon_conversion_preserves_capture_timestamps() {
        let config = Config::default();
        let args = super::cinnamon_conversion_args(
            std::path::Path::new("/tmp/input.webm"),
            std::path::Path::new("/tmp/output.mp4"),
            &config,
        );

        assert!(args
            .windows(2)
            .any(|pair| pair == ["-fps_mode", "passthrough"]));
    }

    #[test]
    fn recording_format_normalizes_supported_values_and_fallback() {
        let cases = [
            ("MP4", "mp4"),
            ("mkv", "mkv"),
            ("MOV", "mov"),
            ("WebM", "webm"),
            ("avi", "mov"),
        ];
        for (input, expected) in cases {
            assert_eq!(recording_format(input), expected, "{input}");
        }
    }
}
