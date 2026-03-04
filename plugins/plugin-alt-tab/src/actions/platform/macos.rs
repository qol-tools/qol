use crate::discovery::platform::macos::ffi;
use crate::discovery::platform::macos::ffi::{
    CFArrayGetCount, CFArrayGetValueAtIndex, CFRelease, CFDictionaryRef,
    CGWindowListCopyWindowInfo,
    K_CG_NULL_WINDOW_ID, K_CG_WINDOW_LIST_EXCLUDE_DESKTOP_ELEMENTS,
};
use crate::discovery::platform::macos::ax::ax_find_window;
use std::ffi::c_void;

pub fn activate_window(window_id: u32) {
    let Some((pid, title)) = cg_window_pid_and_title(window_id) else {
        return;
    };

    // Raise the specific AX window so the correct window comes to front,
    // not just whichever window macOS picks for the app.
    // Also unminimize if needed — AXRaise alone won't restore a minimized window.
    let win = unsafe { ax_find_window(pid, window_id, &title) };
    if !win.is_null() {
        unsafe {
            #[link(name = "ApplicationServices", kind = "framework")]
            extern "C" {
                fn AXUIElementPerformAction(el: *const c_void, action: *const c_void) -> i32;
                fn AXUIElementSetAttributeValue(
                    el: *const c_void,
                    attr: *const c_void,
                    val: *const c_void,
                ) -> i32;
            }
            #[link(name = "CoreFoundation", kind = "framework")]
            extern "C" {
                static kCFBooleanFalse: *const c_void;
            }
            let minimized_attr = ffi::cfstr(b"AXMinimized");
            let _ = AXUIElementSetAttributeValue(win, minimized_attr, kCFBooleanFalse);
            CFRelease(minimized_attr as *const c_void);

            let raise = ffi::cfstr(b"AXRaise");
            let _ = AXUIElementPerformAction(win, raise);
            CFRelease(raise as *const c_void);
            CFRelease(win);
        }
    }

    objc2::rc::autoreleasepool(|_pool| {
        use objc2_app_kit::{NSApplicationActivationOptions, NSRunningApplication};

        if let Some(app) = NSRunningApplication::runningApplicationWithProcessIdentifier(pid) {
            #[allow(deprecated)]
            let _ = app.activateWithOptions(
                NSApplicationActivationOptions::ActivateIgnoringOtherApps,
            );
        }
    });
}

pub fn close_window(window_id: u32) {
    let Some((pid, title)) = cg_window_pid_and_title(window_id) else {
        return;
    };
    let win = unsafe { ax_find_window(pid, window_id, &title) };
    if win.is_null() {
        return;
    }
    unsafe {
        ax_press_window_button(win, b"AXCloseButton");
        CFRelease(win);
    }
}

pub fn quit_app(window_id: u32) {
    let Some((pid, _title)) = cg_window_pid_and_title(window_id) else {
        return;
    };
    objc2::rc::autoreleasepool(|_pool| {
        use objc2_app_kit::NSRunningApplication;
        if let Some(app) = NSRunningApplication::runningApplicationWithProcessIdentifier(pid) {
            let _ = app.terminate();
        }
    });
}

pub fn minimize_window_by_id(window_id: u32) {
    let Some((pid, title)) = cg_window_pid_and_title(window_id) else {
        return;
    };
    let win = unsafe { ax_find_window(pid, window_id, &title) };
    if win.is_null() {
        return;
    }
    unsafe {
        ax_set_minimized(win);
        CFRelease(win);
    }
}

fn cg_window_pid_and_title(window_id: u32) -> Option<(i32, String)> {
    let opts = K_CG_WINDOW_LIST_EXCLUDE_DESKTOP_ELEMENTS;
    let list = unsafe { CGWindowListCopyWindowInfo(opts, K_CG_NULL_WINDOW_ID) };
    if list.is_null() {
        return None;
    }
    let key_pid = ffi::cfstr(b"kCGWindowOwnerPID");
    let key_number = ffi::cfstr(b"kCGWindowNumber");
    let key_name = ffi::cfstr(b"kCGWindowName");

    let count = unsafe { CFArrayGetCount(list) };
    let mut result: Option<(i32, String)> = None;

    for i in 0..count {
        let dict = unsafe { CFArrayGetValueAtIndex(list, i) } as CFDictionaryRef;
        if dict.is_null() {
            continue;
        }
        let Some(num) = ffi::dict_get_i32(dict, key_number) else {
            continue;
        };
        if num as u32 != window_id {
            continue;
        }
        if let Some(pid) = ffi::dict_get_i32(dict, key_pid) {
            let title = ffi::dict_get_string(dict, key_name).unwrap_or_default();
            result = Some((pid, title));
        }
        break;
    }

    unsafe {
        CFRelease(list as *const c_void);
        CFRelease(key_pid as *const c_void);
        CFRelease(key_number as *const c_void);
        CFRelease(key_name as *const c_void);
    }
    result
}

/// Press a named button (e.g. `AXCloseButton`) on an AX window element.
/// `win_el` must be a valid, retained AX element.
unsafe fn ax_press_window_button(win_el: *const c_void, button_attr_name: &[u8]) {
    #[link(name = "ApplicationServices", kind = "framework")]
    extern "C" {
        fn AXUIElementCopyAttributeValue(
            el: *const c_void,
            attr: *const c_void,
            val: *mut *const c_void,
        ) -> i32;
        fn AXUIElementPerformAction(el: *const c_void, action: *const c_void) -> i32;
    }

    let button_attr = ffi::cfstr(button_attr_name);
    let press_action = ffi::cfstr(b"AXPress");

    let mut button_val: *const c_void = std::ptr::null();
    let err = AXUIElementCopyAttributeValue(win_el, button_attr, &mut button_val);
    if err == 0 && !button_val.is_null() {
        let _ = AXUIElementPerformAction(button_val, press_action);
        CFRelease(button_val as *const c_void);
    }

    CFRelease(button_attr as *const c_void);
    CFRelease(press_action as *const c_void);
}

/// Set the `AXMinimized` attribute to true on an AX window element.
/// `win_el` must be a valid, retained AX element.
unsafe fn ax_set_minimized(win_el: *const c_void) {
    #[link(name = "ApplicationServices", kind = "framework")]
    extern "C" {
        fn AXUIElementSetAttributeValue(
            el: *const c_void,
            attr: *const c_void,
            val: *const c_void,
        ) -> i32;
    }
    #[link(name = "CoreFoundation", kind = "framework")]
    extern "C" {
        static kCFBooleanTrue: *const c_void;
    }

    let minimized_attr = ffi::cfstr(b"AXMinimized");
    let _ = AXUIElementSetAttributeValue(win_el, minimized_attr, kCFBooleanTrue);
    CFRelease(minimized_attr as *const c_void);
}
