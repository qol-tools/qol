use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{mpsc, Arc, Mutex, OnceLock};
use std::time::Duration;

use gpui::*;
use qol_gpui::monitor::MonitorTracker;
use qol_gpui::protocol::{RuntimeEvent, RuntimeEventKind};
use qol_gpui::window::PopupPlacement;

use super::run::{SharedPreviewCache, WindowCache};
use crate::app::PICKER_VISIBLE;
use crate::discovery::{Platform, WindowDiscovery};
use crate::{PickerWindowState, SharedIconCache};

const DATA_REFRESH_DELAY_MS: u64 = 75;

static DATA_REFRESH_TX: OnceLock<mpsc::Sender<RefreshRequest>> = OnceLock::new();
static MONITOR_BOUNDS_UNKNOWN_LOGGED: AtomicBool = AtomicBool::new(false);

type MonitorBoundsCache = Arc<Mutex<Option<Vec<qol_gpui::MonitorBounds>>>>;

#[derive(Clone, Copy, Default)]
struct RefreshRequest {
    refresh_frontmost: bool,
    refresh_previous_frontmost: bool,
}

impl RefreshRequest {
    fn frontmost() -> Self {
        Self {
            refresh_frontmost: true,
            refresh_previous_frontmost: false,
        }
    }

    fn previous_frontmost() -> Self {
        Self {
            refresh_frontmost: false,
            refresh_previous_frontmost: true,
        }
    }

    fn merge(self, next: Self) -> Self {
        Self {
            refresh_frontmost: self.refresh_frontmost || next.refresh_frontmost,
            refresh_previous_frontmost: self.refresh_previous_frontmost
                || next.refresh_previous_frontmost,
        }
    }
}

#[derive(Clone)]
pub(crate) struct ListenerInputs {
    pub tracker: MonitorTracker,
    pub current: PickerWindowState,
    pub last_window_count: Arc<AtomicUsize>,
    pub window_cache: WindowCache,
    pub icon_cache: SharedIconCache,
    pub preview_cache: SharedPreviewCache,
    pub refresh_generation: Arc<AtomicUsize>,
}

#[derive(Clone)]
struct ListenerState {
    inputs: ListenerInputs,
    monitor_bounds: MonitorBoundsCache,
}

pub(crate) fn spawn(cx: &mut App, inputs: ListenerInputs) {
    let (refresh_tx, refresh_rx) = mpsc::channel::<RefreshRequest>();
    let _ = DATA_REFRESH_TX.set(refresh_tx);
    let state = ListenerState {
        monitor_bounds: initial_monitor_bounds(&inputs.tracker),
        inputs,
    };
    spawn_data_refresh_listener_thread();
    qol_gpui::event_router::spawn_runtime_event_router(
        cx,
        vec![qol_gpui::protocol::RuntimeEventKind::ActiveMonitorChanged],
        {
            let state = state.clone();
            move |app_cx, event| reposition_ghost_only(&state, event, app_cx)
        },
    );
    qol_gpui::event_router::spawn_runtime_event_router(
        cx,
        vec![qol_gpui::protocol::RuntimeEventKind::MonitorsChanged],
        {
            let state = state.clone();
            move |app_cx, event| rebuild_ghosts_for_topology(&state, event, app_cx)
        },
    );
    spawn_active_window_warmer(cx, state.clone());
    spawn_data_refresh_router(cx, refresh_rx, state.inputs);
}

const ACTIVE_WARM_INTERVAL_MS: u64 = 250;
const WARM_IDLE_GRACE_S: f64 = 1.0;
const FULLSCREEN_EDGE_TOLERANCE_PX: f32 = 8.0;

