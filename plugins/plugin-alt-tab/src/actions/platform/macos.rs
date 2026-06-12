use crate::discovery::macos::ax::{ax_find_window, ax_find_window_brute_force};
use crate::discovery::macos::ffi;
use crate::discovery::macos::ffi::{
    copy_window_list_timed, CFArrayGetCount, CFArrayGetValueAtIndex, CFDictionaryRef, CFRelease,
    K_CG_NULL_WINDOW_ID, K_CG_WINDOW_LAYER_NORMAL, K_CG_WINDOW_LIST_EXCLUDE_DESKTOP_ELEMENTS,
    K_CG_WINDOW_LIST_OPTION_ON_SCREEN_ONLY,
};
use std::ffi::c_void;
use std::sync::OnceLock;

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
    fn AXUIElementCreateApplication(pid: i32) -> *const c_void;
}

#[link(name = "CoreFoundation", kind = "framework")]
extern "C" {
    static kCFBooleanTrue: *const c_void;
    static kCFBooleanFalse: *const c_void;
}

static ACTIVATE_GEN: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

pub fn activate_window(window_id: u32) {
    let commit_gen = ACTIVATE_GEN.fetch_add(1, std::sync::atomic::Ordering::SeqCst) + 1;
    #[cfg(debug_assertions)]
    let started = std::time::Instant::now();
    let Some((pid, title)) = cg_window_pid_and_title(window_id) else {
        return;
    };
    #[cfg(debug_assertions)]
    let lookup_ms = started.elapsed().as_millis();
    qol_runtime::probe!("ACTIVATE_WIN", "wid={window_id} title=\"{title}\"");
    qol_runtime::probe!(
        "ACTIVATE_STACK",
        "phase=req wid={window_id} stack=[{}]",
        stack_snapshot(),
    );
    let mut win = unsafe { ax_find_window(pid, window_id, &title) };
    if win.is_null() {
        win = unsafe { ax_find_window_brute_force(pid, window_id) };
        qol_runtime::probe!("ACTIVATE_BRUTE", "wid={window_id} hit={}", !win.is_null());
    }
    if !win.is_null() {
        unsafe {
            ax_unminimize(win);
            ax_raise(win);
            ax_make_main(win);
            CFRelease(win);
        }
    }
    #[cfg(debug_assertions)]
    let ax_ms = started.elapsed().as_millis() - lookup_ms;
    let forced = force_front(pid, window_id);
    if !forced {
        ns_activate_app(pid);
    }
    unsafe { ax_app_frontmost(pid) };
    qol_runtime::probe!(
        "ACTIVATE_TIMING",
        "wid={window_id} lookup_ms={lookup_ms} ax_ms={ax_ms} total_ms={} forced={forced}",
        started.elapsed().as_millis(),
    );
    qol_gpui::platform::spawn_reassert_driver(
        &ACTIVATE_GEN,
        commit_gen,
        &[60u64, 40, 50, 100, 150, 250, 400],
        {
            let mut settled_logged = false;
            #[cfg(debug_assertions)]
            let mut stack_stuck_logged = false;
            #[cfg(debug_assertions)]
            let mut key_focus_logged = false;
            #[cfg(debug_assertions)]
            let mut key_focus_sampled = false;
            move || {
                let active = target_effectively_front(window_id, pid);
                if active && !settled_logged {
                    settled_logged = true;
                    qol_runtime::probe!(
                        "ACTIVATE_SETTLED",
                        "wid={window_id} elapsed_ms={}",
                        started.elapsed().as_millis(),
                    );
                    qol_runtime::probe!(
                        "ACTIVATE_STACK",
                        "phase=settled wid={window_id} elapsed_ms={} stack=[{}]",
                        started.elapsed().as_millis(),
                        stack_snapshot(),
                    );
                }
                #[cfg(debug_assertions)]
                if !active && !stack_stuck_logged && started.elapsed().as_millis() >= 250 {
                    stack_stuck_logged = true;
                    qol_runtime::probe!(
                        "ACTIVATE_STACK",
                        "phase=stuck wid={window_id} elapsed_ms={} stack=[{}]",
                        started.elapsed().as_millis(),
                        stack_snapshot(),
                    );
                }
                #[cfg(debug_assertions)]
                {
                    let focused = unsafe { crate::discovery::macos::ax::ax_focused_window_id(pid) };
                    if !key_focus_sampled {
                        key_focus_sampled = true;
                        qol_runtime::probe!(
                            "ACTIVATE_KEY_SAMPLE",
                            "wid={window_id} focused={focused:?} elapsed_ms={}",
                            started.elapsed().as_millis(),
                        );
                    }
                    if !key_focus_logged && focused == Some(window_id) {
                        key_focus_logged = true;
                        qol_runtime::probe!(
                            "ACTIVATE_KEY_FOCUS",
                            "wid={window_id} elapsed_ms={}",
                            started.elapsed().as_millis(),
                        );
                    }
                }
                active
            }
        },
        move || {
            qol_runtime::probe!(
                "ACTIVATE_REASSERT",
                "wid={window_id} elapsed_ms={}",
                started.elapsed().as_millis(),
            );
            if forced {
                force_front(pid, window_id);
            }
            ns_activate_app(pid);
            unsafe { ax_app_frontmost(pid) };
            let mut win = unsafe { ax_find_window(pid, window_id, &title) };
            if win.is_null() {
                win = unsafe { ax_find_window_brute_force(pid, window_id) };
            }
            if !win.is_null() {
                unsafe {
                    ax_raise(win);
                    ax_make_main(win);
                    CFRelease(win);
                }
            }
        },
    );
}

