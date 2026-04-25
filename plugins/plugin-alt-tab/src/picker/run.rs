use super::{open_picker, OpenPickerRequest};
use crate::capture;
use crate::config::AltTabConfig;
use crate::daemon;
use crate::discovery::{Platform, WindowDiscovery, WindowInfo};
use crate::picker::gather::build_icon_cache;
use crate::{PickerWindowState, SharedIconCache};
use gpui::*;
use qol_plugin_api::monitor::MonitorTracker;
use std::collections::{HashMap, HashSet};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{mpsc, Arc, Mutex};

pub(crate) type WindowCache = Arc<Mutex<Vec<WindowInfo>>>;
pub(crate) type SharedPreviewCache = Arc<Mutex<crate::PreviewMap>>;

#[derive(Clone)]
struct PickerCaches {
    last_window_count: Arc<AtomicUsize>,
    window_cache: WindowCache,
    icon_cache: SharedIconCache,
    preview_cache: SharedPreviewCache,
}

#[derive(Clone)]
struct PickerState {
    current: PickerWindowState,
    tracker: MonitorTracker,
    caches: PickerCaches,
}

impl PickerCaches {
    fn new() -> Self {
        Self {
            last_window_count: Arc::new(AtomicUsize::new(super::default_estimated_window_count())),
            window_cache: Arc::new(Mutex::new(Vec::new())),
            icon_cache: Arc::new(Mutex::new(HashMap::new())),
            preview_cache: Arc::new(Mutex::new(HashMap::new())),
        }
    }
}

impl PickerState {
    fn open_picker(&self, config: &AltTabConfig, reverse: bool, cx: &mut App) {
        let req = OpenPickerRequest {
            config,
            current: &self.current,
            tracker: &self.tracker,
            last_window_count: self.caches.last_window_count.clone(),
            icon_cache: self.caches.icon_cache.clone(),
            window_cache: self.caches.window_cache.clone(),
            preview_cache: self.caches.preview_cache.clone(),
            reverse,
        };
        open_picker(&req, cx);
    }
}

pub(crate) fn run_app(
    config: AltTabConfig,
    rx: mpsc::Receiver<daemon::Command>,
    show_on_start: bool,
) {
    let app = Application::new();

    app.run(move |cx: &mut App| {
        qol_plugin_api::keepalive::open_keepalive(cx, None);
        super::platform::set_accessory_policy();

        let state = PickerState {
            current: picker_window_state(),
            tracker: MonitorTracker::start(cx),
            caches: PickerCaches::new(),
        };

        super::platform::pre_create_if_supported(&config, &state.current, cx);

        if show_on_start {
            state.open_picker(&config, false, cx);
        }
        spawn_daemon_loop(cx, rx, state);
    });
}

fn picker_window_state() -> PickerWindowState {
    std::rc::Rc::new(std::cell::RefCell::new(
        qol_plugin_api::window::ActiveWindows::default(),
    ))
}

fn spawn_daemon_loop(cx: &mut App, rx: mpsc::Receiver<daemon::Command>, state: PickerState) {
    let rx = Arc::new(Mutex::new(rx));
    cx.spawn(async move |cx: &mut AsyncApp| loop {
        let Some(cmd) = recv_command(cx, rx.clone()).await else {
            shutdown_daemon(cx);
            break;
        };
        match cmd {
            daemon::Command::Show => dispatch_show(cx, false, &state).await,
            daemon::Command::ShowReverse => dispatch_show(cx, true, &state).await,
            daemon::Command::Kill => {
                shutdown_daemon(cx);
                break;
            }
        }
    })
    .detach();
}

async fn recv_command(
    cx: &AsyncApp,
    rx: Arc<Mutex<mpsc::Receiver<daemon::Command>>>,
) -> Option<daemon::Command> {
    cx.background_executor()
        .spawn(async move { rx.lock().ok()?.recv().ok() })
        .await
}

