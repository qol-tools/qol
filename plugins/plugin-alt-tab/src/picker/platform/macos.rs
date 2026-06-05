use crate::picker::create::PICKER_WINDOW_TITLE;
use objc2_foundation::MainThreadMarker;

pub fn hide_picker(title: &str) {
    qol_gpui::popup_window::hide_window_by_title(title);
}

pub fn set_accessory_policy() {
    use objc2_app_kit::{NSApplication, NSApplicationActivationPolicy};
    let mtm = MainThreadMarker::new().expect("must be on main thread");
    let app = NSApplication::sharedApplication(mtm);
    app.setActivationPolicy(NSApplicationActivationPolicy::Accessory);
}

pub fn picker_window_title(_target: qol_gpui::window::MonitorKey) -> String {
    PICKER_WINDOW_TITLE.to_string()
}

pub fn configure_picker_window(title: &str) {
    qol_gpui::popup_window::configure_popup_window(title);
}

pub fn picker_window_kind() -> gpui::WindowKind {
    gpui::WindowKind::Normal
}

pub fn picker_window_decorations(transparent: bool) -> gpui::WindowDecorations {
    if transparent {
        gpui::WindowDecorations::Server
    } else {
        gpui::WindowDecorations::Client
    }
}

pub fn reposition_picker_window(title: &str, gpui_x: f64, gpui_y: f64) -> bool {
    qol_gpui::popup_window::reposition_window_by_title(title, gpui_x, gpui_y)
}

pub fn sync_picker_window_layout(
    title: &str,
    window: &mut gpui::Window,
    origin: gpui::Point<gpui::Pixels>,
    size: gpui::Size<gpui::Pixels>,
) -> bool {
    let backing = qol_gpui::popup_window::window_backing_scale(title);
    qol_gpui::window::resize_or_sync_scale(window, size, backing);
    reposition_picker_window(title, origin.x.to_f64(), origin.y.to_f64())
}

pub fn adjust_picker_bounds(bounds: gpui::Bounds<gpui::Pixels>) -> gpui::Bounds<gpui::Pixels> {
    bounds
}

pub fn reuse_hidden_picker_across_shows() -> bool {
    true
}

pub fn reuse_picker_across_targets() -> bool {
    true
}

fn cg_event_flags() -> u64 {
    const K_CG_EVENT_SOURCE_STATE_COMBINED: i32 = 0;

    #[link(name = "CoreGraphics", kind = "framework")]
    extern "C" {
        fn CGEventSourceFlagsState(state_id: i32) -> u64;
    }

    unsafe { CGEventSourceFlagsState(K_CG_EVENT_SOURCE_STATE_COMBINED) }
}

pub fn is_modifier_held() -> bool {
    const K_CG_EVENT_FLAG_MASK_ALTERNATE: u64 = 0x0008_0000;
    cg_event_flags() & K_CG_EVENT_FLAG_MASK_ALTERNATE != 0
}

pub fn is_shift_held() -> bool {
    const K_CG_EVENT_FLAG_MASK_SHIFT: u64 = 0x0002_0000;
    cg_event_flags() & K_CG_EVENT_FLAG_MASK_SHIFT != 0
}

pub fn disable_window_shadow(title: &str) {
    qol_gpui::popup_window::disable_window_shadow(title);
}

pub fn show_picker(title: &str) {
    #[cfg(debug_assertions)]
    let t = std::time::Instant::now();
    qol_gpui::popup_window::show_window_by_title(title);
    #[cfg(debug_assertions)]
    eprintln!(
        "[alt-tab/show] ignores=false+orderFront+alpha=1 took {}us",
        t.elapsed().as_micros()
    );
}

pub fn pre_create(
    config: &crate::config::AltTabConfig,
    current: &crate::PickerWindowState,
    tracker: &qol_gpui::monitor::MonitorTracker,
    cx: &mut gpui::App,
) {
    qol_gpui::popup_window::set_ghost_debug(
        config.display.ghost_opacity,
        config.display.ghost_debug_color.as_deref(),
    );
    let placement = qol_gpui::window::PopupPlacement::from_tracker(tracker);
    crate::picker::create::pre_create_ghost(config, current, &placement, cx);
}

pub fn destroy_non_target_windows(
    _current: &crate::PickerWindowState,
    _target: qol_gpui::window::MonitorKey,
    _cx: &mut gpui::App,
) {
}

/// Drop the stale `ActiveWindows` slot so a subsequent `create_from_request` fallback doesn't
/// leave a dangling sentinel key. The keep-alive NSWindow is never destroyed.
pub fn discard_old_window(
    current: &crate::PickerWindowState,
    target: qol_gpui::window::MonitorKey,
    _handle: gpui::WindowHandle<crate::app::AltTabApp>,
    _cx: &mut gpui::App,
) {
    #[cfg(debug_assertions)]
    eprintln!(
        "[alt-tab/open] keep-alive reuse failed; dropping stale slot {:?}",
        target
    );
    current.borrow_mut().remove(target);
}
