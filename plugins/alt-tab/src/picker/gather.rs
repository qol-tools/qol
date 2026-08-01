use super::run::{SharedPreviewCache, WindowCache};
use crate::app::AltTabApp;
use crate::capture;
use crate::config::AltTabConfig;
use crate::discovery::WindowInfo;
use crate::picker::layout::{PREVIEW_MAX_HEIGHT, PREVIEW_MAX_WIDTH};
use crate::picker::{IconMap, PreviewMap, SharedIconCache};
use futures::channel::oneshot;
use gpui::*;
use std::collections::HashMap;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::mpsc::{sync_channel, Receiver, SyncSender, TrySendError};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum CaptureLane {
    HiddenWarmer,
    FocusLeave,
}

impl CaptureLane {
    fn name(self) -> &'static str {
        match self {
            Self::HiddenWarmer => "hidden_warmer",
            Self::FocusLeave => "focus_leave",
        }
    }
}

const CAPTURE_WORKER_COUNT: usize = 2;

enum BlockingCaptureResult {
    Live(capture::SendCVBuf),
    Preview(Option<qol_app_icon::RgbaImage>),
}

struct BlockingCaptureJob {
    wid: u32,
    dims: (usize, usize),
    result: oneshot::Sender<BlockingCaptureResult>,
}

type CaptureBackend = dyn Fn(u32, (usize, usize)) -> BlockingCaptureResult + Send + Sync;

struct CaptureWorkerPool {
    jobs: SyncSender<BlockingCaptureJob>,
    active_workers: Arc<AtomicUsize>,
    capacity: usize,
}

impl CaptureWorkerPool {
    fn new() -> Arc<Self> {
        Self::with_backend(CAPTURE_WORKER_COUNT, Arc::new(capture_blocking))
    }

    fn with_backend(worker_count: usize, backend: Arc<CaptureBackend>) -> Arc<Self> {
        let capacity = worker_count.max(1);
        let (jobs, queue) = sync_channel(capacity);
        let active_workers = Arc::new(AtomicUsize::new(0));
        let queue = Arc::new(Mutex::new(queue));
        let pool = Arc::new(Self {
            jobs,
            active_workers: active_workers.clone(),
            capacity,
        });
        for index in 0..capacity {
            spawn_capture_worker(
                index,
                queue.clone(),
                backend.clone(),
                active_workers.clone(),
            );
        }
        pool
    }

    fn submit(
        &self,
        wid: u32,
        dims: (usize, usize),
    ) -> Result<oneshot::Receiver<BlockingCaptureResult>, &'static str> {
        let admitted = self
            .active_workers
            .fetch_update(Ordering::AcqRel, Ordering::Acquire, |active| {
                (active < self.capacity).then_some(active + 1)
            })
            .is_ok();
        if !admitted {
            return Err("capture_workers_full");
        }
        let (result, receiver) = oneshot::channel();
        let job = BlockingCaptureJob { wid, dims, result };
        if let Err(error) = self.jobs.try_send(job) {
            self.active_workers.fetch_sub(1, Ordering::AcqRel);
            return Err(match error {
                TrySendError::Full(_) => "capture_workers_full",
                TrySendError::Disconnected(_) => "capture_workers_closed",
            });
        }
        Ok(receiver)
    }

    #[cfg(test)]
    fn active_workers(&self) -> usize {
        self.active_workers.load(Ordering::Acquire)
    }
}

fn spawn_capture_worker(
    index: usize,
    queue: Arc<Mutex<Receiver<BlockingCaptureJob>>>,
    backend: Arc<CaptureBackend>,
    active_workers: Arc<AtomicUsize>,
) {
    let _ = thread::Builder::new()
        .name(format!("qol-alt-tab-capture-{index}"))
        .spawn(move || loop {
            let job = queue.lock().ok().and_then(|queue| queue.recv().ok());
            let Some(job) = job else {
                return;
            };
            let result = backend(job.wid, job.dims);
            active_workers.fetch_sub(1, Ordering::AcqRel);
            let _ = job.result.send(result);
        });
}

