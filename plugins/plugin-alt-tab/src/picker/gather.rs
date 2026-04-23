use super::run::WindowCache;
use crate::app::AltTabApp;
use crate::capture;
use crate::config::AltTabConfig;
use crate::discovery::WindowInfo;
use crate::{IconMap, PreviewMap, SharedIconCache};
use gpui::*;
use std::collections::HashMap;
use std::sync::Arc;

pub(crate) struct GatheredWindows {
    pub windows: Vec<WindowInfo>,
    pub previews: PreviewMap,
    pub icons: IconMap,
}

pub(super) fn gather(
    config: &AltTabConfig,
    icon_cache: &SharedIconCache,
    window_cache: &WindowCache,
    preview_cache: &super::run::SharedPreviewCache,
) -> GatheredWindows {
    let windows = windows_from_cache_or_discovery(config, window_cache);

    #[cfg(debug_assertions)]
    {
        eprintln!(
            "[alt-tab/gather] show_minimized={} total={}",
            config.display.show_minimized,
            windows.len()
        );
        for w in &windows {
            eprintln!(
                "[alt-tab/gather]   wid={} app={:?} title={:?} minimized={}",
                w.id, w.app_name, w.title, w.is_minimized
            );
        }
    }

    let icons = icon_cache.lock().map(|c| c.clone()).unwrap_or_default();
    let previews = preview_cache.lock().map(|c| c.clone()).unwrap_or_default();
    GatheredWindows {
        windows,
        previews,
        icons,
    }
}

fn windows_from_cache_or_discovery(
    _config: &AltTabConfig,
    window_cache: &WindowCache,
) -> Vec<WindowInfo> {
    // dispatch_show writes the live query into window_cache before open_picker
    // runs, so this is always populated by the time gather() is called.
    window_cache
        .lock()
        .ok()
        .map(|c| c.clone())
        .unwrap_or_default()
}

pub(super) struct IconFillRequest {
    pub handle: WindowHandle<AltTabApp>,
    pub windows: Vec<WindowInfo>,
    pub icon_cache: SharedIconCache,
}

pub(super) fn spawn_icon_fill(req: IconFillRequest, known: &IconMap, cx: &mut App) {
    let has_missing = req.windows.iter().any(|w| !known.contains_key(&w.app_name));
    if !has_missing {
        return;
    }
    cx.spawn(async move |cx: &mut AsyncApp| {
        fill_missing_icons(cx, req).await;
    })
    .detach();
}

async fn fill_missing_icons(cx: &mut AsyncApp, req: IconFillRequest) {
    let executor = cx.background_executor().clone();
    let windows = req.windows;
    let raw = executor
        .spawn(async move { capture::get_app_icons(&windows) })
        .await;
    if raw.is_empty() {
        return;
    }
    let rendered = build_icon_cache(raw);
    merge_into_shared_cache(&req.icon_cache, &rendered);
    update_view_icons(cx, req.handle, rendered);
}

fn merge_into_shared_cache(cache: &SharedIconCache, rendered: &IconMap) {
    let Ok(mut icache) = cache.lock() else { return };
    for (k, v) in rendered {
        icache.insert(k.clone(), v.clone());
    }
}

fn update_view_icons(cx: &mut AsyncApp, handle: WindowHandle<AltTabApp>, icons: IconMap) {
    let _ = cx.update(|cx| {
        let _ = handle.update(cx, |view, _window, cx| {
            view.update_icons(icons, cx);
        });
    });
}

pub(crate) fn build_icon_cache(raw_icons: HashMap<String, crate::discovery::RgbaImage>) -> IconMap {
    let mut cache: IconMap = HashMap::new();
    for (app_name, icon) in raw_icons {
        let buf = image::ImageBuffer::<image::Rgba<u8>, Vec<u8>>::from_raw(
            icon.width as u32,
            icon.height as u32,
            icon.data,
        );
        if let Some(buf) = buf {
            let frame = image::Frame::new(buf);
            cache.insert(
                app_name,
                Arc::new(gpui::RenderImage::new(smallvec::smallvec![frame])),
            );
        }
    }
    cache
}
