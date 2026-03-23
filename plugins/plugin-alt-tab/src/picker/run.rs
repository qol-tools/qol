use super::{open_picker, OpenPickerRequest};
use crate::app::PICKER_VISIBLE;
use crate::capture;
use crate::config::AltTabConfig;
use crate::daemon;
use crate::discovery;
use crate::discovery::WindowInfo;
use crate::picker::gather::build_icon_cache;
use crate::{PickerWindowState, SharedIconCache};
use gpui::*;
use qol_plugin_api::monitor::MonitorTracker;
use std::collections::{HashMap, HashSet};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{mpsc, Arc, Mutex};

const SMALL_WINDOW_SET_MAX: usize = 2;
const STABLE_PREVIOUS_MIN: usize = 6;

pub(crate) type WindowCache = Arc<Mutex<Vec<WindowInfo>>>;

#[derive(Clone)]
struct PickerCaches {
    last_window_count: Arc<AtomicUsize>,
    window_cache: WindowCache,
    icon_cache: SharedIconCache,
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

        let (cache_tx, cache_rx) = std::sync::mpsc::channel();
        #[cfg(target_os = "linux")]
        let _watcher = crate::discovery::watcher::spawn_watcher(cache_tx);
        #[cfg(not(target_os = "linux"))]
        let _ = cache_tx;
        spawn_cache_updater(cx, cache_rx, state.caches.clone());
        spawn_initial_cache_fill(cx, state.caches.clone());

        super::create::pre_create_offscreen(&config, &state.current, cx);

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

fn spawn_cache_updater(
    cx: &mut App,
    rx: std::sync::mpsc::Receiver<discovery::CacheEvent>,
    caches: PickerCaches,
) {
    let rx = Arc::new(Mutex::new(rx));
    cx.spawn(async move |cx: &mut AsyncApp| {
        let executor = cx.background_executor().clone();
        loop {
            let rx_clone = rx.clone();
            let event = executor
                .spawn(async move { rx_clone.lock().ok()?.recv().ok() })
                .await;
            if event.is_none() {
                break;
            }
            drain_cache_events(&rx);
            if !picker_visible() {
                refresh_cache(&executor, &caches).await;
            }
        }
    })
    .detach();
}

fn drain_cache_events(rx: &Arc<Mutex<std::sync::mpsc::Receiver<discovery::CacheEvent>>>) {
    let Ok(rx) = rx.lock() else { return };
    while rx.try_recv().is_ok() {}
}

fn spawn_initial_cache_fill(cx: &mut App, caches: PickerCaches) {
    cx.spawn(async move |cx: &mut AsyncApp| {
        let executor = cx.background_executor().clone();
        refresh_cache(&executor, &caches).await;
    })
    .detach();
}

async fn refresh_cache(executor: &gpui::BackgroundExecutor, caches: &PickerCaches) {
    let Some(windows) = load_stable_windows(executor, &caches.window_cache).await else {
        return;
    };
    caches
        .last_window_count
        .store(windows.len().max(1), Ordering::Relaxed);
    refresh_icon_cache(executor, &windows, &caches.icon_cache).await;
    replace_window_cache(&caches.window_cache, windows);
}

fn picker_visible() -> bool {
    PICKER_VISIBLE.load(Ordering::Relaxed)
}

fn read_window_cache(window_cache: &WindowCache) -> Vec<WindowInfo> {
    window_cache
        .lock()
        .ok()
        .map(|c| c.clone())
        .unwrap_or_default()
}

async fn load_stable_windows(
    executor: &gpui::BackgroundExecutor,
    window_cache: &WindowCache,
) -> Option<Vec<WindowInfo>> {
    let previous_windows = read_window_cache(window_cache);
    let windows = fetch_open_windows(executor).await;
    if picker_visible() {
        return None;
    }
    if !should_retry_small_result(windows.len(), previous_windows.len()) {
        return Some(windows);
    }
    let retry = fetch_open_windows(executor).await;
    Some(choose_stable_windows(windows, retry, previous_windows))
}

async fn fetch_open_windows(executor: &gpui::BackgroundExecutor) -> Vec<WindowInfo> {
    executor
        .spawn(async { discovery::get_open_windows() })
        .await
}

async fn refresh_icon_cache(
    executor: &gpui::BackgroundExecutor,
    windows: &[WindowInfo],
    icon_cache: &SharedIconCache,
) {
    let cached_names = cached_icon_names(icon_cache);
    let icon_windows = missing_icon_windows(windows, &cached_names);
    if !icon_windows.is_empty() {
        let raw_icons = executor
            .spawn(async move { capture::get_app_icons(&icon_windows) })
            .await;
        if !raw_icons.is_empty() {
            let rendered = build_icon_cache(raw_icons);
            merge_icons(icon_cache, rendered);
        }
    }
    retain_active_icons(icon_cache, windows);
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

fn retain_active_icons(icon_cache: &SharedIconCache, windows: &[WindowInfo]) {
    let active: HashSet<&str> = windows.iter().map(|w| w.app_name.as_str()).collect();
    let Ok(mut cache) = icon_cache.lock() else {
        return;
    };
    cache.retain(|name, _| active.contains(name.as_str()));
}

fn replace_window_cache(window_cache: &WindowCache, windows: Vec<WindowInfo>) {
    let Ok(mut cache) = window_cache.lock() else {
        return;
    };
    *cache = windows;
}

fn spawn_daemon_loop(cx: &mut App, rx: mpsc::Receiver<daemon::Command>, state: PickerState) {
    let rx = Arc::new(Mutex::new(rx));
    cx.spawn(async move |cx: &mut AsyncApp| loop {
        let Some(cmd) = recv_command(cx, rx.clone()).await else {
            shutdown_daemon(cx);
            break;
        };
        match cmd {
            daemon::Command::Show => dispatch_show(cx, false, &state),
            daemon::Command::ShowReverse => dispatch_show(cx, true, &state),
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

fn dispatch_show(cx: &AsyncApp, reverse: bool, state: &PickerState) {
    #[cfg(debug_assertions)]
    eprintln!("[alt-tab/daemon] received Show (reverse={})", reverse);
    let state = state.clone();
    let _ = cx.update(move |app_cx| {
        let config = crate::config::load_alt_tab_config();
        refresh_cache_for_show(&config, &state.caches);
        state.open_picker(&config, reverse, app_cx);
    });
}

fn refresh_cache_for_show(config: &AltTabConfig, caches: &PickerCaches) {
    let windows = if config.display.show_minimized {
        discovery::get_open_windows()
    } else {
        discovery::get_on_screen_windows()
    };
    caches
        .last_window_count
        .store(windows.len().max(1), Ordering::Relaxed);
    replace_window_cache(&caches.window_cache, windows);
}

fn shutdown_daemon(cx: &AsyncApp) {
    #[cfg(debug_assertions)]
    eprintln!("[alt-tab/daemon] shutting down");
    cx.update(|app_cx| app_cx.quit()).ok();
}

fn should_retry_small_result(current_len: usize, previous_len: usize) -> bool {
    current_len <= SMALL_WINDOW_SET_MAX && previous_len >= STABLE_PREVIOUS_MIN
}

fn choose_stable_windows(
    current: Vec<WindowInfo>,
    retry: Vec<WindowInfo>,
    previous: Vec<WindowInfo>,
) -> Vec<WindowInfo> {
    let best = if retry.len() > current.len() {
        retry
    } else {
        current
    };
    if best.len() <= SMALL_WINDOW_SET_MAX && previous.len() >= STABLE_PREVIOUS_MIN {
        return previous;
    }
    best
}
