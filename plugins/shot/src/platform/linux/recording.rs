use anyhow::{anyhow, Context, Result};
use serde::{Deserialize, Serialize};
use std::fs::File;
use std::io::Write;
use std::os::unix::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, ExitCode, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::OnceLock;
use std::time::{Duration, Instant};
use x11rb::connection::Connection;
use x11rb::protocol::xfixes::ConnectionExt as XfixesExt;
use x11rb::rust_connection::RustConnection;

use crate::capture::geometry::even_dimensions;
use crate::platform::{CaptureProcess, CaptureSession, SavedRecording};
use crate::{Config, Rect};

use super::super::recorder;
pub use super::super::recorder::recording_format;
use qol_plugin_daemon::notification::send_notification;

const CINNAMON_HELPER_ENV: &str = "QOL_SHOT_CINNAMON_CAPTURE_REQUEST";
const HELPER_READY_TIMEOUT: Duration = Duration::from_secs(3);
const HELPER_POLL_INTERVAL: Duration = Duration::from_millis(40);
const NVFBC_HELPER_ENV: &str = "QOL_SHOT_NVFBC_CAPTURE_REQUEST";
const NVFBC_FINALIZE_TIMEOUT: Duration = Duration::from_secs(10);
static CINNAMON_STOP_REQUESTED: AtomicBool = AtomicBool::new(false);
const CINNAMON_VP8_THREADS: u32 = 4;
const NVENC_ELEMENT: &str = "nvh264enc";
const NVENC_MIN_SIZE: (i32, i32) = (145, 49);
const NVENC_MAX_SIDE: i32 = 4096;
const FFMPEG_NVENC: &str = "h264_nvenc";
static FFMPEG_NVENC_WORKS: OnceLock<bool> = OnceLock::new();

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum X11grabEncoder {
    Nvenc,
    X264,
}

impl X11grabEncoder {
    fn label(self) -> &'static str {
        match self {
            Self::Nvenc => FFMPEG_NVENC,
            Self::X264 => "libx264",
        }
    }

    fn args(self, config: &Config) -> Vec<String> {
        match self {
            Self::Nvenc => vec![
                "-c:v".to_string(),
                FFMPEG_NVENC.to_string(),
                "-preset".to_string(),
                "p4".to_string(),
                "-rc".to_string(),
                "vbr".to_string(),
                "-cq".to_string(),
                config.video.crf.clamp(0, 51).to_string(),
                "-b:v".to_string(),
                "0".to_string(),
            ],
            Self::X264 if recording_format(&config.video.format) == "webm" => {
                recorder::encoder_args(config, true)
            }
            Self::X264 => vec![
                "-c:v".to_string(),
                "libx264".to_string(),
                "-crf".to_string(),
                config.video.crf.clamp(0, 51).to_string(),
                "-preset".to_string(),
                config.video.preset.clone(),
                "-pix_fmt".to_string(),
                "yuv420p".to_string(),
            ],
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum CinnamonEncoder {
    Nvenc,
    Vp8,
}

impl CinnamonEncoder {
    fn label(self) -> &'static str {
        match self {
            Self::Nvenc => NVENC_ELEMENT,
            Self::Vp8 => "vp8enc",
        }
    }

    fn extension(self) -> &'static str {
        match self {
            Self::Nvenc => "mkv",
            Self::Vp8 => "webm",
        }
    }

    fn from_capture_file(capture_file: &Path) -> Self {
        if capture_file
            .extension()
            .is_some_and(|extension| extension == "mkv")
        {
            Self::Nvenc
        } else {
            Self::Vp8
        }
    }
}

#[derive(Debug, Deserialize, Serialize)]
struct CinnamonCaptureRequest {
    rect: Rect,
    framerate: u32,
    file_template: String,
    pipeline: String,
}

#[derive(Debug, Deserialize, Serialize)]
struct NvfbcCaptureRequest {
    rect: Rect,
    framerate: u32,
    ffmpeg_args: Vec<String>,
}

pub(super) fn run_internal_capture_helper() -> Option<ExitCode> {
    if let Ok(request) = std::env::var(NVFBC_HELPER_ENV) {
        return Some(match run_nvfbc_capture_helper(&request) {
            Ok(()) => ExitCode::SUCCESS,
            Err(error) => {
                log::error!("NvFBC capture helper failed: {error:#}");
                ExitCode::FAILURE
            }
        });
    }
    let request = std::env::var(CINNAMON_HELPER_ENV).ok()?;
    Some(match run_cinnamon_capture_helper(&request) {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            log::error!("Cinnamon capture helper failed: {error:#}");
            ExitCode::FAILURE
        }
    })
}

