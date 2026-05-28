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

pub(super) fn compute_layout(input: &LayoutInput, _cx: &mut App) -> ReuseLayout {
    // The picker covers the entire active monitor so it can absorb every click that lands
    // on that monitor (clicks outside the centered card box dismiss the picker, clicks on
    // a card activate that card). Click-through to native gestures like Option+Click on the
    // Dock (which would "hide others") is the bug this prevents - we cannot react to those
    // gestures after they fire, so we must capture the click first.
    let (mw, mh) = input.placement.monitor_size().unwrap_or((1920.0, 1080.0));
    let win_size = size(px(mw), px(mh));
    let origin = input.placement.origin();
    let bounds = Bounds {
        origin,
        size: win_size,
    };
    ReuseLayout {
        bounds,
        size: win_size,
    }
}
