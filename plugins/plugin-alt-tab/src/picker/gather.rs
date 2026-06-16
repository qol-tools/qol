use super::run::{SharedPreviewCache, WindowCache};
use crate::app::AltTabApp;
use crate::capture;
use crate::config::AltTabConfig;
use crate::discovery::WindowInfo;
use crate::{IconMap, PreviewMap, SharedIconCache};
use gpui::*;
use std::collections::{HashMap, HashSet};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

static PREVIEW_FILL_IN_FLIGHT: AtomicBool = AtomicBool::new(false);

struct InFlightGuard;
impl Drop for InFlightGuard {
    fn drop(&mut self) {
        PREVIEW_FILL_IN_FLIGHT.store(false, Ordering::Release);
    }
}

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

    #[cfg(debug_assertions)]
    probe_preview_gather(&windows, &previews);

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

#[cfg(debug_assertions)]
fn probe_preview_gather(windows: &[WindowInfo], previews: &PreviewMap) {
    let cached = windows
        .iter()
        .filter(|w| previews.contains_key(&w.id))
        .count();
    let entries: Vec<String> = windows
        .iter()
        .take(24)
        .map(|w| {
            let has_preview = previews.contains_key(&w.id);
            preview_entry(w.id, has_preview)
        })
        .collect();
    let missed = windows.len().saturating_sub(cached);
    qol_runtime::probe!(
        "PREVIEW_GATHER",
        "windows={} cached={} missed={} entries=[{}]",
        windows.len(),
        cached,
        missed,
        entries.join(" "),
    );
}

#[cfg(debug_assertions)]
fn preview_entry(wid: u32, has_shared_preview: bool) -> String {
    let live = crate::shared::preview_trace::live_snapshot(wid)
        .map(|stamp| format!("/{source}:{}ms", stamp.age_ms, source = stamp.source))
        .unwrap_or_default();
    if !has_shared_preview {
        return format!("{wid}:miss{live}");
    }
    let shared = crate::shared::preview_trace::shared_snapshot(wid)
        .map(|stamp| format!("{}:{}ms", stamp.source, stamp.age_ms))
        .unwrap_or_else(|| "unknown".to_string());
    format!("{wid}:cache:{shared}{live}")
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
    commit_icons_foreground(cx, req.handle, req.icon_cache.clone(), rendered);
}