async fn dispatch_show(cx: &AsyncApp, reverse: bool, state: &PickerState) {
    #[cfg(debug_assertions)]
    let t_total = std::time::Instant::now();
    #[cfg(debug_assertions)]
    eprintln!("[alt-tab/daemon] received Show (reverse={})", reverse);

    #[cfg(debug_assertions)]
    let t_config = std::time::Instant::now();
    let config = crate::config::load_alt_tab_config();
    #[cfg(debug_assertions)]
    let config_ms = t_config.elapsed().as_millis();

    let executor = cx.background_executor().clone();
    let show_minimized = config.display.show_minimized;

    #[cfg(debug_assertions)]
    let t_query = std::time::Instant::now();
    let windows = executor
        .spawn(async move { Platform.visible_windows(show_minimized).unwrap_or_default() })
        .await;
    #[cfg(debug_assertions)]
    let (query_ms, window_count) = (t_query.elapsed().as_millis(), windows.len());

    #[cfg(debug_assertions)]
    let t_icon = std::time::Instant::now();
    refresh_icon_cache(&executor, &windows, &state.caches.icon_cache).await;
    #[cfg(debug_assertions)]
    let icon_ms = t_icon.elapsed().as_millis();

    let state_for_update = state.clone();
    #[cfg(debug_assertions)]
    let t_update = std::time::Instant::now();
    // Previews are refreshed from inside open_picker via spawn_preview_fill so the show
    // path stays snappy and the fresh frames land in the live view as they arrive.
    let _ = cx.update(move |app_cx| {
        apply_show_windows(&state_for_update.caches, windows);
        state_for_update.open_picker(&config, reverse, app_cx);
    });
    #[cfg(debug_assertions)]
    let update_ms = t_update.elapsed().as_millis();

    #[cfg(debug_assertions)]
    {
        let total_ms = t_total.elapsed().as_millis();
        eprintln!(
            "[alt-tab/timing] total={}ms config={}ms query={}ms({} windows) icon={}ms update={}ms (preview fill deferred to view)",
            total_ms, config_ms, query_ms, window_count, icon_ms, update_ms
        );
    }
}

fn apply_show_windows(caches: &PickerCaches, windows: Vec<WindowInfo>) {
    caches
        .last_window_count
        .store(windows.len().max(1), Ordering::Relaxed);
    prune_previews(&caches.preview_cache, &windows);
    prune_icons(&caches.icon_cache, &windows);
    replace_window_cache(&caches.window_cache, windows);
}

fn replace_window_cache(window_cache: &WindowCache, windows: Vec<WindowInfo>) {
    let Ok(mut cache) = window_cache.lock() else {
        return;
    };
    *cache = windows;
}

fn prune_previews(preview_cache: &SharedPreviewCache, windows: &[WindowInfo]) {
    let active: HashSet<u32> = windows.iter().map(|w| w.id).collect();
    let Ok(mut cache) = preview_cache.lock() else {
        return;
    };
    cache.retain(|id, _| active.contains(id));
}

fn prune_icons(icon_cache: &SharedIconCache, windows: &[WindowInfo]) {
    let active: HashSet<&str> = windows.iter().map(|w| w.app_name.as_str()).collect();
    let Ok(mut cache) = icon_cache.lock() else {
        return;
    };
    cache.retain(|name, _| active.contains(name.as_str()));
}

async fn refresh_icon_cache(
    executor: &gpui::BackgroundExecutor,
    windows: &[WindowInfo],
    icon_cache: &SharedIconCache,
) {
    let cached_names = cached_icon_names(icon_cache);
    let icon_windows = missing_icon_windows(windows, &cached_names);
    if icon_windows.is_empty() {
        return;
    }
    let raw_icons = executor
        .spawn(async move { capture::get_app_icons(&icon_windows) })
        .await;
    if raw_icons.is_empty() {
        return;
    }
    let rendered = build_icon_cache(raw_icons);
    merge_icons(icon_cache, rendered);
}

fn cached_icon_names(icon_cache: &SharedIconCache) -> HashSet<String> {
    icon_cache
        .lock()
        .ok()
        .map(|c| c.keys().cloned().collect())
        .unwrap_or_default()
}

fn missing_icon_windows(windows: &[WindowInfo], cached_names: &HashSet<String>) -> Vec<WindowInfo> {
    windows
        .iter()
        .filter(|w| !cached_names.contains(&w.app_name))
        .cloned()
        .collect()
}

fn merge_icons(icon_cache: &SharedIconCache, rendered: crate::IconMap) {
    let Ok(mut cache) = icon_cache.lock() else {
        return;
    };
    for (name, img) in rendered {
        cache.insert(name, img);
    }
}

fn shutdown_daemon(cx: &AsyncApp) {
    #[cfg(debug_assertions)]
    eprintln!("[alt-tab/daemon] shutting down");
    cx.update(|app_cx| app_cx.quit()).ok();
}
