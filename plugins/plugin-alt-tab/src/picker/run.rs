use super::keepalive::open_keepalive;
use super::open_picker;
use crate::app::{AltTabApp, PICKER_VISIBLE};
use crate::config::AltTabConfig;
use crate::daemon;
use crate::icon::build_icon_cache;
use crate::layout::{PREVIEW_MAX_HEIGHT, PREVIEW_MAX_WIDTH};
use crate::monitor::MonitorTracker;
use crate::platform;
use crate::platform::WindowInfo;
use crate::preview::bgra_to_render_image;
use gpui::*;
use std::collections::HashMap;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{mpsc, Arc, Mutex};
use std::time::Duration;

const PREWARM_REFRESH_INTERVAL_MS: u64 = 1200;

pub(crate) fn run_app(
    config: AltTabConfig,
    rx: mpsc::Receiver<daemon::Command>,
    show_on_start: bool,
) {
    let app = Application::new();

    app.run(move |cx: &mut App| {
        let tracker = MonitorTracker::start(cx);

        open_keepalive(cx);

        #[cfg(target_os = "macos")]
        super::set_macos_accessory_policy();

        let current: std::rc::Rc<
            std::cell::RefCell<Option<(WindowHandle<AltTabApp>, Point<Pixels>)>>,
        > = std::rc::Rc::new(std::cell::RefCell::new(None));
        let last_window_count =
            Arc::new(AtomicUsize::new(super::default_estimated_window_count()));
        let window_cache: Arc<Mutex<Vec<WindowInfo>>> = Arc::new(Mutex::new(Vec::new()));
        let preview_cache: Arc<Mutex<HashMap<u32, Arc<RenderImage>>>> =
            Arc::new(Mutex::new(HashMap::new()));
        let icon_cache: Arc<Mutex<HashMap<String, Arc<RenderImage>>>> =
            Arc::new(Mutex::new(HashMap::new()));

        // Prewarm: poll window list + capture CG previews + icons when picker is hidden
        let warm_cache = window_cache.clone();
        let warm_count = last_window_count.clone();
        let warm_previews = preview_cache.clone();
        let warm_icons = icon_cache.clone();
        cx.spawn(async move |cx: &mut AsyncApp| {
            let executor = cx.background_executor().clone();
            loop {
                executor
                    .timer(Duration::from_millis(PREWARM_REFRESH_INTERVAL_MS))
                    .await;
                if PICKER_VISIBLE.load(Ordering::Relaxed) {
                    continue;
                }
                let windows = executor
                    .spawn(async { platform::get_open_windows() })
                    .await;
                warm_count.store(windows.len().max(1), Ordering::Relaxed);

                // Capture CG previews in background so open_picker can grab them instantly
                let targets: Vec<(usize, u32)> =
                    windows.iter().enumerate().map(|(i, w)| (i, w.id)).collect();
                let captured = executor
                    .spawn(async move {
                        platform::capture_previews_cg(
                            &targets,
                            PREVIEW_MAX_WIDTH,
                            PREVIEW_MAX_HEIGHT,
                        )
                    })
                    .await;
                if let Ok(mut pcache) = warm_previews.lock() {
                    // Remove stale entries for windows that no longer exist
                    let live_ids: std::collections::HashSet<u32> =
                        windows.iter().map(|w| w.id).collect();
                    pcache.retain(|id, _| live_ids.contains(id));

                    for (idx, rgba_opt) in captured {
                        let Some(rgba) = rgba_opt else { continue };
                        let Some(win) = windows.get(idx) else { continue };
                        if let Some(img) =
                            bgra_to_render_image(&rgba.data, rgba.width, rgba.height)
                        {
                            pcache.insert(win.id, img);
                        }
                    }
                }

                // Extract app icons in background
                let icon_windows = windows.clone();
                let raw_icons = executor
                    .spawn(async move { platform::get_app_icons(&icon_windows) })
                    .await;
                if !raw_icons.is_empty() {
                    let rendered = build_icon_cache(raw_icons);
                    if let Ok(mut icache) = warm_icons.lock() {
                        *icache = rendered;
                    }
                }

                if let Ok(mut cache) = warm_cache.lock() {
                    *cache = windows;
                }
            }
        })
        .detach();

        if show_on_start {
            open_picker(
                &config,
                &current,
                &tracker,
                last_window_count.clone(),
                window_cache.clone(),
                preview_cache.clone(),
                icon_cache.clone(),
                false,
                cx,
            );
        }

        // Poll the daemon channel for Show/Kill commands
        let rx = Arc::new(std::sync::Mutex::new(rx));
        let tracker_clone = tracker.clone();
        cx.spawn(async move |cx: &mut AsyncApp| loop {
            let rx2 = rx.clone();
            let cmd = cx
                .background_executor()
                .spawn(async move { rx2.lock().ok()?.recv().ok() })
                .await;

            match cmd {
                Some(daemon::Command::Show) | Some(daemon::Command::ShowReverse) => {
                    let reverse = matches!(cmd, Some(daemon::Command::ShowReverse));
                    #[cfg(debug_assertions)]
                    eprintln!("[alt-tab/daemon] received Show (reverse={})", reverse);
                    let current2 = current.clone();
                    let tracker2 = tracker_clone.clone();
                    let last_window_count2 = last_window_count.clone();
                    let window_cache2 = window_cache.clone();
                    let preview_cache2 = preview_cache.clone();
                    let icon_cache2 = icon_cache.clone();
                    let _ = cx.update(|app_cx| {
                        let reloaded_config = crate::config::load_alt_tab_config();
                        open_picker(
                            &reloaded_config,
                            &current2,
                            &tracker2,
                            last_window_count2,
                            window_cache2,
                            preview_cache2,
                            icon_cache2,
                            reverse,
                            app_cx,
                        );
                    });
                }
                Some(daemon::Command::Kill) | None => {
                    #[cfg(debug_assertions)]
                    eprintln!("[alt-tab/daemon] shutting down");
                    cx.update(|cx| cx.quit()).ok();
                    break;
                }
            }
        })
        .detach();
    });
}
