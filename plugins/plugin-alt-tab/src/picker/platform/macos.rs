use crate::picker::create::PICKER_WINDOW_TITLE;
use objc2::rc::Retained;
use objc2_app_kit::{NSWindow, NSWindowAnimationBehavior};
use objc2_foundation::MainThreadMarker;

/// Offscreen origin used while the pre-created picker is hidden. Placed far enough from any
/// real monitor to guarantee no stray pixels leak onto the desktop during the first frame.
pub const OFFSCREEN_X: f64 = -32000.0;
pub const OFFSCREEN_Y: f64 = -32000.0;

/// Hide the keep-alive picker without destroying its NSWindow. Used on first boot (after
/// pre-create) and on every subsequent dismiss.
pub fn hide_picker_offscreen() {
    with_picker_window(|win| {
        win.setAlphaValue(0.0);
        win.orderOut(None);
    });
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

/// Hide the keep-alive picker: set alpha to 0 and orderOut. The window remains allocated so
/// the next open can reposition and fade it back in without paying cold-start Metal costs.
pub fn dismiss_picker(_window: &mut gpui::Window) {
    hide_picker_offscreen();
}

pub fn reposition_picker_window(gpui_x: f64, gpui_y: f64) -> bool {
    use objc2_app_kit::NSScreen;
    use objc2_foundation::NSPoint;
    let Some(mtm) = MainThreadMarker::new() else {
        return false;
    };

    // GPUI uses Y-down; Cocoa uses Y-up relative to primary screen's bottom-left.
    // setFrameTopLeftPoint: expects the top-left corner in Cocoa screen coordinates.
    // Conversion: ns_y = primary_screen_height - gpui_y.
    let primary_h = NSScreen::mainScreen(mtm)
        .map(|s| s.frame().size.height)
        .unwrap_or(1080.0);
    let ns_point = NSPoint::new(gpui_x, primary_h - gpui_y);

    let Some(window) = find_picker_window(mtm) else {
        return false;
    };
    window.setFrameTopLeftPoint(ns_point);
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

pub fn disable_window_shadow() {
    use objc2_app_kit::NSColor;
    let Some(mtm) = MainThreadMarker::new() else {
        return;
    };
    let clear = NSColor::clearColor();
    let Some(window) = find_picker_window(mtm) else {
        return;
    };
    window.setHasShadow(false);
    window.setBackgroundColor(Some(&clear));
}

pub fn show_picker_onscreen() {
    #[cfg(debug_assertions)]
    let t = std::time::Instant::now();
    with_picker_window(|win| {
        win.makeKeyAndOrderFront(None);
        win.setAlphaValue(1.0);
    });
    #[cfg(debug_assertions)]
    eprintln!(
        "[alt-tab/show_onscreen] orderFront+alpha=1 took {}us",
        t.elapsed().as_micros()
    );
}

/// Force alpha=0 before reposition/activate so fresh content can lay out without a flash
/// and any in-flight ModifiersChangedEvent arrives at the already-present picker window.
pub fn prepare_picker_for_show() {
    with_picker_window(|win| win.setAlphaValue(0.0));
}

fn with_picker_window(body: impl FnOnce(&NSWindow)) -> bool {
    let Some(mtm) = MainThreadMarker::new() else {
        return false;
    };
    let Some(window) = find_picker_window(mtm) else {
        return false;
    };
    body(&window);
    true
}

fn find_picker_window(mtm: MainThreadMarker) -> Option<Retained<NSWindow>> {
    use objc2_app_kit::NSApplication;
    let app = NSApplication::sharedApplication(mtm);
    app.windows()
        .iter()
        .find(|win| win.title().to_string() == PICKER_WINDOW_TITLE)
}

pub fn pre_create_if_supported(
    config: &crate::config::AltTabConfig,
    current: &crate::PickerWindowState,
    cx: &mut gpui::App,
) {
    crate::picker::create::pre_create_offscreen(config, current, cx);
    with_picker_window(|win| {
        win.setAnimationBehavior(NSWindowAnimationBehavior::None);
    });
}

pub fn offscreen_origin() -> (f64, f64) {
    (OFFSCREEN_X, OFFSCREEN_Y)
}

/// No-op: the keep-alive NSWindow spans opens and multi-monitor is handled by repositioning.
pub fn destroy_non_target_windows(
    _current: &crate::PickerWindowState,
    _target: qol_plugin_api::window::MonitorKey,
    _cx: &mut gpui::App,
) {
}

/// Drop the stale `ActiveWindows` slot so a subsequent `create_from_request` fallback doesn't
/// leave a dangling sentinel key. The keep-alive NSWindow is never destroyed.
pub fn discard_old_window(
    current: &crate::PickerWindowState,
    target: qol_plugin_api::window::MonitorKey,
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