fn capture_blocking(wid: u32, dims: (usize, usize)) -> BlockingCaptureResult {
    if capture::live_shots_available() {
        if let Some(buffer) = capture_live_frame_blocking(wid, dims) {
            return BlockingCaptureResult::Live(buffer);
        }
    }
    let captured = capture::capture_previews_cg(&[(0, wid)], PREVIEW_MAX_WIDTH, PREVIEW_MAX_HEIGHT);
    BlockingCaptureResult::Preview(captured.into_iter().next().and_then(|(_, image)| image))
}

#[derive(Clone)]
pub(super) struct PreviewCaptureScheduler {
    warmer: Arc<Mutex<LaneState<PreviewCaptureRequest>>>,
    focus_leave: Arc<Mutex<LaneState<PreviewCaptureRequest>>>,
    active_workers: Arc<AtomicUsize>,
    capture_workers: Arc<CaptureWorkerPool>,
}

struct LaneState<T> {
    next_generation: usize,
    active_generation: Option<usize>,
    pending: Option<Queued<T>>,
}

impl<T> Default for LaneState<T> {
    fn default() -> Self {
        Self {
            next_generation: 0,
            active_generation: None,
            pending: None,
        }
    }
}

struct Queued<T> {
    generation: usize,
    value: T,
}

enum Admission<T> {
    Start(Queued<T>),
    Pending {
        generation: usize,
        replaced: Option<Queued<T>>,
    },
}

#[derive(Clone)]
pub(super) struct PreviewCaptureRequest {
    pub handle: WindowHandle<AltTabApp>,
    pub window: WindowInfo,
    pub preview_cache: SharedPreviewCache,
    pub show_id: u64,
}

enum CaptureOutcome {
    Committed,
    Empty(&'static str),
    Cancelled(&'static str),
    Skipped(&'static str),
}

impl CaptureOutcome {
    fn name(&self) -> &'static str {
        match self {
            Self::Committed => "committed",
            Self::Empty(_) => "empty",
            Self::Cancelled(_) => "cancelled",
            Self::Skipped(_) => "skipped",
        }
    }

    fn reason(&self) -> &'static str {
        match self {
            Self::Committed => "none",
            Self::Empty(reason) | Self::Cancelled(reason) | Self::Skipped(reason) => reason,
        }
    }
}

struct CaptureResult {
    outcome: CaptureOutcome,
    capture_duration: Duration,
}

#[derive(Clone, Copy)]
struct CaptureIdentity {
    show_id: u64,
    wid: u32,
}

impl CaptureResult {
    fn no_capture(outcome: CaptureOutcome) -> Self {
        Self {
            outcome,
            capture_duration: Duration::ZERO,
        }
    }

    fn with_capture(outcome: CaptureOutcome, capture_duration: Duration) -> Self {
        Self {
            outcome,
            capture_duration,
        }
    }
}

struct CaptureTrace {
    lane: CaptureLane,
    phase: &'static str,
    generation: usize,
    active_workers: usize,
    show_id: u64,
    wid: u32,
    reason: &'static str,
    capture_duration: Duration,
}

struct CaptureFinishTrace<'a> {
    lane: CaptureLane,
    generation: usize,
    active_workers: usize,
    show_id: u64,
    wid: u32,
    outcome: &'a CaptureOutcome,
    worker_duration: Duration,
    capture_duration: Duration,
    pending: bool,
}

impl<T> LaneState<T> {
    fn admit(&mut self, value: T) -> Admission<T> {
        self.next_generation = self.next_generation.wrapping_add(1);
        let generation = self.next_generation;
        let queued = Queued { generation, value };
        if self.active_generation.is_some() {
            let replaced = self.pending.replace(queued);
            return Admission::Pending {
                generation,
                replaced,
            };
        }
        self.active_generation = Some(generation);
        Admission::Start(queued)
    }

    fn is_current(&self, generation: usize) -> bool {
        self.active_generation == Some(generation) && self.pending.is_none()
    }

    fn finish(&mut self, generation: usize) -> Option<Queued<T>> {
        if self.active_generation != Some(generation) {
            return None;
        }
        let next = self.pending.take();
        self.active_generation = next.as_ref().map(|queued| queued.generation);
        next
    }
}

impl PreviewCaptureScheduler {
    pub(super) fn new() -> Self {
        Self {
            warmer: Arc::new(Mutex::new(LaneState::default())),
            focus_leave: Arc::new(Mutex::new(LaneState::default())),
            active_workers: Arc::new(AtomicUsize::new(0)),
            capture_workers: CaptureWorkerPool::new(),
        }
    }

