use super::keepalive::open_keepalive;
use super::open_picker;
use crate::app::{AltTabApp, PICKER_VISIBLE};
use crate::config::AltTabConfig;
use crate::daemon;
use crate::icon::build_icon_cache;
use crate::layout::{PREVIEW_MAX_HEIGHT, PREVIEW_MAX_WIDTH};
use crate::platform;
use crate::platform::WindowInfo;
use crate::preview::bgra_to_render_image;
use gpui::*;
use qol_plugin_api::monitor::MonitorTracker;
use std::collections::{HashMap, HashSet};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{mpsc, Arc, Mutex};
use std::time::Duration;

const PREWARM_REFRESH_INTERVAL_MS: u64 = 1200;
const SMALL_WINDOW_SET_MAX: usize = 2;
const STABLE_PREVIOUS_MIN: usize = 6;

type PickerWindowState =
    std::rc::Rc<std::cell::RefCell<Option<(WindowHandle<AltTabApp>, Point<Pixels>)>>>;
type WindowCache = Arc<Mutex<Vec<WindowInfo>>>;
type PreviewCache = Arc<Mutex<HashMap<u32, Arc<RenderImage>>>>;
type IconCache = Arc<Mutex<HashMap<String, Arc<RenderImage>>>>;

#[derive(Clone)]
struct PickerCaches {
    last_window_count: Arc<AtomicUsize>,
    window_cache: WindowCache,
    preview_cache: PreviewCache,
    icon_cache: IconCache,
}

struct PrewarmState {
    first_run: bool,
    prev_snapshot: Vec<u32>,
    prev_stream_wids: HashSet<u32>,
}

impl PickerCaches {
    fn new() -> Self {
        Self {
            last_window_count: Arc::new(AtomicUsize::new(super::default_estimated_window_count())),
            window_cache: Arc::new(Mutex::new(Vec::new())),
            preview_cache: Arc::new(Mutex::new(HashMap::new())),
            icon_cache: Arc::new(Mutex::new(HashMap::new())),
        }
    }
}

impl PrewarmState {
    fn new() -> Self {
        Self {
            first_run: true,
            prev_snapshot: Vec::new(),
            prev_stream_wids: HashSet::new(),
        }
    }
}

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

        let current = picker_window_state();
        let caches = PickerCaches::new();

        spawn_prewarm_task(cx, caches.clone());
        open_on_start(show_on_start, &config, &current, &tracker, &caches, cx);
        spawn_daemon_loop(cx, rx, current, tracker, caches);
    });
}

fn picker_window_state() -> PickerWindowState {
    std::rc::Rc::new(std::cell::RefCell::new(None))
}

fn open_on_start(
    show_on_start: bool,
    config: &AltTabConfig,
    current: &PickerWindowState,
    tracker: &MonitorTracker,
    caches: &PickerCaches,
    cx: &mut App,
) {
    if !show_on_start {
        return;
    }
    open_picker_from_state(config, current, tracker, caches, false, cx);
}

fn open_picker_from_state(
    config: &AltTabConfig,
    current: &PickerWindowState,
    tracker: &MonitorTracker,
    caches: &PickerCaches,
    reverse: bool,
    cx: &mut App,
) {
    open_picker(
        config,
        current,
        tracker,
        caches.last_window_count.clone(),
        caches.preview_cache.clone(),
        caches.icon_cache.clone(),
        reverse,
        cx,
    );
}

fn spawn_prewarm_task(cx: &mut App, caches: PickerCaches) {
    cx.spawn(async move |cx: &mut AsyncApp| run_prewarm_loop(cx, caches).await)
        .detach();
}

