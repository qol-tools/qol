use super::LIVE_PREVIEW_INTERVAL_MS;
use crate::app::{AltTabApp, PICKER_VISIBLE};
use crate::delegate::WindowDelegate;
use crate::layout::{PREVIEW_MAX_HEIGHT, PREVIEW_MAX_WIDTH};
use crate::platform;
use crate::preview::{bgra_to_render_image, fast_pixel_hash};
use gpui::{AsyncApp, Entity, Task, WeakEntity};
use std::collections::HashMap;
use std::sync::atomic::Ordering;
use std::time::Duration;

pub(super) fn spawn(
    delegate: Entity<WindowDelegate>,
    cx: &mut gpui::Context<AltTabApp>,
) -> Task<()> {
    cx.spawn(
        move |this: WeakEntity<AltTabApp>, cx: &mut AsyncApp| {
            let cx = cx.clone();
            async move {
                let executor = cx.background_executor().clone();
                let mut prev_hashes: HashMap<u32, u64> = HashMap::new();
                let mut full_capture = true;
                let mut round_robin_pos: usize = 0;
                loop {
                    if full_capture && PICKER_VISIBLE.load(Ordering::Relaxed) {
                        // Skip timer on first visible tick for instant refresh
                    } else {
                        executor
                            .timer(Duration::from_millis(LIVE_PREVIEW_INTERVAL_MS))
                            .await;
                    }
                    if !PICKER_VISIBLE.load(Ordering::Relaxed) {
                        prev_hashes.clear();
                        full_capture = true;
                        round_robin_pos = 0;
                        continue;
                    }
                    let (all_ids, selected_idx): (Vec<(usize, u32)>, Option<usize>) = cx
                        .update(|app_cx| {
                            let state = delegate.read(app_cx);
                            let ids = state.windows.iter().enumerate()
                                .filter(|(_, w)| !w.is_minimized)
                                .map(|(i, w)| (i, w.id))
                                .collect();
                            (ids, state.selected_index)
                        })
                        .unwrap_or_default();
                    if all_ids.is_empty() {
                        continue;
                    }

                    let targets: Vec<(usize, u32)> = if full_capture {
                        full_capture = false;
                        all_ids.clone()
                    } else {
                        build_targets(&all_ids, selected_idx, &mut round_robin_pos)
                    };

                    capture(&targets, &mut prev_hashes, &delegate, &this, &cx, &executor)
                        .await;
                }
            }
        },
    )
}

fn build_targets(
    all_ids: &[(usize, u32)],
    selected_idx: Option<usize>,
    round_robin_pos: &mut usize,
) -> Vec<(usize, u32)> {
    let mut t = Vec::with_capacity(2);
    if let Some(sel) = selected_idx {
        if let Some(&entry) = all_ids.iter().find(|(i, _)| *i == sel) {
            t.push(entry);
        }
    }
    let non_selected: Vec<(usize, u32)> = all_ids.iter()
        .filter(|(i, _)| Some(*i) != selected_idx)
        .copied()
        .collect();
    if !non_selected.is_empty() {
        *round_robin_pos %= non_selected.len();
        t.push(non_selected[*round_robin_pos]);
        *round_robin_pos += 1;
    }
    t
}

async fn capture(
    targets: &[(usize, u32)],
    prev_hashes: &mut HashMap<u32, u64>,
    delegate: &Entity<WindowDelegate>,
    this: &WeakEntity<AltTabApp>,
    cx: &AsyncApp,
    executor: &gpui::BackgroundExecutor,
) {
    let target_map: Vec<(usize, u32)> = targets.to_vec();
    let targets_owned = targets.to_vec();
    let captured = executor
        .spawn(async move {
            platform::capture_previews_cg(&targets_owned, PREVIEW_MAX_WIDTH, PREVIEW_MAX_HEIGHT)
        })
        .await;

    let mut updates = Vec::new();
    for (idx, rgba_opt) in captured {
        let Some(rgba) = rgba_opt else { continue };
        let Some(&(_, wid)) = target_map.iter().find(|(i, _)| *i == idx) else {
            continue;
        };
        let hash = fast_pixel_hash(&rgba.data);
        if prev_hashes.get(&wid) == Some(&hash) {
            continue;
        }
        prev_hashes.insert(wid, hash);
        if let Some(render_img) = bgra_to_render_image(rgba.data, rgba.width, rgba.height) {
            updates.push((wid, render_img));
        }
    }
    if updates.is_empty() {
        return;
    }
    let delegate = delegate.clone();
    let this = this.clone();
    let _ = cx.update(|app_cx| {
        let _ = delegate.update(app_cx, |state, _cx| {
            for (wid, img) in updates {
                state.live_previews.insert(wid, img);
            }
        });
        let _ = this.update(
            app_cx,
            |_, cx: &mut gpui::Context<AltTabApp>| { cx.notify(); },
        );
    });
}