pub fn start_capture(rect: &Rect, config: &Config, output_file: &Path) -> Result<CaptureSession> {
    let rect = &even_dimensions(*rect);
    if nvenc_accepts(rect) && ffmpeg_nvenc_available(config) {
        match start_nvfbc_capture(rect, config, output_file) {
            Ok(session) => {
                qol_runtime::probe!(
                    "SHOT_RECORD_START_BACKEND",
                    "backend=nvfbc outcome=ready encoder={FFMPEG_NVENC}"
                );
                return Ok(session);
            }
            Err(error) => {
                qol_runtime::probe!(
                    "SHOT_RECORD_START_BACKEND",
                    "backend=nvfbc outcome=fallback reason=start_failed"
                );
                log::warn!("NvFBC capture unavailable, using x11grab: {error:#}");
            }
        }
        qol_runtime::probe!(
            "SHOT_RECORD_START_BACKEND",
            "backend=x11grab outcome=ready reason=gpu_encoder encoder={FFMPEG_NVENC}"
        );
        return start_x11grab_capture(rect, config, output_file, X11grabEncoder::Nvenc);
    }
    if cinnamon_desktop_session() {
        for encoder in cinnamon_encoders(rect) {
            match start_cinnamon_capture(rect, config, output_file, encoder) {
                Ok(session) => {
                    qol_runtime::probe!(
                        "SHOT_RECORD_START_BACKEND",
                        "backend=cinnamon_after_paint outcome=ready encoder={}",
                        encoder.label()
                    );
                    return Ok(session);
                }
                Err(error) => {
                    qol_runtime::probe!(
                        "SHOT_RECORD_START_BACKEND",
                        "backend=cinnamon_after_paint outcome=fallback reason=start_failed encoder={}",
                        encoder.label()
                    );
                    log::warn!(
                        "Cinnamon synchronized capture with {} unavailable: {error:#}",
                        encoder.label()
                    );
                }
            }
        }
    }

    qol_runtime::probe!(
        "SHOT_RECORD_START_BACKEND",
        "backend=x11grab outcome=ready reason={} encoder={}",
        if cinnamon_desktop_session() {
            "cinnamon_start_failed"
        } else {
            "non_cinnamon_session"
        },
        X11grabEncoder::X264.label()
    );
    start_x11grab_capture(rect, config, output_file, X11grabEncoder::X264)
}

fn ffmpeg_nvenc_available(config: &Config) -> bool {
    recording_format(&config.video.format) != "webm"
        && *FFMPEG_NVENC_WORKS.get_or_init(ffmpeg_encodes_with_nvenc)
}

fn ffmpeg_encodes_with_nvenc() -> bool {
    Command::new("ffmpeg")
        .args([
            "-hide_banner",
            "-loglevel",
            "error",
            "-f",
            "lavfi",
            "-i",
            "color=c=black:s=256x256:d=0.1",
            "-frames:v",
            "1",
            "-c:v",
            FFMPEG_NVENC,
            "-f",
            "null",
            "-",
        ])
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .is_ok_and(|status| status.success())
}

fn cinnamon_encoders(rect: &Rect) -> Vec<CinnamonEncoder> {
    let mut encoders = Vec::new();
    if nvenc_accepts(rect) && cinnamon_has_element(NVENC_ELEMENT) {
        encoders.push(CinnamonEncoder::Nvenc);
    }
    encoders.push(CinnamonEncoder::Vp8);
    encoders
}

fn nvenc_accepts(rect: &Rect) -> bool {
    (NVENC_MIN_SIZE.0..=NVENC_MAX_SIDE).contains(&rect.w)
        && (NVENC_MIN_SIZE.1..=NVENC_MAX_SIDE).contains(&rect.h)
}

fn cinnamon_has_element(element: &str) -> bool {
    let script = format!(
        r#"(() => {{
try {{
    const Gst = imports.gi.Gst;
    Gst.init(null);
    return Gst.ElementFactory.find({element:?}) !== null;
}} catch (error) {{
    return false;
}}
}})()"#
    );
    match qol_cinnamon::Session::connect().and_then(|session| session.eval(&script)) {
        Ok(found) => found == "true",
        Err(error) => {
            log::warn!("failed to look up the {element} GStreamer element in Cinnamon: {error}");
            false
        }
    }
}