async fn run_prewarm_loop(cx: &mut AsyncApp, caches: PickerCaches) {
    let executor = cx.background_executor().clone();
    let mut state = PrewarmState::new();

    loop {
        wait_for_prewarm_tick(&executor, &mut state.first_run).await;
        if picker_visible() {
            continue;
        }
        if should_skip_prewarm_refresh(&executor, &mut state.prev_snapshot, &caches.window_cache)
            .await
        {
            continue;
        }

        let Some(windows) = load_stable_windows(&executor, &caches.window_cache).await else {
            continue;
        };

        caches
            .last_window_count
            .store(windows.len().max(1), Ordering::Relaxed);
        refresh_preview_cache(&executor, &windows, &caches.preview_cache, &mut state.prev_stream_wids).await;
        refresh_icon_cache(&executor, &windows, &caches.icon_cache).await;
        replace_window_cache(&caches.window_cache, windows);
    }
}

async fn wait_for_prewarm_tick(executor: &gpui::BackgroundExecutor, first_run: &mut bool) {
    if *first_run {
        *first_run = false;
        return;
    }
    executor
        .timer(Duration::from_millis(PREWARM_REFRESH_INTERVAL_MS))
        .await;
}

fn picker_visible() -> bool {
    PICKER_VISIBLE.load(Ordering::Relaxed)
}

async fn should_skip_prewarm_refresh(
    executor: &gpui::BackgroundExecutor,
    prev_snapshot: &mut Vec<u32>,
    window_cache: &WindowCache,
) -> bool {
    let snapshot = executor.spawn(async { platform::on_screen_window_ids() }).await;
    let cached_len = cached_window_len(window_cache);
    if should_skip_refresh(&snapshot, prev_snapshot, cached_len) {
        return true;
    }
    *prev_snapshot = snapshot;
    false
}

fn cached_window_len(window_cache: &WindowCache) -> usize {
    window_cache.lock().ok().map(|c| c.len()).unwrap_or(0)
}

fn read_window_cache(window_cache: &WindowCache) -> Vec<WindowInfo> {
    window_cache.lock().ok().map(|c| c.clone()).unwrap_or_default()
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
    executor.spawn(async { platform::get_open_windows() }).await
}

async fn refresh_preview_cache(
    executor: &gpui::BackgroundExecutor,
    windows: &[WindowInfo],
    preview_cache: &PreviewCache,
    prev_stream_wids: &mut HashSet<u32>,
) {
    if sc_enabled() {
        refresh_sc_preview_cache(executor, windows, prev_stream_wids).await;
        return;
    }
    refresh_cg_preview_cache(executor, windows, preview_cache).await;
}

fn sc_enabled() -> bool {
    #[cfg(target_os = "macos")]
    {
        return platform::sc_available();
    }
    #[cfg(not(target_os = "macos"))]
    {
        false
    }
}

#[cfg(target_os = "macos")]
async fn refresh_sc_preview_cache(
    executor: &gpui::BackgroundExecutor,
    windows: &[WindowInfo],
    prev_stream_wids: &mut HashSet<u32>,
) {
    let targets: Vec<(usize, u32)> = windows
        .iter()
        .enumerate()
        .filter(|(_, w)| !w.is_minimized)
        .map(|(i, w)| (i, w.id))
        .collect();
    let new_wids: HashSet<u32> = targets.iter().map(|(_, wid)| *wid).collect();

    // Only restart streams if the target window set actually changed.
    if new_wids != *prev_stream_wids && !targets.is_empty() {
        let (w, h) = (PREVIEW_MAX_WIDTH, PREVIEW_MAX_HEIGHT);
        executor
            .spawn(async move { platform::sc_start_streams_with_content(&targets, w, h) })
            .await;
        *prev_stream_wids = new_wids;
    }

    let retain_ids: HashSet<u32> = windows.iter().map(|w| w.id).collect();
    platform::sc_prewarm_retain(&retain_ids);
}

#[cfg(not(target_os = "macos"))]
async fn refresh_sc_preview_cache(_executor: &gpui::BackgroundExecutor, _windows: &[WindowInfo], _prev_stream_wids: &mut HashSet<u32>) {}

