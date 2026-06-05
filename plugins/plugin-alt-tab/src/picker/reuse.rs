use super::GatheredWindows;
use crate::app::AltTabApp;
use crate::config::AltTabConfig;
use gpui::*;
use qol_gpui::window::{MonitorKey, PopupPlacement};

pub(crate) struct ReuseLayout {
    pub target: MonitorKey,
    pub bounds: Bounds<Pixels>,
    pub size: Size<Pixels>,
}

pub(crate) struct ReuseRequest<'a> {
    pub handle: &'a WindowHandle<AltTabApp>,
    pub layout: &'a ReuseLayout,
    pub config: &'a AltTabConfig,
    pub gathered: &'a GatheredWindows,
    pub reverse: bool,
}

pub(super) struct LayoutInput<'a> {
    pub placement: &'a PopupPlacement,
}

pub(super) fn try_reuse(req: &ReuseRequest, cx: &mut App) -> bool {
    // The picker window is pre-created at boot and kept alive across dismisses, so the
    // normal path updates an existing view instead of paying GPUI window creation cost.
    // Stale handles can still happen after platform failures; those fall through to create.
    req.handle
        .update(cx, |view, window: &mut Window, cx| {
            if !view.apply_reuse(req, window, cx) {
                return false;
            }
            let title = view.picker_title.clone();
            if !super::platform::sync_picker_window_layout(
                &title,
                window,
                req.layout.bounds.origin,
                req.layout.size,
            ) {
                return false;
            }
            window.focus(&view.focus_handle(cx));
            window.activate_window();
            super::platform::show_picker(&title);
            true
        })
        .unwrap_or(false)
}

pub(super) fn compute_layout(input: &LayoutInput, _cx: &mut App) -> ReuseLayout {
    // The picker covers the active monitor so it can absorb clicks outside the
    // centered card box. Platform code may shave a pixel from the requested
    // bounds when a window manager treats exact monitor-sized windows specially.
    let (mw, mh) = input.placement.monitor_size().unwrap_or((1920.0, 1080.0));
    let win_size = size(px(mw), px(mh));

    let target = input.placement.target();
    let origin = input.placement.origin();
    let bounds = super::platform::adjust_picker_bounds(Bounds {
        origin,
        size: win_size,
    });
    ReuseLayout {
        target,
        bounds,
        size: bounds.size,
    }
}
