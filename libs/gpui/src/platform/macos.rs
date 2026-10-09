use objc2::rc::Retained;
use objc2_app_kit::NSView;
use raw_window_handle::{HasWindowHandle, RawWindowHandle};

pub fn set_accessory_policy() {
    use objc2_app_kit::{NSApplication, NSApplicationActivationPolicy};
    use objc2_foundation::MainThreadMarker;
    let mtm = MainThreadMarker::new().expect("must be on main thread");
    let app = NSApplication::sharedApplication(mtm);
    app.setActivationPolicy(NSApplicationActivationPolicy::Accessory);
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

pub fn is_escape_held() -> bool {
    false
}

pub fn ghost_window_kind() -> gpui::WindowKind {
    gpui::WindowKind::Normal
}

pub fn ghost_window_decorations(transparent: bool) -> gpui::WindowDecorations {
    if transparent {
        gpui::WindowDecorations::Server
    } else {
        gpui::WindowDecorations::Client
    }
}

pub fn adjust_ghost_bounds(bounds: gpui::Bounds<gpui::Pixels>) -> gpui::Bounds<gpui::Pixels> {
    bounds
}

pub fn should_poll_focus() -> bool {
    true
}

#[repr(C)]
#[derive(Clone, Copy)]
struct ProcessSerialNumber {
    high: u32,
    low: u32,
}

#[link(name = "ApplicationServices", kind = "framework")]
extern "C" {
    fn GetFrontProcess(psn: *mut ProcessSerialNumber) -> i32;
    fn GetProcessPID(psn: *const ProcessSerialNumber, pid: *mut i32) -> i32;
}

fn frontmost_pid() -> Option<i32> {
    unsafe {
        #[allow(deprecated)]
        {
            let mut psn = ProcessSerialNumber { high: 0, low: 0 };
            if GetFrontProcess(&mut psn) != 0 {
                return None;
            }
            let mut pid = 0;
            if GetProcessPID(&psn, &mut pid) != 0 {
                return None;
            }
            Some(pid)
        }
    }
}

pub fn has_process_focus() -> bool {
    frontmost_pid() == Some(std::process::id() as i32)
}

pub fn square_window_corners(window: &mut gpui::Window) {
    use objc2_app_kit::NSWindowStyleMask;
    let Ok(handle) = HasWindowHandle::window_handle(window) else {
        return;
    };
    let RawWindowHandle::AppKit(handle) = handle.as_raw() else {
        return;
    };
    let Some(view) = (unsafe { Retained::<NSView>::retain(handle.ns_view.as_ptr().cast()) }) else {
        return;
    };
    let Some(native_window) = view.window() else {
        return;
    };
    native_window.setStyleMask(NSWindowStyleMask::Borderless);
    native_window.makeFirstResponder(Some(&view));
    if native_window.isVisible() {
        native_window.makeKeyWindow();
    }
}

pub fn start_window_move(window: &mut gpui::Window) {
    let Ok(handle) = HasWindowHandle::window_handle(window) else {
        window.start_window_move();
        return;
    };
    let RawWindowHandle::AppKit(handle) = handle.as_raw() else {
        window.start_window_move();
        return;
    };
    let Some(view) = (unsafe { Retained::<NSView>::retain(handle.ns_view.as_ptr().cast()) }) else {
        window.start_window_move();
        return;
    };
    let Some(native_window) = view.window() else {
        window.start_window_move();
        return;
    };
    let Some(event) = native_window.currentEvent() else {
        window.start_window_move();
        return;
    };
    native_window.performWindowDragWithEvent(&event);
}

pub fn settings_surface_taskbar_identity() -> super::SettingsSurfaceTaskbarIdentity {
    super::SettingsSurfaceTaskbarIdentity {
        app_id: qol_conventions::SETTINGS_SURFACE_APP_ID,
        display_name: qol_conventions::SETTINGS_SURFACE_DISPLAY_NAME,
        icon: super::TaskbarIconSource::HostProcess,
    }
}

pub fn apply_settings_surface_identity(_window: &mut gpui::Window) {}

pub(crate) fn native_scale_for(_window: &gpui::Window) -> f32 {
    1.0
}

pub(crate) fn native_readback_position(_title: &str) -> Option<(i32, i32)> {
    None
}

pub(crate) fn readback_matches(
    _readback: Option<(i32, i32)>,
    _native: crate::window::NativeDesktopBounds,
) -> bool {
    true
}
