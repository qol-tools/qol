use anyhow::{anyhow, Context, Result};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Condvar, Mutex};
use std::time::{Duration, Instant};

use crate::capture::frozen_frame::{FrozenCrop, FrozenFrame};
use crate::{capture::geometry, platform, Rect};

const PREVIEW_MAX_WIDTH: u32 = 360;
const PREVIEW_MAX_HEIGHT: u32 = 240;

pub fn capture_screenshot() -> Result<Option<PathBuf>> {
    let Some(output_file) = capture_to_file()? else {
        return Ok(None);
    };
    let config = crate::config::load();
    let completion = crate::capture::completion::PreviewCompletion::new(
        &output_file,
        config.capture.open_folder_after_save,
    );
    completion.announce_saved();
    present_capture(&output_file, completion);
    Ok(Some(output_file))
}

pub fn capture_to_file() -> Result<Option<PathBuf>> {
    let Some(_capture) = crate::capture::gate::try_acquire("cli-screenshot") else {
        qol_runtime::probe!("SHOT_SKIP", "action=screenshot reason=busy");
        return Ok(None);
    };
    let frozen_frame = freeze_frame();
    let Some(selected) = platform::select_region(
        crate::capture::space::CaptureKind::Screenshot,
        frozen_frame.clone(),
    )?
    else {
        return Ok(None);
    };
    let rect = prepare_screenshot_rect(selected)?;
    capture_rect_to_file(rect, frozen_frame.as_ref()).map(Some)
}

pub struct PreviewCapture {
    pub(crate) path: PathBuf,
    pub(crate) pixels: Option<PreviewPixels>,
    pub(crate) file_ready: CaptureFileReady,
    pub(crate) file_start: CaptureFileStart,
    pub(crate) started_at: Instant,
    pub(crate) completion: Option<crate::capture::completion::PreviewCompletion>,
}

pub(crate) enum PreviewPixels {
    Rgba(Vec<u8>, u32, u32),
    Bgra(Vec<u8>, u32, u32),
}

impl PreviewPixels {
    fn rgba(pixels: Vec<u8>, width: u32, height: u32) -> Self {
        Self::Rgba(pixels, width, height)
    }

    fn bgra(pixels: Vec<u8>, width: u32, height: u32) -> Self {
        Self::Bgra(pixels, width, height)
    }

    pub(crate) fn into_bgra_parts(self) -> (Vec<u8>, u32, u32) {
        match self {
            Self::Rgba(mut pixels, width, height) => {
                swap_red_blue(&mut pixels);
                (pixels, width, height)
            }
            Self::Bgra(pixels, width, height) => (pixels, width, height),
        }
    }
}

#[derive(Clone)]
pub(crate) struct CaptureFileReady {
    state: Arc<FileReadyState>,
}

#[derive(Clone)]
pub(crate) struct CaptureFileStart {
    state: Arc<(Mutex<bool>, Condvar)>,
}

type FileWriteResult = std::result::Result<(), String>;
type FileReadyState = (Mutex<Option<FileWriteResult>>, Condvar);

impl CaptureFileReady {
    pub(crate) fn ready() -> Self {
        Self {
            state: Arc::new((Mutex::new(Some(Ok(()))), Condvar::new())),
        }
    }

    fn pending() -> Self {
        Self {
            state: Arc::new((Mutex::new(None), Condvar::new())),
        }
    }

    fn complete(&self, result: FileWriteResult) {
        let outcome = if result.is_ok() { "ok" } else { "failed" };
        let (state, wake) = &*self.state;
        if let Ok(mut state) = state.lock() {
            *state = Some(result);
            wake.notify_all();
            qol_runtime::probe!("SHOT_FILE_READY", "phase=signaled outcome={outcome}");
        }
    }

    pub(crate) fn wait(&self) -> Result<()> {
        let (state, wake) = &*self.state;
        let state = state
            .lock()
            .map_err(|_| anyhow!("screenshot file readiness lock poisoned"))?;
        let (state, timeout) = wake
            .wait_timeout_while(state, Duration::from_secs(10), |state| state.is_none())
            .map_err(|_| anyhow!("screenshot file readiness lock poisoned"))?;
        if timeout.timed_out() && state.is_none() {
            return Err(anyhow!("timed out waiting for screenshot file"));
        }
        match state.as_ref() {
            Some(Ok(())) => Ok(()),
            Some(Err(error)) => Err(anyhow!(error.clone())),
            None => Err(anyhow!("screenshot file did not become ready")),
        }
    }

