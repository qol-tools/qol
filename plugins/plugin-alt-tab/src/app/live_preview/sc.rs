use super::perf::PerfCounters;
use super::{LIVE_PREVIEW_INTERVAL_MS, SC_POLL_INTERVAL_MS};
use crate::app::{AltTabApp, PICKER_VISIBLE};
use crate::delegate::WindowDelegate;
use crate::layout::{PREVIEW_MAX_HEIGHT, PREVIEW_MAX_WIDTH};
use crate::platform;
use crate::preview::{bgra_to_render_image, fast_pixel_hash};
use gpui::{AsyncApp, Entity, Task, WeakEntity};
use std::collections::{HashMap, HashSet};
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

const SC_STALL_TIMEOUT_MS: u64 = 900;
const SC_STARTUP_STALL_TIMEOUT_MS: u64 = 180;
const SC_BRIDGE_CG_INTERVAL_MS: u64 = 120;
const CG_FALLBACK_FG_INTERVAL_MS: u64 = 33;
const CG_FALLBACK_BG_INTERVAL_MS: u64 = 120;
const CG_FALLBACK_BG_BATCH_SIZE: usize = 2;
static SC_DISABLED_FOR_SESSION: AtomicBool = AtomicBool::new(false);

/// Returns an `Instant` that is `ms` milliseconds in the past (or `Instant::now()` if underflow).
fn instant_past(ms: u64) -> Instant {
    Instant::now()
        .checked_sub(Duration::from_millis(ms))
        .unwrap_or_else(Instant::now)
}

struct StreamState {
    active: bool,
    /// [0] = selected, [1] = hovered — both promoted to 30fps
    promoted: [Option<u32>; 2],
}

impl StreamState {
    fn new() -> Self {
        Self { active: false, promoted: [None; 2] }
    }

    fn reset(&mut self) {
        self.active = false;
        self.promoted = [None; 2];
    }
}

/// CG capture scheduling and deduplication state.
struct CgState {
    full_capture: bool,
    round_robin_pos: usize,
    prev_hashes: HashMap<u32, u64>,
    last_fg_capture: Instant,
    last_bg_capture: Instant,
    last_bridge_capture: Instant,
}

impl CgState {
    fn new() -> Self {
        Self {
            full_capture: true,
            round_robin_pos: 0,
            prev_hashes: HashMap::new(),
            last_fg_capture: instant_past(CG_FALLBACK_FG_INTERVAL_MS),
            last_bg_capture: instant_past(CG_FALLBACK_BG_INTERVAL_MS),
            last_bridge_capture: instant_past(SC_BRIDGE_CG_INTERVAL_MS),
        }
    }

    fn reset(&mut self) {
        self.full_capture = true;
        self.round_robin_pos = 0;
        self.prev_hashes.clear();
        self.last_fg_capture = instant_past(CG_FALLBACK_FG_INTERVAL_MS);
        self.last_bg_capture = instant_past(CG_FALLBACK_BG_INTERVAL_MS);
        self.last_bridge_capture = instant_past(SC_BRIDGE_CG_INTERVAL_MS);
    }

    fn bridge_due(&self) -> bool {
        self.last_bridge_capture.elapsed() >= Duration::from_millis(SC_BRIDGE_CG_INTERVAL_MS)
    }
}

fn clear_delegate_previews(delegate: &Entity<WindowDelegate>, cx: &AsyncApp) {
    let delegate = delegate.clone();
    let _ = cx.update(|app_cx| {
        let _ = delegate.update(app_cx, |d, _cx| {
            d.live_previews.clear();
        });
    });
}

/// Demote promoted streams back to 5fps and clear local state.
/// Does NOT stop streams — the prewarm loop owns stream lifecycle.
async fn deactivate(
    state: &mut StreamState,
    executor: &gpui::BackgroundExecutor,
    delegate: &Entity<WindowDelegate>,
    cx: &AsyncApp,
) {
    let (w, h) = (PREVIEW_MAX_WIDTH, PREVIEW_MAX_HEIGHT);
    for &wid in state.promoted.iter().flatten() {
        executor.spawn(async move { platform::sc_demote_stream(wid, w, h) }).await;
    }
    state.reset();
    clear_delegate_previews(delegate, cx);
}