#[cfg(debug_assertions)]
fn stack_snapshot() -> String {
    let list = copy_window_list_timed(
        K_CG_WINDOW_LIST_OPTION_ON_SCREEN_ONLY | K_CG_WINDOW_LIST_EXCLUDE_DESKTOP_ELEMENTS,
        K_CG_NULL_WINDOW_ID,
        "stack_snapshot",
    );
    if list.is_null() {
        return "unavailable".to_string();
    }
    let key_num = ffi::cfstr(b"kCGWindowNumber");
    let key_layer = ffi::cfstr(b"kCGWindowLayer");
    let key_app = ffi::cfstr(b"kCGWindowOwnerName");
    let key_bounds = ffi::cfstr(b"kCGWindowBounds");

    let mut entries = Vec::new();
    let count = unsafe { CFArrayGetCount(list) };
    for i in 0..count {
        if entries.len() >= 8 {
            break;
        }
        let dict = unsafe { CFArrayGetValueAtIndex(list, i) } as CFDictionaryRef;
        if dict.is_null() {
            continue;
        }
        let Some(layer) = ffi::dict_get_i32(dict, key_layer) else {
            continue;
        };
        if !(0..=1000).contains(&layer) {
            continue;
        }
        let wid = ffi::dict_get_i32(dict, key_num).unwrap_or(-1);
        let app = ffi::dict_get_string(dict, key_app).unwrap_or_default();
        let (y, h) = ffi::dict_get_rect(dict, key_bounds)
            .map(|(_, y, _, h)| (y as i32, h as i32))
            .unwrap_or((-1, -1));
        entries.push(format!("{wid}:l{layer}:{app}:y{y}h{h}"));
    }
    unsafe {
        CFRelease(key_num);
        CFRelease(key_layer);
        CFRelease(key_app);
        CFRelease(key_bounds);
        CFRelease(list);
    }
    entries.join(" ")
}

fn target_effectively_front(window_id: u32, pid: i32) -> bool {
    let list = copy_window_list_timed(
        K_CG_WINDOW_LIST_OPTION_ON_SCREEN_ONLY | K_CG_WINDOW_LIST_EXCLUDE_DESKTOP_ELEMENTS,
        K_CG_NULL_WINDOW_ID,
        "settle_check",
    );
    if list.is_null() {
        return false;
    }
    let key_num = ffi::cfstr(b"kCGWindowNumber");
    let key_layer = ffi::cfstr(b"kCGWindowLayer");
    let key_pid = ffi::cfstr(b"kCGWindowOwnerPID");

    let mut stack = Vec::new();
    let count = unsafe { CFArrayGetCount(list) };
    for i in 0..count {
        let dict = unsafe { CFArrayGetValueAtIndex(list, i) } as CFDictionaryRef;
        if dict.is_null() {
            continue;
        }
        if ffi::dict_get_i32(dict, key_layer) != Some(K_CG_WINDOW_LAYER_NORMAL) {
            continue;
        }
        let Some(wid) = ffi::dict_get_i32(dict, key_num) else {
            continue;
        };
        let Some(owner) = ffi::dict_get_i32(dict, key_pid) else {
            continue;
        };
        stack.push((wid as u32, owner));
    }

    unsafe {
        CFRelease(key_num);
        CFRelease(key_layer);
        CFRelease(key_pid);
        CFRelease(list);
    }
    front_before_other_apps(window_id, pid, &stack)
}