// While the picker is hidden and the user is actively working, re-shoot the
// active window (idx 0) on a light timer so the next show reveals a current
// cache with no on-open capture. The previous-focused window (idx 1) is kept
// fresh by the FocusChanged capture-on-leave, not here - it does not change
// while it is in the background. Paused while the picker is visible and while
// the user is idle, since nothing is changing in either case.
fn spawn_active_window_warmer(cx: &mut App, state: ListenerState) {
    cx.spawn(async move |cx: &mut AsyncApp| {
        let executor = cx.background_executor().clone();
        loop {
            executor
                .timer(Duration::from_millis(ACTIVE_WARM_INTERVAL_MS))
                .await;
            if PICKER_VISIBLE.load(Ordering::Relaxed) {
                continue;
            }
            if cx
                .update(|app_cx| warm_active_window(&state, app_cx))
                .is_err()
            {
                break;
            }
        }
    })
    .detach();
}

fn warm_active_window(state: &ListenerState, app_cx: &mut App) {
    if super::platform::seconds_since_last_input().is_some_and(|idle| idle > WARM_IDLE_GRACE_S) {
        return;
    }
    let windows = state
        .inputs
        .window_cache
        .lock()
        .map(|c| c.clone())
        .unwrap_or_default();
    if windows.is_empty() {
        return;
    }
    match frontmost_window_fullscreen_guard(&windows, &state.monitor_bounds) {
        FullscreenGuard::NotFullscreen => {}
        FullscreenGuard::Fullscreen => {
            qol_runtime::probe!(
                "PREVIEW_WARM_SKIP",
                "reason=fullscreen wid={}",
                windows[0].id
            );
            return;
        }
        FullscreenGuard::UnknownMonitors => {
            log_unknown_monitor_bounds_once();
            qol_runtime::probe!(
                "PREVIEW_WARM_SKIP",
                "reason=unknown_monitors wid={}",
                windows[0].id
            );
            return;
        }
    }
    super::gather::spawn_frontmost_warm(
        super::gather::FrontmostWarmRequest {
            windows,
            preview_cache: state.inputs.preview_cache.clone(),
        },
        app_cx,
    );
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum FullscreenGuard {
    NotFullscreen,
    Fullscreen,
    UnknownMonitors,
}

fn frontmost_window_fullscreen_guard(
    windows: &[crate::discovery::WindowInfo],
    monitor_bounds: &MonitorBoundsCache,
) -> FullscreenGuard {
    let Some(frontmost) = windows.first() else {
        return FullscreenGuard::NotFullscreen;
    };
    let Some(monitors) = snapshot_monitor_bounds(monitor_bounds) else {
        return FullscreenGuard::UnknownMonitors;
    };
    if window_covers_any_monitor(frontmost, &monitors) {
        return FullscreenGuard::Fullscreen;
    }
    FullscreenGuard::NotFullscreen
}

fn initial_monitor_bounds(tracker: &MonitorTracker) -> MonitorBoundsCache {
    let monitors = tracker
        .all_monitors()
        .into_iter()
        .map(|monitor| {
            let bounds = monitor.bounds();
            qol_gpui::MonitorBounds {
                x: bounds.origin.x.to_f64() as f32,
                y: bounds.origin.y.to_f64() as f32,
                width: bounds.size.width.to_f64() as f32,
                height: bounds.size.height.to_f64() as f32,
            }
        })
        .collect();
    Arc::new(Mutex::new(known_monitor_bounds(monitors)))
}

fn update_monitor_bounds_from_event(cache: &MonitorBoundsCache, event: &RuntimeEvent) {
    let RuntimeEvent::MonitorsChanged { monitors } = event else {
        return;
    };
    write_monitor_bounds(cache, monitors.clone());
}

fn write_monitor_bounds(cache: &MonitorBoundsCache, monitors: Vec<qol_gpui::MonitorBounds>) {
    let next = known_monitor_bounds(monitors);
    if next.is_some() {
        MONITOR_BOUNDS_UNKNOWN_LOGGED.store(false, Ordering::Release);
    }
    if let Ok(mut slot) = cache.lock() {
        *slot = next;
    }
}

fn known_monitor_bounds(
    monitors: Vec<qol_gpui::MonitorBounds>,
) -> Option<Vec<qol_gpui::MonitorBounds>> {
    if monitors.is_empty() {
        return None;
    }
    Some(monitors)
}

fn snapshot_monitor_bounds(cache: &MonitorBoundsCache) -> Option<Vec<qol_gpui::MonitorBounds>> {
    cache.lock().ok().and_then(|slot| slot.clone())
}

fn log_unknown_monitor_bounds_once() {
    if MONITOR_BOUNDS_UNKNOWN_LOGGED.swap(true, Ordering::AcqRel) {
        return;
    }
    eprintln!("[alt-tab/warmer] skipping hidden preview warm: monitor bounds unavailable");
}

fn window_covers_any_monitor(
    window: &crate::discovery::WindowInfo,
    monitors: &[qol_gpui::MonitorBounds],
) -> bool {
    if window.is_minimized {
        return false;
    }
    monitors
        .iter()
        .any(|monitor| window_covers_monitor(window, *monitor))
}

fn window_covers_monitor(
    window: &crate::discovery::WindowInfo,
    monitor: qol_gpui::MonitorBounds,
) -> bool {
    if window.width <= 0.0 || window.height <= 0.0 || monitor.width <= 0.0 || monitor.height <= 0.0
    {
        return false;
    }
    let window_right = window.x + window.width;
    let window_bottom = window.y + window.height;
    let monitor_right = monitor.x + monitor.width;
    let monitor_bottom = monitor.y + monitor.height;
    window.x <= monitor.x + FULLSCREEN_EDGE_TOLERANCE_PX
        && window.y <= monitor.y + FULLSCREEN_EDGE_TOLERANCE_PX
        && window_right >= monitor_right - FULLSCREEN_EDGE_TOLERANCE_PX
        && window_bottom >= monitor_bottom - FULLSCREEN_EDGE_TOLERANCE_PX
}

pub(crate) fn request_data_refresh() {
    request_refresh(RefreshRequest::default());
}

pub(crate) fn request_frontmost_preview_refresh() {
    request_refresh(RefreshRequest::frontmost());
}

pub(crate) fn request_previous_frontmost_preview_refresh() {
    request_refresh(RefreshRequest::previous_frontmost());
}

fn request_refresh(request: RefreshRequest) {
    qol_runtime::probe!(
        "REFRESH_REQ",
        "queued refresh_frontmost={} refresh_previous_frontmost={}",
        request.refresh_frontmost,
        request.refresh_previous_frontmost
    );
    if let Some(tx) = DATA_REFRESH_TX.get() {
        let _ = tx.send(request);
    }
}

fn spawn_data_refresh_listener_thread() {
    std::thread::spawn(data_refresh_listener_loop);
}

#[cfg(unix)]
fn data_refresh_listener_loop() {
    let client = qol_gpui::PlatformStateClient::from_env();
    let Some(mut subscription) = client.subscribe(vec![
        RuntimeEventKind::WindowListChanged,
        RuntimeEventKind::FocusChanged,
    ]) else {
        return;
    };
    while let Some(event) = subscription.next_event() {
        match event {
            RuntimeEvent::FocusChanged { .. } => request_previous_frontmost_preview_refresh(),
            _ => request_data_refresh(),
        }
    }
}

#[cfg(not(unix))]
fn data_refresh_listener_loop() {}

fn spawn_data_refresh_router(
    cx: &mut App,
    rx: mpsc::Receiver<RefreshRequest>,
    inputs: ListenerInputs,
) {
    let rx = Arc::new(Mutex::new(rx));
    cx.spawn(async move |cx: &mut AsyncApp| loop {
        let Some(request) = recv(cx, rx.clone()).await else {
            return;
        };
        let request = drain(&rx, request);
        let _ = cx.update(|app_cx| trigger_data_refresh(&inputs, app_cx, request));
    })
    .detach();
}

async fn recv(
    cx: &AsyncApp,
    rx: Arc<Mutex<mpsc::Receiver<RefreshRequest>>>,
) -> Option<RefreshRequest> {
    cx.background_executor()
        .spawn(async move { rx.lock().ok()?.recv().ok() })
        .await
}

fn drain(
    rx: &Arc<Mutex<mpsc::Receiver<RefreshRequest>>>,
    mut request: RefreshRequest,
) -> RefreshRequest {
    if let Ok(guard) = rx.lock() {
        while let Ok(next) = guard.try_recv() {
            request = request.merge(next);
        }
    }
    request
}

fn reposition_ghost_only(state: &ListenerState, event: &RuntimeEvent, app_cx: &mut App) {
    let inputs = &state.inputs;
    #[cfg(debug_assertions)]
    if let RuntimeEvent::ActiveMonitorChanged { monitor_idx, .. } = event {
        qol_runtime::probe!("PLUGIN_RECV_AMC", "monitor_idx={:?}", monitor_idx);
    }
    qol_gpui::ghost::record_active_monitor(event);
    if PICKER_VISIBLE.load(Ordering::Relaxed) {
        #[cfg(debug_assertions)]
        eprintln!("[alt-tab/listener] picker visible, skipping ghost reposition");
        return;
    }
    let _reason = qol_gpui::popup_window::reason_scope("amc");
    let reconciled = if super::platform::reuse_picker_across_targets() {
        recenter_single_ghost(inputs, event, app_cx)
    } else {
        qol_gpui::ghost::reconcile_from_event(
            event,
            &inputs.current.borrow(),
            super::platform::picker_window_title,
            || inputs.tracker.snapshot_monitor(),
        )
    };
    if reconciled {
        request_data_refresh();
    }
}

fn recenter_single_ghost(inputs: &ListenerInputs, event: &RuntimeEvent, app_cx: &mut App) -> bool {
    let monitor =
        qol_gpui::ghost::record_active_monitor(event).or_else(|| inputs.tracker.snapshot_monitor());
    let Some(monitor) = monitor else {
        return false;
    };
    let placement = PopupPlacement::from_monitor(Some(monitor));
    let target = placement.target();
    let Some((source_key, handle)) = inputs.current.borrow().iter().into_iter().next() else {
        return false;
    };
    let layout = super::reuse::compute_layout(
        &super::reuse::LayoutInput {
            placement: &placement,
        },
        app_cx,
    );
    let synced = handle
        .update(app_cx, |view, window: &mut Window, _cx| {
            let title = view.picker_title.clone();
            super::platform::sync_picker_window_layout(
                &title,
                window,
                layout.bounds.origin,
                layout.size,
            )
        })
        .unwrap_or(false);
    if synced && source_key != target {
        let mut current = inputs.current.borrow_mut();
        current.remove(source_key);
        current.insert(target, handle);
    }
    qol_runtime::probe!(
        "GHOST_RECENTER",
        "target={},{} synced={synced}",
        target.x,
        target.y
    );
    synced
}

fn rebuild_ghosts_for_topology(state: &ListenerState, event: &RuntimeEvent, app_cx: &mut App) {
    update_monitor_bounds_from_event(&state.monitor_bounds, event);
    let inputs = &state.inputs;
    if PICKER_VISIBLE.load(Ordering::Relaxed) {
        #[cfg(debug_assertions)]
        eprintln!("[alt-tab/listener] picker visible, skipping topology rebuild");
        return;
    }
    let _reason = qol_gpui::popup_window::reason_scope("topology");
    let config = crate::config::load_alt_tab_config();
    let rebuilt =
        qol_gpui::ghost::rebuild_on_topology(event, false, &inputs.current, app_cx, |cx| {
            super::platform::pre_create(
                &config,
                &inputs.current,
                inputs.preview_cache.clone(),
                &inputs.tracker,
                cx,
            );
        });
    if rebuilt {
        request_data_refresh();
    }
}

fn trigger_data_refresh(inputs: &ListenerInputs, app_cx: &mut App, request: RefreshRequest) {
    let generation = inputs.refresh_generation.fetch_add(1, Ordering::AcqRel) + 1;
    qol_runtime::probe!(
        "REFRESH_TRIGGER",
        "gen={generation} refresh_frontmost={} refresh_previous_frontmost={}",
        request.refresh_frontmost,
        request.refresh_previous_frontmost
    );
    let inputs = inputs.clone();
    app_cx
        .spawn(async move |cx: &mut AsyncApp| {
            refresh_data(cx, inputs, generation, request).await;
        })
        .detach();
}

async fn refresh_data(
    cx: &mut AsyncApp,
    inputs: ListenerInputs,
    generation: usize,
    request: RefreshRequest,
) {
    let config = crate::config::load_alt_tab_config();
    let show_minimized = config.display.show_minimized;
    let executor = cx.background_executor().clone();
    executor
        .timer(Duration::from_millis(DATA_REFRESH_DELAY_MS))
        .await;
    if inputs.refresh_generation.load(Ordering::Acquire) != generation {
        qol_runtime::probe!("REFRESH_RUN", "gen={generation} outcome=superseded");
        return;
    }
    let windows = executor
        .spawn(async move { Platform.visible_windows(show_minimized).unwrap_or_default() })
        .await;
    if windows.is_empty() {
        qol_runtime::probe!("REFRESH_RUN", "gen={generation} outcome=empty");
        return;
    }
    let rendered_icons =
        super::run::refresh_icon_cache(&executor, &windows, &inputs.icon_cache).await;
    let windows_for_previews = windows.clone();
    let _ = cx.update(move |app_cx| {
        if inputs.refresh_generation.load(Ordering::Acquire) != generation {
            qol_runtime::probe!("REFRESH_RUN", "gen={generation} outcome=stale_apply");
            return;
        }
        qol_runtime::probe!("REFRESH_RUN", "gen={generation} outcome=applied");
        super::run::apply_window_cache(
            &inputs.last_window_count,
            &inputs.window_cache,
            &inputs.icon_cache,
            &inputs.preview_cache,
            windows,
            app_cx,
        );
        if let Some(icons) = rendered_icons {
            super::run::commit_icons_to_shared_cache(&inputs.icon_cache, icons, app_cx);
        }
        let gathered = super::gather::gather(
            &config,
            &inputs.icon_cache,
            &inputs.window_cache,
            &inputs.preview_cache,
        );
        let picker_visible = PICKER_VISIBLE.load(Ordering::Relaxed);
        let reset_selection = if picker_visible {
            false
        } else {
            config.reset_selection_on_open
        };
        let active_target = if picker_visible {
            *crate::app::ACTIVE_PICKER_MONITOR.lock().unwrap()
        } else {
            let active_monitor =
                qol_gpui::ghost::active_monitor().or_else(|| inputs.tracker.snapshot_monitor());
            active_monitor.map(|m| PopupPlacement::from_monitor(Some(m)).target())
        };

        let rest_forward =
            reset_selection && config.open_behavior == crate::config::OpenBehavior::CycleOnce;
        apply_view_windows(
            &inputs.current,
            &gathered,
            reset_selection,
            rest_forward,
            app_cx,
        );

        let active_handle = active_target
            .and_then(|target| inputs.current.borrow().existing(target))
            .or_else(|| {
                inputs
                    .current
                    .borrow()
                    .iter()
                    .into_iter()
                    .next()
                    .map(|(_, h)| h)
            });

        if let Some(handle) = active_handle {
            super::gather::spawn_preview_fill(
                super::gather::PreviewFillRequest {
                    handle,
                    windows: windows_for_previews,
                    preview_cache: inputs.preview_cache.clone(),
                    refresh_frontmost: picker_visible || request.refresh_frontmost,
                    refresh_previous_frontmost: request.refresh_previous_frontmost,
                },
                app_cx,
            );
        }
        #[cfg(debug_assertions)]
        eprintln!(
            "[alt-tab/data-refresh] windows={} reset={} visible={}",
            gathered.windows.len(),
            reset_selection,
            picker_visible,
        );
    });
}

fn apply_view_windows(
    current: &PickerWindowState,
    gathered: &super::gather::GatheredWindows,
    reset_selection: bool,
    rest_forward: bool,
    app_cx: &mut App,
) {
    let handles: Vec<_> = current
        .borrow()
        .iter()
        .into_iter()
        .map(|(_, handle)| handle)
        .collect();
    for handle in handles {
        let _ = handle.update(app_cx, |view, window: &mut Window, cx| {
            view.apply_ghost_gathered(gathered, reset_selection, rest_forward, window, cx);
        });
    }
}

#[cfg(test)]
mod fullscreen_warm_policy_tests {
    use super::{
        frontmost_window_fullscreen_guard, known_monitor_bounds, window_covers_any_monitor,
        FullscreenGuard, MonitorBoundsCache,
    };
    use crate::discovery::WindowInfo;
    use qol_gpui::MonitorBounds;
    use std::sync::{Arc, Mutex};

    fn window(x: f32, y: f32, width: f32, height: f32) -> WindowInfo {
        WindowInfo {
            id: 1,
            title: String::new(),
            app_name: String::new(),
            preview_path: None,
            icon: None,
            x,
            y,
            width,
            height,
            is_minimized: false,
        }
    }

    fn monitor(x: f32, y: f32, width: f32, height: f32) -> MonitorBounds {
        MonitorBounds {
            x,
            y,
            width,
            height,
        }
    }

    fn cache(monitors: Vec<MonitorBounds>) -> MonitorBoundsCache {
        Arc::new(Mutex::new(known_monitor_bounds(monitors)))
    }

    #[test]
    fn exact_monitor_cover_is_fullscreen() {
        let win = window(0.0, 0.0, 1920.0, 1080.0);
        let monitors = [monitor(0.0, 0.0, 1920.0, 1080.0)];
        assert!(window_covers_any_monitor(&win, &monitors));
    }

    #[test]
    fn edge_tolerance_allows_borderless_jitter() {
        let win = window(-4.0, 3.0, 1927.0, 1074.0);
        let monitors = [monitor(0.0, 0.0, 1920.0, 1080.0)];
        assert!(window_covers_any_monitor(&win, &monitors));
    }

    #[test]
    fn maximized_under_menu_bar_is_not_fullscreen() {
        let win = window(0.0, 24.0, 1920.0, 1056.0);
        let monitors = [monitor(0.0, 0.0, 1920.0, 1080.0)];
        assert!(!window_covers_any_monitor(&win, &monitors));
    }

    #[test]
    fn minimized_window_is_not_fullscreen() {
        let mut win = window(0.0, 0.0, 1920.0, 1080.0);
        win.is_minimized = true;
        let monitors = [monitor(0.0, 0.0, 1920.0, 1080.0)];
        assert!(!window_covers_any_monitor(&win, &monitors));
    }

    #[test]
    fn negative_coordinate_monitor_can_match() {
        let win = window(-1440.0, 0.0, 1440.0, 900.0);
        let monitors = [
            monitor(0.0, 0.0, 1920.0, 1080.0),
            monitor(-1440.0, 0.0, 1440.0, 900.0),
        ];
        assert!(window_covers_any_monitor(&win, &monitors));
    }

    #[test]
    fn secondary_monitor_root_coordinates_match() {
        let win = window(2560.0, 0.0, 1920.0, 1080.0);
        let monitors = [
            monitor(0.0, 0.0, 2560.0, 1440.0),
            monitor(2560.0, 0.0, 1920.0, 1080.0),
        ];
        assert!(window_covers_any_monitor(&win, &monitors));
    }

    #[test]
    fn unknown_monitor_bounds_skip_warm() {
        let cache = Arc::new(Mutex::new(None));
        let windows = [window(0.0, 0.0, 1920.0, 1080.0)];
        assert_eq!(
            frontmost_window_fullscreen_guard(&windows, &cache),
            FullscreenGuard::UnknownMonitors
        );
    }

    #[test]
    fn empty_monitor_bounds_skip_warm() {
        let cache = cache(Vec::new());
        let windows = [window(0.0, 0.0, 1920.0, 1080.0)];
        assert_eq!(
            frontmost_window_fullscreen_guard(&windows, &cache),
            FullscreenGuard::UnknownMonitors
        );
    }

    #[test]
    fn known_non_fullscreen_bounds_allow_warm() {
        let cache = cache(vec![monitor(0.0, 0.0, 1920.0, 1080.0)]);
        let windows = [window(0.0, 24.0, 1920.0, 1056.0)];
        assert_eq!(
            frontmost_window_fullscreen_guard(&windows, &cache),
            FullscreenGuard::NotFullscreen
        );
    }
}
