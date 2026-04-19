pub fn set_accessory_policy() {
    use objc2_app_kit::{NSApplication, NSApplicationActivationPolicy};
    use objc2_foundation::MainThreadMarker;
    let mtm = MainThreadMarker::new().expect("must be on main thread");
    let app = NSApplication::sharedApplication(mtm);
    app.setActivationPolicy(NSApplicationActivationPolicy::Accessory);
}

pub fn picker_window_kind() -> gpui::WindowKind {
    gpui::WindowKind::Normal
}

pub fn dismiss_picker(window: &mut gpui::Window) {
    // Destroy via GPUI first to invalidate the WindowHandle synchronously.
    window.remove_window();
    // Belt-and-suspenders for stale pickers from multi-monitor opens.
    use objc2_app_kit::NSApplication;
    use objc2_foundation::MainThreadMarker;
    let mtm = MainThreadMarker::new().expect("must be on main thread");
    let app = NSApplication::sharedApplication(mtm);
    for win in app.windows().iter() {
        if win.title().to_string() == "qol-alt-tab-picker" {
            win.orderOut(None);
        }
    }
}

/// Move the picker window so its top-left sits at (gpui_x, gpui_y) in GPUI global
/// coordinates (Y-down, origin = top-left of primary screen). Returns true on success.
fn reposition_picker(gpui_x: f64, gpui_y: f64) -> bool {
    use objc2_app_kit::{NSApplication, NSScreen};
    use objc2_foundation::{MainThreadMarker, NSPoint};
    let mtm = MainThreadMarker::new().expect("must be on main thread");

    // GPUI uses Y-down; Cocoa uses Y-up relative to primary screen's bottom-left.
    // setFrameTopLeftPoint: expects the top-left corner in Cocoa screen coordinates.
    // Conversion: ns_y = primary_screen_height - gpui_y.
    let primary_h = NSScreen::mainScreen(mtm)
        .map(|s| s.frame().size.height)
        .unwrap_or(1080.0);
    let ns_point = NSPoint::new(gpui_x, primary_h - gpui_y);

    let app = NSApplication::sharedApplication(mtm);
    for win in app.windows().iter() {
        if win.title().to_string() == "qol-alt-tab-picker" {
            win.setFrameTopLeftPoint(ns_point);
            return true;
        }
    }
    false
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

pub fn reposition_picker_window(x: f64, y: f64) -> bool {
    reposition_picker(x, y)
}

pub fn disable_window_shadow() {
    use objc2_app_kit::{NSApplication, NSColor};
    use objc2_foundation::MainThreadMarker;

    let mtm = MainThreadMarker::new().expect("must be on main thread");
    let app = NSApplication::sharedApplication(mtm);
    let clear = NSColor::clearColor();
    for window in app.windows().iter() {
        window.setHasShadow(false);
        window.setBackgroundColor(Some(&clear));
    }
}
