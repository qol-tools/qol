use crate::app::AltTabApp;
use crate::capture;
use crate::config::AltTabConfig;
use crate::discovery;
use crate::discovery::WindowInfo;
use crate::shared::layout::{PREVIEW_MAX_HEIGHT, PREVIEW_MAX_WIDTH};
use crate::shared::preview::bgra_to_render_image;
use crate::{IconMap, PreviewMap, SharedIconCache, SharedPreviewCache};
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
    preview_cache: &SharedPreviewCache,
    icon_cache: &SharedIconCache,
) -> GatheredWindows {
    let windows = recover_small_window_set(config, initial_display_windows(config));

    #[cfg(debug_assertions)]
    {
        eprintln!("[alt-tab/gather] show_minimized={} total={}", config.display.show_minimized, windows.len());
        for w in &windows {
            eprintln!("[alt-tab/gather]   wid={} app={:?} title={:?} minimized={}", w.id, w.app_name, w.title, w.is_minimized);
        }
    }

    let mut previews = capture_initial_previews(&windows);
    merge_cached_minimized(&mut previews, &windows, preview_cache);

    #[cfg(debug_assertions)]
    {
        let non_min = windows.iter().filter(|w| !w.is_minimized).count();
        eprintln!("[alt-tab/gather] initial_previews: {} ok out of {} non-minimized", previews.len(), non_min);
    }

    let icons = icon_cache.lock().map(|c| c.clone()).unwrap_or_default();
    GatheredWindows { windows, previews, icons }
}

fn initial_display_windows(config: &AltTabConfig) -> Vec<WindowInfo> {
    if !config.display.show_minimized {
        return discovery::get_on_screen_windows();
    }
    discovery::get_open_windows()
}

fn recover_small_window_set(config: &AltTabConfig, display_windows: Vec<WindowInfo>) -> Vec<WindowInfo> {
    if display_windows.len() > 2 {
        return display_windows;
    }
    let recovered = discovery::get_on_screen_windows();
    if recovered.len() <= display_windows.len() {
        return display_windows;
    }
    if config.display.show_minimized {
        return recovered;
    }
    recovered.into_iter().filter(|w| !w.is_minimized).collect()
}

fn capture_initial_previews(windows: &[WindowInfo]) -> PreviewMap {
    let mut previews = HashMap::new();
    let cg_targets: Vec<(usize, u32)> = windows.iter().enumerate()
        .filter(|(_, w)| !w.is_minimized)
        .map(|(i, w)| (i, w.id))
        .collect();
    if cg_targets.is_empty() {
        return previews;
    }

    #[cfg(debug_assertions)]
    let cg_start = std::time::Instant::now();
    let cg_results = capture::capture_previews_cg(&cg_targets, PREVIEW_MAX_WIDTH, PREVIEW_MAX_HEIGHT);
    #[cfg(debug_assertions)]
    eprintln!("[alt-tab/gather] CG capture: {} targets, {} results, {:?}",
        cg_targets.len(), cg_results.len(), cg_start.elapsed());

    for (idx, rgba_opt) in cg_results {
        let Some(rgba) = rgba_opt else { continue };
        let Some(win) = windows.get(idx) else { continue };
        let Some(img) = bgra_to_render_image(rgba.data, rgba.width, rgba.height) else { continue };
        previews.insert(win.id, img);
    }
    previews
}

fn merge_cached_minimized(
    previews: &mut PreviewMap,
    windows: &[WindowInfo],
    preview_cache: &SharedPreviewCache,
) {
    let Ok(pcache) = preview_cache.lock() else { return };
    for win in windows {
        if !win.is_minimized {
            continue;
        }
        let Some(img) = pcache.get(&win.id) else { continue };
        previews.entry(win.id).or_insert_with(|| img.clone());
    }
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
    let raw = executor.spawn(async move { capture::get_app_icons(&windows) }).await;
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

pub(crate) fn build_icon_cache(
    raw_icons: HashMap<String, crate::discovery::RgbaImage>,
) -> IconMap {
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
