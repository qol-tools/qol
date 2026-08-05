use futures::{channel::mpsc as futures_mpsc, future::select, FutureExt, StreamExt};
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
use std::sync::{mpsc, Arc, Mutex, OnceLock};
use std::time::{Duration, Instant};

use gpui::*;
use qol_gpui::monitor::MonitorTracker;
use qol_gpui::protocol::RuntimeEvent;
use qol_gpui::window::PopupPlacement;

use super::gather::{
    capture_target_for_lane, CaptureLane, PreviewCaptureRequest, PreviewCaptureScheduler,
};
use super::run::{SharedPreviewCache, WindowCache};
use crate::app::PICKER_VISIBLE;
use crate::discovery::{Platform, WindowDiscovery};
use crate::picker::{PickerWindowState, SharedIconCache};
use crate::rendering::RenderingFlow;

mod platform;

const DATA_REFRESH_DELAY_MS: u64 = 75;
const WARMER_DELAY_MS: u64 = 250;

static DATA_REFRESH_TX: OnceLock<mpsc::Sender<RefreshRequest>> = OnceLock::new();
static WARMER_CONTROL: OnceLock<WarmerControl> = OnceLock::new();
static WARMER_WAKE_TX: OnceLock<futures_mpsc::UnboundedSender<()>> = OnceLock::new();
static CAPTURE_SCHEDULER: OnceLock<PreviewCaptureScheduler> = OnceLock::new();
static REFRESH_GENERATION: OnceLock<Arc<AtomicUsize>> = OnceLock::new();
static LATEST_SHOW_ID: OnceLock<Arc<AtomicU64>> = OnceLock::new();
static FLOW_FILLS_PREVIEWS: AtomicBool = AtomicBool::new(true);

#[derive(Clone, Copy, Default)]
struct RefreshRequest {
    refresh_frontmost: bool,
    refresh_previous_frontmost: bool,
    focus_generation: u64,
}

impl RefreshRequest {
    fn previous_frontmost() -> Self {
        Self {
            refresh_frontmost: false,
            refresh_previous_frontmost: true,
            focus_generation: 0,
        }
    }

    fn merge(self, next: Self) -> Self {
        Self {
            refresh_frontmost: self.refresh_frontmost || next.refresh_frontmost,
            refresh_previous_frontmost: self.refresh_previous_frontmost
                || next.refresh_previous_frontmost,
            focus_generation: self.focus_generation.max(next.focus_generation),
        }
    }

    fn lane_name(self) -> &'static str {
        if self.refresh_frontmost && self.refresh_previous_frontmost {
            return "hidden_warmer+focus_leave";
        }
        if self.refresh_frontmost {
            return "hidden_warmer";
        }
        if self.refresh_previous_frontmost {
            return "focus_leave";
        }
        "data"
    }
}

#[derive(Clone)]
pub(crate) struct WarmerControl {
    state: Arc<Mutex<WarmerState>>,
}

#[derive(Default)]
struct WarmerState {
    next_activity: u64,
    last_activity: Option<Instant>,
    last_activity_generation: Option<u64>,
    latest_focus: Option<u64>,
    focus_result: Option<(u64, bool)>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum WarmerDecision {
    Idle,
    Waiting,
    Capture(u64),
    Skip(u64),
}

impl WarmerControl {
    pub(crate) fn new() -> Self {
        Self {
            state: Arc::new(Mutex::new(WarmerState::default())),
        }
    }

    fn record_activity(&self, focus_refresh: bool) -> u64 {
        let generation = self
            .state
            .lock()
            .map(|mut state| state.record_activity_at(Instant::now(), focus_refresh))
            .unwrap_or(0);
        wake_warmer();
        generation
    }

    fn prepare_refresh_request(&self, request: RefreshRequest) -> RefreshRequest {
        self.state
            .lock()
            .map(|state| state.prepare_refresh_request(request))
            .unwrap_or(request)
    }

    fn mark_refresh_applied(&self, focus_generation: u64, success: bool) {
        if focus_generation == 0 {
            return;
        }
        if let Ok(mut state) = self.state.lock() {
            state.mark_refresh_applied(focus_generation, success);
        }
        wake_warmer();
    }

    fn next_wait(&self, now: Instant) -> Option<Duration> {
        self.state
            .lock()
            .ok()
            .and_then(|state| state.next_wait(now))
    }