    pub(super) fn active_workers(&self) -> usize {
        self.active_workers.load(Ordering::Acquire)
    }

    pub(super) fn enqueue(&self, lane: CaptureLane, request: PreviewCaptureRequest, cx: &mut App) {
        let show_id = request.show_id;
        let wid = request.window.id;
        let state = self.state(lane);
        let Ok(mut state) = state.lock() else {
            return;
        };
        let admission = state.admit(request);
        match admission {
            Admission::Start(job) => {
                drop(state);
                self.start(lane, job, cx);
            }
            Admission::Pending {
                generation,
                replaced,
            } => {
                let active_workers = self.active_workers();
                drop(state);
                if let Some(replaced) = replaced {
                    trace_capture(CaptureTrace {
                        lane,
                        phase: "cancel",
                        generation: replaced.generation,
                        active_workers,
                        show_id: replaced.value.show_id,
                        wid: replaced.value.window.id,
                        reason: "pending_replaced",
                        capture_duration: Duration::ZERO,
                    });
                }
                trace_capture(CaptureTrace {
                    lane,
                    phase: "enqueue_pending",
                    generation,
                    active_workers,
                    show_id,
                    wid,
                    reason: "active_worker",
                    capture_duration: Duration::ZERO,
                });
            }
        }
    }

    fn state(&self, lane: CaptureLane) -> &Arc<Mutex<LaneState<PreviewCaptureRequest>>> {
        match lane {
            CaptureLane::HiddenWarmer => &self.warmer,
            CaptureLane::FocusLeave => &self.focus_leave,
        }
    }

    fn is_current(&self, lane: CaptureLane, generation: usize) -> bool {
        self.state(lane)
            .lock()
            .map(|state| state.is_current(generation))
            .unwrap_or(false)
    }

    fn start(&self, lane: CaptureLane, job: Queued<PreviewCaptureRequest>, cx: &mut App) {
        let active_workers = self.active_workers.fetch_add(1, Ordering::AcqRel) + 1;
        trace_capture(CaptureTrace {
            lane,
            phase: "start",
            generation: job.generation,
            active_workers,
            show_id: job.value.show_id,
            wid: job.value.window.id,
            reason: "none",
            capture_duration: Duration::ZERO,
        });
        let scheduler = self.clone();
        let generation = job.generation;
        let identity = CaptureIdentity {
            show_id: job.value.show_id,
            wid: job.value.window.id,
        };
        cx.spawn(async move |cx: &mut AsyncApp| {
            let started = Instant::now();
            let result = run_capture(cx, lane, job, &scheduler).await;
            scheduler.finish(lane, generation, identity, result, started, cx);
        })
        .detach();
    }

    fn finish(
        &self,
        lane: CaptureLane,
        generation: usize,
        identity: CaptureIdentity,
        result: CaptureResult,
        started: Instant,
        cx: &mut AsyncApp,
    ) {
        let next = self
            .state(lane)
            .lock()
            .ok()
            .and_then(|mut state| state.finish(generation));
        let active_workers = self.active_workers.fetch_sub(1, Ordering::AcqRel) - 1;
        let elapsed = started.elapsed();
        let pending = next.is_some();
        trace_capture_finish(CaptureFinishTrace {
            lane,
            generation,
            active_workers,
            show_id: identity.show_id,
            wid: identity.wid,
            outcome: &result.outcome,
            worker_duration: elapsed,
            capture_duration: result.capture_duration,
            pending,
        });
        let Some(next) = next else {
            return;
        };
        let scheduler = self.clone();
        let _ = cx.update(|app_cx| scheduler.start(lane, next, app_cx));
    }
}

fn trace_capture(trace: CaptureTrace) {
    qol_runtime::probe!(
        "PREVIEW_CAPTURE",
        "lane={} phase={} generation={} active_workers={} show_id={} wid={} reason={} capture_duration_ms={} reveal_frame=show_cache_or_placeholder first_paint_latency_ms=not_measured",
        trace.lane.name(),
        trace.phase,
        trace.generation,
        trace.active_workers,
        trace.show_id,
        trace.wid,
        trace.reason,
        trace.capture_duration.as_millis(),
    );
}