/// Emergency stop: kill all streams (stall recovery).
async fn force_stop(
    state: &mut StreamState,
    executor: &gpui::BackgroundExecutor,
    delegate: &Entity<WindowDelegate>,
    cx: &AsyncApp,
) {
    executor.spawn(async { platform::sc_stop_streams() }).await;
    state.reset();
    clear_delegate_previews(delegate, cx);
}

/// Activate SC live preview: promote selected window to 30fps.
/// Streams must already be running (started by prewarm loop).
async fn activate(
    state: &mut StreamState,
    delegate: &Entity<WindowDelegate>,
    executor: &gpui::BackgroundExecutor,
    cx: &AsyncApp,
) -> bool {
    if !platform::sc_streams_active() {
        return false;
    }
    state.active = true;

    let selected_wid = cx
        .update(|app_cx| {
            let d = delegate.read(app_cx);
            d.selected_index.and_then(|ix| d.windows.get(ix)).map(|w| w.id)
        })
        .unwrap_or_default();
    let Some(wid) = selected_wid else { return true; };
    let (w, h) = (PREVIEW_MAX_WIDTH, PREVIEW_MAX_HEIGHT);
    executor.spawn(async move { platform::sc_promote_stream(wid, w, h) }).await;
    state.promoted[0] = Some(wid);
    true
}

/// Sync promoted streams with desired [selected, hovered]. Promotes new, demotes removed.
async fn sync_promoted(
    state: &mut StreamState,
    desired: [Option<u32>; 2],
    executor: &gpui::BackgroundExecutor,
) {
    if desired == state.promoted { return; }

    let (w, h) = (PREVIEW_MAX_WIDTH, PREVIEW_MAX_HEIGHT);
    let was = state.promoted;

    // Demote wids no longer in desired set
    for &wid in was.iter().flatten() {
        if desired.iter().flatten().any(|&d| d == wid) { continue; }
        #[cfg(debug_assertions)]
        eprintln!("[alt-tab/sc] demote wid={}", wid);
        executor.spawn(async move { platform::sc_demote_stream(wid, w, h) }).await;
    }
    // Promote wids newly in desired set
    for &wid in desired.iter().flatten() {
        if was.iter().flatten().any(|&p| p == wid) { continue; }
        #[cfg(debug_assertions)]
        eprintln!("[alt-tab/sc] promote wid={}", wid);
        executor.spawn(async move { platform::sc_promote_stream(wid, w, h) }).await;
    }

    state.promoted = desired;
}

fn update_surfaces(surfaces: HashMap<u32, platform::SendCVBuf>, delegate: &Entity<WindowDelegate>, cx: &AsyncApp) {
    let delegate = delegate.clone();
    let _ = cx.update(|app_cx| {
        let _ = delegate.update(app_cx, |d, _cx| {
            for (wid, buf) in surfaces { d.live_surfaces.insert(wid, buf.into_cvpixelbuffer()); }
        });
    });
}

fn read_targets(
    delegate: &Entity<WindowDelegate>,
    cx: &AsyncApp,
) -> (Vec<(usize, u32)>, Option<u32>, Option<u32>, HashSet<u32>) {
    cx.update(|app_cx| {
        let d = delegate.read(app_cx);
        let all_ids: Vec<(usize, u32)> = d.windows
            .iter()
            .enumerate()
            .filter(|(_, w)| !w.is_minimized)
            .map(|(i, w)| (i, w.id))
            .collect();
        let sel = d.selected_index
            .and_then(|ix| d.windows.get(ix))
            .map(|w| w.id);
        let hov = d.hovered_index
            .and_then(|ix| d.windows.get(ix))
            .map(|w| w.id);
        let surface_wids: HashSet<u32> = d.live_surfaces.keys().copied().collect();
        (all_ids, sel, hov, surface_wids)
    })
    .unwrap_or_default()
}

