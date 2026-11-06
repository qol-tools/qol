mod lanes;

use self::lanes::LaneScheduler;
use crate::app::{AltTabApp, PICKER_VISIBLE};
use crate::capture;
use crate::capture::{SendCVBuf, ShotReply};
use crate::picker::layout::{PREVIEW_MAX_HEIGHT, PREVIEW_MAX_WIDTH};
use crate::picker::run::SharedPreviewCache;
use crate::picker::state::{current_open, PickerState};
use crate::picker::PreviewMap;
use crate::rendering::preview_image::{fast_pixel_hash, shot_request_dims};
use gpui::{AsyncApp, Entity, RenderImage, Task, WeakEntity};
use std::collections::HashMap;
use std::sync::atomic::Ordering;
use std::sync::mpsc;
use std::sync::Arc;
use std::time::{Duration, Instant};

const TICK_MS: u64 = 50;
const CAPTURE_INTERVAL: Duration = Duration::from_millis(500);
const SELECTION_SETTLE: Duration = Duration::from_millis(50);
const SHOTS_TICK_MS: u64 = 16;
const SHOTS_BACKGROUND_LANES: usize = 2;
const MAX_SHOT_FAILURES: u32 = 5;
const SHOT_TIMEOUT: Duration = Duration::from_secs(3);

pub(crate) fn spawn(
    delegate: Entity<PickerState>,
    preview_cache: SharedPreviewCache,
    cx: &mut gpui::Context<AltTabApp>,
) -> Task<()> {
    cx.spawn(move |this: WeakEntity<AltTabApp>, cx: &mut AsyncApp| {
        let cx = cx.clone();
        async move { run(delegate, preview_cache, this, cx).await }
    })
}

async fn run(
    delegate: Entity<PickerState>,
    preview_cache: SharedPreviewCache,
    this: WeakEntity<AltTabApp>,
    cx: AsyncApp,
) {
    if shots_loop(&delegate, &this, &cx).await {
        clear_task_handle(&this, &cx);
        return;
    }
    preview_loop(delegate, preview_cache, this, cx).await
}

async fn shots_loop(
    delegate: &Entity<PickerState>,
    this: &WeakEntity<AltTabApp>,
    cx: &AsyncApp,
) -> bool {
    if !capture::live_shots_available() {
        return false;
    }
    let executor = cx.background_executor().clone();
    if let Some(settled) = take_fresh_settled(delegate, cx) {
        let _ = settled.await;
    }
    let (_, visible, _) = read_shot_targets(delegate, cx);
    let Some(session) = executor
        .spawn(async move { capture::warm_shots_session(&visible) })
        .await
    else {
        return false;
    };
    let (tx, rx) = mpsc::channel();
    let mut scheduler = LaneScheduler::new();
    let mut open = current_open();
    let mut in_flight: Vec<InFlightShot> = Vec::new();
    let mut failures = 0u32;
    qol_runtime::probe!("PREVIEW_LIVE", "source=shots outcome=started");

    while PICKER_VISIBLE.load(Ordering::Relaxed) {
        executor.timer(Duration::from_millis(SHOTS_TICK_MS)).await;
        if !PICKER_VISIBLE.load(Ordering::Relaxed) {
            break;
        }
        if current_open() != open {
            open = current_open();
            scheduler = LaneScheduler::new();
        }
        let (frames, failed) = drain_shot_results(&rx, &mut in_flight, &mut failures);
        let expired = expire_stale_shots(&mut in_flight, &mut failures);
        if failures >= MAX_SHOT_FAILURES {
            qol_runtime::probe!(
                "PREVIEW_LIVE",
                "source=shots outcome=degraded reason=consecutive_failures"
            );
            clear_live_frames(delegate, this, cx);
            return false;
        }
        let (live, visible, dims) = read_shot_targets(delegate, cx);
        for wid in failed.into_iter().chain(expired) {
            scheduler.retry(wid);
        }
        push_live_frames(frames, delegate, this, cx);
        let in_flight_wids: Vec<u32> = in_flight.iter().map(|shot| shot.wid).collect();
        for wid in scheduler.plan(&live, &visible, &in_flight_wids, SHOTS_BACKGROUND_LANES) {
            in_flight.push(InFlightShot {
                wid,
                launched: Instant::now(),
                open,
            });
            let (w, h) = dims
                .get(&wid)
                .copied()
                .unwrap_or((PREVIEW_MAX_WIDTH, PREVIEW_MAX_HEIGHT));
            if !session.request_capture(wid, w, h, &tx) {
                let _ = tx.send((wid, None));
            }
        }
    }
    qol_runtime::probe!(
        "PREVIEW_LIVE",
        "source=shots outcome=stopped reason=picker_hidden"
    );
    true
}