fn front_before_other_apps(window_id: u32, pid: i32, stack: &[(u32, i32)]) -> bool {
    for &(wid, owner) in stack {
        if wid == window_id {
            return true;
        }
        if owner != pid {
            return false;
        }
    }
    false
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

fn ns_activate_app(pid: i32) -> bool {
    objc2::rc::autoreleasepool(|_| {
        use objc2_app_kit::{NSApplicationActivationOptions, NSRunningApplication};
        let Some(app) = NSRunningApplication::runningApplicationWithProcessIdentifier(pid) else {
            return false;
        };
        #[allow(deprecated)]
        app.activateWithOptions(NSApplicationActivationOptions::ActivateIgnoringOtherApps)
    })
}

#[repr(C)]
#[derive(Clone, Copy)]
struct ProcessSerialNumber {
    high: u32,
    low: u32,
}

#[link(name = "ApplicationServices", kind = "framework")]
extern "C" {
    fn GetProcessForPID(pid: i32, psn: *mut ProcessSerialNumber) -> i32;
}

type SetFrontFn = unsafe extern "C" fn(*const ProcessSerialNumber, u32, u32) -> i32;
type PostEventFn = unsafe extern "C" fn(*const ProcessSerialNumber, *const u8) -> i32;

struct SkyLight {
    set_front: SetFrontFn,
    post_event: PostEventFn,
}

extern "C" {
    fn dlopen(path: *const std::ffi::c_char, mode: i32) -> *mut c_void;
    fn dlsym(handle: *mut c_void, symbol: *const std::ffi::c_char) -> *mut c_void;
}
const RTLD_NOW: i32 = 2;

fn skylight() -> Option<&'static SkyLight> {
    static SL: OnceLock<Option<SkyLight>> = OnceLock::new();
    SL.get_or_init(|| unsafe {
        let path = c"/System/Library/PrivateFrameworks/SkyLight.framework/SkyLight";
        let handle = dlopen(path.as_ptr(), RTLD_NOW);
        if handle.is_null() {
            return None;
        }
        let set_front = dlsym(handle, c"_SLPSSetFrontProcessWithOptions".as_ptr());
        let post_event = dlsym(handle, c"SLPSPostEventRecordTo".as_ptr());
        if set_front.is_null() || post_event.is_null() {
            return None;
        }
        Some(SkyLight {
            set_front: std::mem::transmute::<*mut c_void, SetFrontFn>(set_front),
            post_event: std::mem::transmute::<*mut c_void, PostEventFn>(post_event),
        })
    })
    .as_ref()
}

fn force_front(pid: i32, wid: u32) -> bool {
    let Some(sl) = skylight() else {
        return false;
    };
    unsafe {
        let mut psn = ProcessSerialNumber { high: 0, low: 0 };
        if GetProcessForPID(pid, &mut psn) != 0 {
            return false;
        }
        const K_CPS_USER_GENERATED: u32 = 0x200;
        (sl.set_front)(&psn, wid, K_CPS_USER_GENERATED);
        make_key_window(sl, &psn, wid);
    }
    true
}

unsafe fn ax_app_frontmost(pid: i32) {
    let app = AXUIElementCreateApplication(pid);
    if app.is_null() {
        return;
    }
    let attr = ffi::cfstr(b"AXFrontmost");
    let _ = AXUIElementSetAttributeValue(app, attr, kCFBooleanTrue);
    CFRelease(attr);
    CFRelease(app);
}

unsafe fn make_key_window(sl: &SkyLight, psn: &ProcessSerialNumber, wid: u32) {
    let mut bytes1 = [0u8; 0xf8];
    bytes1[0x04] = 0xf8;
    bytes1[0x08] = 0x01;
    bytes1[0x3a] = 0x10;
    bytes1[0x3c..0x40].copy_from_slice(&wid.to_ne_bytes());
    for b in bytes1[0x20..0x30].iter_mut() {
        *b = 0xff;
    }
    let mut bytes2 = bytes1;
    bytes2[0x08] = 0x02;
    (sl.post_event)(psn, bytes1.as_ptr());
    (sl.post_event)(psn, bytes2.as_ptr());
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

unsafe fn ax_make_main(win: *const c_void) {
    ax_set_bool_attr(win, b"AXMain", kCFBooleanTrue);
    ax_set_bool_attr(win, b"AXFocused", kCFBooleanTrue);
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
    let list = copy_window_list_timed(
        K_CG_WINDOW_LIST_EXCLUDE_DESKTOP_ELEMENTS,
        K_CG_NULL_WINDOW_ID,
        "activate_lookup",
    );
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

#[cfg(test)]
mod tests {
    use super::front_before_other_apps;

    type Case = (&'static str, u32, i32, &'static [(u32, i32)], bool);

    #[test]
    fn settle_skips_same_app_siblings_but_not_other_apps() {
        let cases: &[Case] = &[
            ("target on top", 10, 1, &[(10, 1), (20, 2)], true),
            (
                "same-app chrome above target",
                10,
                1,
                &[(11, 1), (10, 1), (20, 2)],
                true,
            ),
            ("other app above target", 10, 1, &[(20, 2), (10, 1)], false),
            ("target absent", 10, 1, &[(20, 2), (30, 3)], false),
            ("empty stack", 10, 1, &[], false),
            (
                "same-app then other app, target below",
                10,
                1,
                &[(11, 1), (20, 2), (10, 1)],
                false,
            ),
        ];
        for (name, wid, pid, stack, expected) in cases {
            assert_eq!(
                front_before_other_apps(*wid, *pid, stack),
                *expected,
                "case: {name}"
            );
        }
    }
}
