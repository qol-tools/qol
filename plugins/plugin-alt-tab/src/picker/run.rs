use super::{open_picker, OpenPickerRequest};
use crate::app::PICKER_VISIBLE;
use crate::capture;
use crate::config::AltTabConfig;
use crate::daemon;
use crate::discovery;
#[cfg(target_os = "macos")]
use crate::discovery::platform::macos::SharedWindowStore;
use crate::discovery::WindowInfo;
use crate::picker::gather::build_icon_cache;
use crate::{PickerWindowState, SharedIconCache};
use gpui::*;
use qol_plugin_api::monitor::MonitorTracker;
use std::collections::{HashMap, HashSet};
use std::sync::atomic::{AtomicI64, AtomicUsize, Ordering};
use std::sync::{mpsc, Arc, Mutex};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

const BACKGROUND_REFRESH_MIN_INTERVAL: Duration = Duration::from_secs(1);

pub(crate) type WindowCache = Arc<Mutex<Vec<WindowInfo>>>;
pub(crate) type SharedPreviewCache = Arc<Mutex<crate::PreviewMap>>;

#[cfg(target_os = "macos")]
type SharedFocusHistory = Arc<Mutex<FocusHistory>>;

#[cfg(target_os = "macos")]
#[derive(Default)]
struct FocusHistory {
    last_window_id: Option<u32>,
    window_ids: Vec<u32>,
}

#[derive(Clone)]
struct PickerCaches {
    last_window_count: Arc<AtomicUsize>,
    window_cache: WindowCache,
    icon_cache: SharedIconCache,
    preview_cache: SharedPreviewCache,
    last_refresh_ns: Arc<AtomicI64>,
    #[cfg(target_os = "macos")]
    focus_history: SharedFocusHistory,
    #[cfg(target_os = "macos")]
    window_store: SharedWindowStore,
}

#[derive(Clone)]
struct PickerState {
    current: PickerWindowState,
    tracker: MonitorTracker,
    caches: PickerCaches,
}

impl PickerCaches {
    fn new(#[cfg(target_os = "macos")] window_store: SharedWindowStore) -> Self {
        Self {
            last_window_count: Arc::new(AtomicUsize::new(super::default_estimated_window_count())),
            window_cache: Arc::new(Mutex::new(Vec::new())),
            icon_cache: Arc::new(Mutex::new(HashMap::new())),
            preview_cache: Arc::new(Mutex::new(HashMap::new())),
            last_refresh_ns: Arc::new(AtomicI64::new(0)),
            #[cfg(target_os = "macos")]
            focus_history: Arc::new(Mutex::new(FocusHistory::default())),
            #[cfg(target_os = "macos")]
            window_store,
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
        #[cfg(target_os = "macos")]
        crate::discovery::platform::macos::ax::init_messaging_timeout();

        #[cfg(target_os = "macos")]
        let window_store = crate::discovery::platform::macos::shared_window_store();

        let state = PickerState {
            current: picker_window_state(),
            tracker: MonitorTracker::start(cx),
            caches: PickerCaches::new(
                #[cfg(target_os = "macos")]
                window_store.clone(),
            ),
        };

        let (cache_tx, cache_rx) = std::sync::mpsc::channel();
        #[cfg(target_os = "linux")]
        let _watcher = crate::discovery::watcher::spawn_watcher(cache_tx);
        #[cfg(target_os = "macos")]
        let _window_observer =
            crate::discovery::platform::macos::start_window_observer(window_store, cache_tx);
        #[cfg(not(any(target_os = "linux", target_os = "macos")))]
        let _ = cache_tx;
        spawn_cache_updater(cx, cache_rx, state.caches.clone());
        spawn_initial_cache_fill(cx, state.caches.clone());

        #[cfg(target_os = "macos")]
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
        refresh_cache_with_previews(&executor, &caches).await;
    })
    .detach();
}

async fn refresh_cache(executor: &gpui::BackgroundExecutor, caches: &PickerCaches) {
    refresh_cache_inner(executor, caches, false).await;
}

async fn refresh_cache_with_previews(executor: &gpui::BackgroundExecutor, caches: &PickerCaches) {
    refresh_cache_inner(executor, caches, true).await;
}

async fn refresh_cache_inner(
    executor: &gpui::BackgroundExecutor,
    caches: &PickerCaches,
    include_previews: bool,
) {
    let Some(windows) = load_stable_windows(executor).await else {
        return;
    };
    stamp_last_refresh(&caches.last_refresh_ns);
    #[cfg(target_os = "macos")]
    sync_focus_history_from_store(&caches.focus_history, &caches.window_store);
    #[cfg(target_os = "macos")]
    let windows = apply_focus_history(windows, &caches.focus_history);
    caches
        .last_window_count
        .store(windows.len().max(1), Ordering::Relaxed);
    refresh_icon_cache(executor, &windows, &caches.icon_cache).await;
    if include_previews {
        refresh_preview_cache(executor, &windows, &caches.preview_cache).await;
    }
    if !include_previews {
        retain_active_previews(&caches.preview_cache, &windows);
    }
    #[cfg(target_os = "macos")]
    store_replace_all(&caches.window_store, &windows);
    replace_window_cache(&caches.window_cache, windows);
}