fn build_cg_foreground_targets(
    all_ids: &[(usize, u32)],
    selected_wid: Option<u32>,
    hovered_wid: Option<u32>,
) -> Vec<(usize, u32)> {
    let mut targets = Vec::with_capacity(2);
    if let Some(entry) = selected_wid
        .and_then(|wid| all_ids.iter().find(|(_, id)| *id == wid))
    {
        targets.push(*entry);
    }

    let Some(hovered_wid) = hovered_wid else {
        return targets;
    };
    if selected_wid == Some(hovered_wid) {
        return targets;
    }
    let Some(entry) = all_ids.iter().find(|(_, id)| *id == hovered_wid) else {
        return targets;
    };
    targets.push(*entry);
    targets
}

fn build_cg_background_targets(
    all_ids: &[(usize, u32)],
    selected_wid: Option<u32>,
    hovered_wid: Option<u32>,
    round_robin_pos: &mut usize,
) -> Vec<(usize, u32)> {
    let pool: Vec<(usize, u32)> = all_ids
        .iter()
        .filter(|(_, wid)| Some(*wid) != selected_wid && Some(*wid) != hovered_wid)
        .copied()
        .collect();
    if pool.is_empty() {
        return Vec::new();
    }
    *round_robin_pos %= pool.len();

    let mut targets = Vec::with_capacity(CG_FALLBACK_BG_BATCH_SIZE);
    for _ in 0..CG_FALLBACK_BG_BATCH_SIZE.min(pool.len()) {
        targets.push(pool[*round_robin_pos]);
        *round_robin_pos += 1;
        if *round_robin_pos >= pool.len() {
            *round_robin_pos = 0;
        }
    }
    targets
}

async fn capture_cg(
    targets: &[(usize, u32)],
    prev_hashes: &mut HashMap<u32, u64>,
    delegate: &Entity<WindowDelegate>,
    cx: &AsyncApp,
    executor: &gpui::BackgroundExecutor,
) -> u32 {
    let target_map: HashMap<usize, u32> = targets.iter().copied().collect();
    let targets_owned = targets.to_vec();
    let captured = executor
        .spawn(async move {
            platform::capture_previews_cg(&targets_owned, PREVIEW_MAX_WIDTH, PREVIEW_MAX_HEIGHT)
        })
        .await;

    let mut updates = Vec::new();
    for (idx, rgba_opt) in captured {
        let Some(rgba) = rgba_opt else { continue };
        let Some(&wid) = target_map.get(&idx) else { continue };
        let hash = fast_pixel_hash(&rgba.data);
        if prev_hashes.get(&wid) == Some(&hash) {
            continue;
        }
        prev_hashes.insert(wid, hash);
        let Some(img) = bgra_to_render_image(rgba.data, rgba.width, rgba.height) else { continue };
        updates.push((wid, img));
    }
    let count = updates.len() as u32;
    if count == 0 {
        return 0;
    }

    let delegate = delegate.clone();
    let _ = cx.update(|app_cx| {
        let _ = delegate.update(app_cx, |d, _cx| {
            for (wid, img) in updates {
                d.live_previews.insert(wid, img);
            }
        });
    });
    count
}

/// Try to take SC frames. Returns true if frames were consumed and pushed to surfaces.
async fn poll_sc_frames(
    last_sc_frame: &mut Instant,
    perf: &mut PerfCounters,
    delegate: &Entity<WindowDelegate>,
    cx: &AsyncApp,
    executor: &gpui::BackgroundExecutor,
) -> bool {
    if !platform::sc_has_new_frames() {
        perf.add_skip();
        return false;
    }
    let frames = executor.spawn(async { platform::sc_take_frames() }).await;
    if frames.is_empty() {
        perf.add_skip();
        return false;
    }
    *last_sc_frame = Instant::now();
    perf.add_frames(frames.len() as u32);
    perf.add_frame_wids(frames.keys().copied());
    update_surfaces(frames, delegate, cx);
    true
}