fn start_cinnamon_capture(
    rect: &Rect,
    config: &Config,
    output_file: &Path,
    encoder: CinnamonEncoder,
) -> Result<CaptureSession> {
    let capture_file = cinnamon_capture_file(output_file, encoder);
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
        pipeline: cinnamon_pipeline(config, encoder),
    };
    let request_json =
        serde_json::to_string(&request).context("failed to encode capture request")?;

    qol_runtime::probe!(
        "SHOT_RECORD_START_PLAN",
        "backend=cinnamon_after_paint rect={}x{}+{},{} fps={} format={} encoder={} audio_inputs={} cursor=xfixes_actor_fallback_builtin cursor_update=before_paint",
        rect.w,
        rect.h,
        rect.x,
        rect.y,
        request.framerate,
        format,
        encoder.label(),
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

    if wait_for_helper_ready(&mut child, &capture_file, "cinnamon_after_paint")? {
        return Ok(capture_session(
            child.id(),
            *rect,
            output_file,
            &capture_file,
        ));
    }

    stop_failed_helper_start(&mut child);
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

fn start_nvfbc_capture(rect: &Rect, config: &Config, output_file: &Path) -> Result<CaptureSession> {
    if output_file.symlink_metadata().is_ok() {
        return Err(anyhow!(
            "refusing to replace existing capture output {}",
            output_file.display()
        ));
    }
    let framerate = config.video.framerate.clamp(1, 240);
    let request = NvfbcCaptureRequest {
        rect: *rect,
        framerate,
        ffmpeg_args: nvfbc_ffmpeg_args(rect, framerate, config, output_file),
    };
    let request_json =
        serde_json::to_string(&request).context("failed to encode capture request")?;

    qol_runtime::probe!(
        "SHOT_RECORD_START_PLAN",
        "backend=nvfbc rect={}x{}+{},{} fps={} encoder={FFMPEG_NVENC} audio_inputs={}",
        rect.w,
        rect.h,
        rect.x,
        rect.y,
        framerate,
        audio_inputs(config).len()
    );

    let log_file =
        File::create(super::super::CAPTURE_LOG).context("failed to create recording log file")?;
    let stdout_log = log_file
        .try_clone()
        .context("failed to clone recording log file")?;
    let executable = std::env::current_exe().context("failed to resolve qol-shot executable")?;
    let mut child = Command::new(executable)
        .env(NVFBC_HELPER_ENV, request_json)
        .stdin(Stdio::null())
        .stdout(Stdio::from(stdout_log))
        .stderr(Stdio::from(log_file))
        .spawn()
        .context("failed to start NvFBC capture helper")?;

    if wait_for_helper_ready(&mut child, output_file, "nvfbc")? {
        return Ok(capture_session(child.id(), *rect, output_file, output_file));
    }

    stop_failed_helper_start(&mut child);
    if child.try_wait().ok().flatten().is_none() {
        let _ = child.kill();
        let _ = child.wait();
    }
    if output_file.symlink_metadata().is_ok() {
        std::fs::remove_file(output_file).with_context(|| {
            format!(
                "failed to remove incomplete NvFBC capture {}",
                output_file.display()
            )
        })?;
    }
    Err(anyhow!("NvFBC capture did not create its output file"))
}

fn nvfbc_ffmpeg_args(
    rect: &Rect,
    framerate: u32,
    config: &Config,
    output_file: &Path,
) -> Vec<String> {
    let input = vec![
        "-thread_queue_size".to_string(),
        "512".to_string(),
        "-use_wallclock_as_timestamps".to_string(),
        "1".to_string(),
        "-f".to_string(),
        "rawvideo".to_string(),
        "-pix_fmt".to_string(),
        "bgra".to_string(),
        "-video_size".to_string(),
        format!("{}x{}", rect.w, rect.h),
        "-framerate".to_string(),
        framerate.to_string(),
        "-i".to_string(),
        "pipe:0".to_string(),
    ];
    let mut args = ffmpeg_args(input, config, X11grabEncoder::Nvenc);
    args.extend(["-fps_mode", "cfr", "-shortest"].map(str::to_string));
    args.push(output_file.to_string_lossy().to_string());
    args
}

fn run_nvfbc_capture_helper(request_json: &str) -> Result<()> {
    let request: NvfbcCaptureRequest =
        serde_json::from_str(request_json).context("invalid NvFBC capture request")?;
    let stop = qol_process::CancellationToken::install()
        .context("failed to listen for the recording stop request")?;
    let mut capture = super::nvfbc::Capture::open(&request.rect)?;
    qol_runtime::probe!(
        "SHOT_RECORD_HELPER",
        "backend=nvfbc step=open output={}",
        capture.output_name()
    );
    let mut command = Command::new("ffmpeg");
    command
        .args(&request.ffmpeg_args)
        .env_remove(NVFBC_HELPER_ENV)
        .stdin(Stdio::piped());
    // SAFETY: prctl is async-signal-safe and only changes the forked child before exec.
    unsafe {
        command.pre_exec(|| {
            if libc::prctl(libc::PR_SET_PDEATHSIG, libc::SIGKILL, 0, 0, 0) == -1 {
                return Err(std::io::Error::last_os_error());
            }
            Ok(())
        });
    }
    let mut ffmpeg = command.spawn().context("failed to start ffmpeg")?;
    let stdin = ffmpeg
        .stdin
        .take()
        .ok_or_else(|| anyhow!("ffmpeg has no frame input"))?;
    let pumped = pump_nvfbc_frames(&mut capture, stdin, request.framerate, &stop);
    drop(capture);
    let finished = finish_nvfbc_ffmpeg(&mut ffmpeg);
    let frames = pumped?;
    qol_runtime::probe!(
        "SHOT_RECORD_HELPER",
        "backend=nvfbc step=exit frames={frames}"
    );
    finished
}

fn pump_nvfbc_frames(
    capture: &mut super::nvfbc::Capture,
    mut stdin: std::process::ChildStdin,
    framerate: u32,
    stop: &qol_process::CancellationToken,
) -> Result<u64> {
    let period = Duration::from_secs(1) / framerate;
    let mut next = Instant::now();
    let mut frames = 0;
    while !stop.is_cancelled() {
        let frame = capture.grab()?;
        stdin
            .write_all(frame)
            .context("ffmpeg stopped reading frames")?;
        frames += 1;
        next += period;
        let now = Instant::now();
        if next > now {
            std::thread::sleep(next - now);
        } else if now - next > period {
            next = now;
        }
    }
    Ok(frames)
}

fn finish_nvfbc_ffmpeg(ffmpeg: &mut Child) -> Result<()> {
    let deadline = Instant::now() + NVFBC_FINALIZE_TIMEOUT;
    let mut interrupted = false;
    loop {
        if let Some(status) = ffmpeg.try_wait().context("failed to inspect ffmpeg")? {
            if status.success() || interrupted {
                return Ok(());
            }
            return Err(anyhow!("ffmpeg exited with {status}"));
        }
        if Instant::now() >= deadline {
            if interrupted {
                let _ = ffmpeg.kill();
                return Err(anyhow!("ffmpeg did not finish the recording"));
            }
            interrupted = true;
            let _ = super::super::unix::signal_process(ffmpeg.id(), libc::SIGINT);
        }
        std::thread::sleep(HELPER_POLL_INTERVAL);
    }
}

fn wait_for_helper_ready(child: &mut Child, output_file: &Path, backend: &str) -> Result<bool> {
    let deadline = Instant::now() + HELPER_READY_TIMEOUT;
    while Instant::now() < deadline {
        if child
            .try_wait()
            .context("failed to inspect the capture helper")?
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
                "backend={backend} len={}",
                metadata.len()
            );
            true
        }) {
            return Ok(true);
        }
        std::thread::sleep(HELPER_POLL_INTERVAL);
    }
    Ok(false)
}

