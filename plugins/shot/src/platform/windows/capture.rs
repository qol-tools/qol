use anyhow::{anyhow, Context, Result};
use std::path::Path;

use crate::capture::frozen_frame::{bgra_to_rgba, FrozenFrame};
use crate::capture::geometry::rect_label;
use crate::Rect;

use super::display::{native_displays, physical_rect, NativeDisplay};

struct ScreenPixels {
    pixels: Vec<u8>,
    width: u32,
    height: u32,
}

pub fn capture_screenshot(rect: &Rect, output_file: &Path) -> Result<()> {
    let grab = grab_rgba(rect)?;
    image::save_buffer_with_format(
        output_file,
        &grab.pixels,
        grab.width,
        grab.height,
        image::ColorType::Rgba8,
        image::ImageFormat::Png,
    )
    .with_context(|| format!("failed to save screenshot: {}", output_file.display()))
}

pub fn grab_preview_rgba(rect: &Rect) -> Option<(Vec<u8>, u32, u32)> {
    match grab_rgba(rect) {
        Ok(grab) => Some((grab.pixels, grab.width, grab.height)),
        Err(error) => {
            log::warn!("Windows preview grab failed: {error:#}");
            None
        }
    }
}

pub fn capture_frozen_frame() -> Result<Option<FrozenFrame>> {
    let segments = native_displays()
        .into_iter()
        .map(frozen_segment)
        .collect::<Result<Vec<_>>>()?;
    if segments.is_empty() {
        return Ok(None);
    }
    FrozenFrame::from_bgra_segments(segments)
        .map(Some)
        .context("captured Windows displays could not form a frozen frame")
}

fn frozen_segment(display: NativeDisplay) -> Result<(Rect, Vec<u8>, u32, u32)> {
    let grab = grab_bgra(display.physical)?;
    Ok((display.logical(), grab.pixels, grab.width, grab.height))
}

fn grab_rgba(rect: &Rect) -> Result<ScreenPixels> {
    let mut grab = grab_bgra(physical_rect(*rect, &native_displays()))?;
    bgra_to_rgba(&mut grab.pixels);
    Ok(grab)
}

fn grab_bgra(physical: Rect) -> Result<ScreenPixels> {
    let width = u32::try_from(physical.w)
        .ok()
        .filter(|width| *width > 0)
        .with_context(|| format!("invalid capture width in {}", rect_label(physical)))?;
    let height = u32::try_from(physical.h)
        .ok()
        .filter(|height| *height > 0)
        .with_context(|| format!("invalid capture height in {}", rect_label(physical)))?;
    let pixels = qol_windowing::platform::windows::capture_screen_bgra(
        physical.x,
        physical.y,
        width as usize,
        height as usize,
    )
    .ok_or_else(|| anyhow!("GDI screen capture failed for {}", rect_label(physical)))?;
    Ok(ScreenPixels {
        pixels,
        width,
        height,
    })
}