fn trace_capture_finish(trace: CaptureFinishTrace) {
    qol_runtime::probe!(
        "PREVIEW_CAPTURE",
        "lane={} phase=finish generation={} active_workers={} show_id={} wid={} reason={} worker_duration_ms={} capture_duration_ms={} reveal_frame=show_cache_or_placeholder outcome={} pending={} first_paint_latency_ms=not_measured",
        trace.lane.name(),
        trace.generation,
        trace.active_workers,
        trace.show_id,
        trace.wid,
        trace.outcome.reason(),
        trace.worker_duration.as_millis(),
        trace.capture_duration.as_millis(),
        trace.outcome.name(),
        trace.pending,
    );
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

    #[cfg(debug_assertions)]
    probe_preview_gather(&windows, &previews);

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
    window_cache
        .lock()
        .ok()
        .map(|c| c.clone())
        .unwrap_or_default()
}

#[cfg(debug_assertions)]
fn probe_preview_gather(windows: &[WindowInfo], previews: &PreviewMap) {
    let cached = windows
        .iter()
        .filter(|w| previews.contains_key(&w.id))
        .count();
    let entries: Vec<String> = windows
        .iter()
        .take(24)
        .map(|w| {
            let has_preview = previews.contains_key(&w.id);
            preview_entry(w.id, has_preview)
        })
        .collect();
    let missed = windows.len().saturating_sub(cached);
    qol_runtime::probe!(
        "PREVIEW_GATHER",
        "windows={} cached={} missed={} entries=[{}]",
        windows.len(),
        cached,
        missed,
        entries.join(" "),
    );
}

#[cfg(debug_assertions)]
fn preview_entry(wid: u32, has_shared_preview: bool) -> String {
    let live = crate::rendering::preview_trace::live_snapshot(wid)
        .map(|stamp| format!("/{source}:{}ms", stamp.age_ms, source = stamp.source))
        .unwrap_or_default();
    if !has_shared_preview {
        return format!("{wid}:miss{live}");
    }
    let shared = crate::rendering::preview_trace::shared_snapshot(wid)
        .map(|stamp| format!("{}:{}ms", stamp.source, stamp.age_ms))
        .unwrap_or_else(|| "unknown".to_string());
    format!("{wid}:cache:{shared}{live}")
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
            crate::rendering::image_registry::extend_with(&mut *icache, rendered.clone(), cx, None);
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

pub(super) fn capture_target_for_lane(
    lane: CaptureLane,
    windows: &[WindowInfo],
) -> Option<WindowInfo> {
    let index = match lane {
        CaptureLane::HiddenWarmer => 0,
        CaptureLane::FocusLeave => 1,
    };
    windows
        .get(index)
        .cloned()
        .filter(|window| !window.is_minimized)
}

fn cancellation_reason(
    scheduler: &PreviewCaptureScheduler,
    lane: CaptureLane,
    generation: usize,
) -> Option<&'static str> {
    if !scheduler.is_current(lane, generation) {
        return Some("superseded");
    }
    if picker_visibility_cancels(lane, crate::app::PICKER_VISIBLE.load(Ordering::Acquire)) {
        return Some("picker_visible");
    }
    None
}

fn picker_visibility_cancels(lane: CaptureLane, picker_visible: bool) -> bool {
    lane == CaptureLane::HiddenWarmer && picker_visible
}

