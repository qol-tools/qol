use super::GatheredWindows;
use crate::app::AltTabApp;
use crate::config::AltTabConfig;
use crate::shared::layout::*;
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
    pub config: &'a AltTabConfig,
    pub window_count: usize,
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
            let backing =
                qol_gpui::popup_window::window_backing_scale(super::create::PICKER_WINDOW_TITLE);
            qol_gpui::window::resize_or_sync_scale(window, req.layout.size, backing);
            window.focus(&view.focus_handle(cx));
            window.activate_window();
            super::platform::show_picker();
            true
        })
        .unwrap_or(false)
}

pub(super) fn compute_layout(input: &LayoutInput, cx: &mut App) -> ReuseLayout {
    let size = picker_size(input);
    let bounds = input.placement.centered_bounds(size, cx);
    ReuseLayout { bounds, size }
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
