use crate::discovery::macos::ax::ax_find_window;
use crate::discovery::macos::ffi;
use crate::discovery::macos::ffi::{
    CFArrayGetCount, CFArrayGetValueAtIndex, CFDictionaryRef, CFRelease,
    CGWindowListCopyWindowInfo, K_CG_NULL_WINDOW_ID, K_CG_WINDOW_LIST_EXCLUDE_DESKTOP_ELEMENTS,
};
use std::ffi::c_void;

#[link(name = "ApplicationServices", kind = "framework")]
extern "C" {
    fn AXUIElementCopyAttributeValue(
        el: *const c_void,
        attr: *const c_void,
        val: *mut *const c_void,
    ) -> i32;
    fn AXUIElementPerformAction(el: *const c_void, action: *const c_void) -> i32;
    fn AXUIElementSetAttributeValue(
        el: *const c_void,
        attr: *const c_void,
        val: *const c_void,
    ) -> i32;
}

#[link(name = "CoreFoundation", kind = "framework")]
extern "C" {
    static kCFBooleanTrue: *const c_void;
    static kCFBooleanFalse: *const c_void;
}

pub fn activate_window(window_id: u32) {
    let Some((pid, title)) = cg_window_pid_and_title(window_id) else {
        return;
    };
    let win = unsafe { ax_find_window(pid, window_id, &title) };
    if !win.is_null() {
        // AXRaise alone won't restore a minimized window — unminimize first.
        unsafe { ax_unminimize(win) };
        unsafe { ax_raise(win) };
        unsafe { CFRelease(win) };
    }
    ns_activate_app(pid);
}

pub fn close_window(window_id: u32) {
    let Some((pid, title)) = cg_window_pid_and_title(window_id) else {
        return;
    };
    let win = unsafe { ax_find_window(pid, window_id, &title) };
    if win.is_null() {
        return;
    }
    unsafe { ax_press_button(win, b"AXCloseButton") };
    unsafe { CFRelease(win) };
}

pub fn quit_app(window_id: u32) {
    let Some((pid, _)) = cg_window_pid_and_title(window_id) else {
        return;
    };
    ns_terminate_app(pid);
}

pub fn minimize_window_by_id(window_id: u32) {
    let Some((pid, title)) = cg_window_pid_and_title(window_id) else {
        return;
    };
    let win = unsafe { ax_find_window(pid, window_id, &title) };
    if win.is_null() {
        return;
    }
    unsafe { ax_set_bool_attr(win, b"AXMinimized", kCFBooleanTrue) };
    unsafe { CFRelease(win) };
}

fn ns_activate_app(pid: i32) {
    objc2::rc::autoreleasepool(|_| {
        use objc2_app_kit::{NSApplicationActivationOptions, NSRunningApplication};
        let Some(app) = NSRunningApplication::runningApplicationWithProcessIdentifier(pid) else {
            return;
        };
        #[allow(deprecated)]
        let _ = app.activateWithOptions(NSApplicationActivationOptions::ActivateIgnoringOtherApps);
    });
}

fn ns_terminate_app(pid: i32) {
    objc2::rc::autoreleasepool(|_| {
        use objc2_app_kit::NSRunningApplication;
        let Some(app) = NSRunningApplication::runningApplicationWithProcessIdentifier(pid) else {
            return;
        };
        let _ = app.terminate();
    });
}

unsafe fn ax_unminimize(win: *const c_void) {
    ax_set_bool_attr(win, b"AXMinimized", kCFBooleanFalse);
}

unsafe fn ax_raise(win: *const c_void) {
    let action = ffi::cfstr(b"AXRaise");
    let _ = AXUIElementPerformAction(win, action);
    CFRelease(action);
}

unsafe fn ax_press_button(win: *const c_void, button_attr: &[u8]) {
    let attr = ffi::cfstr(button_attr);
    let action = ffi::cfstr(b"AXPress");
    let mut button: *const c_void = std::ptr::null();
    let err = AXUIElementCopyAttributeValue(win, attr, &mut button);
    if err == 0 && !button.is_null() {
        let _ = AXUIElementPerformAction(button, action);
        CFRelease(button);
    }
    CFRelease(attr);
    CFRelease(action);
}

unsafe fn ax_set_bool_attr(win: *const c_void, name: &[u8], val: *const c_void) {
    let attr = ffi::cfstr(name);
    let _ = AXUIElementSetAttributeValue(win, attr, val);
    CFRelease(attr);
}

fn cg_window_pid_and_title(window_id: u32) -> Option<(i32, String)> {
    let list = unsafe {
        CGWindowListCopyWindowInfo(
            K_CG_WINDOW_LIST_EXCLUDE_DESKTOP_ELEMENTS,
            K_CG_NULL_WINDOW_ID,
        )
    };
    if list.is_null() {
        return None;
    }
    let result = find_window_in_list(list, window_id);
    unsafe { CFRelease(list) };
    result
}

fn find_window_in_list(list: ffi::CFArrayRef, window_id: u32) -> Option<(i32, String)> {
    let key_pid = ffi::cfstr(b"kCGWindowOwnerPID");
    let key_num = ffi::cfstr(b"kCGWindowNumber");
    let key_name = ffi::cfstr(b"kCGWindowName");

    let mut result = None;
    let count = unsafe { CFArrayGetCount(list) };
    for i in 0..count {
        let dict = unsafe { CFArrayGetValueAtIndex(list, i) } as CFDictionaryRef;
        if dict.is_null() {
            continue;
        }
        let Some(num) = ffi::dict_get_i32(dict, key_num) else {
            continue;
        };
        if num as u32 != window_id {
            continue;
        }
        if let Some(pid) = ffi::dict_get_i32(dict, key_pid) {
            result = Some((
                pid,
                ffi::dict_get_string(dict, key_name).unwrap_or_default(),
            ));
        }
        break;
    }

    unsafe {
        CFRelease(key_pid);
        CFRelease(key_num);
        CFRelease(key_name);
    }
    result
}