    fn take_due(&self, now: Instant) -> WarmerDecision {
        self.state
            .lock()
            .map(|mut state| state.take_due(now))
            .unwrap_or(WarmerDecision::Idle)
    }
}

impl WarmerState {
    fn record_activity_at(&mut self, now: Instant, focus_refresh: bool) -> u64 {
        self.next_activity = self.next_activity.wrapping_add(1);
        self.last_activity = Some(now);
        self.last_activity_generation = Some(self.next_activity);
        if focus_refresh {
            self.latest_focus = Some(self.next_activity);
            self.focus_result = None;
        } else if self.focus_result.is_some_and(|(_, success)| !success) {
            self.latest_focus = None;
            self.focus_result = None;
        }
        self.next_activity
    }

    fn next_wait(&self, now: Instant) -> Option<Duration> {
        let last_activity = self.last_activity?;
        let deadline = last_activity + Duration::from_millis(WARMER_DELAY_MS);
        if deadline <= now && self.focus_pending() {
            return None;
        }
        Some(deadline.saturating_duration_since(now))
    }

    fn mark_refresh_applied(&mut self, focus_generation: u64, success: bool) {
        if self.latest_focus == Some(focus_generation) {
            self.focus_result = Some((focus_generation, success));
        }
    }

    fn prepare_refresh_request(&self, mut request: RefreshRequest) -> RefreshRequest {
        request.focus_generation = self.latest_focus.unwrap_or(0);
        if self.focus_pending() {
            request.refresh_previous_frontmost = true;
        }
        request
    }

    fn take_due(&mut self, now: Instant) -> WarmerDecision {
        let Some(last_activity) = self.last_activity else {
            return WarmerDecision::Idle;
        };
        let deadline = last_activity + Duration::from_millis(WARMER_DELAY_MS);
        if now < deadline {
            return WarmerDecision::Waiting;
        }
        if self.focus_pending() {
            return WarmerDecision::Waiting;
        }
        let activity = self.next_activity;
        let activity_generation = self.last_activity_generation;
        self.last_activity = None;
        self.last_activity_generation = None;
        if let Some((focus_generation, false)) = self.focus_result {
            self.focus_result = None;
            if self.latest_focus == Some(focus_generation) {
                self.latest_focus = None;
            }
            if activity_generation == Some(focus_generation) {
                return WarmerDecision::Skip(activity);
            }
        }
        WarmerDecision::Capture(activity)
    }