fn take_fresh_settled(
    delegate: &Entity<PickerState>,
    cx: &AsyncApp,
) -> Option<futures::channel::oneshot::Receiver<()>> {
    cx.update(|app_cx| delegate.update(app_cx, |state, _| state.fresh.take_settled()))
        .ok()
        .flatten()
}

struct InFlightShot {
    wid: u32,
    launched: Instant,
    open: u64,
}

fn drain_shot_results(
    rx: &mpsc::Receiver<ShotReply>,
    in_flight: &mut Vec<InFlightShot>,
    failures: &mut u32,
) -> (Vec<(u32, SendCVBuf, u64)>, Vec<u32>) {
    let mut frames = Vec::new();
    let mut failed = Vec::new();
    while let Ok((wid, result)) = rx.try_recv() {
        let Some(at) = in_flight.iter().position(|shot| shot.wid == wid) else {
            continue;
        };
        let open = in_flight.remove(at).open;
        match result {
            Some(buf) if buf.pixel_format() == capture::PIXEL_FORMAT_420F => {
                *failures = 0;
                qol_runtime::probe!("PREVIEW_LIVE", "source=shots outcome=frame wid={wid}");
                frames.push((wid, buf, open));
            }
            Some(_buf) => {
                qol_runtime::probe!(
                    "PREVIEW_LIVE",
                    "source=shots outcome=dropped reason=pixel_format format={:#x}",
                    _buf.pixel_format()
                );
                *failures += 1;
                failed.push(wid);
            }
            None => {
                *failures += 1;
                failed.push(wid);
            }
        }
    }
    (frames, failed)
}

fn expire_stale_shots(in_flight: &mut Vec<InFlightShot>, failures: &mut u32) -> Vec<u32> {
    let mut expired = Vec::new();
    in_flight.retain(|shot| {
        if shot.launched.elapsed() < SHOT_TIMEOUT {
            return true;
        }
        qol_runtime::probe!(
            "PREVIEW_LIVE",
            "source=shots outcome=timeout wid={}",
            shot.wid
        );
        *failures += 1;
        expired.push(shot.wid);
        false
    });
    expired
}

fn push_live_frames(
    frames: Vec<(u32, SendCVBuf, u64)>,
    delegate: &Entity<PickerState>,
    this: &WeakEntity<AltTabApp>,
    cx: &AsyncApp,
) {
    if frames.is_empty() {
        return;
    }
    let delegate = delegate.clone();
    let this = this.clone();
    let _ = cx.update(|app_cx| {
        delegate.update(app_cx, |state, _| {
            for (wid, buf, open) in frames {
                if let Some(frame) = buf.into_live_frame() {
                    state.land_live_frames([(wid, frame)].into_iter().collect(), open);
                }
            }
        });
        let _ = this.update(app_cx, |_, cx: &mut gpui::Context<AltTabApp>| cx.notify());
    });
}

type ShotTargets = (Vec<u32>, Vec<u32>, HashMap<u32, (usize, usize)>);