    #[cfg(test)]
    pub(crate) fn test_pending() -> Self {
        Self::pending()
    }

    #[cfg(test)]
    pub(crate) fn test_complete(&self, result: FileWriteResult) {
        self.complete(result);
    }
}

impl CaptureFileStart {
    pub(crate) fn ready() -> Self {
        Self {
            state: Arc::new((Mutex::new(true), Condvar::new())),
        }
    }

    fn pending() -> Self {
        Self {
            state: Arc::new((Mutex::new(false), Condvar::new())),
        }
    }

    pub(crate) fn start(&self) {
        let (state, wake) = &*self.state;
        if let Ok(mut started) = state.lock() {
            *started = true;
            wake.notify_all();
        }
    }

    fn wait(&self) -> Result<()> {
        let (state, wake) = &*self.state;
        let started = state
            .lock()
            .map_err(|_| anyhow!("screenshot file start lock poisoned"))?;
        let _started = wake
            .wait_while(started, |started| !*started)
            .map_err(|_| anyhow!("screenshot file start lock poisoned"))?;
        Ok(())
    }
}

pub(crate) fn capture_selected_for_preview(
    selected: Rect,
    frozen_frame: Option<&FrozenFrame>,
) -> Result<PreviewCapture> {
    let rect = prepare_screenshot_rect(selected)?;
    capture_rect_for_preview(rect, frozen_frame)
}

pub(crate) fn freeze_frame() -> Option<FrozenFrame> {
    let started = std::time::Instant::now();
    match platform::capture_frozen_frame() {
        Ok(Some(frame)) => {
            let bounds = frame.bounds();
            qol_runtime::probe!(
                "SHOT_FREEZE",
                "result=ok ms={} bounds={}x{}",
                started.elapsed().as_millis(),
                bounds.w,
                bounds.h
            );
            Some(frame)
        }
        Ok(None) => {
            qol_runtime::probe!(
                "SHOT_FREEZE",
                "result=unavailable ms={}",
                started.elapsed().as_millis()
            );
            None
        }
        Err(error) => {
            eprintln!("[qol-shot] failed to freeze screenshot frame: {error:#}");
            qol_runtime::probe!(
                "SHOT_FREEZE",
                "result=error ms={}",
                started.elapsed().as_millis()
            );
            None
        }
    }
}

fn prepare_screenshot_rect(selected: Rect) -> Result<Rect> {
    let monitors = platform::get_monitors().unwrap_or_default();
    let fallback_bounds = match geometry::monitor_for_selection(selected, &monitors) {
        Some(monitor) => monitor,
        None => platform::full_screen_bounds()?,
    };
    let rect = geometry::prepare_screenshot_rect(selected, &monitors, fallback_bounds);
    if rect.w <= 0 || rect.h <= 0 {
        platform::show_notification(
            "Screenshot failed",
            &format!("Invalid area: {}x{}", rect.w, rect.h),
            1200,
        );
        return Err(anyhow!("invalid screenshot area {}x{}", rect.w, rect.h));
    }

    qol_runtime::probe!("SHOT_SELECT_DONE", "rect={}x{}", rect.w, rect.h);
    Ok(rect)
}

fn capture_rect_to_file(rect: Rect, frozen_frame: Option<&FrozenFrame>) -> Result<PathBuf> {
    let output_file = crate::capture::output::screenshot_output_file_path()?;
    let started = std::time::Instant::now();
    if let Some(crop) = frozen_crop(frozen_frame, rect)? {
        crop.save_png(&output_file)?;
        qol_runtime::probe!(
            "SHOT_CAPTURE",
            "source=frozen ms={} rect={}x{}",
            started.elapsed().as_millis(),
            rect.w,
            rect.h
        );
        return Ok(output_file);
    }
    platform::capture_screenshot(&rect, &output_file)?;
    qol_runtime::probe!(
        "SHOT_CAPTURE",
        "source=live ms={} rect={}x{}",
        started.elapsed().as_millis(),
        rect.w,
        rect.h
    );
    Ok(output_file)
}

