use super::monitor_listener::WarmerControl;
use super::{open_picker, OpenPickerRequest};
use crate::capture;
use crate::config::AltTabConfig;
use crate::discovery::{Platform, WindowDiscovery, WindowInfo};
use crate::picker::gather::{build_icon_cache, PreviewCaptureScheduler};
use crate::picker::{PickerWindowState, SharedIconCache};
use crate::runtime::daemon;
use gpui::*;
use qol_gpui::command_loop::LoopFlow;
use qol_gpui::monitor::MonitorTracker;
use std::collections::{HashMap, HashSet};
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
use std::sync::{mpsc, Arc, Mutex};

pub(crate) type WindowCache = Arc<Mutex<Vec<WindowInfo>>>;
pub(crate) type SharedPreviewCache = Arc<Mutex<crate::picker::PreviewMap>>;
static NEXT_SHOW_ID: AtomicU64 = AtomicU64::new(1);

#[derive(Clone)]
pub(super) struct PickerCaches {
    last_window_count: Arc<AtomicUsize>,
    window_cache: WindowCache,
    icon_cache: SharedIconCache,
    preview_cache: SharedPreviewCache,
    refresh_generation: Arc<AtomicUsize>,
    show_generation: Arc<AtomicUsize>,
    show_id: Arc<AtomicU64>,
    capture_scheduler: PreviewCaptureScheduler,
    warmer: WarmerControl,
}

#[derive(Clone)]
struct PickerState {
    current: PickerWindowState,
    tracker: MonitorTracker,
    caches: PickerCaches,
    has_shown_once: Arc<AtomicBool>,
}

impl PickerCaches {
    fn new() -> Self {
        Self {
            last_window_count: Arc::new(AtomicUsize::new(super::default_estimated_window_count())),
            window_cache: Arc::new(Mutex::new(Vec::new())),
            icon_cache: Arc::new(Mutex::new(HashMap::new())),
            preview_cache: Arc::new(Mutex::new(HashMap::new())),
            refresh_generation: Arc::new(AtomicUsize::new(0)),
            show_generation: Arc::new(AtomicUsize::new(0)),
            show_id: Arc::new(AtomicU64::new(0)),
            capture_scheduler: PreviewCaptureScheduler::new(),
            warmer: WarmerControl::new(),
        }
    }

    fn next_show_generation(&self) -> usize {
        self.show_generation.fetch_add(1, Ordering::AcqRel) + 1
    }

    fn record_show(&self, show_id: u64) {
        self.show_id.store(show_id, Ordering::Release);
    }
}

impl PickerState {
    fn open_picker(&self, config: &AltTabConfig, reverse: bool, show_id: u64, cx: &mut App) {
        let req = OpenPickerRequest {
            config,
            current: &self.current,
            tracker: &self.tracker,
            last_window_count: self.caches.last_window_count.clone(),
            icon_cache: self.caches.icon_cache.clone(),
            window_cache: self.caches.window_cache.clone(),
            preview_cache: self.caches.preview_cache.clone(),
            has_shown_once: self.has_shown_once.clone(),
            reverse,
            show_id,
        };
        open_picker(&req, cx);
    }
}

fn next_show_id() -> u64 {
    NEXT_SHOW_ID.fetch_add(1, Ordering::Relaxed)
}

pub(crate) fn run_app(
    config: AltTabConfig,
    rx: mpsc::Receiver<daemon::Command>,
    show_on_start: bool,
) {
    let app = Application::new();

    app.run(move |cx: &mut App| {
        qol_gpui::keepalive::open_keepalive(cx, None);
        super::platform::set_accessory_policy();

        let state = PickerState {
            current: picker_window_state(),
            tracker: MonitorTracker::start(cx),
            caches: PickerCaches::new(),
            has_shown_once: Arc::new(AtomicBool::new(false)),
        };

        super::monitor_listener::spawn(
            cx,
            super::monitor_listener::ListenerInputs {
                tracker: state.tracker.clone(),
                current: state.current.clone(),
                last_window_count: state.caches.last_window_count.clone(),
                window_cache: state.caches.window_cache.clone(),
                icon_cache: state.caches.icon_cache.clone(),
                preview_cache: state.caches.preview_cache.clone(),
                refresh_generation: state.caches.refresh_generation.clone(),
                show_id: state.caches.show_id.clone(),
                capture_scheduler: state.caches.capture_scheduler.clone(),
                warmer: state.caches.warmer.clone(),
            },
        );
        super::platform::pre_create(
            &config,
            &state.current,
            state.caches.preview_cache.clone(),
            &state.tracker,
            cx,
        );

        if show_on_start {
            let show_id = next_show_id();
            state.caches.record_show(show_id);
            state.open_picker(&config, false, show_id, cx);
        }
        spawn_daemon_loop(cx, rx, state);
    });
}