fn read_shot_targets(delegate: &Entity<PickerState>, cx: &AsyncApp) -> ShotTargets {
    cx.update(|app_cx| {
        let state = delegate.read(app_cx);
        let mut live: Vec<u32> = [state.selected_index, Some(0)]
            .into_iter()
            .flatten()
            .filter_map(|ix| state.windows.get(ix))
            .filter(|w| !w.is_minimized)
            .map(|w| w.id)
            .collect();
        live.dedup();
        let mut visible = Vec::new();
        let mut dims = HashMap::new();
        for w in state.windows.iter().filter(|w| !w.is_minimized) {
            if live.contains(&w.id) || !state.fresh.is_target(w.id) {
                visible.push(w.id);
            }
            dims.insert(w.id, shot_request_dims(w.width, w.height));
        }
        (live, visible, dims)
    })
    .unwrap_or_default()
}

fn clear_live_frames(delegate: &Entity<PickerState>, this: &WeakEntity<AltTabApp>, cx: &AsyncApp) {
    let delegate = delegate.clone();
    let this = this.clone();
    let _ = cx.update(|app_cx| {
        delegate.update(app_cx, |state, _| state.clear_live_frames());
        let _ = this.update(app_cx, |_, cx: &mut gpui::Context<AltTabApp>| cx.notify());
    });
}

fn clear_task_handle(this: &WeakEntity<AltTabApp>, cx: &AsyncApp) {
    let this = this.clone();
    let _ = cx.update(|app_cx| {
        let Some(entity) = this.upgrade() else {
            return;
        };
        entity.update(app_cx, |app, _| {
            app._live_preview_task = None;
        });
    });
}

async fn preview_loop(
    delegate: Entity<PickerState>,
    preview_cache: SharedPreviewCache,
    this: WeakEntity<AltTabApp>,
    cx: AsyncApp,
) {
    let executor = cx.background_executor().clone();
    let mut prev_hash: Option<(u32, u64, u64)> = None;
    let mut last_captured = Instant::now() - CAPTURE_INTERVAL;
    let mut last_selection: Option<u32> = None;
    let mut selection_changed_at = Instant::now();

    while PICKER_VISIBLE.load(Ordering::Relaxed) {
        executor.timer(Duration::from_millis(TICK_MS)).await;
        if !PICKER_VISIBLE.load(Ordering::Relaxed) {
            break;
        }
        let rendering = crate::rendering::RenderingFlow::current();
        if !rendering.captures_live_selection() {
            qol_runtime::probe!(
                "PREVIEW_LIVE",
                "outcome=stopped reason=preview_plane backend={}",
                rendering.preview_plane_backend().unwrap_or("none")
            );
            break;
        }
        let snap = read_snapshot(&delegate, &cx);
        let Some(selected) = snap.selected_target() else {
            continue;
        };
        let now = Instant::now();
        if last_selection != Some(selected.1) {
            last_selection = Some(selected.1);
            selection_changed_at = now;
            // A newly selected card must not inherit the previous card's
            // capture-interval throttle, or cycling lands on stale frames.
            last_captured = now - CAPTURE_INTERVAL;
            continue;
        }
        if now.duration_since(selection_changed_at) < SELECTION_SETTLE {
            continue;
        }
        if now.duration_since(last_captured) < CAPTURE_INTERVAL {
            continue;
        }
        last_captured = now;
        #[cfg(debug_assertions)]
        let stable_ms = now.duration_since(selection_changed_at).as_millis();
        let target = vec![selected];
        #[cfg(debug_assertions)]
        let t_capture = Instant::now();
        let open = current_open();
        let captured = run_capture(&target, &executor).await;
        #[cfg(debug_assertions)]
        let capture_ms = t_capture.elapsed().as_millis();
        let Some((_, Some(rgba))) = captured.into_iter().next() else {
            continue;
        };
        #[cfg(debug_assertions)]
        let t_render = Instant::now();
        let hash = fast_pixel_hash(&rgba.data);
        if prev_hash
            .as_ref()
            .is_some_and(|&(id, h, at)| id == selected.1 && h == hash && at == open)
        {
            #[cfg(debug_assertions)]
            probe_live_preview(
                selected.1,
                false,
                capture_ms,
                0,
                stable_ms,
                PreviewUpdateResult::default(),
            );
            continue;
        }
        prev_hash = Some((selected.1, hash, open));
        let Some(img) =
            qol_gpui::image::render_image(rgba.data, rgba.width as u32, rgba.height as u32)
        else {
            continue;
        };
        #[cfg(debug_assertions)]
        let render_ms = t_render.elapsed().as_millis();
        #[cfg(debug_assertions)]
        let wid = selected.1;
        let update_result = push_updates(
            preview_update(selected.1, img),
            open,
            &preview_cache,
            &delegate,
            &this,
            &cx,
        );
        #[cfg(not(debug_assertions))]
        let _ = (update_result.state_update, update_result.cache_write);
        #[cfg(debug_assertions)]
        {
            if update_result.cache_write {
                crate::rendering::preview_trace::record_shared_live(wid);
            }
            if update_result.state_update {
                crate::rendering::preview_trace::record_live_update(wid);
            }
            probe_live_preview(wid, true, capture_ms, render_ms, stable_ms, update_result);
        }
    }

    clear_task_handle(&this, &cx);
}

