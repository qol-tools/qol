use super::GatheredWindows;
use crate::app::{AltTabApp, PICKER_VISIBLE};
use crate::config::AltTabConfig;
use crate::rendering::RenderingFlow;
use gpui::*;
use qol_gpui::window::PopupPlacement;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;

pub(crate) struct ReuseLayout {
    pub bounds: Bounds<Pixels>,
    pub size: Size<Pixels>,
}

pub(crate) struct ReuseRequest<'a> {
    pub handle: &'a WindowHandle<AltTabApp>,
    pub layout: &'a ReuseLayout,
    pub config: &'a AltTabConfig,
    pub gathered: &'a GatheredWindows,
    pub all_titles: &'a [String],
    pub reverse: bool,
    pub monitor_size: Option<(f32, f32)>,
    pub show_id: u64,
    pub rendering: RenderingFlow,
}

pub(super) struct LayoutInput<'a> {
    pub placement: &'a PopupPlacement,
}

pub(super) fn try_reuse(req: &ReuseRequest, cx: &mut App) -> bool {
    let reason = format!("show#{}", req.show_id);
    let _reason = qol_gpui::popup_window::reason_scope(reason);
    #[cfg(debug_assertions)]
    let t_show = Some(std::time::Instant::now());
    #[cfg(not(debug_assertions))]
    let t_show = None::<std::time::Instant>;
    req.handle
        .update(cx, |view, window: &mut Window, cx| {
            let was_visible = PICKER_VISIBLE.swap(true, Ordering::Relaxed);
            qol_runtime::probe!(
                "REUSE_BEGIN",
                "show_id={} title={} origin=({},{}) size={}x{}",
                req.show_id,
                view.picker_title,
                req.layout.bounds.origin.x.to_f64(),
                req.layout.bounds.origin.y.to_f64(),
                req.layout.size.width.to_f64(),
                req.layout.size.height.to_f64(),
            );
            if !view.apply_reuse(req, window, cx) {
                PICKER_VISIBLE.store(was_visible, Ordering::Relaxed);
                qol_runtime::probe!("REUSE_ABORT", "show_id={} phase=apply_reuse", req.show_id);
                return false;
            }
            let title = view.picker_title.clone();
            if !super::platform::sync_picker_window_layout(
                &title,
                window,
                req.layout.bounds.origin,
                req.layout.size,
            ) {
                PICKER_VISIBLE.store(was_visible, Ordering::Relaxed);
                qol_runtime::probe!("REUSE_ABORT", "show_id={} phase=layout_sync", req.show_id);
                return false;
            }
            qol_runtime::probe!(
                "REUSE_LAYOUT_SYNC",
                "show_id={} title={} result=ok",
                req.show_id,
                title
            );
            view.focus_for_keys("reuse-before-show", Some(req.show_id), window);
            qol_runtime::probe!(
                "REUSE_FOCUS_ACTIVATE",
                "show_id={} title={}",
                req.show_id,
                title
            );
            view.sync_preview_plane(Some(req.show_id), window, cx);
            super::platform::show_picker_window(&title, req.all_titles);
            qol_runtime::probe!(
                "REUSE_SHOW_WINDOW",
                "show_id={} title={}",
                req.show_id,
                title
            );
            view.focus_for_keys("reuse-after-show", Some(req.show_id), window);
            let painted = Arc::new(AtomicBool::new(false));
            let painted_for_frame = painted.clone();
            #[cfg(debug_assertions)]
            let show_id = req.show_id;
            #[cfg(not(debug_assertions))]
            let show_id = 0;
            window.on_next_frame(move |_, _| {
                painted_for_frame.store(true, Ordering::Release);
                qol_runtime::probe!(
                    "SHOW_PAINTED",
                    "show_id={show_id} capture_lane=none reveal_frame=painted capture_duration_ms=none first_paint_latency_ms={}ms",
                    t_show
                        .as_ref()
                        .map(|started| started.elapsed().as_millis())
                        .unwrap_or(0)
                );
            });
            cx.spawn(move |_, cx: &mut AsyncApp| {
                let cx = cx.clone();
                async move {
                    cx.background_executor()
                        .timer(Duration::from_millis(120))
                        .await;
                    if !painted.load(Ordering::Acquire) {
                        qol_runtime::probe!("SHOW_PAINT_TIMEOUT", "show_id={show_id} after=120ms");
                    }
                }
            })
            .detach();
            true
        })
        .unwrap_or(false)
}

pub(super) fn compute_layout(input: &LayoutInput, _cx: &mut App) -> ReuseLayout {
    let (mw, mh) = input.placement.monitor_size().unwrap_or((1920.0, 1080.0));
    let win_size = size(px(mw), px(mh));

    let origin = input.placement.origin();
    let bounds = super::platform::adjust_picker_bounds(Bounds {
        origin,
        size: win_size,
    });
    ReuseLayout {
        bounds,
        size: bounds.size,
    }
}
