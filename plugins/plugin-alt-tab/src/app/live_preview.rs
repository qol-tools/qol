use crate::app::{AltTabApp, PICKER_VISIBLE};
use crate::capture;
use crate::picker::state::PickerState;
use crate::shared::layout::{PREVIEW_MAX_HEIGHT, PREVIEW_MAX_WIDTH};
use crate::shared::preview::{bgra_to_render_image, fast_pixel_hash};
use gpui::{AsyncApp, Entity, RenderImage, Task, WeakEntity};
use qol_plugin_api::app_icon::RgbaImage;
use std::collections::HashMap;
use std::sync::atomic::Ordering;
use std::time::Duration;

const LIVE_PREVIEW_INTERVAL_MS: u64 = 500;
const VISIBILITY_POLL_MS: u64 = 16;

pub(crate) fn spawn(
    delegate: Entity<PickerState>,
    cx: &mut gpui::Context<AltTabApp>,
) -> Task<()> {
    cx.spawn(move |this: WeakEntity<AltTabApp>, cx: &mut AsyncApp| {
        let cx = cx.clone();
        async move { preview_loop(delegate, this, cx).await }
    })
}

struct LoopState {
    prev_hashes: HashMap<u32, u64>,
    skip_timer: bool,
    round_robin_pos: usize,
}

async fn preview_loop(
    delegate: Entity<PickerState>,
    this: WeakEntity<AltTabApp>,
    cx: AsyncApp,
) {
    let executor = cx.background_executor().clone();
    let mut state = LoopState {
        prev_hashes: HashMap::new(),
        skip_timer: true,
        round_robin_pos: 0,
    };

    loop {
        if !wait_for_visible(&executor, &mut state).await {
            continue;
        }
        let snapshot = read_snapshot(&delegate, &cx);
        if snapshot.all_ids.is_empty() {
            continue;
        }
        let targets = pick_targets(&snapshot, &mut state.round_robin_pos);
        let captured = run_capture(&targets, &executor).await;
        let updates = diff_captures(captured, &targets, &mut state.prev_hashes);
        if !updates.is_empty() {
            push_updates(updates, &delegate, &this, &cx);
        }
    }
}

async fn wait_for_visible(executor: &gpui::BackgroundExecutor, state: &mut LoopState) -> bool {
    if state.skip_timer {
        while !PICKER_VISIBLE.load(Ordering::Relaxed) {
            executor.timer(Duration::from_millis(VISIBILITY_POLL_MS)).await;
        }
        state.skip_timer = false;
        return true;
    }
    executor.timer(Duration::from_millis(LIVE_PREVIEW_INTERVAL_MS)).await;
    if !PICKER_VISIBLE.load(Ordering::Relaxed) {
        state.prev_hashes.clear();
        state.skip_timer = true;
        state.round_robin_pos = 0;
        return false;
    }
    true
}

struct Snapshot {
    all_ids: Vec<(usize, u32)>,
    selected_idx: Option<usize>,
}

fn read_snapshot(delegate: &Entity<PickerState>, cx: &AsyncApp) -> Snapshot {
    cx.update(|app_cx| {
        let state = delegate.read(app_cx);
        let all_ids = state.windows.iter().enumerate()
            .filter(|(_, w)| !w.is_minimized)
            .map(|(i, w)| (i, w.id))
            .collect();
        Snapshot { all_ids, selected_idx: state.selected_index }
    })
    .unwrap_or_else(|_| Snapshot { all_ids: Vec::new(), selected_idx: None })
}

fn pick_targets(snap: &Snapshot, round_robin_pos: &mut usize) -> Vec<(usize, u32)> {
    let mut t = Vec::with_capacity(2);
    if let Some(sel) = snap.selected_idx {
        if let Some(&entry) = snap.all_ids.iter().find(|(i, _)| *i == sel) {
            t.push(entry);
        }
    }
    let non_selected: Vec<(usize, u32)> = snap.all_ids.iter()
        .filter(|(i, _)| Some(*i) != snap.selected_idx)
        .copied()
        .collect();
    if non_selected.is_empty() {
        return t;
    }
    *round_robin_pos %= non_selected.len();
    t.push(non_selected[*round_robin_pos]);
    *round_robin_pos += 1;
    t
}

async fn run_capture(
    targets: &[(usize, u32)],
    executor: &gpui::BackgroundExecutor,
) -> Vec<(usize, Option<RgbaImage>)> {
    let owned = targets.to_vec();
    executor.spawn(async move {
        capture::capture_previews_cg(&owned, PREVIEW_MAX_WIDTH, PREVIEW_MAX_HEIGHT)
    }).await
}

fn diff_captures(
    captured: Vec<(usize, Option<RgbaImage>)>,
    targets: &[(usize, u32)],
    prev_hashes: &mut HashMap<u32, u64>,
) -> Vec<(u32, std::sync::Arc<RenderImage>)> {
    captured.into_iter()
        .filter_map(|(idx, rgba)| diff_one(idx, rgba?, targets, prev_hashes))
        .collect()
}

fn diff_one(
    idx: usize,
    rgba: RgbaImage,
    targets: &[(usize, u32)],
    prev_hashes: &mut HashMap<u32, u64>,
) -> Option<(u32, std::sync::Arc<RenderImage>)> {
    let &(_, wid) = targets.iter().find(|(i, _)| *i == idx)?;
    let hash = fast_pixel_hash(&rgba.data);
    if prev_hashes.get(&wid) == Some(&hash) {
        return None;
    }
    prev_hashes.insert(wid, hash);
    let img = bgra_to_render_image(rgba.data, rgba.width, rgba.height)?;
    Some((wid, img))
}

fn push_updates(
    updates: Vec<(u32, std::sync::Arc<RenderImage>)>,
    delegate: &Entity<PickerState>,
    this: &WeakEntity<AltTabApp>,
    cx: &AsyncApp,
) {
    let delegate = delegate.clone();
    let this = this.clone();
    let _ = cx.update(|app_cx| {
        let _ = delegate.update(app_cx, |state, _cx| {
            for (wid, img) in updates {
                state.live_previews.insert(wid, img);
            }
        });
        let _ = this.update(app_cx, |_, cx: &mut gpui::Context<AltTabApp>| cx.notify());
    });
}