fn picker_window_state() -> PickerWindowState {
    std::rc::Rc::new(std::cell::RefCell::new(
        qol_gpui::window::ActiveWindows::default(),
    ))
}

fn spawn_daemon_loop(cx: &mut App, rx: mpsc::Receiver<daemon::Command>, state: PickerState) {
    qol_gpui::command_loop::spawn_command_loop(cx, rx, move |cx, cmd| {
        let state = state.clone();
        async move {
            match cmd {
                daemon::Command::Show => {
                    dispatch_show(&cx, false, &state).await;
                    LoopFlow::Continue
                }
                daemon::Command::ShowReverse => {
                    dispatch_show(&cx, true, &state).await;
                    LoopFlow::Continue
                }
                daemon::Command::Reload => {
                    dispatch_reload(&cx, &state).await;
                    LoopFlow::Continue
                }
                daemon::Command::Settings => {
                    dispatch_settings(&cx, &state).await;
                    LoopFlow::Continue
                }
                daemon::Command::Kill => LoopFlow::Stop,
            }
        }
    });
}

async fn dispatch_settings(cx: &AsyncApp, state: &PickerState) {
    let opened = qol_gpui::settings_panel::open_plugin_settings(
        crate::config::PLUGIN_ID,
        "Alt Tab Settings",
        crate::config::contract(),
        state.tracker.clone(),
        qol_gpui::settings_panel::SettingsRuntime::empty(),
        cx,
    )
    .await;
    if let Err(error) = opened {
        eprintln!("[alt-tab/daemon] settings panel failed, opening browser: {error:#}");
        crate::runtime::open_settings_page();
    }
}

async fn dispatch_reload(cx: &AsyncApp, state: &PickerState) {
    let config = crate::config::load_alt_tab_config();
    let current = state.current.clone();
    let _ = cx.update(move |_cx| {
        qol_gpui::popup_window::set_ghost_debug(
            config.display.ghost_opacity,
            config.display.ghost_debug_color.as_deref(),
        );
        if crate::app::PICKER_VISIBLE.load(Ordering::Relaxed) {
            return;
        }
        let keys = current.borrow().keys();
        qol_gpui::ghost::reconcile_active(&keys, super::platform::picker_window_title);
    });
}

