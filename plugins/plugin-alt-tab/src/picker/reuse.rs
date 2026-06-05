use super::GatheredWindows;
use crate::app::AltTabApp;
use crate::config::AltTabConfig;
use gpui::*;
use qol_gpui::window::PopupPlacement;

pub(crate) struct ReuseLayout {
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
            qol_gpui::popup_window::dump_ghost_windows(&format!("alt-tab-show title={title}"));
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