async fn run_capture(
    cx: &mut AsyncApp,
    lane: CaptureLane,
    job: Queued<PreviewCaptureRequest>,
    scheduler: &PreviewCaptureScheduler,
) -> CaptureResult {
    use crate::rendering::preview_image::{bgra_to_render_image, shot_request_dims};

    let generation = job.generation;
    let request = job.value;
    let show_id = request.show_id;
    let wid = request.window.id;
    if let Some(reason) = cancellation_reason(scheduler, lane, generation) {
        trace_capture(CaptureTrace {
            lane,
            phase: "cancel",
            generation,
            active_workers: scheduler.active_workers(),
            show_id,
            wid,
            reason,
            capture_duration: Duration::ZERO,
        });
        return CaptureResult::no_capture(CaptureOutcome::Cancelled(reason));
    }
    if request.window.is_minimized {
        return CaptureResult::no_capture(CaptureOutcome::Empty("minimized"));
    }
    let rendering = crate::rendering::RenderingFlow::current();
    if !rendering.captures_preview_fill() {
        return CaptureResult::no_capture(CaptureOutcome::Skipped("preview_plane"));
    }
    let dims = shot_request_dims(request.window.width, request.window.height);
    let receiver = match scheduler.capture_workers.submit(wid, dims) {
        Ok(receiver) => receiver,
        Err(reason) => return CaptureResult::no_capture(CaptureOutcome::Empty(reason)),
    };
    let capture_started = Instant::now();
    let captured = match receiver.await {
        Ok(result) => result,
        Err(_) => {
            return CaptureResult::no_capture(CaptureOutcome::Empty("capture_workers_closed"));
        }
    };
    let capture_duration = capture_started.elapsed();
    if let Some(reason) = cancellation_reason(scheduler, lane, generation) {
        trace_capture(CaptureTrace {
            lane,
            phase: "cancel",
            generation,
            active_workers: scheduler.active_workers(),
            show_id,
            wid,
            reason,
            capture_duration,
        });
        return CaptureResult::with_capture(CaptureOutcome::Cancelled(reason), capture_duration);
    }
    match captured {
        BlockingCaptureResult::Live(buffer) => {
            let outcome = commit_live_frame(
                cx,
                lane,
                generation,
                scheduler,
                request,
                buffer,
                capture_duration,
            );
            CaptureResult::with_capture(outcome, capture_duration)
        }
        BlockingCaptureResult::Preview(Some(rgba)) => {
            let Some(image) = bgra_to_render_image(rgba.data, rgba.width, rgba.height) else {
                return CaptureResult::with_capture(
                    CaptureOutcome::Empty("decode_failed"),
                    capture_duration,
                );
            };
            let outcome = commit_preview(
                cx,
                lane,
                generation,
                scheduler,
                request,
                [(wid, image)].into_iter().collect(),
                capture_duration,
            );
            CaptureResult::with_capture(outcome, capture_duration)
        }
        BlockingCaptureResult::Preview(None) => {
            CaptureResult::with_capture(CaptureOutcome::Empty("capture_failed"), capture_duration)
        }
    }
}

fn capture_live_frame_blocking(
    wid: u32,
    (width, height): (usize, usize),
) -> Option<capture::SendCVBuf> {
    let session = capture::warm_shots_session(&[wid])?;
    let (tx, rx) = std::sync::mpsc::channel();
    if !session.request_capture(wid, width, height, &tx) {
        return None;
    }
    match rx.recv_timeout(Duration::from_millis(120)) {
        Ok((reply_wid, Some(buf)))
            if reply_wid == wid && buf.pixel_format() == capture::PIXEL_FORMAT_420F =>
        {
            Some(buf)
        }
        _ => None,
    }
}

fn trace_preview_paint(
    window: &mut Window,
    lane: CaptureLane,
    generation: usize,
    active_workers: usize,
    show_id: u64,
    wid: u32,
    capture_duration: Duration,
) {
    let committed_at = Instant::now();
    window.on_next_frame(move |_, _| {
        qol_runtime::probe!(
            "PREVIEW_CAPTURE",
            "lane={} phase=paint generation={} active_workers={} show_id={} wid={} reason=none capture_duration_ms={} reveal_frame=painted first_paint_latency_ms={}",
            lane.name(),
            generation,
            active_workers,
            show_id,
            wid,
            capture_duration.as_millis(),
            committed_at.elapsed().as_millis(),
        );
    });
}