async fn dispatch_show(cx: &AsyncApp, reverse: bool, state: &PickerState) {
    #[cfg(debug_assertions)]
    let t_total = std::time::Instant::now();
    let show_id = next_show_id();
    let generation = state.caches.next_show_generation();
    state.caches.record_show(show_id);
    qol_runtime::probe!("SHOW_RECV", "show_id={show_id} reverse={reverse}");

    if crate::app::PICKER_VISIBLE.load(Ordering::Relaxed) {
        let state_fast = state.clone();
        let cycled = cx
            .update(move |app_cx| {
                #[cfg(debug_assertions)]
                crate::app::set_cycle_origin(t_total);
                let cycled =
                    super::try_cycle_visible(&state_fast.current, reverse, show_id, app_cx);
                #[cfg(debug_assertions)]
                crate::app::clear_cycle_origin();
                cycled
            })
            .unwrap_or(false);
        qol_runtime::probe!("SHOW_CYCLE_FAST", "show_id={show_id} cycled={cycled}");
        if cycled {
            #[cfg(debug_assertions)]
            eprintln!(
                "[alt-tab/daemon] fast cycle, no requery ({}ms)",
                t_total.elapsed().as_millis()
            );
            let cached_previews = state
                .caches
                .preview_cache
                .lock()
                .map(|cache| cache.len())
                .unwrap_or(0);
            let reveal_frame = if cached_previews > 0 {
                "cached"
            } else {
                "placeholder"
            };
            #[cfg(debug_assertions)]
            let total_ms = t_total.elapsed().as_millis();
            #[cfg(not(debug_assertions))]
            let total_ms = 0_u128;
            qol_runtime::probe!(
                "SHOW_TIMING",
                "show_id={show_id} show_generation={generation} total={total_ms}ms config=0ms query=0ms(0 windows) update=0ms capture_lane=none active_workers={} cancellation_reason=none reveal_frame={reveal_frame} reveal_frame_ms={total_ms} first_paint_latency_ms=not_measured",
                state.caches.capture_scheduler.active_workers(),
            );
            return;
        }
    }

    #[cfg(debug_assertions)]
    let t_config = std::time::Instant::now();
    let config = crate::config::load_alt_tab_config();
    #[cfg(debug_assertions)]
    let config_ms = t_config.elapsed().as_millis();
    qol_gpui::popup_window::set_ghost_debug(
        config.display.ghost_opacity,
        config.display.ghost_debug_color.as_deref(),
    );

    let executor = cx.background_executor().clone();
    let show_minimized = config.display.show_minimized;
    let switchable = crate::config::SwitchablePanels::resolve(&config.switchable_panels);

    #[cfg(debug_assertions)]
    let t_query = std::time::Instant::now();
    let windows = executor
        .spawn(async move {
            Platform
                .visible_windows(show_minimized, &switchable)
                .unwrap_or_default()
        })
        .await;
    let rendered_icons = refresh_icon_cache(&executor, &windows, &state.caches.icon_cache).await;
    #[cfg(debug_assertions)]
    let (query_ms, window_count) = (t_query.elapsed().as_millis(), windows.len());

    if !has_windows(&windows) {
        #[cfg(debug_assertions)]
        eprintln!("[alt-tab/daemon] no windows, skipping open");
        return;
    }
    let state_for_update = state.clone();
    #[cfg(debug_assertions)]
    let t_update = std::time::Instant::now();
    let _ = cx.update(move |app_cx| {
        apply_show_windows(&state_for_update.caches, windows, app_cx);
        if let Some(icons) = rendered_icons {
            commit_icons_to_shared_cache(&state_for_update.caches.icon_cache, icons, app_cx);
        }
        #[cfg(debug_assertions)]
        crate::app::set_cycle_origin(t_total);
        state_for_update.open_picker(&config, reverse, show_id, app_cx);
        #[cfg(debug_assertions)]
        crate::app::clear_cycle_origin();
    });
    #[cfg(debug_assertions)]
    let update_ms = t_update.elapsed().as_millis();
    #[cfg(debug_assertions)]
    let total_ms = t_total.elapsed().as_millis();
    #[cfg(not(debug_assertions))]
    let (config_ms, query_ms, window_count, update_ms, total_ms) =
        (0_u128, 0_u128, 0_usize, 0_u128, 0_u128);
    let cached_previews = state
        .caches
        .preview_cache
        .lock()
        .map(|cache| cache.len())
        .unwrap_or(0);
    let reveal_frame = if cached_previews > 0 {
        "cached"
    } else {
        "placeholder"
    };

    qol_runtime::probe!(
        "SHOW_TIMING",
        "show_id={show_id} show_generation={generation} total={total_ms}ms config={config_ms}ms query={query_ms}ms({window_count} windows) update={update_ms}ms capture_lane=none active_workers={} cancellation_reason=none reveal_frame={reveal_frame} reveal_frame_ms={update_ms} first_paint_latency_ms=not_measured",
        state.caches.capture_scheduler.active_workers(),
    );
}

pub(super) fn apply_show_windows(caches: &PickerCaches, windows: Vec<WindowInfo>, app: &mut App) {
    apply_window_cache(
        &caches.last_window_count,
        &caches.window_cache,
        &caches.icon_cache,
        &caches.preview_cache,
        windows,
        app,
    );
}

