pub(crate) mod keepalive;
pub(crate) mod run;
mod create;
mod reuse;

use crate::app::{AltTabApp, PICKER_VISIBLE};
use crate::config::{parse_hex_color, ActionMode, AltTabConfig, DisplayConfig};
use crate::icon::build_icon_cache;
use crate::layout::*;
use crate::platform;
use crate::platform::WindowInfo;
use crate::preview::bgra_to_render_image;
use gpui::*;
use qol_plugin_api::monitor::MonitorTracker;
use std::collections::{HashMap, HashSet};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;

const DEFAULT_ESTIMATED_WINDOW_COUNT: usize = 8;

pub(crate) fn default_estimated_window_count() -> usize {
    DEFAULT_ESTIMATED_WINDOW_COUNT
}

pub(crate) fn open_picker(
    config: &AltTabConfig,
    current: &std::rc::Rc<std::cell::RefCell<Option<(WindowHandle<AltTabApp>, Point<Pixels>)>>>,
    tracker: &MonitorTracker,
    last_window_count: Arc<AtomicUsize>,
    preview_cache: Arc<std::sync::Mutex<HashMap<u32, Arc<RenderImage>>>>,
    icon_cache: Arc<std::sync::Mutex<HashMap<String, Arc<RenderImage>>>>,
    reverse: bool,
    cx: &mut App,
) {
    #[cfg(debug_assertions)]
    eprintln!("[alt-tab/open] show request (reverse={})", reverse);

    if reverse && current.borrow().is_none() {
        return;
    }

    let existing = current.borrow().clone();
    if let Some((ref handle, _)) = existing {
        let cycled = handle
            .update(cx, |view, window: &mut Window, cx| -> bool {
                if !PICKER_VISIBLE.load(Ordering::Relaxed) {
                    return false;
                }
                if view.action_mode != ActionMode::HoldToSwitch {
                    return false;
                }
                if view._alt_poll_task.is_none() {
                    view.start_alt_poll(window.window_handle(), cx);
                }
                #[cfg(debug_assertions)]
                eprintln!("[alt-tab/hold] window already visible (reverse={}) — cycling", reverse);
                view.delegate.update(cx, |s, _cx| {
                    if reverse { s.select_prev(); } else { s.select_next(); }
                });
                cx.notify();
                true
            })
            .unwrap_or(false);
        if cycled {
            PICKER_VISIBLE.store(true, Ordering::Relaxed);
            cx.activate(true);
            return;
        }
    }

    // On macOS: query which wids already have SC prewarm frames so gather() can
    // skip the redundant ~150ms CG capture for those windows.
    #[cfg(target_os = "macos")]
    let skip_cg_wids = platform::sc_prewarm_wids();
    #[cfg(not(target_os = "macos"))]
    let skip_cg_wids = HashSet::new();

    let (display_windows, initial_previews, icons) =
        gather(config, &preview_cache, &icon_cache, &skip_cg_wids);

    // Grab prewarm surfaces before first render so the picker opens with SC
    // surfaces already populated (avoids the visible CG→SC flash on open).
    #[cfg(target_os = "macos")]
    let prewarm_surfaces = platform::sc_take_prewarm_surfaces();

    if let Some((handle, created_on_origin)) = existing {
        if let Some(new_origin) = reuse::try_reuse(
            handle, created_on_origin, config, display_windows.clone(),
            initial_previews.clone(), icons.clone(), tracker, icon_cache.clone(), cx,
        ) {
            *current.borrow_mut() = Some((handle, new_origin));
            #[cfg(target_os = "macos")]
            inject_prewarm_surfaces(handle, prewarm_surfaces, cx);
            return;
        }
        *current.borrow_mut() = None;
    }

    create::create_new(
        config, display_windows, initial_previews, icons, tracker,
        last_window_count, icon_cache, current, cx,
    );

    #[cfg(target_os = "macos")]
    if let Some(h) = current.borrow().as_ref().map(|(h, _)| *h) {
        inject_prewarm_surfaces(h, prewarm_surfaces, cx);
    }
}

