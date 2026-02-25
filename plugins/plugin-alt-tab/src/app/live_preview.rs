use super::PICKER_VISIBLE;
use crate::delegate::WindowDelegate;
use crate::layout::{PREVIEW_MAX_HEIGHT, PREVIEW_MAX_WIDTH};
use crate::platform;
use crate::preview::{bgra_to_render_image, fast_pixel_hash};
use gpui::{AsyncApp, Entity, Task, WeakEntity};
use std::collections::HashMap;
use std::sync::atomic::Ordering;
use std::time::Duration;

const LIVE_PREVIEW_INTERVAL_MS: u64 = 500;

pub(crate) fn spawn(
    delegate: Entity<WindowDelegate>,
    cx: &mut gpui::Context<super::AltTabApp>,
) -> Task<()> {
    cx.spawn(
        move |this: WeakEntity<super::AltTabApp>, cx: &mut AsyncApp| {
            let cx = cx.clone();
            async move {
                let executor = cx.background_executor().clone();
                let mut prev_hashes: HashMap<u32, u64> = HashMap::new();
                let mut first_visible = true;
                loop {
                    if first_visible && PICKER_VISIBLE.load(Ordering::Relaxed) {
                        first_visible = false;
                    } else {
                        executor
                            .timer(Duration::from_millis(LIVE_PREVIEW_INTERVAL_MS))
                            .await;
                    }
                    if !PICKER_VISIBLE.load(Ordering::Relaxed) {
                        prev_hashes.clear();
                        first_visible = true;
                        continue;
                    }
                    let window_ids: Vec<(usize, u32)> = cx
                        .update(|app_cx| {
                            delegate
                                .read(app_cx)
                                .windows
                                .iter()
                                .enumerate()
                                .map(|(i, w)| (i, w.id))
                                .collect()
                        })
                        .unwrap_or_default();
                    if window_ids.is_empty() {
                        continue;
                    }
                    let id_map: Vec<(usize, u32)> = window_ids.clone();
                    let captured = executor
                        .spawn(async move {
                            platform::capture_previews_cg(
                                &window_ids,
                                PREVIEW_MAX_WIDTH,
                                PREVIEW_MAX_HEIGHT,
                            )
                        })
                        .await;
                    let mut changed = false;
                    let list = delegate.clone();
                    for (idx, rgba_opt) in captured {
                        let Some(rgba) = rgba_opt else { continue };
                        let Some(&(_, wid)) = id_map.iter().find(|(i, _)| *i == idx) else {
                            continue;
                        };
                        let hash = fast_pixel_hash(&rgba.data);
                        if prev_hashes.get(&wid) == Some(&hash) {
                            continue;
                        }
                        prev_hashes.insert(wid, hash);
                        if let Some(render_img) =
                            bgra_to_render_image(&rgba.data, rgba.width, rgba.height)
                        {
                            let _ = cx.update(|app_cx| {
                                let _ = list.update(app_cx, |state, cx| {
                                    state.live_previews.insert(wid, render_img);
                                    cx.notify();
                                });
                            });
                            changed = true;
                        }
                    }
                    if changed {
                        let _ = cx.update(|app_cx| {
                            let _ = this.update(
                                app_cx,
                                |_, cx: &mut gpui::Context<super::AltTabApp>| {
                                    cx.notify();
                                },
                            );
                        });
                    }
                }
            }
        },
    )
}