fn picker_visible() -> bool {
    PICKER_VISIBLE.load(Ordering::Relaxed)
}

async fn load_stable_windows(executor: &gpui::BackgroundExecutor) -> Option<Vec<WindowInfo>> {
    let windows = fetch_open_windows(executor).await;
    if picker_visible() {
        return None;
    }
    Some(windows)
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

async fn refresh_preview_cache(
    executor: &gpui::BackgroundExecutor,
    windows: &[WindowInfo],
    preview_cache: &SharedPreviewCache,
) {
    use crate::shared::layout::{PREVIEW_MAX_HEIGHT, PREVIEW_MAX_WIDTH};
    use crate::shared::preview::bgra_to_render_image;

    let targets: Vec<(usize, u32)> = windows
        .iter()
        .enumerate()
        .filter(|(_, w)| !w.is_minimized)
        .map(|(i, w)| (i, w.id))
        .collect();
    if targets.is_empty() {
        return;
    }
    let id_for_idx: HashMap<usize, u32> = targets.iter().copied().collect();
    let captured = executor
        .spawn(async move {
            capture::capture_previews_cg(&targets, PREVIEW_MAX_WIDTH, PREVIEW_MAX_HEIGHT)
        })
        .await;
    let mut new_previews = crate::PreviewMap::new();
    for (idx, rgba) in captured {
        let Some(rgba) = rgba else { continue };
        let Some(&wid) = id_for_idx.get(&idx) else {
            continue;
        };
        if let Some(img) = bgra_to_render_image(rgba.data, rgba.width, rgba.height) {
            new_previews.insert(wid, img);
        }
    }
    if new_previews.is_empty() {
        return;
    }
    let Ok(mut cache) = preview_cache.lock() else {
        return;
    };
    let active_ids: HashSet<u32> = windows.iter().map(|w| w.id).collect();
    cache.extend(new_previews);
    cache.retain(|id, _| active_ids.contains(id));
}

#[cfg(target_os = "macos")]
fn sync_focus_history_from_store(
    focus_history: &SharedFocusHistory,
    window_store: &SharedWindowStore,
) {
    let (last, order) = {
        let Ok(store) = window_store.lock() else {
            return;
        };
        (store.focused_window_id(), store.mru_order())
    };
    let Ok(mut history) = focus_history.lock() else {
        return;
    };
    history.last_window_id = last;
    history.window_ids = order;
}

#[cfg(target_os = "macos")]
fn store_replace_all(window_store: &SharedWindowStore, windows: &[WindowInfo]) {
    let Ok(mut store) = window_store.lock() else {
        return;
    };
    store.replace_all(windows.to_vec());
}

#[cfg(target_os = "macos")]
fn store_snapshot(window_store: &SharedWindowStore) -> Vec<WindowInfo> {
    window_store
        .lock()
        .ok()
        .map(|s| s.snapshot())
        .unwrap_or_default()
}

fn stamp_last_refresh(last_refresh_ns: &AtomicI64) {
    let now = now_nanos();
    last_refresh_ns.store(now, Ordering::Relaxed);
}

fn last_refresh_elapsed(last_refresh_ns: &AtomicI64) -> Duration {
    let last = last_refresh_ns.load(Ordering::Relaxed);
    if last <= 0 {
        return Duration::from_secs(u64::MAX / 2);
    }
    let diff = now_nanos().saturating_sub(last).max(0) as u64;
    Duration::from_nanos(diff)
}

fn now_nanos() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos() as i64)
        .unwrap_or(0)
}

#[cfg(target_os = "macos")]
fn apply_focus_history(
    windows: Vec<WindowInfo>,
    focus_history: &SharedFocusHistory,
) -> Vec<WindowInfo> {
    let order = {
        let active_ids = windows
            .iter()
            .map(|window| window.id)
            .collect::<HashSet<_>>();
        let Ok(mut history) = focus_history.lock() else {
            return windows;
        };
        if let Some(window_id) = history
            .last_window_id
            .filter(|window_id| active_ids.contains(window_id))
        {
            move_window_id_to_front(&mut history.window_ids, window_id);
        }
        history
            .window_ids
            .retain(|window_id| active_ids.contains(window_id));
        history.window_ids.clone()
    };
    reorder_windows_by_ids(windows, &order)
}

#[cfg(target_os = "macos")]
fn move_window_id_to_front(window_ids: &mut Vec<u32>, window_id: u32) {
    if let Some(index) = window_ids
        .iter()
        .position(|existing| *existing == window_id)
    {
        window_ids.remove(index);
    }
    window_ids.insert(0, window_id);
}