fn commit_live_frame(
    cx: &mut AsyncApp,
    lane: CaptureLane,
    generation: usize,
    scheduler: &PreviewCaptureScheduler,
    request: PreviewCaptureRequest,
    buffer: capture::SendCVBuf,
    capture_duration: Duration,
) -> CaptureOutcome {
    let wid = request.window.id;
    let show_id = request.show_id;
    let handle = request.handle;
    let committed = cx
        .update(|cx| {
            if cancellation_reason(scheduler, lane, generation).is_some() {
                return false;
            }
            handle
                .update(cx, |view, window, cx| -> bool {
                    let Some(frame) = buffer.into_live_frame() else {
                        return false;
                    };
                    view.delegate.update(cx, |state, _| {
                        state.insert_live_frames([(wid, frame)].into_iter().collect());
                    });
                    trace_preview_paint(
                        window,
                        lane,
                        generation,
                        scheduler.active_workers(),
                        show_id,
                        wid,
                        capture_duration,
                    );
                    cx.notify();
                    true
                })
                .unwrap_or(false)
        })
        .unwrap_or(false);
    if committed {
        return CaptureOutcome::Committed;
    }
    let reason = cancellation_reason(scheduler, lane, generation).unwrap_or("handle_unavailable");
    if reason == "handle_unavailable" {
        return CaptureOutcome::Empty(reason);
    }
    trace_capture(CaptureTrace {
        lane,
        phase: "cancel",
        generation,
        active_workers: scheduler.active_workers(),
        show_id,
        wid,
        reason,
        capture_duration,
    });
    CaptureOutcome::Cancelled(reason)
}

fn commit_preview(
    cx: &mut AsyncApp,
    lane: CaptureLane,
    generation: usize,
    scheduler: &PreviewCaptureScheduler,
    request: PreviewCaptureRequest,
    previews: PreviewMap,
    capture_duration: Duration,
) -> CaptureOutcome {
    let wid = request.window.id;
    let show_id = request.show_id;
    let handle = request.handle;
    let cache = request.preview_cache;
    let shared_previews = previews.clone();
    let committed = cx
        .update(|cx| {
            if cancellation_reason(scheduler, lane, generation).is_some() {
                return false;
            }
            if let Ok(mut cache) = cache.lock() {
                crate::rendering::image_registry::extend_with(
                    &mut *cache,
                    shared_previews,
                    cx,
                    None,
                );
                #[cfg(debug_assertions)]
                crate::rendering::preview_trace::record_shared_fill([wid]);
            }
            handle
                .update(cx, |view, window, cx| {
                    view.update_previews(previews, window, cx);
                    trace_preview_paint(
                        window,
                        lane,
                        generation,
                        scheduler.active_workers(),
                        show_id,
                        wid,
                        capture_duration,
                    );
                })
                .is_ok()
        })
        .unwrap_or(false);
    if committed {
        return CaptureOutcome::Committed;
    }
    let reason = cancellation_reason(scheduler, lane, generation).unwrap_or("handle_unavailable");
    if reason == "handle_unavailable" {
        return CaptureOutcome::Empty(reason);
    }
    trace_capture(CaptureTrace {
        lane,
        phase: "cancel",
        generation,
        active_workers: scheduler.active_workers(),
        show_id,
        wid,
        reason,
        capture_duration,
    });
    CaptureOutcome::Cancelled(reason)
}

#[cfg(test)]
mod capture_lane_tests {
    use super::{
        capture_target_for_lane, picker_visibility_cancels, Admission, CaptureLane, LaneState,
    };
    use crate::discovery::WindowInfo;

    fn w(id: u32, minimized: bool) -> WindowInfo {
        WindowInfo {
            id,
            title: String::new(),
            app_name: String::new(),
            preview_path: None,
            icon: None,
            width: 0.0,
            height: 0.0,
            is_minimized: minimized,
        }
    }

    #[test]
    fn hidden_warmer_only_targets_frontmost_window() {
        let windows = vec![w(10, false), w(20, false)];
        assert_eq!(
            capture_target_for_lane(CaptureLane::HiddenWarmer, &windows).map(|window| window.id),
            Some(10)
        );
    }

    #[test]
    fn focus_leave_only_targets_just_defocused_window() {
        let windows = vec![w(10, false), w(20, false), w(30, false)];
        assert_eq!(
            capture_target_for_lane(CaptureLane::FocusLeave, &windows).map(|window| window.id),
            Some(20)
        );
    }

    #[test]
    fn minimized_or_missing_lane_target_is_skipped() {
        assert!(capture_target_for_lane(CaptureLane::HiddenWarmer, &[]).is_none());
        assert!(
            capture_target_for_lane(CaptureLane::FocusLeave, &[w(10, false), w(20, true)])
                .is_none()
        );
        assert!(capture_target_for_lane(CaptureLane::HiddenWarmer, &[w(10, true)]).is_none());
    }

