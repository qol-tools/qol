use crate::app::{AltTabApp, PICKER_VISIBLE};
use crate::capture;
use crate::picker::run::SharedPreviewCache;
use crate::picker::state::PickerState;
use crate::shared::layout::{PREVIEW_MAX_HEIGHT, PREVIEW_MAX_WIDTH};
use crate::shared::preview::{bgra_to_render_image, fast_pixel_hash};
use crate::PreviewMap;
use gpui::{AsyncApp, Entity, RenderImage, Task, WeakEntity};
use std::sync::atomic::Ordering;
use std::sync::Arc;
use std::time::{Duration, Instant};

const TICK_MS: u64 = 200;
const CAPTURE_INTERVAL: Duration = Duration::from_millis(500);
const SELECTION_SETTLE: Duration = Duration::from_millis(300);

pub(crate) fn spawn(
    delegate: Entity<PickerState>,
    preview_cache: SharedPreviewCache,
    cx: &mut gpui::Context<AltTabApp>,
) -> Task<()> {
    cx.spawn(move |this: WeakEntity<AltTabApp>, cx: &mut AsyncApp| {
        let cx = cx.clone();
        async move { preview_loop(delegate, preview_cache, this, cx).await }
    })
}

async fn preview_loop(
    delegate: Entity<PickerState>,
    preview_cache: SharedPreviewCache,
    this: WeakEntity<AltTabApp>,
    cx: AsyncApp,
) {
    let executor = cx.background_executor().clone();
    let mut prev_hash: Option<(u32, u64)> = None;
    let mut last_captured = Instant::now() - CAPTURE_INTERVAL;
    let mut last_selection: Option<u32> = None;
    let mut selection_changed_at = Instant::now();

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
        if last_selection != Some(selected.1) {
            last_selection = Some(selected.1);
            selection_changed_at = now;
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
            .is_some_and(|&(id, h)| id == selected.1 && h == hash)
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
        prev_hash = Some((selected.1, hash));
        let Some(img) = bgra_to_render_image(rgba.data, rgba.width, rgba.height) else {
            continue;
        };
        #[cfg(debug_assertions)]
        let render_ms = t_render.elapsed().as_millis();
        #[cfg(debug_assertions)]
        let wid = selected.1;
        let update_result = push_updates(
            preview_update(selected.1, img),
            &preview_cache,
            &delegate,
            &this,
            &cx,
        );
        #[cfg(not(debug_assertions))]
        let _ = update_result;
        #[cfg(debug_assertions)]
        {
            if update_result.cache_write {
                crate::shared::preview_trace::record_shared_live(wid);
            }
            if update_result.state_update {
                crate::shared::preview_trace::record_live_update(wid);
            }
            probe_live_preview(wid, true, capture_ms, render_ms, stable_ms, update_result);
        }
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
) -> Vec<(usize, Option<qol_app_icon::RgbaImage>)> {
    let owned = targets.to_vec();
    executor
        .spawn(async move {
            capture::capture_previews_cg(&owned, PREVIEW_MAX_WIDTH, PREVIEW_MAX_HEIGHT)
        })
        .await
}

#[derive(Clone, Copy, Default)]
#[cfg_attr(not(debug_assertions), allow(dead_code))]
struct PreviewUpdateResult {
    state_update: bool,
    cache_write: bool,
}

fn preview_update(wid: u32, img: Arc<RenderImage>) -> PreviewMap {
    [(wid, img)].into_iter().collect()
}

fn push_updates(
    updates: PreviewMap,
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
            crate::shared::image_registry::extend_with(&mut *cache, shared_updates, app_cx, None);
            cache_write = true;
        }
        // App-level: no Window leased here, so insert_preview's release path
        // passes None. The picker window stays in App::windows and gets
        // touched via the iteration in App::drop_image.
        delegate.update(app_cx, |state, ctx| {
            state.insert_previews(updates, ctx, None);
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
