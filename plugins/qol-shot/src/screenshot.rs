use anyhow::{anyhow, Result};
use std::path::{Path, PathBuf};

use crate::{geometry, platform, Config, Rect};

pub fn capture_screenshot(config: &Config) -> Result<Option<PathBuf>> {
    let Some(rect) = select_screenshot_rect()? else {
        return Ok(None);
    };

    let output_file = crate::output::screenshot_output_file_path()?;
    platform::capture_screenshot(&rect, &output_file)?;
    copy_screenshot_if_enabled(config, &output_file);
    platform::show_notification("Screenshot saved", &output_file.display().to_string(), 1800);
    Ok(Some(output_file))
}

fn copy_screenshot_if_enabled(config: &Config, output_file: &Path) {
    if !config.screenshot.auto_copy {
        return;
    }

    if let Err(error) = platform::copy_image_to_clipboard(output_file) {
        eprintln!("[qol-shot] failed to copy screenshot to clipboard: {error:#}");
    }
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
