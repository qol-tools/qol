use crate::picker::create::PICKER_WINDOW_TITLE;
use objc2_foundation::MainThreadMarker;

pub fn hide_picker() {
    qol_gpui::popup_window::hide_window_by_title(PICKER_WINDOW_TITLE);
}

pub fn set_accessory_policy() {
    use objc2_app_kit::{NSApplication, NSApplicationActivationPolicy};
    let mtm = MainThreadMarker::new().expect("must be on main thread");
    let app = NSApplication::sharedApplication(mtm);
    app.setActivationPolicy(NSApplicationActivationPolicy::Accessory);
}

pub fn picker_window_kind() -> gpui::WindowKind {
    gpui::WindowKind::Normal
}

pub fn dismiss_picker(_window: &mut gpui::Window) {
    hide_picker();
}

pub fn reposition_picker_window(gpui_x: f64, gpui_y: f64) -> bool {
    qol_gpui::popup_window::reposition_window_by_title(PICKER_WINDOW_TITLE, gpui_x, gpui_y)
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

pub fn disable_window_shadow() {
    qol_gpui::popup_window::disable_window_shadow(PICKER_WINDOW_TITLE);
}

pub fn show_picker() {
    #[cfg(debug_assertions)]
    let t = std::time::Instant::now();
    qol_gpui::popup_window::show_window_by_title(PICKER_WINDOW_TITLE);
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
    qol_gpui::popup_window::configure_popup_window(PICKER_WINDOW_TITLE);
    qol_gpui::popup_window::disable_window_shadow(PICKER_WINDOW_TITLE);
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