fn capture_rect_for_preview(
    rect: Rect,
    frozen_frame: Option<&FrozenFrame>,
) -> Result<PreviewCapture> {
    let started_at = Instant::now();
    let output_file = crate::capture::output::screenshot_output_file_path()?;
    let started = Instant::now();
    if let Some(frame) = frozen_frame {
        let crop = frame
            .thumbnail(rect, PREVIEW_MAX_WIDTH, PREVIEW_MAX_HEIGHT)
            .ok_or_else(|| frozen_frame_crop_error(frame, rect))?;
        let (bgra, width, height) = crop.into_bgra_parts();
        let (file_ready, file_start) =
            spawn_frozen_file_write(frame.clone(), rect, output_file.clone());
        qol_runtime::probe!(
            "SHOT_CAPTURE",
            "source=frozen-preview ms={} rect={}x{} preview={}x{}",
            started.elapsed().as_millis(),
            rect.w,
            rect.h,
            width,
            height
        );
        return Ok(PreviewCapture {
            completion: Some(preview_completion(&output_file)),
            path: output_file,
            pixels: Some(PreviewPixels::bgra(bgra, width, height)),
            file_ready,
            file_start,
            started_at,
        });
    }
    let grabbed = Instant::now();
    if let Some((rgba, w, h)) = platform::grab_preview_rgba(&rect) {
        qol_runtime::probe!(
            "SHOT_GRAB",
            "ms={} dims={w}x{h}",
            grabbed.elapsed().as_millis()
        );
        let (file_ready, file_start) = spawn_file_write(rect, output_file.clone());
        return Ok(PreviewCapture {
            completion: Some(preview_completion(&output_file)),
            path: output_file,
            pixels: Some(PreviewPixels::rgba(rgba, w, h)),
            file_ready,
            file_start,
            started_at,
        });
    }

    platform::capture_screenshot(&rect, &output_file)?;
    qol_runtime::probe!(
        "SHOT_CAPTURE",
        "source=live ms={} rect={}x{}",
        started.elapsed().as_millis(),
        rect.w,
        rect.h
    );
    Ok(PreviewCapture {
        completion: Some(preview_completion(&output_file)),
        path: output_file,
        pixels: None,
        file_ready: CaptureFileReady::ready(),
        file_start: CaptureFileStart::ready(),
        started_at,
    })
}

fn preview_completion(path: &Path) -> crate::capture::completion::PreviewCompletion {
    let config = crate::config::load();
    crate::capture::completion::PreviewCompletion::new(path, config.capture.open_folder_after_save)
}

fn frozen_crop(frozen_frame: Option<&FrozenFrame>, rect: Rect) -> Result<Option<FrozenCrop>> {
    frozen_frame
        .map(|frame| {
            frame
                .crop(rect)
                .ok_or_else(|| frozen_frame_crop_error(frame, rect))
        })
        .transpose()
}

fn frozen_frame_crop_error(frame: &FrozenFrame, rect: Rect) -> anyhow::Error {
    let bounds = frame.bounds();
    anyhow!(
        "selected screenshot {}x{}+{},{} falls outside frozen frame {}x{}+{},{}",
        rect.w,
        rect.h,
        rect.x,
        rect.y,
        bounds.w,
        bounds.h,
        bounds.x,
        bounds.y
    )
}

fn spawn_frozen_file_write(
    frame: FrozenFrame,
    rect: Rect,
    path: PathBuf,
) -> (CaptureFileReady, CaptureFileStart) {
    let ready = CaptureFileReady::pending();
    let worker_ready = ready.clone();
    let start = CaptureFileStart::pending();
    let worker_start = start.clone();
    std::thread::spawn(move || {
        if let Err(error) = worker_start.wait() {
            worker_ready.complete(Err(format!("{error:#}")));
            return;
        }
        let started = Instant::now();
        let result = frame
            .crop(rect)
            .ok_or_else(|| frozen_frame_crop_error(&frame, rect))
            .and_then(|crop| save_frozen_crop_atomic(crop, &path));
        match &result {
            Ok(()) => qol_runtime::probe!(
                "SHOT_FILE",
                "source=frozen ms={} result=ok",
                started.elapsed().as_millis()
            ),
            Err(error) => eprintln!("[qol-shot] background screenshot file failed: {error:#}"),
        }
        worker_ready.complete(result.map_err(|error| format!("{error:#}")));
    });
    (ready, start)
}