struct Snapshot {
    selected: Option<(usize, u32)>,
}

impl Snapshot {
    fn selected_target(&self) -> Option<(usize, u32)> {
        self.selected
    }
}

fn read_snapshot(delegate: &Entity<PickerState>, cx: &AsyncApp) -> Snapshot {
    cx.update(|app_cx| {
        let state = delegate.read(app_cx);
        let idx = state.selected_index.unwrap_or(0);
        let selected = state
            .windows
            .get(idx)
            .filter(|w| !w.is_minimized)
            .map(|w| (idx, w.id));
        Snapshot { selected }
    })
    .unwrap_or(Snapshot { selected: None })
}

async fn run_capture(
    targets: &[(usize, u32)],
    executor: &gpui::BackgroundExecutor,
) -> Vec<(usize, Option<qol_app_icon::RgbaImage>)> {
    let owned = targets.to_vec();
    executor
        .spawn(async move {
            capture::capture_previews_cg(&owned, PREVIEW_MAX_WIDTH, PREVIEW_MAX_HEIGHT)
        })
        .await
}

#[derive(Clone, Copy, Default)]
struct PreviewUpdateResult {
    state_update: bool,
    cache_write: bool,
}

fn preview_update(wid: u32, img: Arc<RenderImage>) -> PreviewMap {
    [(wid, img)].into_iter().collect()
}

fn push_updates(
    updates: PreviewMap,
    open: u64,
    preview_cache: &SharedPreviewCache,
    delegate: &Entity<PickerState>,
    this: &WeakEntity<AltTabApp>,
    cx: &AsyncApp,
) -> PreviewUpdateResult {
    let shared_updates = updates.clone();
    let preview_cache = preview_cache.clone();
    let delegate = delegate.clone();
    let this = this.clone();
    cx.update(|app_cx| {
        let mut cache_write = false;
        if let Ok(mut cache) = preview_cache.lock() {
            qol_gpui::image_registry::extend_with(&mut *cache, shared_updates, app_cx, None);
            cache_write = true;
        }
        // App-level: no Window leased here, so insert_preview's release path
        // passes None. The picker window stays in App::windows and gets
        // touched via the iteration in App::drop_image.
        delegate.update(app_cx, |state, ctx| {
            state.land_previews(updates, open, ctx, None);
        });
        let _ = this.update(app_cx, |_, cx: &mut gpui::Context<AltTabApp>| cx.notify());
        PreviewUpdateResult {
            state_update: true,
            cache_write,
        }
    })
    .unwrap_or_default()
}

#[cfg(debug_assertions)]
fn probe_live_preview(
    wid: u32,
    changed: bool,
    capture_ms: u128,
    render_ms: u128,
    stable_ms: u128,
    update_result: PreviewUpdateResult,
) {
    qol_runtime::probe!(
        "PREVIEW_LIVE",
        "source=live wid={} changed={} state_update={} cache_write={} capture={}ms render={}ms stable={}ms",
        wid,
        changed,
        update_result.state_update,
        update_result.cache_write,
        capture_ms,
        render_ms,
        stable_ms,
    );
}