    fn focus_pending(&self) -> bool {
        self.latest_focus.is_some_and(|generation| {
            self.focus_result
                .is_none_or(|(applied, _)| applied != generation)
        })
    }
}

fn wake_warmer() {
    if let Some(tx) = WARMER_WAKE_TX.get() {
        let _ = tx.unbounded_send(());
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
    pub show_id: Arc<AtomicU64>,
    pub capture_scheduler: PreviewCaptureScheduler,
    pub warmer: WarmerControl,
}

#[derive(Clone)]
struct ListenerState {
    inputs: ListenerInputs,
}

pub(crate) fn spawn(cx: &mut App, inputs: ListenerInputs) {
    refresh_flow_flag();
    let (refresh_tx, refresh_rx) = mpsc::channel::<RefreshRequest>();
    let (warmer_wake_tx, warmer_wake_rx) = futures_mpsc::unbounded();
    let _ = DATA_REFRESH_TX.set(refresh_tx);
    let state = ListenerState { inputs };
    let _ = WARMER_CONTROL.set(state.inputs.warmer.clone());
    let _ = WARMER_WAKE_TX.set(warmer_wake_tx);
    let _ = CAPTURE_SCHEDULER.set(state.inputs.capture_scheduler.clone());
    let _ = REFRESH_GENERATION.set(state.inputs.refresh_generation.clone());
    let _ = LATEST_SHOW_ID.set(state.inputs.show_id.clone());
    spawn_data_refresh_listener_thread();
    spawn_hidden_warmer(cx, state.inputs.clone(), warmer_wake_rx);
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
    spawn_data_refresh_router(cx, refresh_rx, state.inputs);
}

pub(crate) fn request_data_refresh() {
    request_refresh(RefreshRequest::default());
}

pub(crate) fn request_frontmost_preview_refresh() {
    record_hid_activity(false);
    qol_runtime::probe!(
        "REFRESH_REQ",
        "show_id={} generation={} lane=hidden_warmer queued_lane=hidden_warmer source=dismiss active_workers={} cancellation_reason=none reveal_frame=show_cache_or_placeholder first_paint_latency_ms=pending",
        latest_show_id(),
        refresh_generation(),
        active_capture_workers(),
    );
}

pub(crate) fn request_previous_frontmost_preview_refresh() {
    record_hid_activity(true);
    request_refresh(RefreshRequest::previous_frontmost());
}

pub(crate) fn record_recent_hid_activity() {
    record_hid_activity(false);
}

pub(crate) fn store_flow_fill(fills: bool) {
    FLOW_FILLS_PREVIEWS.store(fills, Ordering::Release);
}

pub(crate) fn refresh_flow_flag() {
    store_flow_fill(RenderingFlow::current().captures_preview_fill());
}

fn flow_fills_previews() -> bool {
    FLOW_FILLS_PREVIEWS.load(Ordering::Acquire)
}

fn record_hid_activity(focus_refresh: bool) -> u64 {
    if !flow_fills_previews() {
        return 0;
    }
    WARMER_CONTROL
        .get()
        .map(|control| control.record_activity(focus_refresh))
        .unwrap_or(0)
}

fn request_refresh(request: RefreshRequest) {
    let request = WARMER_CONTROL
        .get()
        .map(|warmer| warmer.prepare_refresh_request(request))
        .unwrap_or(request);
    qol_runtime::probe!(
        "REFRESH_REQ",
        "show_id={} generation={} lane=data queued_lane={} refresh_frontmost={} refresh_previous_frontmost={} focus_generation={} active_workers={} cancellation_reason=none reveal_frame=show_cache_or_placeholder first_paint_latency_ms=pending",
        latest_show_id(),
        refresh_generation(),
        request.lane_name(),
        request.refresh_frontmost,
        request.refresh_previous_frontmost,
        request.focus_generation,
        active_capture_workers(),
    );
    if let Some(tx) = DATA_REFRESH_TX.get() {
        let _ = tx.send(request);
    }
}

fn active_capture_workers() -> usize {
    CAPTURE_SCHEDULER
        .get()
        .map(PreviewCaptureScheduler::active_workers)
        .unwrap_or(0)
}

fn refresh_generation() -> usize {
    REFRESH_GENERATION
        .get()
        .map(|generation| generation.load(Ordering::Acquire))
        .unwrap_or(0)
}

fn latest_show_id() -> u64 {
    LATEST_SHOW_ID
        .get()
        .map(|show_id| show_id.load(Ordering::Acquire))
        .unwrap_or(0)
}

fn spawn_hidden_warmer(
    cx: &mut App,
    inputs: ListenerInputs,
    mut wake_rx: futures_mpsc::UnboundedReceiver<()>,
) {
    cx.spawn(async move |cx: &mut AsyncApp| {
        let executor = cx.background_executor().clone();
        loop {
            if PICKER_VISIBLE.load(Ordering::Acquire) {
                if wake_rx.next().await.is_none() {
                    return;
                }
                continue;
            }
            let Some(delay) = inputs.warmer.next_wait(Instant::now()) else {
                if wake_rx.next().await.is_none() {
                    return;
                }
                continue;
            };
            let timer = executor.timer(delay).fuse();
            let wake = wake_rx.next().fuse();
            futures::pin_mut!(timer, wake);
            match select(timer, wake).await {
                futures::future::Either::Left(_) => {
                    let _ = cx.update(|app_cx| run_warmer(&inputs, app_cx));
                }
                futures::future::Either::Right((signal, _)) => {
                    if signal.is_none() {
                        return;
                    }
                }
            }
        }
    })
    .detach();
}

fn run_warmer(inputs: &ListenerInputs, app_cx: &mut App) {
    if PICKER_VISIBLE.load(Ordering::Acquire) {
        return;
    }
    let decision = inputs.warmer.take_due(Instant::now());
    match decision {
        WarmerDecision::Capture(activity) => enqueue_warmer_from_cache(inputs, activity, app_cx),
        WarmerDecision::Skip(activity) => {
            qol_runtime::probe!(
                "REFRESH_RUN",
                "show_id={} lane=hidden_warmer outcome=skipped activity_generation={} active_workers={} cancellation_reason=focus_refresh_failed reveal_frame=show_cache_or_placeholder first_paint_latency_ms=none",
                latest_show_id(),
                activity,
                active_capture_workers(),
            );
        }
        WarmerDecision::Idle | WarmerDecision::Waiting => {}
    }
}

fn enqueue_warmer_from_cache(inputs: &ListenerInputs, activity_generation: u64, app_cx: &mut App) {
    if !RenderingFlow::current().captures_preview_fill() {
        qol_runtime::probe!(
            "REFRESH_RUN",
            "show_id={} lane=hidden_warmer outcome=skipped activity_generation={} active_workers={} cancellation_reason=preview_plane_flow reveal_frame=show_cache_or_placeholder first_paint_latency_ms=none",
            latest_show_id(),
            activity_generation,
            active_capture_workers(),
        );
        return;
    }
    let windows = inputs
        .window_cache
        .lock()
        .ok()
        .map(|windows| windows.clone())
        .unwrap_or_default();
    let Some(window) = capture_target_for_lane(CaptureLane::HiddenWarmer, &windows) else {
        return;
    };
    let Some(handle) = active_picker_handle(inputs) else {
        return;
    };
    inputs.capture_scheduler.enqueue(
        CaptureLane::HiddenWarmer,
        PreviewCaptureRequest {
            handle,
            window,
            preview_cache: inputs.preview_cache.clone(),
            show_id: inputs.show_id.load(Ordering::Acquire),
        },
        app_cx,
    );
}

fn active_picker_handle(inputs: &ListenerInputs) -> Option<WindowHandle<crate::app::AltTabApp>> {
    let active_target = qol_gpui::ghost::active_monitor()
        .or_else(|| inputs.tracker.snapshot_monitor())
        .map(|monitor| PopupPlacement::from_monitor(Some(monitor)).target());
    active_target
        .and_then(|target| inputs.current.borrow().existing(target))
        .or_else(|| {
            inputs
                .current
                .borrow()
                .iter()
                .into_iter()
                .next()
                .map(|(_, handle)| handle)
        })
}

fn spawn_data_refresh_listener_thread() {
    std::thread::spawn(platform::data_refresh_listener_loop);
}

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
        let request = inputs.warmer.prepare_refresh_request(request);
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
    let show_id = inputs.show_id.load(Ordering::Acquire);
    let generation = inputs.refresh_generation.fetch_add(1, Ordering::AcqRel) + 1;
    qol_runtime::probe!(
        "REFRESH_TRIGGER",
        "show_id={show_id} lane=data generation={generation} focus_generation={} refresh_frontmost={} refresh_previous_frontmost={} capture_lane={} active_workers={} cancellation_reason=none reveal_frame=show_cache_or_placeholder first_paint_latency_ms=pending",
        request.focus_generation,
        request.refresh_frontmost,
        request.refresh_previous_frontmost,
        request.lane_name(),
        inputs.capture_scheduler.active_workers(),
    );
    let inputs = inputs.clone();
    app_cx
        .spawn(async move |cx: &mut AsyncApp| {
            refresh_data(cx, inputs, generation, request, show_id).await;
        })
        .detach();
}

fn refresh_stale_reason(
    current_generation: usize,
    generation: usize,
    current_show_id: u64,
    show_id: u64,
) -> Option<&'static str> {
    if current_generation != generation {
        return Some("refresh_superseded");
    }
    (current_show_id != show_id).then_some("show_superseded")
}