pub(super) fn apply_window_cache(
    last_window_count: &AtomicUsize,
    window_cache: &WindowCache,
    icon_cache: &SharedIconCache,
    preview_cache: &SharedPreviewCache,
    windows: Vec<WindowInfo>,
    app: &mut App,
) {
    last_window_count.store(windows.len().max(1), Ordering::Relaxed);
    prune_previews(preview_cache, &windows, app);
    prune_icons(icon_cache, &windows, app);
    replace_window_cache(window_cache, windows);
}

fn replace_window_cache(window_cache: &WindowCache, windows: Vec<WindowInfo>) {
    let Ok(mut cache) = window_cache.lock() else {
        return;
    };
    *cache = windows;
}

fn prune_previews(preview_cache: &SharedPreviewCache, windows: &[WindowInfo], app: &mut App) {
    let active: HashSet<u32> = windows.iter().map(|w| w.id).collect();
    #[cfg(debug_assertions)]
    crate::rendering::preview_trace::retain_active(&active);
    let Ok(mut cache) = preview_cache.lock() else {
        return;
    };
    crate::rendering::image_registry::retain_or_release(&mut *cache, app, None, |id| {
        active.contains(id)
    });
}

fn prune_icons(icon_cache: &SharedIconCache, windows: &[WindowInfo], app: &mut App) {
    let active: HashSet<String> = windows.iter().map(|w| w.app_name.clone()).collect();
    let Ok(mut cache) = icon_cache.lock() else {
        return;
    };
    crate::rendering::image_registry::retain_or_release(&mut *cache, app, None, |name| {
        active.contains(name)
    });
}

pub(super) async fn refresh_icon_cache(
    executor: &gpui::BackgroundExecutor,
    windows: &[WindowInfo],
    icon_cache: &SharedIconCache,
) -> Option<crate::picker::IconMap> {
    let cached_names = cached_icon_names(icon_cache);
    let icon_windows = missing_icon_windows(windows, &cached_names);
    if icon_windows.is_empty() {
        return None;
    }
    let raw_icons = executor
        .spawn(async move { capture::get_app_icons(&icon_windows) })
        .await;
    if raw_icons.is_empty() {
        return None;
    }
    Some(build_icon_cache(raw_icons))
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

pub(super) fn commit_icons_to_shared_cache(
    icon_cache: &SharedIconCache,
    rendered: crate::picker::IconMap,
    app: &mut App,
) {
    let Ok(mut cache) = icon_cache.lock() else {
        return;
    };
    crate::rendering::image_registry::extend_with(&mut *cache, rendered, app, None);
}

fn has_windows(windows: &[WindowInfo]) -> bool {
    !windows.is_empty()
}

#[cfg(test)]
mod show_guard_tests {
    use super::{has_windows, PickerCaches};
    use crate::discovery::WindowInfo;
    use std::sync::atomic::Ordering;

    fn w(id: u32) -> WindowInfo {
        WindowInfo {
            id,
            title: String::new(),
            app_name: String::new(),
            icon: None,
            width: 0.0,
            height: 0.0,
            is_minimized: false,
        }
    }

    #[test]
    fn empty_list_does_not_show() {
        let cases: &[(&[WindowInfo], bool)] = &[
            (&[], false),
            (&[w(1)], true),
            (&[w(1), w(2)], true),
            (&[w(1), w(2), w(3)], true),
        ];
        for (windows, expected) in cases {
            assert_eq!(
                has_windows(windows),
                *expected,
                "windows.len()={}",
                windows.len()
            );
        }
    }

    #[test]
    fn show_generation_does_not_supersede_data_refresh() {
        let caches = PickerCaches::new();
        let data_generation = caches.refresh_generation.fetch_add(1, Ordering::AcqRel) + 1;
        let show_generation = caches.next_show_generation();

        assert_eq!(data_generation, 1);
        assert_eq!(show_generation, 1);
        assert_eq!(
            caches.refresh_generation.load(Ordering::Acquire),
            data_generation
        );
    }
}