    #[test]
    fn picker_visibility_only_cancels_hidden_warmer() {
        assert!(picker_visibility_cancels(CaptureLane::HiddenWarmer, true));
        assert!(!picker_visibility_cancels(CaptureLane::FocusLeave, true));
        assert!(!picker_visibility_cancels(CaptureLane::FocusLeave, false));
    }

    #[test]
    fn lane_queue_hands_off_one_latest_pending_request() {
        let mut lane = LaneState::<u32>::default();
        let first = match lane.admit(10) {
            Admission::Start(job) => job,
            Admission::Pending { .. } => panic!("first request must start"),
        };
        assert!(lane.is_current(first.generation));
        let Admission::Pending {
            generation: pending_generation,
            replaced,
        } = lane.admit(20)
        else {
            panic!("second request must remain pending");
        };
        assert!(replaced.is_none());
        assert!(!lane.is_current(first.generation));
        let Admission::Pending { replaced, .. } = lane.admit(30) else {
            panic!("third request must replace the pending request");
        };
        assert_eq!(replaced.map(|job| job.value), Some(20));
        let next = lane
            .finish(first.generation)
            .expect("latest request handoff");
        assert_eq!(next.generation, pending_generation + 1);
        assert_eq!(next.value, 30);
        assert!(lane.is_current(next.generation));
        assert!(lane.finish(next.generation).is_none());
    }
}

#[cfg(test)]
mod capture_lane_independence_tests {
    use super::{Admission, LaneState};

    #[test]
    fn warmer_and_focus_leave_states_can_progress_independently() {
        let mut warmer = LaneState::<u32>::default();
        let mut focus_leave = LaneState::<u32>::default();
        let warmer_job = match warmer.admit(1) {
            Admission::Start(job) => job,
            Admission::Pending { .. } => panic!("warmer must start"),
        };
        let focus_job = match focus_leave.admit(2) {
            Admission::Start(job) => job,
            Admission::Pending { .. } => panic!("focus leave must start"),
        };
        assert!(warmer.is_current(warmer_job.generation));
        assert!(focus_leave.is_current(focus_job.generation));
    }
}

#[cfg(test)]
mod capture_worker_tests {
    use super::{BlockingCaptureResult, CaptureWorkerPool};
    use futures::executor::block_on;
    use futures::future::join;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::{Arc, Barrier};
    use std::thread;

    #[test]
    fn dedicated_capture_workers_bound_backend_and_keep_waiters_async() {
        let active = Arc::new(AtomicUsize::new(0));
        let peak = Arc::new(AtomicUsize::new(0));
        let started = Arc::new(Barrier::new(3));
        let release = Arc::new(Barrier::new(3));
        let backend = {
            let active = active.clone();
            let peak = peak.clone();
            let started = started.clone();
            let release = release.clone();
            Arc::new(move |_, _| {
                let current = active.fetch_add(1, Ordering::AcqRel) + 1;
                peak.fetch_max(current, Ordering::AcqRel);
                started.wait();
                release.wait();
                active.fetch_sub(1, Ordering::AcqRel);
                BlockingCaptureResult::Preview(None)
            })
        };
        let pool = CaptureWorkerPool::with_backend(2, backend);
        let first = pool.submit(10, (1, 1)).expect("first capture admission");
        let second = pool.submit(20, (1, 1)).expect("second capture admission");
        started.wait();
        assert_eq!(pool.active_workers(), 2);
        assert_eq!(pool.submit(30, (1, 1)).err(), Some("capture_workers_full"));

        let release_thread = {
            let release = release.clone();
            thread::spawn(move || release.wait())
        };
        let heartbeat = Arc::new(AtomicUsize::new(0));
        let heartbeat_for_future = heartbeat.clone();
        let ((first, second), ()) = block_on(join(join(first, second), async move {
            heartbeat_for_future.fetch_add(1, Ordering::AcqRel);
        }));
        release_thread.join().expect("capture release thread");
        assert!(first.is_ok());
        assert!(second.is_ok());
        assert_eq!(heartbeat.load(Ordering::Acquire), 1);
        assert_eq!(peak.load(Ordering::Acquire), 2);
        assert_eq!(pool.active_workers(), 0);
    }
}