fn requeue_focus_refresh_after_show(inputs: &ListenerInputs, request: RefreshRequest) -> bool {
    if !request.refresh_previous_frontmost {
        return false;
    }
    let pending = inputs
        .warmer
        .prepare_refresh_request(RefreshRequest::default())
        .refresh_previous_frontmost;
    if !pending {
        return false;
    }
    request_refresh(request);
    true
}

fn reconcile_stale_refresh(
    inputs: &ListenerInputs,
    generation: usize,
    request: RefreshRequest,
    show_id: u64,
    outcome: &'static str,
) -> bool {
    let current_generation = inputs.refresh_generation.load(Ordering::Acquire);
    let current_show_id = inputs.show_id.load(Ordering::Acquire);
    let Some(reason) =
        refresh_stale_reason(current_generation, generation, current_show_id, show_id)
    else {
        return false;
    };
    let requeued = reason == "show_superseded" && requeue_focus_refresh_after_show(inputs, request);
    qol_runtime::probe!(
        "REFRESH_RUN",
        "show_id={show_id} lane=data generation={generation} focus_generation={} outcome={outcome} capture_lane={} active_workers={} cancellation_reason={reason} requeued={requeued} reveal_frame=show_cache_or_placeholder first_paint_latency_ms=none",
        request.focus_generation,
        request.lane_name(),
        inputs.capture_scheduler.active_workers(),
    );
    true
}

