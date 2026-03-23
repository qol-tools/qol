use crate::app::{AltTabApp, PICKER_VISIBLE};
use crate::capture;
use crate::picker::state::PickerState;
use crate::shared::layout::{PREVIEW_MAX_HEIGHT, PREVIEW_MAX_WIDTH};
use crate::shared::preview::{bgra_to_render_image, fast_pixel_hash};
use gpui::{AsyncApp, Entity, RenderImage, Task, WeakEntity};
use std::collections::HashMap;
use std::sync::atomic::Ordering;
use std::sync::Arc;
use std::time::{Duration, Instant};

const TICK_MS: u64 = 33;
const SELECTED_INTERVAL: Duration = Duration::from_millis(33);
const NEIGHBOR_INTERVAL: Duration = Duration::from_millis(250);
const REST_INTERVAL: Duration = Duration::from_millis(2000);
const NEIGHBOR_RADIUS: usize = 5;
const MAX_BATCH: usize = 10;
const VISIBILITY_POLL_MS: u64 = 16;

pub(crate) fn spawn(delegate: Entity<PickerState>, cx: &mut gpui::Context<AltTabApp>) -> Task<()> {
    cx.spawn(move |this: WeakEntity<AltTabApp>, cx: &mut AsyncApp| {
        let cx = cx.clone();
        async move { preview_loop(delegate, this, cx).await }
    })
}

struct LoopState {
    prev_hashes: HashMap<u32, u64>,
    last_captured: HashMap<u32, Instant>,
}

async fn preview_loop(delegate: Entity<PickerState>, this: WeakEntity<AltTabApp>, cx: AsyncApp) {
    let executor = cx.background_executor().clone();
    let mut state = LoopState {
        prev_hashes: HashMap::new(),
        last_captured: HashMap::new(),
    };

    loop {
        while !PICKER_VISIBLE.load(Ordering::Relaxed) {
            executor
                .timer(Duration::from_millis(VISIBILITY_POLL_MS))
                .await;
        }
        state.prev_hashes.clear();
        state.last_captured.clear();

        while PICKER_VISIBLE.load(Ordering::Relaxed) {
            executor.timer(Duration::from_millis(TICK_MS)).await;
            if !PICKER_VISIBLE.load(Ordering::Relaxed) {
                break;
            }
            let snap = read_snapshot(&delegate, &cx);
            if snap.all_ids.is_empty() {
                continue;
            }
            let now = Instant::now();
            let batch = build_batch(&snap, &state.last_captured, now);
            if batch.is_empty() {
                continue;
            }
            let captured = run_capture(&batch, &executor).await;
            let updates = diff_and_update(
                captured,
                &batch,
                &mut state.prev_hashes,
                &mut state.last_captured,
                now,
            );
            if !updates.is_empty() {
                push_updates(updates, &delegate, &this, &cx);
            }
        }
    }
}

struct Snapshot {
    all_ids: Vec<(usize, u32)>,
    selected_idx: Option<usize>,
}

fn read_snapshot(delegate: &Entity<PickerState>, cx: &AsyncApp) -> Snapshot {
    cx.update(|app_cx| {
        let state = delegate.read(app_cx);
        let all_ids = state
            .windows
            .iter()
            .enumerate()
            .filter(|(_, w)| !w.is_minimized)
            .map(|(i, w)| (i, w.id))
            .collect();
        Snapshot {
            all_ids,
            selected_idx: state.selected_index,
        }
    })
    .unwrap_or_else(|_| Snapshot {
        all_ids: Vec::new(),
        selected_idx: None,
    })
}

fn build_batch(
    snap: &Snapshot,
    last_captured: &HashMap<u32, Instant>,
    now: Instant,
) -> Vec<(usize, u32)> {
    let mut batch = Vec::with_capacity(MAX_BATCH);
    let selected = snap.selected_idx.unwrap_or(0);

    // Sort by priority: selected first, then neighbors by distance, then rest
    let mut sorted: Vec<&(usize, u32)> = snap.all_ids.iter().collect();
    sorted.sort_by_key(|(idx, _)| {
        let d = idx.abs_diff(selected);
        if d == 0 {
            0
        } else if d <= NEIGHBOR_RADIUS {
            d
        } else {
            NEIGHBOR_RADIUS + 1 + d
        }
    });

    for &&(idx, wid) in &sorted {
        let interval = tier_interval(idx, selected);
        let elapsed = last_captured
            .get(&wid)
            .map(|t| now.duration_since(*t))
            .unwrap_or(Duration::MAX);
        if elapsed >= interval {
            batch.push((idx, wid));
            if batch.len() >= MAX_BATCH {
                break;
            }
        }
    }
    batch
}

fn tier_interval(idx: usize, selected: usize) -> Duration {
    if idx == selected {
        return SELECTED_INTERVAL;
    }
    if idx.abs_diff(selected) <= NEIGHBOR_RADIUS {
        return NEIGHBOR_INTERVAL;
    }
    REST_INTERVAL
}

async fn run_capture(
    targets: &[(usize, u32)],
    executor: &gpui::BackgroundExecutor,
) -> Vec<(usize, Option<qol_plugin_api::app_icon::RgbaImage>)> {
    let owned = targets.to_vec();
    executor
        .spawn(async move {
            capture::capture_previews_cg(&owned, PREVIEW_MAX_WIDTH, PREVIEW_MAX_HEIGHT)
        })
        .await
}

fn diff_and_update(
    captured: Vec<(usize, Option<qol_plugin_api::app_icon::RgbaImage>)>,
    targets: &[(usize, u32)],
    prev_hashes: &mut HashMap<u32, u64>,
    last_captured: &mut HashMap<u32, Instant>,
    now: Instant,
) -> Vec<(u32, Arc<RenderImage>)> {
    let id_map: HashMap<usize, u32> = targets.iter().copied().collect();
    let mut updates = Vec::new();
    for (idx, rgba) in captured {
        let Some(rgba) = rgba else { continue };
        let Some(&wid) = id_map.get(&idx) else {
            continue;
        };
        last_captured.insert(wid, now);
        let hash = fast_pixel_hash(&rgba.data);
        if prev_hashes.get(&wid) == Some(&hash) {
            continue;
        }
        prev_hashes.insert(wid, hash);
        if let Some(img) = bgra_to_render_image(rgba.data, rgba.width, rgba.height) {
            updates.push((wid, img));
        }
    }
    updates
}

fn push_updates(
    updates: Vec<(u32, Arc<RenderImage>)>,
    delegate: &Entity<PickerState>,
    this: &WeakEntity<AltTabApp>,
    cx: &AsyncApp,
) {
    let delegate = delegate.clone();
    let this = this.clone();
    let _ = cx.update(|app_cx| {
        delegate.update(app_cx, |state, _cx| {
            for (wid, img) in updates {
                state.live_previews.insert(wid, img);
            }
        });
        let _ = this.update(app_cx, |_, cx: &mut gpui::Context<AltTabApp>| cx.notify());
    });
}