fn gather(
    config: &AltTabConfig,
    preview_cache: &Arc<std::sync::Mutex<HashMap<u32, Arc<RenderImage>>>>,
    icon_cache: &Arc<std::sync::Mutex<HashMap<String, Arc<RenderImage>>>>,
    skip_cg_wids: &HashSet<u32>,
) -> (Vec<WindowInfo>, HashMap<u32, Arc<RenderImage>>, HashMap<String, Arc<RenderImage>>) {
    let mut display_windows = initial_display_windows(config);
    display_windows = recover_small_window_set(config, display_windows);

    #[cfg(debug_assertions)]
    {
        eprintln!("[alt-tab/gather] show_minimized={} total={}", config.display.show_minimized, display_windows.len());
        for w in &display_windows {
            eprintln!("[alt-tab/gather]   wid={} app={:?} title={:?} minimized={}", w.id, w.app_name, w.title, w.is_minimized);
        }
    }

    let mut initial_previews: HashMap<u32, Arc<RenderImage>> = HashMap::new();
    // Skip CG capture for windows that already have SC prewarm surfaces — they
    // will be injected into live_surfaces before the first render, so CG is redundant.
    let cg_targets: Vec<(usize, u32)> = display_windows.iter().enumerate()
        .filter(|(_, w)| !w.is_minimized && !skip_cg_wids.contains(&w.id))
        .map(|(i, w)| (i, w.id))
        .collect();
    if !cg_targets.is_empty() {
        #[cfg(debug_assertions)]
        let cg_start = std::time::Instant::now();
        let cg_results = platform::capture_previews_cg(&cg_targets, PREVIEW_MAX_WIDTH, PREVIEW_MAX_HEIGHT);
        #[cfg(debug_assertions)]
        eprintln!("[alt-tab/gather] CG capture: {} targets, {} results, {:?}",
            cg_targets.len(), cg_results.len(), cg_start.elapsed());
        for (idx, rgba_opt) in cg_results {
            let Some(rgba) = rgba_opt else {
                #[cfg(debug_assertions)]
                if let Some(win) = display_windows.get(idx) {
                    eprintln!("[alt-tab/gather] CG FAILED wid={} app={:?}", win.id, win.app_name);
                }
                continue;
            };
            let Some(win) = display_windows.get(idx) else { continue };
            if let Some(img) = bgra_to_render_image(rgba.data, rgba.width, rgba.height) {
                initial_previews.insert(win.id, img);
            }
        }
    }
    #[cfg(debug_assertions)]
    {
        let non_min = display_windows.iter().filter(|w| !w.is_minimized).count();
        let skipped = non_min - cg_targets.len();
        if skipped > 0 {
            eprintln!("[alt-tab/gather] CG skipped {}/{} windows (prewarm cached)", skipped, non_min);
        }
        eprintln!("[alt-tab/gather] initial_previews: {} ok out of {} non-minimized",
            initial_previews.len(), non_min);
    }
    if let Ok(pcache) = preview_cache.lock() {
        for win in &display_windows {
            if !win.is_minimized { continue; }
            if let Some(img) = pcache.get(&win.id) {
                initial_previews.entry(win.id).or_insert_with(|| img.clone());
            }
        }
    }

    let icons = icon_cache.lock().map(|c| c.clone()).unwrap_or_default();
    (display_windows, initial_previews, icons)
}

fn initial_display_windows(config: &AltTabConfig) -> Vec<WindowInfo> {
    if !config.display.show_minimized {
        return platform::get_on_screen_windows();
    }
    // Full enumeration: single CG snapshot partitioned into on-screen + minimized.
    // Avoids stale cache where recently-minimized windows are missing.
    platform::get_open_windows()
}

fn recover_small_window_set(config: &AltTabConfig, display_windows: Vec<WindowInfo>) -> Vec<WindowInfo> {
    if display_windows.len() > 2 {
        return display_windows;
    }

    let recovered = platform::get_on_screen_windows();
    if recovered.len() <= display_windows.len() {
        return display_windows;
    }
    if config.display.show_minimized {
        return recovered;
    }
    filter_non_minimized(recovered)
}

fn filter_non_minimized(windows: Vec<WindowInfo>) -> Vec<WindowInfo> {
    windows
        .into_iter()
        .filter(|w| !w.is_minimized)
        .collect()
}

pub(super) fn spawn_icon_fill(
    handle: WindowHandle<AltTabApp>,
    display_windows: Vec<WindowInfo>,
    icons: &HashMap<String, Arc<RenderImage>>,
    icon_cache: Arc<std::sync::Mutex<HashMap<String, Arc<RenderImage>>>>,
    cx: &mut App,
) {
    let missing: Vec<String> = display_windows.iter()
        .map(|w| w.app_name.clone())
        .filter(|name| !icons.contains_key(name))
        .collect::<HashSet<_>>()
        .into_iter()
        .collect();
    if missing.is_empty() { return; }
    cx.spawn(async move |cx: &mut AsyncApp| {
        let executor = cx.background_executor().clone();
        let raw = executor.spawn(async move { platform::get_app_icons(&display_windows) }).await;
        if raw.is_empty() { return; }
        let rendered = build_icon_cache(raw);
        if let Ok(mut icache) = icon_cache.lock() {
            for (k, v) in &rendered { icache.insert(k.clone(), v.clone()); }
        }
        let _ = cx.update(|cx| {
            let _ = handle.update(cx, |view, _window, cx| {
                view.delegate.update(cx, |state, cx| {
                    for (name, img) in rendered { state.icon_cache.insert(name, img); }
                    cx.notify();
                });
            });
        });
    })
    .detach();
}

/// Inject prewarm SC surfaces as fallback for windows that have no live surface yet.
/// Uses `entry().or_insert_with()` so it never overwrites fresher SC frames already
/// present in `live_surfaces` from the previous session.
#[cfg(target_os = "macos")]
fn inject_prewarm_surfaces(
    handle: WindowHandle<AltTabApp>,
    surfaces: HashMap<u32, platform::SendCVBuf>,
    cx: &mut App,
) {
    if surfaces.is_empty() { return; }
    let _ = handle.update(cx, |view, _window, cx| {
        view.delegate.update(cx, |d, _cx| {
            for (wid, buf) in surfaces {
                d.live_surfaces.entry(wid).or_insert_with(|| buf.into_cvpixelbuffer());
            }
        });
    });
}

pub(super) fn resolve_card_bg(display: &DisplayConfig) -> (u32, f32) {
    let (r, g, b) = parse_hex_color(&display.card_background_color).unwrap_or((0x1a, 0x1e, 0x2a));
    let color = ((r as u32) << 16) | ((g as u32) << 8) | (b as u32);
    (color, display.card_background_opacity.clamp(0.0, 1.0))
}

#[cfg(target_os = "macos")]
pub(crate) fn set_macos_accessory_policy() {
    use objc2_app_kit::{NSApplication, NSApplicationActivationPolicy};
    use objc2_foundation::MainThreadMarker;
    let mtm = MainThreadMarker::new().expect("must be on main thread");
    let app = NSApplication::sharedApplication(mtm);
    app.setActivationPolicy(NSApplicationActivationPolicy::Accessory);
}