async fn refresh_data(
    cx: &mut AsyncApp,
    inputs: ListenerInputs,
    generation: usize,
    request: RefreshRequest,
    show_id: u64,
) {
    let config = crate::config::load_alt_tab_config();
    let show_minimized = config.display.show_minimized;
    let executor = cx.background_executor().clone();
    executor
        .timer(Duration::from_millis(DATA_REFRESH_DELAY_MS))
        .await;
    if reconcile_stale_refresh(&inputs, generation, request, show_id, "superseded") {
        return;
    }
    let windows = executor
        .spawn(async move { Platform.visible_windows(show_minimized).unwrap_or_default() })
        .await;
    if reconcile_stale_refresh(&inputs, generation, request, show_id, "superseded") {
        return;
    }
    if windows.is_empty() {
        inputs
            .warmer
            .mark_refresh_applied(request.focus_generation, false);
        qol_runtime::probe!(
            "REFRESH_RUN",
            "show_id={show_id} lane=data generation={generation} focus_generation={} outcome=empty capture_lane={} active_workers={} cancellation_reason=discovery_empty reveal_frame=show_cache_or_placeholder first_paint_latency_ms=none",
            request.focus_generation,
            request.lane_name(),
            inputs.capture_scheduler.active_workers(),
        );
        return;
    }
    let rendered_icons =
        super::run::refresh_icon_cache(&executor, &windows, &inputs.icon_cache).await;
    let _ = cx.update(move |app_cx| {
        if reconcile_stale_refresh(&inputs, generation, request, show_id, "stale_apply") {
            return;
        }
        qol_runtime::probe!(
            "REFRESH_RUN",
            "show_id={show_id} lane=data generation={generation} focus_generation={} outcome=applied capture_lane={} active_workers={} cancellation_reason=none reveal_frame=show_cache_or_placeholder first_paint_latency_ms=pending",
            request.focus_generation,
            request.lane_name(),
            inputs.capture_scheduler.active_workers(),
        );
        inputs
            .warmer
            .mark_refresh_applied(request.focus_generation, true);
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
            enqueue_requested_capture(
                &inputs,
                request,
                handle,
                &gathered.windows,
                picker_visible,
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

fn enqueue_requested_capture(
    inputs: &ListenerInputs,
    request: RefreshRequest,
    handle: WindowHandle<crate::app::AltTabApp>,
    windows: &[crate::discovery::WindowInfo],
    picker_visible: bool,
    app_cx: &mut App,
) {
    if !RenderingFlow::current().captures_preview_fill() {
        return;
    }
    if capture_lane_requested(request, CaptureLane::HiddenWarmer, picker_visible) {
        enqueue_capture_lane(inputs, CaptureLane::HiddenWarmer, handle, windows, app_cx);
    }
    if capture_lane_requested(request, CaptureLane::FocusLeave, picker_visible) {
        enqueue_capture_lane(inputs, CaptureLane::FocusLeave, handle, windows, app_cx);
    }
}

fn capture_lane_requested(
    request: RefreshRequest,
    lane: CaptureLane,
    picker_visible: bool,
) -> bool {
    match lane {
        CaptureLane::HiddenWarmer => request.refresh_frontmost && !picker_visible,
        CaptureLane::FocusLeave => request.refresh_previous_frontmost,
    }
}

fn enqueue_capture_lane(
    inputs: &ListenerInputs,
    lane: CaptureLane,
    handle: WindowHandle<crate::app::AltTabApp>,
    windows: &[crate::discovery::WindowInfo],
    app_cx: &mut App,
) {
    let Some(window) = capture_target_for_lane(lane, windows) else {
        return;
    };
    inputs.capture_scheduler.enqueue(
        lane,
        PreviewCaptureRequest {
            handle,
            window,
            preview_cache: inputs.preview_cache.clone(),
            show_id: inputs.show_id.load(Ordering::Acquire),
        },
        app_cx,
    );
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
mod tests {
    use super::{
        capture_lane_requested, refresh_stale_reason, RefreshRequest, WarmerDecision, WarmerState,
    };
    use crate::discovery::WindowInfo;
    use crate::picker::gather::{capture_target_for_lane, CaptureLane};
    use std::sync::{mpsc, Arc, Mutex};
    use std::time::{Duration, Instant};

    fn window(id: u32) -> WindowInfo {
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
    fn flow_fill_gate_suppresses_hid_activity_arming() {
        super::store_flow_fill(false);
        assert!(!super::flow_fills_previews());
        assert_eq!(super::record_hid_activity(false), 0);
        assert_eq!(super::record_hid_activity(true), 0);

        super::store_flow_fill(true);
        assert!(super::flow_fills_previews());
    }

    #[test]
    fn ordinary_refresh_has_no_capture_lane() {
        let request = RefreshRequest::default();
        assert_eq!(request.lane_name(), "data");
        assert!(!request.refresh_frontmost);
        assert!(!request.refresh_previous_frontmost);
    }

    #[test]
    fn merged_refresh_keeps_warmer_and_focus_leave_lanes() {
        let request = RefreshRequest::default().merge(RefreshRequest::previous_frontmost());
        assert_eq!(request.lane_name(), "focus_leave");
        assert!(request.refresh_previous_frontmost);
        assert!(!request.refresh_frontmost);
    }

    #[test]
    fn router_reconciles_pre_stamped_ordinary_after_focus_supersession() {
        let warmer = super::WarmerControl::new();
        let pre_stamped_ordinary = warmer.prepare_refresh_request(RefreshRequest::default());
        assert_eq!(pre_stamped_ordinary.lane_name(), "data");

        let focus_generation = warmer.record_activity(true);
        let focus_request = warmer.prepare_refresh_request(RefreshRequest::previous_frontmost());
        assert_eq!(focus_request.focus_generation, focus_generation);
        assert!(focus_request.refresh_previous_frontmost);

        let (tx, rx) = mpsc::channel();
        let rx = Arc::new(Mutex::new(rx));
        tx.send(focus_request).unwrap();
        let first_request = rx.lock().unwrap().recv().unwrap();
        let first = super::drain(&rx, first_request);
        assert_eq!(first.lane_name(), "focus_leave");

        tx.send(pre_stamped_ordinary).unwrap();
        let stale_request = rx.lock().unwrap().recv().unwrap();
        let stale_successor = super::drain(&rx, stale_request);
        let successor = warmer.prepare_refresh_request(stale_successor);
        assert_eq!(successor.focus_generation, focus_generation);
        assert_eq!(successor.lane_name(), "focus_leave");

        let windows = vec![window(10), window(20), window(30)];
        let mut focus_targets = Vec::new();
        if successor.refresh_previous_frontmost {
            focus_targets.push(
                capture_target_for_lane(CaptureLane::FocusLeave, &windows).map(|window| window.id),
            );
        }
        assert_eq!(focus_targets, vec![Some(20)]);

        warmer.mark_refresh_applied(focus_generation, true);
        let later = warmer.prepare_refresh_request(RefreshRequest::default());
        assert_eq!(later.lane_name(), "data");
        assert!(!later.refresh_previous_frontmost);
    }

    #[test]
    fn newer_show_keeps_older_refresh_from_applying_and_preserves_focus_lane() {
        let start = Instant::now();
        let mut warmer = WarmerState::default();
        let old_order = vec![window(10), window(20), window(30)];
        let show_order = vec![window(20), window(10), window(30)];
        let focus_generation = warmer.record_activity_at(start, true);
        let stale_request = warmer.prepare_refresh_request(RefreshRequest::previous_frontmost());

        assert_eq!(stale_request.lane_name(), "focus_leave");
        assert!(refresh_stale_reason(1, 1, 2, 1).is_some());

        let mut applied_order = show_order.clone();
        if refresh_stale_reason(1, 1, 2, 1).is_none() {
            applied_order = old_order;
        }
        assert_eq!(
            applied_order
                .iter()
                .map(|window| window.id)
                .collect::<Vec<_>>(),
            vec![20, 10, 30]
        );

        let successor = warmer.prepare_refresh_request(RefreshRequest::default());
        assert_eq!(successor.lane_name(), "focus_leave");
        assert!(capture_lane_requested(
            successor,
            CaptureLane::FocusLeave,
            true
        ));
        assert!(!capture_lane_requested(
            successor,
            CaptureLane::HiddenWarmer,
            true
        ));
        assert_eq!(
            capture_target_for_lane(CaptureLane::FocusLeave, &applied_order)
                .map(|window| window.id),
            Some(10)
        );

        warmer.mark_refresh_applied(focus_generation, true);
        assert_eq!(
            warmer.take_due(start + Duration::from_millis(250)),
            WarmerDecision::Capture(focus_generation)
        );
    }

    #[test]
    fn show_superseded_empty_refresh_keeps_focus_generation_pending() {
        let start = Instant::now();
        let mut warmer = WarmerState::default();
        let focus_generation = warmer.record_activity_at(start, true);
        let request = warmer.prepare_refresh_request(RefreshRequest::previous_frontmost());

        assert_eq!(request.focus_generation, focus_generation);
        assert_eq!(refresh_stale_reason(1, 1, 2, 1), Some("show_superseded"));
        if refresh_stale_reason(1, 1, 2, 1).is_none() {
            warmer.mark_refresh_applied(request.focus_generation, false);
        }
        assert_eq!(
            warmer.take_due(start + Duration::from_millis(250)),
            WarmerDecision::Waiting
        );

        warmer.mark_refresh_applied(focus_generation, true);
        assert_eq!(
            warmer.take_due(start + Duration::from_millis(250)),
            WarmerDecision::Capture(focus_generation)
        );
    }

    #[test]
    fn warmer_waits_for_focus_refresh_before_using_new_frontmost_order() {
        let start = Instant::now();
        let mut warmer = WarmerState::default();
        let old_order = vec![window(10), window(20)];
        let new_order = vec![window(20), window(10)];
        warmer.record_activity_at(start, false);
        let focus_generation = warmer.record_activity_at(start + Duration::from_millis(1), true);

        assert_eq!(
            warmer.take_due(start + Duration::from_millis(251)),
            WarmerDecision::Waiting
        );
        assert_eq!(
            capture_target_for_lane(CaptureLane::HiddenWarmer, &old_order).map(|window| window.id),
            Some(10)
        );

        warmer.mark_refresh_applied(focus_generation, true);
        assert_eq!(
            warmer.take_due(start + Duration::from_millis(251)),
            WarmerDecision::Capture(focus_generation)
        );
        assert_eq!(
            capture_target_for_lane(CaptureLane::HiddenWarmer, &new_order).map(|window| window.id),
            Some(20)
        );
        assert_eq!(
            warmer.take_due(start + Duration::from_millis(500)),
            WarmerDecision::Idle
        );
    }

    #[test]
    fn failed_focus_refresh_does_not_skip_newer_activity() {
        let start = Instant::now();
        for failure_before_activity in [true, false] {
            let mut warmer = WarmerState::default();
            let focus_generation = warmer.record_activity_at(start, true);
            if failure_before_activity {
                warmer.mark_refresh_applied(focus_generation, false);
            }
            let ordinary_generation =
                warmer.record_activity_at(start + Duration::from_millis(100), false);
            if !failure_before_activity {
                warmer.mark_refresh_applied(focus_generation, false);
            }

            assert_eq!(
                warmer.take_due(start + Duration::from_millis(350)),
                WarmerDecision::Capture(ordinary_generation)
            );
        }
    }

    #[test]
    fn failed_focus_refresh_only_skips_one_warmer_activity() {
        let start = Instant::now();
        let mut warmer = WarmerState::default();
        let focus_generation = warmer.record_activity_at(start, true);
        warmer.mark_refresh_applied(focus_generation, false);

        assert_eq!(
            warmer.take_due(start + Duration::from_millis(250)),
            WarmerDecision::Skip(focus_generation)
        );

        let ordinary_generation =
            warmer.record_activity_at(start + Duration::from_millis(251), false);
        assert_eq!(
            warmer.take_due(start + Duration::from_millis(501)),
            WarmerDecision::Capture(ordinary_generation)
        );
    }
}