fn stop_failed_helper_start(child: &mut Child) {
    if child.try_wait().ok().flatten().is_some() {
        return;
    }
    let _ = super::super::unix::signal_process(child.id(), libc::SIGINT);
    let deadline = Instant::now() + Duration::from_secs(2);
    while Instant::now() < deadline {
        if child.try_wait().ok().flatten().is_some() {
            return;
        }
        std::thread::sleep(HELPER_POLL_INTERVAL);
    }
}

fn cinnamon_capture_file(output_file: &Path, encoder: CinnamonEncoder) -> PathBuf {
    let stem = output_file
        .file_stem()
        .map(|stem| stem.to_string_lossy())
        .unwrap_or_else(|| "recording".into());
    output_file.with_file_name(format!(".{stem}.cinnamon.{}", encoder.extension()))
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
        processes: vec![CaptureProcess::spawned(pid)],
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
            log::warn!(
                "XFixes cursor isolation unavailable, using Cinnamon's built-in cursor: {error:#}"
            );
            None
        }
    };
    let session = qol_cinnamon::Session::connect().map_err(anyhow::Error::msg)?;
    session
        .eval(&cinnamon_start_script(&request, cursor_guard.is_some())?)
        .map_err(anyhow::Error::msg)?;

    while !CINNAMON_STOP_REQUESTED.load(Ordering::Acquire) {
        std::thread::sleep(HELPER_POLL_INTERVAL);
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
            log::warn!("failed to restore the X11 cursor: {error:#}");
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

fn cinnamon_pipeline(config: &Config, encoder: CinnamonEncoder) -> String {
    let (muxer, video) = match encoder {
        CinnamonEncoder::Nvenc => (
            "matroskamux",
            format!(
                "video/x-raw,format=BGRA ! {NVENC_ELEMENT} preset=low-latency-hq rc-mode=vbr const-quality={quality} zerolatency=true ! h264parse ! queue",
                quality = config.video.crf.clamp(0, 51)
            ),
        ),
        CinnamonEncoder::Vp8 => (
            "webmmux",
            format!(
                "vp8enc deadline=1 cpu-used=8 min-quantizer={quality} max-quantizer={quality} threads={CINNAMON_VP8_THREADS} ! queue",
                quality = config.video.crf.clamp(0, 63)
            ),
        ),
    };
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
    encoder: X11grabEncoder,
) -> Result<CaptureSession> {
    let args = x11grab_args(rect, config, output_file, encoder);
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

fn x11grab_args(
    rect: &Rect,
    config: &Config,
    output_file: &Path,
    encoder: X11grabEncoder,
) -> Vec<String> {
    let input = vec![
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
    let mut args = ffmpeg_args(input, config, encoder);
    args.push(output_file.to_string_lossy().to_string());
    args
}

fn ffmpeg_args(video_input: Vec<String>, config: &Config, encoder: X11grabEncoder) -> Vec<String> {
    let mut args = video_input;
    let audio_inputs = audio_inputs(config);
    for input in &audio_inputs {
        args.extend(["-thread_queue_size", "128", "-f", "pulse", "-i"].map(str::to_string));
        args.push(input.clone());
    }
    if audio_inputs.len() == 2 {
        args.extend(
            [
                "-filter_complex",
                "[1:a][2:a]amix=inputs=2:duration=longest,volume=2[aout]",
                "-map",
                "0:v",
                "-map",
                "[aout]",
            ]
            .map(str::to_string),
        );
    }
    if !audio_inputs.is_empty() && recording_format(&config.video.format) != "webm" {
        args.extend(["-c:a", "aac", "-b:a", "192k"].map(str::to_string));
    }

    args.extend(encoder.args(config));
    args.extend(["-r".to_string(), config.video.framerate.to_string()]);
    args
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

pub fn recording_started(_session: &CaptureSession) {}

pub fn recording_stopped(session: &CaptureSession, config: &Config) -> Option<SavedRecording> {
    qol_plugin_daemon::notification::send_banner("Recording stopped", "Saving recording", None);
    let output_file = session.output_file.as_deref()?;
    let capture_file = session.capture_file.as_deref().unwrap_or(output_file);
    recorder::await_capture_file(session, capture_file)?;
    let saved_file = if capture_file == output_file {
        output_file.to_path_buf()
    } else {
        match convert_cinnamon_recording(capture_file, output_file, config) {
            Ok(()) => output_file.to_path_buf(),
            Err(error) => {
                log::warn!("Cinnamon recording conversion failed: {error:#}");
                send_notification(
                    "Recording conversion failed",
                    "Saved the unconverted recording instead",
                );
                capture_file.to_path_buf()
            }
        }
    };
    let message = saved_file
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("Saved in Videos")
        .to_string();
    Some(SavedRecording {
        path: saved_file,
        message,
    })
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
    let mut args = vec![
        "-y".to_string(),
        "-i".to_string(),
        capture_file.to_string_lossy().to_string(),
    ];
    let copied = match CinnamonEncoder::from_capture_file(capture_file) {
        CinnamonEncoder::Nvenc => h264_copy_args(config),
        CinnamonEncoder::Vp8 => None,
    };
    args.extend(copied.unwrap_or_else(|| recorder::encoder_args(config, false)));
    args.extend(["-fps_mode", "passthrough"].map(str::to_string));
    args.push(output_file.to_string_lossy().to_string());
    args
}

fn h264_copy_args(config: &Config) -> Option<Vec<String>> {
    let format = recording_format(&config.video.format);
    if format == "webm" {
        return None;
    }
    let mut args = ["-c:v", "copy"].map(str::to_string).to_vec();
    args.extend(recorder::h264_container_args(&format));
    Some(args)
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
        cinnamon_capture_file, cinnamon_pipeline, cinnamon_start_script, gst_quote, nvenc_accepts,
        nvfbc_ffmpeg_args, x11grab_args, CinnamonCaptureRequest, CinnamonEncoder, X11grabEncoder,
    };
    use crate::{Config, Rect};

    #[test]
    fn cinnamon_pipeline_fits_every_encoder_and_audio_shape() {
        let encoders = [
            (CinnamonEncoder::Vp8, "webmmux", "vp8enc deadline=1 cpu-used=8 min-quantizer=18 max-quantizer=18 threads=4 "),
            (CinnamonEncoder::Nvenc, "matroskamux", "video/x-raw,format=BGRA ! nvh264enc preset=low-latency-hq rc-mode=vbr const-quality=18 zerolatency=true ! h264parse"),
        ];
        let cases = [
            (false, &[][..], None),
            (true, &["mic"][..], Some("pulsesrc")),
            (true, &["mic", "system"][..], Some("audiomixer")),
            (true, &["system"][..], Some("opusenc")),
        ];
        for (encoder, muxer, video) in encoders {
            for (enabled, inputs, audio) in cases {
                let mut config = Config::default();
                config.video.crf = 18;
                config.audio.enabled = enabled;
                config.audio.inputs = inputs.iter().map(|input| (*input).to_string()).collect();
                let pipeline = cinnamon_pipeline(&config, encoder);
                assert!(pipeline.contains(muxer), "{pipeline}");
                assert!(pipeline.contains(video), "{pipeline}");
                if let Some(audio) = audio {
                    assert!(pipeline.contains(audio), "{pipeline}");
                } else {
                    assert!(!pipeline.contains("pulsesrc"), "{pipeline}");
                }
            }
        }
    }

    #[test]
    fn cinnamon_capture_file_is_hidden_beside_requested_output_and_names_its_encoder() {
        let output = std::path::Path::new("/tmp/recording.mp4");
        for (encoder, expected) in [
            (CinnamonEncoder::Vp8, "/tmp/.recording.cinnamon.webm"),
            (CinnamonEncoder::Nvenc, "/tmp/.recording.cinnamon.mkv"),
        ] {
            let capture_file = cinnamon_capture_file(output, encoder);
            assert_eq!(capture_file, std::path::Path::new(expected));
            assert_eq!(CinnamonEncoder::from_capture_file(&capture_file), encoder);
        }
    }

    #[test]
    fn nvenc_accepts_only_sizes_its_sink_caps_allow() {
        let cases = [
            ((2560, 1440), true),
            ((145, 49), true),
            ((4096, 4096), true),
            ((144, 1440), false),
            ((2560, 48), false),
            ((4480, 1440), false),
            ((2560, 4098), false),
        ];
        for ((w, h), expected) in cases {
            let rect = Rect { x: 0, y: 0, w, h };
            assert_eq!(nvenc_accepts(&rect), expected, "{w}x{h}");
        }
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
    fn cinnamon_conversion_copies_nvenc_video_unless_the_target_is_webm() {
        let cases = [
            ("/tmp/.r.cinnamon.mkv", "mp4", "copy"),
            ("/tmp/.r.cinnamon.mkv", "mkv", "copy"),
            ("/tmp/.r.cinnamon.mkv", "webm", "libvpx-vp9"),
            ("/tmp/.r.cinnamon.webm", "mp4", "libx264"),
        ];
        for (capture_file, format, codec) in cases {
            let mut config = Config::default();
            config.video.format = format.to_string();
            let args = super::cinnamon_conversion_args(
                std::path::Path::new(capture_file),
                std::path::Path::new("/tmp/output"),
                &config,
            );
            assert!(
                args.windows(2).any(|pair| pair == ["-c:v", codec]),
                "{capture_file} -> {format}: {args:?}"
            );
        }
    }

    #[test]
    fn x11grab_args_pick_the_encoder_and_mix_both_audio_inputs() {
        let rect = Rect {
            x: 1920,
            y: 0,
            w: 2560,
            h: 1440,
        };
        let mut config = Config::default();
        config.video.crf = 18;
        config.audio.enabled = true;
        config.audio.inputs = vec!["mic".to_string(), "system".to_string()];
        let output = std::path::Path::new("/tmp/out.mp4");
        let cases = [
            (
                X11grabEncoder::Nvenc,
                &[["-c:v", "h264_nvenc"], ["-cq", "18"], ["-b:v", "0"]][..],
                "-pix_fmt",
            ),
            (
                X11grabEncoder::X264,
                &[["-c:v", "libx264"], ["-crf", "18"], ["-pix_fmt", "yuv420p"]][..],
                "-cq",
            ),
        ];
        for (encoder, expected, absent) in cases {
            let args = x11grab_args(&rect, &config, output, encoder);
            let joined = args.join(" ");
            assert!(joined.contains(":0.0+1920,0"), "{joined}");
            assert!(
                joined.contains("[1:a][2:a]amix=inputs=2:duration=longest,volume=2[aout]"),
                "{joined}"
            );
            for pair in expected {
                assert!(args.windows(2).any(|window| window == pair), "{joined}");
            }
            assert!(!args.iter().any(|arg| arg == absent), "{joined}");
            assert_eq!(args.last().map(String::as_str), Some("/tmp/out.mp4"));
        }
    }

    #[test]
    fn nvfbc_args_read_raw_frames_from_stdin_and_stop_with_them() {
        let rect = Rect {
            x: 1920,
            y: 0,
            w: 2560,
            h: 1440,
        };
        let mut config = Config::default();
        config.audio.enabled = true;
        config.audio.inputs = vec!["system".to_string()];
        let args = nvfbc_ffmpeg_args(&rect, 60, &config, std::path::Path::new("/tmp/out.mp4"));
        let joined = args.join(" ");
        for pair in [
            ["-f", "rawvideo"],
            ["-pix_fmt", "bgra"],
            ["-video_size", "2560x1440"],
            ["-framerate", "60"],
            ["-i", "pipe:0"],
            ["-use_wallclock_as_timestamps", "1"],
            ["-f", "pulse"],
            ["-c:v", "h264_nvenc"],
            ["-fps_mode", "cfr"],
        ] {
            assert!(args.windows(2).any(|window| window == pair), "{joined}");
        }
        assert!(args.iter().any(|arg| arg == "-shortest"), "{joined}");
        assert!(!joined.contains("x11grab"), "{joined}");
        assert_eq!(args.last().map(String::as_str), Some("/tmp/out.mp4"));
    }

    #[test]
    fn x11grab_args_use_container_codecs_and_clamp_crf() {
        let rect = Rect {
            x: 0,
            y: 0,
            w: 1280,
            h: 720,
        };
        let cases = [
            ("webm", "libvpx-vp9", "libopus", "63"),
            ("mp4", "libx264", "aac", "51"),
        ];
        for (format, video, audio, crf) in cases {
            let mut config = Config::default();
            config.video.format = format.to_string();
            config.video.crf = 99;
            config.audio.enabled = true;
            config.audio.inputs = vec!["mic".to_string()];
            let output = std::path::Path::new("/tmp/out");
            let args = x11grab_args(&rect, &config, output, X11grabEncoder::X264);
            let has = |pair: [&str; 2]| args.windows(2).any(|window| window == pair);
            assert!(has(["-c:v", video]), "{format}: {args:?}");
            assert!(has(["-c:a", audio]), "{format}: {args:?}");
            assert!(has(["-crf", crf]), "{format}: {args:?}");
            assert_eq!(
                args.iter().filter(|arg| *arg == "-c:a").count(),
                1,
                "{format}"
            );
            assert_eq!(args.iter().any(|arg| arg == "-preset"), format == "mp4");
        }
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
}
