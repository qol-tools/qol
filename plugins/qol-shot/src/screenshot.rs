use anyhow::{anyhow, Result};
use std::path::{Path, PathBuf};

use crate::{geometry, platform, Rect};

pub fn capture_screenshot() -> Result<Option<PathBuf>> {
    let Some(output_file) = capture_to_file()? else {
        return Ok(None);
    };
    present_capture(&output_file);
    Ok(Some(output_file))
}

pub fn capture_to_file() -> Result<Option<PathBuf>> {
    let Some(_capture) = crate::capture_gate::try_acquire("cli-screenshot") else {
        qol_runtime::probe!("SHOT_SKIP", "action=screenshot reason=busy");
        return Ok(None);
    };
    let Some(selected) = platform::select_region(crate::space::CaptureKind::Screenshot)? else {
        return Ok(None);
    };
    let rect = prepare_screenshot_rect(selected)?;
    capture_rect_to_file(rect).map(Some)
}

pub struct PreviewCapture {
    pub path: PathBuf,
    pub rgba: Option<(Vec<u8>, u32, u32)>,
}

pub fn capture_for_preview() -> Result<Option<PreviewCapture>> {
    let Some(_capture) = crate::capture_gate::try_acquire("cli-preview-capture") else {
        qol_runtime::probe!("SHOT_SKIP", "action=preview-capture reason=busy");
        return Ok(None);
    };
    let Some(selected) = platform::select_region(crate::space::CaptureKind::Screenshot)? else {
        return Ok(None);
    };
    capture_selected_for_preview(selected).map(Some)
}

pub fn capture_selected_for_preview(selected: Rect) -> Result<PreviewCapture> {
    let rect = prepare_screenshot_rect(selected)?;
    capture_rect_for_preview(rect)
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

fn capture_rect_to_file(rect: Rect) -> Result<PathBuf> {
    let output_file = crate::output::screenshot_output_file_path()?;
    let started = std::time::Instant::now();
    platform::capture_screenshot(&rect, &output_file)?;
    qol_runtime::probe!(
        "SHOT_CAPTURE",
        "ms={} rect={}x{}",
        started.elapsed().as_millis(),
        rect.w,
        rect.h
    );
    Ok(output_file)
}

fn capture_rect_for_preview(rect: Rect) -> Result<PreviewCapture> {
    let output_file = crate::output::screenshot_output_file_path()?;
    let grabbed = std::time::Instant::now();
    if let Some((rgba, w, h)) = platform::grab_preview_rgba(&rect) {
        qol_runtime::probe!(
            "SHOT_GRAB",
            "ms={} dims={w}x{h}",
            grabbed.elapsed().as_millis()
        );
        spawn_file_write(rect, output_file.clone());
        return Ok(PreviewCapture {
            path: output_file,
            rgba: Some((rgba, w, h)),
        });
    }

    let started = std::time::Instant::now();
    platform::capture_screenshot(&rect, &output_file)?;
    qol_runtime::probe!(
        "SHOT_CAPTURE",
        "ms={} rect={}x{}",
        started.elapsed().as_millis(),
        rect.w,
        rect.h
    );
    Ok(PreviewCapture {
        path: output_file,
        rgba: None,
    })
}

fn spawn_file_write(rect: Rect, path: PathBuf) {
    std::thread::spawn(move || {
        let started = std::time::Instant::now();
        match platform::capture_screenshot(&rect, &path) {
            Ok(()) => qol_runtime::probe!("SHOT_FILE", "ms={}", started.elapsed().as_millis()),
            Err(error) => eprintln!("[qol-shot] background screenshot file failed: {error:#}"),
        }
    });
}

fn present_capture(output_file: &Path) {
    if let Err(error) = show_preview(output_file) {
        eprintln!("[qol-shot] preview unavailable, copying instead: {error:#}");
        if let Err(error) = platform::copy_image_to_clipboard(output_file) {
            eprintln!("[qol-shot] failed to copy screenshot to clipboard: {error:#}");
        }
        platform::show_notification("Screenshot saved", &output_file.display().to_string(), 1800);
    }
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
fn show_preview(output_file: &Path) -> Result<()> {
    crate::preview::show(output_file)
}

#[cfg(not(any(target_os = "linux", target_os = "macos")))]
fn show_preview(_output_file: &Path) -> Result<()> {
    Err(anyhow!("preview is not supported on this platform"))
}
