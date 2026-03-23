use crate::app::{AltTabApp, PICKER_VISIBLE};
use crate::capture;
use crate::picker::state::PickerState;
use crate::shared::layout::{PREVIEW_MAX_HEIGHT, PREVIEW_MAX_WIDTH};
use crate::shared::preview::{bgra_to_render_image, fast_pixel_hash};
use gpui::{AsyncApp, Entity, RenderImage, Task, WeakEntity};
use std::sync::atomic::Ordering;
use std::sync::Arc;
use std::time::{Duration, Instant};

const TICK_MS: u64 = 200;
const CAPTURE_INTERVAL: Duration = Duration::from_millis(500);

pub(crate) fn spawn(delegate: Entity<PickerState>, cx: &mut gpui::Context<AltTabApp>) -> Task<()> {
    cx.spawn(move |this: WeakEntity<AltTabApp>, cx: &mut AsyncApp| {
        let cx = cx.clone();
        async move { preview_loop(delegate, this, cx).await }
    })
}

async fn preview_loop(delegate: Entity<PickerState>, this: WeakEntity<AltTabApp>, cx: AsyncApp) {
    let executor = cx.background_executor().clone();
    let mut prev_hash: Option<(u32, u64)> = None;
    let mut last_captured = Instant::now() - CAPTURE_INTERVAL;

    while PICKER_VISIBLE.load(Ordering::Relaxed) {
        executor.timer(Duration::from_millis(TICK_MS)).await;
        if !PICKER_VISIBLE.load(Ordering::Relaxed) {
            break;
        }
        let snap = read_snapshot(&delegate, &cx);
        let Some(selected) = snap.selected_target() else {
            continue;
        };
        let now = Instant::now();
        if now.duration_since(last_captured) < CAPTURE_INTERVAL {
            continue;
        }
        last_captured = now;
        let target = vec![selected];
        let captured = run_capture(&target, &executor).await;
        let Some((_, Some(rgba))) = captured.into_iter().next() else {
            continue;
        };
        let hash = fast_pixel_hash(&rgba.data);
        if prev_hash
            .as_ref()
            .is_some_and(|&(id, h)| id == selected.1 && h == hash)
        {
            continue;
        }
        prev_hash = Some((selected.1, hash));
        let Some(img) = bgra_to_render_image(rgba.data, rgba.width, rgba.height) else {
            continue;
        };
        push_updates(vec![(selected.1, img)], &delegate, &this, &cx);
    }

    let _ = cx.update(|app_cx| {
        let Some(entity) = this.upgrade() else {
            return;
        };
        entity.update(app_cx, |app, _| {
            app._live_preview_task = None;
        });
    });
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
) -> Vec<(usize, Option<qol_plugin_api::app_icon::RgbaImage>)> {
    let owned = targets.to_vec();
    executor
        .spawn(async move {
            capture::capture_previews_cg(&owned, PREVIEW_MAX_WIDTH, PREVIEW_MAX_HEIGHT)
        })
        .await
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