fn commit_icons_foreground(
    cx: &mut AsyncApp,
    handle: WindowHandle<AltTabApp>,
    cache: SharedIconCache,
    rendered: IconMap,
) {
    let _ = cx.update(|cx| {
        // SharedCache mutation runs at App level (no Window leased): pass
        // None. View update enters handle.update where window IS leased and
        // forwards Some(window) into the registry release path.
        if let Ok(mut icache) = cache.lock() {
            crate::shared::image_registry::extend_with(&mut *icache, rendered.clone(), cx, None);
        }
        let _ = handle.update(cx, |view, window, cx| {
            view.update_icons(rendered, window, cx);
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

pub(super) struct PreviewFillRequest {
    pub handle: WindowHandle<AltTabApp>,
    pub windows: Vec<WindowInfo>,
    pub preview_cache: SharedPreviewCache,
    pub refresh_frontmost: bool,
    pub refresh_previous_frontmost: bool,
}

pub(super) fn spawn_preview_fill(req: PreviewFillRequest, cx: &mut App) {
    if req.windows.iter().all(|w| w.is_minimized) {
        return;
    }
    if PREVIEW_FILL_IN_FLIGHT
        .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
        .is_err()
    {
        #[cfg(debug_assertions)]
        eprintln!("[alt-tab/timing] preview_fill=skipped (in-flight)");
        return;
    }
    cx.spawn(async move |cx: &mut AsyncApp| {
        let _guard = InFlightGuard;
        fill_previews(cx, req).await;
    })
    .detach();
}

// idx 0 is the window the user was just on before Alt+Tab - the most-visible
// card - so it is re-shot only while the picker is visible (refresh_frontmost).
// idx 1 is usually the app window that just lost focus after MRU reorder.
// Uncached windows always need a first capture.
#[cfg(test)]
fn select_capture_targets(
    windows: &[WindowInfo],
    cached_ids: &HashSet<u32>,
    refresh_frontmost: bool,
) -> Vec<(usize, u32)> {
    select_capture_targets_with_focus(windows, cached_ids, refresh_frontmost, false)
}

fn select_capture_targets_with_focus(
    windows: &[WindowInfo],
    cached_ids: &HashSet<u32>,
    refresh_frontmost: bool,
    refresh_previous_frontmost: bool,
) -> Vec<(usize, u32)> {
    windows
        .iter()
        .enumerate()
        .filter(|(_, w)| !w.is_minimized)
        .filter(|(idx, w)| {
            !cached_ids.contains(&w.id)
                || (refresh_frontmost && *idx == 0)
                || (refresh_previous_frontmost && *idx == 1)
        })
        .map(|(i, w)| (i, w.id))
        .collect()
}

async fn fill_previews(cx: &mut AsyncApp, req: PreviewFillRequest) {
    use crate::shared::layout::{PREVIEW_MAX_HEIGHT, PREVIEW_MAX_WIDTH};
    use crate::shared::preview::bgra_to_render_image;

    #[cfg(debug_assertions)]
    let t_start = std::time::Instant::now();

    let cached_ids = snapshot_preview_keys(&req.preview_cache);
    let eligible = req.windows.iter().filter(|w| !w.is_minimized).count();
    let targets = select_capture_targets_with_focus(
        &req.windows,
        &cached_ids,
        req.refresh_frontmost,
        req.refresh_previous_frontmost,
    );
    if targets.is_empty() {
        return;
    }
    #[cfg(debug_assertions)]
    let (target_count, skipped_count) = (targets.len(), eligible.saturating_sub(targets.len()));
    #[cfg(not(debug_assertions))]
    let _ = eligible;
    #[cfg(debug_assertions)]
    let target_ids = sorted_target_ids(&targets);
    #[cfg(debug_assertions)]
    qol_runtime::probe!(
        "PREVIEW_CAPTURE",
        "source=fill refresh_frontmost={} refresh_previous_frontmost={} targets={} skipped={} ids=[{}]",
        req.refresh_frontmost,
        req.refresh_previous_frontmost,
        target_count,
        skipped_count,
        format_ids(&target_ids),
    );

    let id_for_idx: HashMap<usize, u32> = targets.iter().copied().collect();
    let executor = cx.background_executor().clone();
    let capture_targets = targets.clone();
    let captured = executor
        .spawn(async move {
            capture::capture_previews_cg(&capture_targets, PREVIEW_MAX_WIDTH, PREVIEW_MAX_HEIGHT)
        })
        .await;

    let mut previews = PreviewMap::new();
    for (idx, rgba) in captured {
        let Some(rgba) = rgba else { continue };
        let Some(&wid) = id_for_idx.get(&idx) else {
            continue;
        };
        if let Some(img) = bgra_to_render_image(rgba.data, rgba.width, rgba.height) {
            previews.insert(wid, img);
        }
    }
    if previews.is_empty() {
        return;
    }
    commit_previews_foreground(cx, req.handle, req.preview_cache.clone(), previews);

    #[cfg(debug_assertions)]
    eprintln!(
        "[alt-tab/timing] preview_fill={}ms ({} windows, {} skipped)",
        t_start.elapsed().as_millis(),
        target_count,
        skipped_count,
    );
}

fn snapshot_preview_keys(cache: &SharedPreviewCache) -> HashSet<u32> {
    cache
        .lock()
        .ok()
        .map(|c| c.keys().copied().collect())
        .unwrap_or_default()
}

fn commit_previews_foreground(
    cx: &mut AsyncApp,
    handle: WindowHandle<AltTabApp>,
    cache: SharedPreviewCache,
    previews: PreviewMap,
) {
    #[cfg(debug_assertions)]
    let preview_count = previews.len();
    #[cfg(debug_assertions)]
    let preview_ids = sorted_preview_ids(&previews);
    let _ = cx.update(|cx| {
        if let Ok(mut pcache) = cache.lock() {
            crate::shared::image_registry::extend_with(&mut *pcache, previews.clone(), cx, None);
            #[cfg(debug_assertions)]
            crate::shared::preview_trace::record_shared_fill(preview_ids.iter().copied());
        }
        let _ = handle.update(cx, |view, window, cx| {
            view.update_previews(previews, window, cx);
        });
        #[cfg(debug_assertions)]
        qol_runtime::probe!(
            "PREVIEW_CACHE_WRITE",
            "source=fill cache=shared n={} ids=[{}]",
            preview_count,
            format_ids(&preview_ids),
        );
    });
}

#[cfg(debug_assertions)]
fn sorted_target_ids(targets: &[(usize, u32)]) -> Vec<u32> {
    let mut ids: Vec<u32> = targets.iter().map(|(_, id)| *id).collect();
    ids.sort_unstable();
    ids
}

#[cfg(debug_assertions)]
fn sorted_preview_ids(previews: &PreviewMap) -> Vec<u32> {
    let mut ids: Vec<u32> = previews.keys().copied().collect();
    ids.sort_unstable();
    ids
}

#[cfg(debug_assertions)]
fn format_ids(ids: &[u32]) -> String {
    ids.iter()
        .take(24)
        .map(|id| id.to_string())
        .collect::<Vec<_>>()
        .join(" ")
}

#[cfg(test)]
mod preview_target_selection_tests {
    use super::{select_capture_targets, select_capture_targets_with_focus};
    use crate::discovery::WindowInfo;
    use std::collections::HashSet;

    fn w(id: u32, minimized: bool) -> WindowInfo {
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
            is_minimized: minimized,
        }
    }

    fn ids(v: &[(usize, u32)]) -> Vec<u32> {
        v.iter().map(|(_, id)| *id).collect()
    }

    #[test]
    fn cold_cache_captures_all_non_minimized() {
        let windows = vec![w(10, false), w(20, false), w(30, true), w(40, false)];
        let cached = HashSet::new();
        let got = select_capture_targets(&windows, &cached, true);
        assert_eq!(ids(&got), vec![10, 20, 40]);
        assert!(!got.iter().any(|(_, id)| *id == 30));
    }

    #[test]
    fn warm_cache_all_cached_captures_only_frontmost() {
        let windows = vec![w(10, false), w(20, false), w(30, false)];
        let cached: HashSet<u32> = [10, 20, 30].into_iter().collect();
        let got = select_capture_targets(&windows, &cached, true);
        assert_eq!(ids(&got), vec![10]);
        assert_eq!(got[0].0, 0, "idx 0 must be the frontmost");
    }

    #[test]
    fn warm_cache_missing_id_is_also_captured() {
        let windows = vec![w(10, false), w(20, false), w(30, false)];
        let cached: HashSet<u32> = [10].into_iter().collect();
        let got = select_capture_targets(&windows, &cached, true);
        assert_eq!(ids(&got), vec![10, 20, 30]);
    }

    #[test]
    fn warm_cache_missing_id_when_frontmost_already_cached() {
        let windows = vec![w(10, false), w(20, false), w(30, false), w(40, false)];
        let cached: HashSet<u32> = [10, 20, 40].into_iter().collect();
        let got = select_capture_targets(&windows, &cached, true);
        assert_eq!(ids(&got), vec![10, 30]);
    }

    #[test]
    fn minimized_frontmost_not_captured_even_when_first() {
        let windows = vec![w(10, true), w(20, false)];
        let cached: HashSet<u32> = [20].into_iter().collect();
        let got = select_capture_targets(&windows, &cached, true);
        assert!(
            got.is_empty(),
            "minimized windows must never be capture targets"
        );
    }

    #[test]
    fn minimized_windows_without_cache_still_skipped() {
        let windows = vec![w(10, true), w(20, true), w(30, false)];
        let cached = HashSet::new();
        let got = select_capture_targets(&windows, &cached, true);
        assert_eq!(ids(&got), vec![30]);
    }

    #[test]
    fn cold_cache_with_minimized_frontmost_still_captures_visible_windows() {
        let windows = vec![w(10, true), w(20, false), w(30, false)];
        let cached = HashSet::new();
        let got = select_capture_targets(&windows, &cached, true);
        assert_eq!(ids(&got), vec![20, 30]);
        assert!(!got.iter().any(|(_, id)| *id == 10));
    }

    #[test]
    fn empty_windows_yields_empty() {
        let got = select_capture_targets(&[], &HashSet::new(), true);
        assert!(got.is_empty());
    }

    #[test]
    fn invisible_skips_already_cached_frontmost() {
        let windows = vec![w(10, false), w(20, false), w(30, false)];
        let cached: HashSet<u32> = [10, 20, 30].into_iter().collect();
        let got = select_capture_targets(&windows, &cached, false);
        assert!(
            got.is_empty(),
            "invisible refresh must not re-capture an already-cached frontmost"
        );
    }

    #[test]
    fn focus_refresh_captures_previous_frontmost_even_when_cached() {
        let windows = vec![w(10, false), w(20, false), w(30, false)];
        let cached: HashSet<u32> = [10, 20, 30].into_iter().collect();
        let got = select_capture_targets_with_focus(&windows, &cached, false, true);
        assert_eq!(ids(&got), vec![20]);
        assert_eq!(got[0].0, 1, "idx 1 is the previous frontmost after focus");
    }

    #[test]
    fn focus_refresh_skips_minimized_previous_frontmost() {
        let windows = vec![w(10, false), w(20, true), w(30, false)];
        let cached: HashSet<u32> = [10, 20, 30].into_iter().collect();
        let got = select_capture_targets_with_focus(&windows, &cached, false, true);
        assert!(got.is_empty());
    }

    #[test]
    fn invisible_still_captures_uncached_windows() {
        let windows = vec![w(10, false), w(20, false), w(30, false)];
        let cached: HashSet<u32> = [10].into_iter().collect();
        let got = select_capture_targets(&windows, &cached, false);
        assert_eq!(
            ids(&got),
            vec![20, 30],
            "uncached windows still need a first capture even while invisible"
        );
    }

    #[test]
    fn invisible_skips_minimized_but_captures_uncached() {
        let windows = vec![w(10, true), w(20, false)];
        let got = select_capture_targets(&windows, &HashSet::new(), false);
        assert_eq!(
            ids(&got),
            vec![20],
            "minimized filter wins regardless of refresh_frontmost"
        );
    }

    #[test]
    fn invisible_cold_cache_still_captures_all_non_minimized() {
        let windows = vec![w(10, false), w(20, false)];
        let got = select_capture_targets(&windows, &HashSet::new(), false);
        assert_eq!(
            ids(&got),
            vec![10, 20],
            "cold cache captures all non-minimized whether or not frontmost is forced"
        );
    }

    #[test]
    fn invisible_mixed_captures_only_uncached_visible() {
        let windows = vec![w(10, true), w(20, false), w(30, false)];
        let cached: HashSet<u32> = [20].into_iter().collect();
        let got = select_capture_targets(&windows, &cached, false);
        assert_eq!(
            ids(&got),
            vec![30],
            "invisible: skip minimized, skip cached, capture uncached visible"
        );
    }

    #[test]
    fn indices_returned_are_the_original_window_positions() {
        let windows = vec![w(10, true), w(20, false), w(30, false)];
        let cached: HashSet<u32> = [20, 30].into_iter().collect();
        let got = select_capture_targets(&windows, &cached, true);
        assert!(
            got.is_empty(),
            "no non-minimized frontmost + all cached = nothing to do"
        );
    }
}