/// Pick CG capture targets based on scheduling cadence.
fn pick_cg_targets(
    all_ids: &[(usize, u32)],
    sel_wid: Option<u32>,
    hov_wid: Option<u32>,
    cg: &mut CgState,
) -> Vec<(usize, u32)> {
    let now = Instant::now();
    if cg.full_capture {
        cg.full_capture = false;
        cg.last_fg_capture = now;
        cg.last_bg_capture = now;
        return all_ids.to_vec();
    }

    let fg_due = cg.last_fg_capture.elapsed() >= Duration::from_millis(CG_FALLBACK_FG_INTERVAL_MS);
    let bg_due = cg.last_bg_capture.elapsed() >= Duration::from_millis(CG_FALLBACK_BG_INTERVAL_MS);
    if !fg_due && !bg_due { return Vec::new(); }

    let mut merged = Vec::new();
    if fg_due {
        merged.extend(build_cg_foreground_targets(all_ids, sel_wid, hov_wid));
        cg.last_fg_capture = now;
    }
    if bg_due {
        for entry in build_cg_background_targets(all_ids, sel_wid, hov_wid, &mut cg.round_robin_pos) {
            if merged.iter().any(|(_, wid)| *wid == entry.1) { continue; }
            merged.push(entry);
        }
        cg.last_bg_capture = now;
    }
    merged
}

/// Throttled UI notification (max ~10fps).
fn maybe_notify(
    last_notify: &mut Instant,
    perf: &mut PerfCounters,
    this: &WeakEntity<AltTabApp>,
    cx: &AsyncApp,
) {
    if last_notify.elapsed() < Duration::from_millis(100) { return; }
    perf.add_notify();
    notify_ui(this, cx);
    *last_notify = Instant::now();
}

fn notify_ui(this: &WeakEntity<AltTabApp>, cx: &AsyncApp) {
    let this = this.clone();
    let _ = cx.update(|app_cx| {
        let _ = this.update(app_cx, |_, cx: &mut gpui::Context<AltTabApp>| { cx.notify(); });
    });
}

fn sc_is_stalled(
    saw_sc_frames: bool,
    last_sc_start: Instant,
    last_sc_frame: Instant,
) -> bool {
    if !saw_sc_frames {
        return last_sc_start.elapsed() >= Duration::from_millis(SC_STARTUP_STALL_TIMEOUT_MS);
    }
    last_sc_frame.elapsed() >= Duration::from_millis(SC_STALL_TIMEOUT_MS)
}

