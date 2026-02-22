use crate::layout::{PREVIEW_MAX_HEIGHT, PREVIEW_MAX_WIDTH};
use crate::platform;
use crate::platform::{PreviewFrame, WindowInfo};
use gpui::*;
use image::{Frame, RgbaImage};
use std::collections::HashMap;
use std::sync::Arc;

pub async fn load_windows_with_previews(
    executor: &BackgroundExecutor,
    cached_windows: Vec<WindowInfo>,
    refresh_all_previews: bool,
) -> Vec<WindowInfo> {
    let mut windows = executor
        .spawn(async move { platform::get_open_windows() })
        .await;
    if windows.is_empty() {
        return windows;
    }

    let mut cached_paths = HashMap::new();
    for win in cached_windows {
        if let Some(path) = win.preview_path {
            cached_paths.insert(win.id, path);
        }
    }

    for win in &mut windows {
        if let Some(path) = cached_paths.get(&win.id) {
            win.preview_path = Some(path.clone());
            continue;
        }
        if let Some(path) = platform::cached_preview_path(win.id) {
            win.preview_path = Some(path);
        }
    }

    let capture_targets: Vec<(usize, u32)> = windows
        .iter()
        .enumerate()
        .filter(|(_, win)| refresh_all_previews || win.preview_path.is_none())
        .map(|(i, win)| (i, win.id))
        .collect();

    if capture_targets.is_empty() {
        return windows;
    }

    let targets = capture_targets.clone();
    let captured = executor
        .spawn(async move {
            platform::capture_previews_batch(&targets, PREVIEW_MAX_WIDTH, PREVIEW_MAX_HEIGHT)
        })
        .await;

    for (i, path_opt) in captured {
        if let Some(path) = path_opt {
            if i < windows.len() {
                windows[i].preview_path = Some(path);
            }
        }
    }

    windows
}

fn frame_to_render_image(frame: &PreviewFrame) -> Arc<RenderImage> {
    let rgba_image = RgbaImage::from_raw(frame.width, frame.height, (*frame.rgba).clone())
        .unwrap_or_else(|| RgbaImage::new(1, 1));
    let img_frame = Frame::new(rgba_image);
    Arc::new(RenderImage::new(smallvec::smallvec![img_frame]))
}

pub fn preview_tile(frame: &Option<PreviewFrame>, width: f32, height: f32) -> AnyElement {
    if let Some(f) = frame {
        let render_image = frame_to_render_image(f);
        img(ImageSource::Render(render_image))
            .w(px(width))
            .h(px(height))
            .rounded_md()
            .into_any_element()
    } else {
        div()
            .w(px(width))
            .h(px(height))
            .bg(rgb(0x1e2130))
            .rounded_md()
            .border_1()
            .border_color(rgb(0x3a4252))
            .flex()
            .items_center()
            .justify_center()
            .text_xs()
            .text_color(rgb(0x4a5268))
            .child("...")
            .into_any_element()
    }
}
