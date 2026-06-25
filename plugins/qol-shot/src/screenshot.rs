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
    let Some(rect) = select_screenshot_rect()? else {
        return Ok(None);
    };

    let output_file = crate::output::screenshot_output_file_path()?;
    platform::capture_screenshot(&rect, &output_file)?;
    Ok(Some(output_file))
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

fn select_screenshot_rect() -> Result<Option<Rect>> {
    let Some(selected) = platform::select_region()? else {
        return Ok(None);
    };

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

    Ok(Some(rect))
}