pub(super) fn spawn(
    delegate: Entity<WindowDelegate>,
    cx: &mut gpui::Context<AltTabApp>,
) -> Task<()> {
    cx.spawn(
        move |this: WeakEntity<AltTabApp>, cx: &mut AsyncApp| {
            let cx = cx.clone();
            async move {
                let executor = cx.background_executor().clone();
                let mut state = StreamState::new();
                let mut perf = PerfCounters::new();
                let mut last_notify = Instant::now();
                let mut visible_since: Option<Instant> = None;
                let mut prev_hov: Option<u32> = None;
                let mut last_sc_frame = Instant::now();
                let mut last_sc_start = Instant::now();
                let mut saw_sc_frames = false;
                let mut sc_unhealthy = SC_DISABLED_FOR_SESSION.load(Ordering::Relaxed);
                let mut cg = CgState::new();
                loop {
                    let visible = PICKER_VISIBLE.load(Ordering::Relaxed);

                    // On hide: demote promoted streams back to 5fps.
                    // Streams stay alive (prewarm loop owns lifecycle).
                    if !visible && state.active {
                        deactivate(&mut state, &executor, &delegate, &cx).await;
                    }
                    if !visible {
                        visible_since = None;
                        prev_hov = None;
                        last_sc_frame = Instant::now();
                        last_sc_start = Instant::now();
                        saw_sc_frames = false;
                        cg.reset();
                        executor.timer(Duration::from_millis(LIVE_PREVIEW_INTERVAL_MS)).await;
                        continue;
                    }
                    if visible_since.is_none() {
                        visible_since = Some(Instant::now());
                    }

                    // Activate: if prewarm streams are running, promote and start polling.
                    if !sc_unhealthy && !state.active && activate(&mut state, &delegate, &executor, &cx).await {
                        #[cfg(debug_assertions)]
                        eprintln!("[alt-tab/sc] activated (streams already warm)");
                        last_sc_start = Instant::now();
                        saw_sc_frames = true;
                        last_sc_frame = Instant::now();
                        // Flush any frames already buffered by background streams
                        let frames = executor.spawn(async { platform::sc_take_frames() }).await;
                        if !frames.is_empty() {
                            perf.add_frames(frames.len() as u32);
                            update_surfaces(frames, &delegate, &cx);
                            notify_ui(&this, &cx);
                        }
                    }

                    executor.timer(Duration::from_millis(SC_POLL_INTERVAL_MS)).await;
                    perf.tick();

                    if !PICKER_VISIBLE.load(Ordering::Relaxed) {
                        continue;
                    }

                    let (all_ids, sel_wid, hov_wid, surface_wids) = read_targets(&delegate, &cx);
                    if all_ids.is_empty() {
                        continue;
                    }

                    #[cfg(debug_assertions)]
                    if hov_wid != prev_hov {
                        eprintln!("[alt-tab/sc] hover changed: {:?} -> {:?}", prev_hov, hov_wid);
                    }
                    prev_hov = hov_wid;

                    if !sc_unhealthy && state.active {
                        sync_promoted(&mut state, [sel_wid, hov_wid], &executor).await;

                        let got_sc_frames = poll_sc_frames(
                            &mut last_sc_frame,
                            &mut perf,
                            &delegate,
                            &cx,
                            &executor,
                        ).await;

                        if got_sc_frames {
                            saw_sc_frames = true;
                            maybe_notify(&mut last_notify, &mut perf, &this, &cx);
                            continue;
                        }

                        if cg.bridge_due() {
                            let bridge_targets: Vec<(usize, u32)> = if saw_sc_frames {
                                build_cg_foreground_targets(&all_ids, sel_wid, hov_wid)
                            } else {
                                all_ids.clone()
                            }
                            .into_iter()
                            .filter(|(_, wid)| !surface_wids.contains(wid))
                            .collect();
                            if !bridge_targets.is_empty() {
                                let updated = capture_cg(
                                    &bridge_targets,
                                    &mut cg.prev_hashes,
                                    &delegate,
                                    &cx,
                                    &executor,
                                ).await;
                                if updated > 0 {
                                    perf.add_frames(updated);
                                    perf.add_frame_wids(
                                        bridge_targets.iter().map(|(_, wid)| *wid),
                                    );
                                    maybe_notify(&mut last_notify, &mut perf, &this, &cx);
                                }
                            }
                            cg.last_bridge_capture = Instant::now();
                        }

                        if !sc_is_stalled(saw_sc_frames, last_sc_start, last_sc_frame) {
                            continue;
                        }

                        force_stop(&mut state, &executor, &delegate, &cx).await;
                        sc_unhealthy = true;
                        SC_DISABLED_FOR_SESSION.store(true, Ordering::Relaxed);
                        eprintln!("[alt-tab/sc] SC stalled, switching to CG fallback");
                        cg.reset();
                        continue;
                    }

                    let targets = pick_cg_targets(&all_ids, sel_wid, hov_wid, &mut cg);
                    if targets.is_empty() {
                        perf.add_skip();
                        continue;
                    }

                    let updated = capture_cg(&targets, &mut cg.prev_hashes, &delegate, &cx, &executor).await;
                    if updated == 0 {
                        perf.add_skip();
                        continue;
                    }
                    perf.add_frames(updated);
                    perf.add_frame_wids(targets.iter().map(|(_, wid)| *wid));
                    maybe_notify(&mut last_notify, &mut perf, &this, &cx);
                }
            }
        },
    )
}
