use super::run::WindowCache;
use crate::app::AltTabApp;
use crate::capture;
use crate::config::AltTabConfig;
use crate::discovery;
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
    GatheredWindows {
        windows,
        previews: HashMap::new(),
        icons,
    }
}

fn windows_from_cache_or_discovery(
    config: &AltTabConfig,
    window_cache: &WindowCache,
) -> Vec<WindowInfo> {
    let cached = window_cache
        .lock()
        .ok()
        .map(|c| c.clone())
        .unwrap_or_default();
    if !cached.is_empty() && cache_snapshot_matches(&cached) {
        return apply_minimized_filter(config, cached);
    }
    recover_small_window_set(config, initial_display_windows(config))
}

fn cache_snapshot_matches(cached: &[WindowInfo]) -> bool {
    let snapshot = discovery::on_screen_window_ids();
    if snapshot.is_empty() {
        return true;
    }
    let matches = snapshot_matches_cached_visible_ids(&snapshot, cached);

    #[cfg(debug_assertions)]
    {
        let cached_vis: Vec<u32> = cached
            .iter()
            .filter(|w| !w.is_minimized)
            .map(|w| w.id)
            .collect();
        eprintln!(
            "[alt-tab/cache] snapshot={:?} cached={:?} hit={}",
            snapshot, cached_vis, matches
        );
    }

    matches
}

fn snapshot_matches_cached_visible_ids(snapshot: &[u32], cached: &[WindowInfo]) -> bool {
    let cached_visible: Vec<u32> = cached
        .iter()
        .filter(|w| !w.is_minimized)
        .map(|w| w.id)
        .collect();
    snapshot == cached_visible
}

fn apply_minimized_filter(config: &AltTabConfig, windows: Vec<WindowInfo>) -> Vec<WindowInfo> {
    if config.display.show_minimized {
        return windows;
    }
    windows.into_iter().filter(|w| !w.is_minimized).collect()
}

fn initial_display_windows(config: &AltTabConfig) -> Vec<WindowInfo> {
    if !config.display.show_minimized {
        return discovery::get_on_screen_windows();
    }
    discovery::get_open_windows()
}

fn recover_small_window_set(
    config: &AltTabConfig,
    display_windows: Vec<WindowInfo>,
) -> Vec<WindowInfo> {
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

#[cfg(test)]
mod tests {
    use super::snapshot_matches_cached_visible_ids;
    use crate::discovery::WindowInfo;

    fn window(id: u32, is_minimized: bool) -> WindowInfo {
        WindowInfo {
            id,
            title: String::new(),
            app_name: String::new(),
            preview_path: None,
            icon: None,
            x: 0.0,
            y: 0.0,
            width: 0.0,
            height: 0.0,
            is_minimized,
        }
    }

    #[test]
    fn snapshot_match_ignores_minimized_tail_but_keeps_order() {
        let cached = vec![window(11, false), window(7, false), window(3, true)];
        assert!(snapshot_matches_cached_visible_ids(&[11, 7], &cached));
        assert!(!snapshot_matches_cached_visible_ids(&[7, 11], &cached));
    }

    #[test]
    fn minimized_windows_do_not_break_snapshot_match() {
        let cached = vec![window(11, false), window(7, false), window(3, true)];
        assert!(snapshot_matches_cached_visible_ids(&[11, 7], &cached));
    }
}