async fn refresh_cg_preview_cache(
    executor: &gpui::BackgroundExecutor,
    windows: &[WindowInfo],
    preview_cache: &PreviewCache,
) {
    let targets: Vec<(usize, u32)> = windows.iter().enumerate().map(|(i, w)| (i, w.id)).collect();
    let captured = executor
        .spawn(async move { platform::capture_previews_cg(&targets, PREVIEW_MAX_WIDTH, PREVIEW_MAX_HEIGHT) })
        .await;

    let live_ids: HashSet<u32> = windows.iter().map(|w| w.id).collect();
    let Ok(mut cache) = preview_cache.lock() else {
        return;
    };
    cache.retain(|id, _| live_ids.contains(id));

    for (idx, rgba_opt) in captured {
        let Some(rgba) = rgba_opt else { continue };
        let Some(win) = windows.get(idx) else { continue };
        let Some(img) = bgra_to_render_image(rgba.data, rgba.width, rgba.height) else {
            continue;
        };
        cache.insert(win.id, img);
    }
}

async fn refresh_icon_cache(
    executor: &gpui::BackgroundExecutor,
    windows: &[WindowInfo],
    icon_cache: &IconCache,
) {
    let cached_names = cached_icon_names(icon_cache);
    let icon_windows = missing_icon_windows(windows, &cached_names);
    if !icon_windows.is_empty() {
        let raw_icons = executor
            .spawn(async move { platform::get_app_icons(&icon_windows) })
            .await;
        if !raw_icons.is_empty() {
            let rendered = build_icon_cache(raw_icons);
            merge_icons(icon_cache, rendered);
        }
    }
    retain_active_icons(icon_cache, windows);
}

fn cached_icon_names(icon_cache: &IconCache) -> HashSet<String> {
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

fn merge_icons(icon_cache: &IconCache, rendered: HashMap<String, Arc<RenderImage>>) {
    let Ok(mut cache) = icon_cache.lock() else {
        return;
    };
    for (name, img) in rendered {
        cache.insert(name, img);
    }
}

fn retain_active_icons(icon_cache: &IconCache, windows: &[WindowInfo]) {
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

fn spawn_daemon_loop(
    cx: &mut App,
    rx: mpsc::Receiver<daemon::Command>,
    current: PickerWindowState,
    tracker: MonitorTracker,
    caches: PickerCaches,
) {
    let rx = Arc::new(Mutex::new(rx));
    cx.spawn(async move |cx: &mut AsyncApp| {
        loop {
            let Some(cmd) = recv_command(cx, rx.clone()).await else {
                shutdown_daemon(cx);
                break;
            };
            match cmd {
                daemon::Command::Show => {
                    dispatch_show(cx, false, current.clone(), tracker.clone(), caches.clone());
                }
                daemon::Command::ShowReverse => {
                    dispatch_show(cx, true, current.clone(), tracker.clone(), caches.clone());
                }
                daemon::Command::Kill => {
                    shutdown_daemon(cx);
                    break;
                }
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

fn dispatch_show(
    cx: &AsyncApp,
    reverse: bool,
    current: PickerWindowState,
    tracker: MonitorTracker,
    caches: PickerCaches,
) {
    #[cfg(debug_assertions)]
    eprintln!("[alt-tab/daemon] received Show (reverse={})", reverse);
    let _ = cx.update(|app_cx| {
        let reloaded_config = crate::config::load_alt_tab_config();
        open_picker_from_state(
            &reloaded_config,
            &current,
            &tracker,
            &caches,
            reverse,
            app_cx,
        );
    });
}

fn shutdown_daemon(cx: &AsyncApp) {
    #[cfg(debug_assertions)]
    eprintln!("[alt-tab/daemon] shutting down");
    cx.update(|app_cx| app_cx.quit()).ok();
}

fn should_skip_refresh(snapshot: &[u32], prev_snapshot: &[u32], cached_len: usize) -> bool {
    snapshot == prev_snapshot && cached_len > SMALL_WINDOW_SET_MAX
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
