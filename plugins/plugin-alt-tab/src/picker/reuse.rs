use crate::app::AltTabApp;
use crate::config::AltTabConfig;
use crate::shared::layout::*;
use super::GatheredWindows;
use gpui::*;
use qol_plugin_api::monitor::{ActiveMonitor, MonitorTracker};

pub(crate) struct ReuseLayout {
    pub bounds: Bounds<Pixels>,
    pub size: Size<Pixels>,
    pub origin: Point<Pixels>,
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
    pub tracker: &'a MonitorTracker,
    pub created_on_origin: Point<Pixels>,
}

pub(super) fn try_reuse(req: &ReuseRequest, cx: &mut App) -> bool {
    req.handle.update(cx, |view, window: &mut Window, cx| {
        if !view.apply_reuse(req, window, cx) {
            return false;
        }
        resize_if_needed(window, req.layout.size);
        window.focus(&view.focus_handle(cx));
        window.activate_window();
        true
    }).unwrap_or(false)
}

pub(super) fn compute_layout(input: &LayoutInput, cx: &mut App) -> ReuseLayout {
    let monitor = input.tracker.snapshot().map(|(m, _)| m);
    let size = picker_size(input, &monitor);
    let bounds = centered_bounds(&monitor, size, cx);
    let origin = monitor_origin(&monitor);
    let monitor_changed = origin_diverged(input.created_on_origin, origin);
    ReuseLayout { bounds, size, origin, monitor_changed }
}

fn picker_size(input: &LayoutInput, monitor: &Option<ActiveMonitor>) -> Size<Pixels> {
    let count = input.window_count.max(1);
    let monitor_size = monitor.as_ref().map(|m| m.size());
    let (w, h) = picker_dimensions(count, input.config.display.max_columns, monitor_size, input.config.display.show_hotkey_hints);
    size(px(w), px(h))
}

pub(super) fn centered_bounds(monitor: &Option<ActiveMonitor>, win_size: Size<Pixels>, cx: &mut App) -> Bounds<Pixels> {
    match monitor.as_ref() {
        Some(active) => active.centered_bounds(win_size),
        None => Bounds::centered(None, win_size, cx),
    }
}

pub(super) fn monitor_origin(monitor: &Option<ActiveMonitor>) -> Point<Pixels> {
    monitor.as_ref().map(|m| m.bounds().origin).unwrap_or(point(px(0.0), px(0.0)))
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
    eprintln!("[alt-tab/reuse] resize {}x{} → {}x{}", current.width, current.height, target.width, target.height);
    window.resize(target);
}
