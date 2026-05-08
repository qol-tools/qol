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
            crate::shared::image_registry::extend_with(
                &mut *icache,
                rendered.clone(),
                cx,
                None,
            );
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

/// Pick the minimal set of windows to (re)capture on this fill pass.
///
/// Cold cache → capture all non-minimized. Warm cache → always refresh the
/// frontmost (idx 0, which is the window the user was just on before Alt+Tab)
/// plus any non-minimized window with no cached preview (new windows that
/// appeared since the last capture). Every other card reuses its existing
/// cached preview.
fn select_capture_targets(windows: &[WindowInfo], cached_ids: &HashSet<u32>) -> Vec<(usize, u32)> {
    let cache_is_cold = cached_ids.is_empty();
    windows
        .iter()
        .enumerate()
        .filter(|(_, w)| !w.is_minimized)
        .filter(|(idx, w)| cache_is_cold || *idx == 0 || !cached_ids.contains(&w.id))
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
    let targets = select_capture_targets(&req.windows, &cached_ids);
    if targets.is_empty() {
        return;
    }
    #[cfg(debug_assertions)]
    let (target_count, skipped_count) = (targets.len(), eligible.saturating_sub(targets.len()));
    #[cfg(not(debug_assertions))]
    let _ = eligible;

    let id_for_idx: HashMap<usize, u32> = targets.iter().copied().collect();
    let executor = cx.background_executor().clone();
    let captured = executor
        .spawn(async move {
            capture::capture_previews_cg(&targets, PREVIEW_MAX_WIDTH, PREVIEW_MAX_HEIGHT)
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
    let _ = cx.update(|cx| {
        if let Ok(mut pcache) = cache.lock() {
            crate::shared::image_registry::extend_with(
                &mut *pcache,
                previews.clone(),
                cx,
                None,
            );
        }
        let _ = handle.update(cx, |view, window, cx| {
            view.update_previews(previews, window, cx);
        });
    });
}

#[cfg(test)]
mod preview_target_selection_tests {
    use super::select_capture_targets;
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
        let got = select_capture_targets(&windows, &cached);
        assert_eq!(ids(&got), vec![10, 20, 40]);
        assert!(!got.iter().any(|(_, id)| *id == 30));
    }

    #[test]
    fn warm_cache_all_cached_captures_only_frontmost() {
        let windows = vec![w(10, false), w(20, false), w(30, false)];
        let cached: HashSet<u32> = [10, 20, 30].into_iter().collect();
        let got = select_capture_targets(&windows, &cached);
        assert_eq!(ids(&got), vec![10]);
        assert_eq!(got[0].0, 0, "idx 0 must be the frontmost");
    }

    #[test]
    fn warm_cache_missing_id_is_also_captured() {
        // 10 cached, 20 and 30 uncached (newly appeared). Expect [10 (frontmost), 20, 30].
        let windows = vec![w(10, false), w(20, false), w(30, false)];
        let cached: HashSet<u32> = [10].into_iter().collect();
        let got = select_capture_targets(&windows, &cached);
        assert_eq!(ids(&got), vec![10, 20, 30]);
    }

    #[test]
    fn warm_cache_missing_id_when_frontmost_already_cached() {
        // Only idx 2 is missing. Frontmost (idx 0) always re-captured + the missing one.
        let windows = vec![w(10, false), w(20, false), w(30, false), w(40, false)];
        let cached: HashSet<u32> = [10, 20, 40].into_iter().collect();
        let got = select_capture_targets(&windows, &cached);
        assert_eq!(ids(&got), vec![10, 30]);
    }

    #[test]
    fn minimized_frontmost_not_captured_even_when_first() {
        // If the OS put a minimized window at idx 0, it must not be captured
        // (and idx 1 is not promoted to "frontmost" — that's fine, the next non-
        // minimized is still cached if present).
        let windows = vec![w(10, true), w(20, false)];
        let cached: HashSet<u32> = [20].into_iter().collect();
        let got = select_capture_targets(&windows, &cached);
        assert!(
            got.is_empty(),
            "minimized windows must never be capture targets"
        );
    }

    #[test]
    fn minimized_windows_without_cache_still_skipped() {
        // Even on cold boot, minimized windows are skipped — matches capture_previews_cg's contract.
        let windows = vec![w(10, true), w(20, true), w(30, false)];
        let cached = HashSet::new();
        let got = select_capture_targets(&windows, &cached);
        assert_eq!(ids(&got), vec![30]);
    }

    #[test]
    fn cold_cache_with_minimized_frontmost_still_captures_visible_windows() {
        // Cold-cache short-circuits to "all non-minimized". idx 0 being minimized must
        // not accidentally pull in the minimized entry via the frontmost branch.
        let windows = vec![w(10, true), w(20, false), w(30, false)];
        let cached = HashSet::new();
        let got = select_capture_targets(&windows, &cached);
        assert_eq!(ids(&got), vec![20, 30]);
        assert!(!got.iter().any(|(_, id)| *id == 10));
    }

    #[test]
    fn empty_windows_yields_empty() {
        let got = select_capture_targets(&[], &HashSet::new());
        assert!(got.is_empty());
    }

    #[test]
    fn indices_returned_are_the_original_window_positions() {
        // idx 0 minimized → idx 1 is a "new" window (frontmost test should NOT match it since idx!=0)
        let windows = vec![w(10, true), w(20, false), w(30, false)];
        let cached: HashSet<u32> = [20, 30].into_iter().collect();
        let got = select_capture_targets(&windows, &cached);
        // idx 0 is minimized → skipped. idx 1 is cached and not idx 0 → skipped. idx 2 cached → skipped.
        assert!(
            got.is_empty(),
            "no non-minimized frontmost + all cached = nothing to do"
        );
    }
}
