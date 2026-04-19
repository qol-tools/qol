use super::GatheredWindows;
use crate::app::{AltTabApp, PICKER_VISIBLE};
use crate::config::AltTabConfig;
use crate::shared::layout::*;
use gpui::*;
use qol_plugin_api::window::PopupPlacement;
use std::sync::atomic::Ordering;

pub(crate) struct ReuseLayout {
    pub bounds: Bounds<Pixels>,
    pub size: Size<Pixels>,
    pub monitor_changed: bool,
}

pub(crate) struct ReuseRequest<'a> {
    pub handle: &'a WindowHandle<AltTabApp>,
    pub layout: &'a ReuseLayout,
    pub config: &'a AltTabConfig,
    pub gathered: &'a GatheredWindows,
}

pub(super) struct LayoutInput<'a> {
    pub config: &'a AltTabConfig,
    pub window_count: usize,
    pub placement: &'a PopupPlacement,
    pub created_on_origin: Point<Pixels>,
}

pub(super) fn try_reuse(req: &ReuseRequest, cx: &mut App) -> bool {
    req.handle
        .update(cx, |view, window: &mut Window, cx| {
            // WindowHandle can outlive the platform window's destruction; don't resurrect a dismissed picker.
            if !PICKER_VISIBLE.load(Ordering::Relaxed) {
                return false;
            }
            if !view.apply_reuse(req, window, cx) {
                return false;
            }
            resize_if_needed(window, req.layout.size);
            window.focus(&view.focus_handle(cx));
            window.activate_window();
            true
        })
        .unwrap_or(false)
}

pub(super) fn compute_layout(input: &LayoutInput, cx: &mut App) -> ReuseLayout {
    let size = picker_size(input);
    let bounds = input.placement.centered_bounds(size, cx);
    let monitor_changed = origin_diverged(input.created_on_origin, input.placement.origin());
    ReuseLayout {
        bounds,
        size,
        monitor_changed,
    }
}

fn picker_size(input: &LayoutInput) -> Size<Pixels> {
    let count = input.window_count.max(1);
    let (w, h) = picker_dimensions(
        count,
        input.config.display.max_columns,
        input.placement.monitor_size(),
        input.config.display.show_hotkey_hints,
    );
    size(px(w), px(h))
}

fn origin_diverged(a: Point<Pixels>, b: Point<Pixels>) -> bool {
    const TOLERANCE_PX: f64 = 6.0;
    let dx = (a.x.to_f64() - b.x.to_f64()).abs();
    let dy = (a.y.to_f64() - b.y.to_f64()).abs();
    dx > TOLERANCE_PX || dy > TOLERANCE_PX
}

fn resize_if_needed(window: &mut Window, target: Size<Pixels>) {
    let current = window.window_bounds().get_bounds().size;
    let dw = (current.width.to_f64() - target.width.to_f64()).abs();
    let dh = (current.height.to_f64() - target.height.to_f64()).abs();
    if dw < 1.0 && dh < 1.0 {
        return;
    }
    #[cfg(debug_assertions)]
    eprintln!(
        "[alt-tab/reuse] resize {}x{} → {}x{}",
        current.width, current.height, target.width, target.height
    );
    window.resize(target);
}
