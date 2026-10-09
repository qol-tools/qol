use std::collections::HashMap;

use crate::discovery::WindowInfo;
use qol_app_icon::RgbaImage;
use qol_windowing::platform::windows::{Window, WindowPixels};
use qol_windowing::WindowId;

pub(crate) use super::fallback::{
    live_frame_element, live_shots_available, warm_shots_session, LiveFrame, SendCVBuf, ShotReply,
    PIXEL_FORMAT_420F,
};

const ICON_SIZE: usize = 32;

pub fn capture_previews_cg(
    targets: &[(usize, u32)],
    max_w: usize,
    max_h: usize,
) -> Vec<(usize, Option<RgbaImage>)> {
    targets
        .iter()
        .map(|&(index, window_id)| (index, preview(window_id, max_w, max_h)))
        .collect()
}

fn preview(window_id: u32, max_w: usize, max_h: usize) -> Option<RgbaImage> {
    let window = Window::from_id(&WindowId::from_u32(window_id))?;
    if window.is_minimized() {
        return None;
    }
    let pixels = window.capture_rgba()?;
    (pixels.width > 0 && pixels.height > 0).then(|| fit(&pixels, max_w, max_h))
}

fn fit(pixels: &WindowPixels, max_w: usize, max_h: usize) -> RgbaImage {
    let scale = (max_w as f64 / pixels.width as f64)
        .min(max_h as f64 / pixels.height as f64)
        .min(1.0);
    let width = ((pixels.width as f64 * scale).round() as usize).max(1);
    let height = ((pixels.height as f64 * scale).round() as usize).max(1);
    let mut data = Vec::with_capacity(width * height * 4);
    for y in 0..height {
        let rows = span(y, height, pixels.height);
        for x in 0..width {
            let columns = span(x, width, pixels.width);
            data.extend_from_slice(&average(pixels, rows.clone(), columns));
        }
    }
    RgbaImage {
        data,
        width,
        height,
    }
}

fn span(index: usize, out_len: usize, in_len: usize) -> std::ops::Range<usize> {
    let start = index * in_len / out_len;
    let end = ((index + 1) * in_len / out_len).clamp(start + 1, in_len);
    start..end
}

fn average(
    pixels: &WindowPixels,
    rows: std::ops::Range<usize>,
    columns: std::ops::Range<usize>,
) -> [u8; 4] {
    let mut sum = [0u64; 4];
    let mut count = 0u64;
    for row in rows {
        for column in columns.clone() {
            let offset = (row * pixels.width + column) * 4;
            for (channel, total) in sum.iter_mut().enumerate() {
                *total += u64::from(pixels.rgba[offset + channel]);
            }
            count += 1;
        }
    }
    sum.map(|total| (total / count.max(1)) as u8)
}

pub fn get_app_icons(windows: &[WindowInfo]) -> HashMap<String, RgbaImage> {
    let mut icons: HashMap<String, RgbaImage> = windows
        .iter()
        .filter_map(|window| {
            window
                .icon
                .as_ref()
                .map(|icon| (window.app_name.clone(), icon.clone()))
        })
        .collect();
    for window in windows {
        if icons.contains_key(&window.app_name) {
            continue;
        }
        let icon = Window::from_id(&WindowId::from_u32(window.id))
            .and_then(Window::pid)
            .and_then(|pid| qol_app_icon::icon_for_pid(i32::try_from(pid).ok()?, ICON_SIZE));
        if let Some(icon) = icon {
            icons.insert(window.app_name.clone(), icon);
        }
    }
    icons
}

#[cfg(test)]
mod tests {
    use super::*;

    fn solid(width: usize, height: usize, value: u8) -> WindowPixels {
        WindowPixels {
            width,
            height,
            rgba: vec![value; width * height * 4],
        }
    }

    #[test]
    fn fit_scales_down_inside_the_bounds_and_never_up() {
        let cases = [
            ("wide", solid(400, 200, 9), 100, 100, (100, 50)),
            ("tall", solid(200, 400, 9), 100, 100, (50, 100)),
            ("small stays", solid(40, 20, 9), 100, 100, (40, 20)),
        ];
        for (name, pixels, max_w, max_h, (width, height)) in cases {
            let image = fit(&pixels, max_w, max_h);
            assert_eq!((image.width, image.height), (width, height), "{name}");
            assert!(image.data.iter().all(|value| *value == 9), "{name}");
        }
    }
}