fn save_frozen_crop_atomic(crop: FrozenCrop, path: &Path) -> Result<()> {
    let temporary = path.with_extension(format!("png.{}.part", std::process::id()));
    let result = crop.save_png(&temporary).and_then(|()| {
        std::fs::rename(&temporary, path)
            .with_context(|| format!("failed to publish frozen screenshot: {}", path.display()))
    });
    if result.is_err() {
        let _ = std::fs::remove_file(temporary);
    }
    result
}

fn spawn_file_write(rect: Rect, path: PathBuf) -> (CaptureFileReady, CaptureFileStart) {
    let ready = CaptureFileReady::pending();
    let worker_ready = ready.clone();
    let start = CaptureFileStart::pending();
    let worker_start = start.clone();
    std::thread::spawn(move || {
        if let Err(error) = worker_start.wait() {
            worker_ready.complete(Err(format!("{error:#}")));
            return;
        }
        let started = Instant::now();
        let result = platform::capture_screenshot(&rect, &path);
        match &result {
            Ok(()) => qol_runtime::probe!("SHOT_FILE", "ms={}", started.elapsed().as_millis()),
            Err(error) => eprintln!("[qol-shot] background screenshot file failed: {error:#}"),
        }
        worker_ready.complete(result.map_err(|error| format!("{error:#}")));
    });
    (ready, start)
}

fn swap_red_blue(data: &mut [u8]) {
    for pixel in data.as_chunks_mut::<4>().0 {
        pixel.swap(0, 2);
    }
}

fn present_capture(output_file: &Path, completion: crate::capture::completion::PreviewCompletion) {
    let fallback = completion.clone();
    if let Err(error) = show_preview(output_file, completion) {
        eprintln!("[qol-shot] preview unavailable, copying instead: {error:#}");
        if let Err(error) = platform::copy_image_to_clipboard(output_file) {
            eprintln!("[qol-shot] failed to copy screenshot to clipboard: {error:#}");
        }
        platform::show_notification("Screenshot saved", &output_file.display().to_string(), 1800);
        fallback.finish(crate::capture::completion::PreviewExit::Unavailable);
    }
}

fn show_preview(
    output_file: &Path,
    completion: crate::capture::completion::PreviewCompletion,
) -> Result<()> {
    crate::ui::preview::show_saved(output_file, completion)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn preview_pixels_normalize_both_input_orders_for_rendering() {
        let cases = [
            PreviewPixels::rgba(vec![10, 20, 30, 255], 1, 1),
            PreviewPixels::bgra(vec![30, 20, 10, 255], 1, 1),
        ];

        for pixels in cases {
            let (pixels, width, height) = pixels.into_bgra_parts();
            assert_eq!((width, height), (1, 1));
            assert_eq!(pixels, vec![30, 20, 10, 255]);
        }
    }

    #[test]
    fn capture_file_readiness_preserves_background_errors() {
        let ready = CaptureFileReady::pending();
        ready.complete(Err("encode failed".to_string()));

        assert_eq!(ready.wait().unwrap_err().to_string(), "encode failed");
    }

    #[test]
    fn file_write_start_waits_for_the_preview_signal() {
        let start = CaptureFileStart::pending();
        let worker_start = start.clone();
        let (tx, rx) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            worker_start.wait().unwrap();
            tx.send(()).unwrap();
        });

        assert!(rx.recv_timeout(Duration::from_millis(20)).is_err());
        start.start();
        rx.recv_timeout(Duration::from_secs(1)).unwrap();
    }

    #[test]
    fn deferred_frozen_write_preserves_full_selected_resolution() {
        let bounds = Rect {
            x: 10,
            y: 20,
            w: 4,
            h: 3,
        };
        let pixels = [40, 20, 10, 255].repeat(4 * 3);
        let frame =
            FrozenFrame::from_bgra_segments(vec![(bounds, pixels, 4, 3)]).expect("frozen frame");
        let selected = Rect {
            x: 11,
            y: 20,
            w: 2,
            h: 3,
        };
        let output = std::env::temp_dir().join(format!(
            "qol-shot-deferred-write-{}-{}.png",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let (ready, start) = spawn_frozen_file_write(frame, selected, output.clone());

        assert!(!output.exists());
        start.start();
        ready.wait().unwrap();

        assert_eq!(image::image_dimensions(&output).unwrap(), (2, 3));
        let _ = std::fs::remove_file(output);
    }
}