#[cfg(target_os = "macos")]
fn reorder_windows_by_ids(windows: Vec<WindowInfo>, order: &[u32]) -> Vec<WindowInfo> {
    if order.is_empty() {
        return windows;
    }
    let mut by_id = HashMap::with_capacity(windows.len());
    let mut original_ids = Vec::with_capacity(windows.len());
    for window in windows {
        original_ids.push(window.id);
        by_id.insert(window.id, window);
    }

    let mut result = Vec::with_capacity(original_ids.len());
    let mut seen = HashSet::new();
    for window_id in order {
        if !seen.insert(*window_id) {
            continue;
        }
        let Some(window) = by_id.remove(window_id) else {
            continue;
        };
        result.push(window);
    }
    for window_id in original_ids {
        if !seen.insert(window_id) {
            continue;
        }
        let Some(window) = by_id.remove(&window_id) else {
            continue;
        };
        result.push(window);
    }
    result
}

fn replace_window_cache(window_cache: &WindowCache, windows: Vec<WindowInfo>) {
    let Ok(mut cache) = window_cache.lock() else {
        return;
    };
    *cache = windows;
}

fn retain_active_previews(preview_cache: &SharedPreviewCache, windows: &[WindowInfo]) {
    let active_ids: HashSet<u32> = windows.iter().map(|w| w.id).collect();
    let Ok(mut cache) = preview_cache.lock() else {
        return;
    };
    cache.retain(|id, _| active_ids.contains(id));
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
    eprintln!("[alt-tab/daemon] received Show (reverse={})", reverse);
    let config = crate::config::load_alt_tab_config();
    let (windows, used_store) = resolve_show_windows(cx, &config, &state.caches).await;
    let state_for_update = state.clone();
    let _ = cx.update(move |app_cx| {
        apply_show_windows(&state_for_update.caches, windows);
        state_for_update.open_picker(&config, reverse, app_cx);
    });
    if used_store {
        maybe_kick_background_refresh(cx, state.caches.clone());
    }
}

#[cfg(target_os = "macos")]
async fn resolve_show_windows(
    cx: &AsyncApp,
    config: &AltTabConfig,
    caches: &PickerCaches,
) -> (Vec<WindowInfo>, bool) {
    let snapshot = store_snapshot(&caches.window_store);
    if !snapshot.is_empty() {
        let windows = filter_show_windows(snapshot, config.display.show_minimized);
        return (windows, true);
    }
    let executor = cx.background_executor().clone();
    let fallback = load_show_windows(&executor, config.display.show_minimized).await;
    (fallback, false)
}

#[cfg(not(target_os = "macos"))]
async fn resolve_show_windows(
    cx: &AsyncApp,
    config: &AltTabConfig,
    _caches: &PickerCaches,
) -> (Vec<WindowInfo>, bool) {
    let executor = cx.background_executor().clone();
    let windows = load_show_windows(&executor, config.display.show_minimized).await;
    (windows, false)
}

#[cfg(target_os = "macos")]
fn filter_show_windows(windows: Vec<WindowInfo>, show_minimized: bool) -> Vec<WindowInfo> {
    if show_minimized {
        return windows;
    }
    windows.into_iter().filter(|w| !w.is_minimized).collect()
}

fn maybe_kick_background_refresh(cx: &AsyncApp, caches: PickerCaches) {
    if last_refresh_elapsed(&caches.last_refresh_ns) < BACKGROUND_REFRESH_MIN_INTERVAL {
        return;
    }
    let executor = cx.background_executor().clone();
    executor
        .clone()
        .spawn(async move {
            refresh_cache(&executor, &caches).await;
        })
        .detach();
}

fn apply_show_windows(caches: &PickerCaches, windows: Vec<WindowInfo>) {
    #[cfg(target_os = "macos")]
    sync_focus_history_from_store(&caches.focus_history, &caches.window_store);
    #[cfg(target_os = "macos")]
    let windows = apply_focus_history(windows, &caches.focus_history);
    caches
        .last_window_count
        .store(windows.len().max(1), Ordering::Relaxed);
    replace_window_cache(&caches.window_cache, windows);
}

async fn load_show_windows(
    executor: &gpui::BackgroundExecutor,
    show_minimized: bool,
) -> Vec<WindowInfo> {
    fetch_show_windows(executor, show_minimized).await
}

fn discover_windows_for_show(show_minimized: bool) -> Vec<WindowInfo> {
    if show_minimized {
        return discovery::get_open_windows();
    }
    discovery::get_on_screen_windows()
}

async fn fetch_show_windows(
    executor: &gpui::BackgroundExecutor,
    show_minimized: bool,
) -> Vec<WindowInfo> {
    executor
        .spawn(async move { discover_windows_for_show(show_minimized) })
        .await
}

fn shutdown_daemon(cx: &AsyncApp) {
    #[cfg(debug_assertions)]
    eprintln!("[alt-tab/daemon] shutting down");
    cx.update(|app_cx| app_cx.quit()).ok();
}
