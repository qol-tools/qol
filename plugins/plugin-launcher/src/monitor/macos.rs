use super::{monitor_for_bounds, ActiveMonitor, InputState};
use gpui::*;
use std::ffi::c_void;
use std::sync::{Arc, Mutex};

type CGDisplayReconfigurationCallBack =
    unsafe extern "C" fn(CGDirectDisplayID, u32, *mut c_void);

#[repr(C)]
#[derive(Copy, Clone)]
struct CGRect {
    origin: CGPoint,
    size: CGSize,
}
#[repr(C)]
#[derive(Copy, Clone)]
struct CGPoint {
    x: f64,
    y: f64,
}
#[repr(C)]
#[derive(Copy, Clone)]
struct CGSize {
    width: f64,
    height: f64,
}

type CGDirectDisplayID = u32;
type CFDictionaryRef = *const c_void;
type CFArrayRef = *const c_void;
type CFStringRef = *const c_void;
type CFNumberRef = *const c_void;
#[link(name = "CoreGraphics", kind = "framework")]
extern "C" {
    fn CGGetActiveDisplayList(max: u32, displays: *mut CGDirectDisplayID, count: *mut u32) -> i32;
    fn CGDisplayBounds(display: CGDirectDisplayID) -> CGRect;
    fn CGWindowListCopyWindowInfo(option: u32, relative_to: u32) -> CFArrayRef;
    fn CGDisplayRegisterReconfigurationCallback(
        callback: CGDisplayReconfigurationCallBack,
        user_info: *mut c_void,
    ) -> i32;
}

#[link(name = "CoreFoundation", kind = "framework")]
extern "C" {
    fn CFArrayGetCount(arr: CFArrayRef) -> isize;
    fn CFArrayGetValueAtIndex(arr: CFArrayRef, idx: isize) -> *const c_void;
    fn CFDictionaryGetValue(dict: CFDictionaryRef, key: *const c_void) -> *const c_void;
    fn CFNumberGetValue(num: CFNumberRef, the_type: isize, value_ptr: *mut c_void) -> bool;
    fn CFRelease(cf: *const c_void);
}

const K_CG_WINDOW_LIST_OPTION_ON_SCREEN_ONLY: u32 = 1;
const K_CG_WINDOW_LIST_EXCLUDE_DESKTOP_ELEMENTS: u32 = 1 << 4;
const K_CF_NUMBER_INT32_TYPE: isize = 3;
const K_CF_NUMBER_FLOAT64_TYPE: isize = 13;
const K_CG_WINDOW_LAYER_NORMAL: i32 = 0;

fn cfstr(s: &[u8]) -> CFStringRef {
    extern "C" {
        fn CFStringCreateWithBytes(
            alloc: *const c_void,
            bytes: *const u8,
            num_bytes: isize,
            encoding: u32,
            is_external: bool,
        ) -> CFStringRef;
    }
    unsafe { CFStringCreateWithBytes(std::ptr::null(), s.as_ptr(), s.len() as isize, 0x08000100, false) }
}

fn dict_get_i32(dict: CFDictionaryRef, key: CFStringRef) -> Option<i32> {
    unsafe {
        let val = CFDictionaryGetValue(dict, key as *const c_void);
        if val.is_null() {
            return None;
        }
        let mut result: i32 = 0;
        if CFNumberGetValue(val as CFNumberRef, K_CF_NUMBER_INT32_TYPE, &mut result as *mut i32 as *mut c_void) {
            Some(result)
        } else {
            None
        }
    }
}

fn dict_get_f64(dict: CFDictionaryRef, key: CFStringRef) -> Option<f64> {
    unsafe {
        let val = CFDictionaryGetValue(dict, key as *const c_void);
        if val.is_null() {
            return None;
        }
        let mut result: f64 = 0.0;
        if CFNumberGetValue(val as CFNumberRef, K_CF_NUMBER_FLOAT64_TYPE, &mut result as *mut f64 as *mut c_void) {
            Some(result)
        } else {
            None
        }
    }
}

const K_CG_DISPLAY_BEGIN_CONFIGURATION_FLAG: u32 = 1;

unsafe extern "C" fn display_reconfig_callback(
    _display: CGDirectDisplayID,
    flags: u32,
    user_info: *mut c_void,
) {
    if flags & K_CG_DISPLAY_BEGIN_CONFIGURATION_FLAG != 0 {
        return;
    }
    let monitors = &*(user_info as *const Mutex<Vec<Bounds<Pixels>>>);
    if let Ok(mut guard) = monitors.lock() {
        let updated = display_bounds();
        #[cfg(debug_assertions)]
        eprintln!(
            "[monitor/macos] display reconfigured: {} -> {} displays",
            guard.len(),
            updated.len()
        );
        *guard = updated;
    }
}

pub(super) fn start_focus_tracking(
    _state: Arc<Mutex<InputState>>,
    monitors: Arc<Mutex<Vec<Bounds<Pixels>>>>,
) {
    #[cfg(debug_assertions)]
    eprintln!("[monitor/macos] registering display reconfiguration callback");
    let ptr = Arc::into_raw(monitors) as *mut c_void;
    unsafe {
        CGDisplayRegisterReconfigurationCallback(display_reconfig_callback, ptr);
    }
}

pub(super) fn poll_active_monitor(monitors: &[Bounds<Pixels>]) -> Option<ActiveMonitor> {
    let own_pid = std::process::id() as i32;
    let snapshot = poll_focus_once(own_pid);
    #[cfg(debug_assertions)]
    eprintln!(
        "[monitor/macos] poll_active_monitor: {:?}",
        snapshot.as_ref().map(|s| &s.bounds)
    );
    let matched = snapshot
        .as_ref()
        .and_then(|snap| monitor_for_bounds(monitors, &snap.bounds));
    #[cfg(debug_assertions)]
    eprintln!("[monitor/macos] matched monitor: {:?}", matched);
    matched.map(|bounds| ActiveMonitor { bounds })
}

pub(super) fn display_bounds() -> Vec<Bounds<Pixels>> {
    let mut ids = [0u32; 16];
    let mut count = 0u32;

    let ret = unsafe { CGGetActiveDisplayList(16, ids.as_mut_ptr(), &mut count) };
    if ret != 0 {
        #[cfg(debug_assertions)]
        eprintln!("[monitor/macos] CGGetActiveDisplayList failed: {}", ret);
        return Vec::new();
    }

    let result: Vec<_> = (0..count as usize)
        .map(|i| {
            let rect = unsafe { CGDisplayBounds(ids[i]) };
            Bounds::new(
                point(px(rect.origin.x as f32), px(rect.origin.y as f32)),
                size(px(rect.size.width as f32), px(rect.size.height as f32)),
            )
        })
        .collect();

    #[cfg(debug_assertions)]
    eprintln!("[monitor/macos] display_bounds: count={}, bounds={:?}", count, result);

    result
}

struct WindowBoundsResult {
    bounds: Bounds<Pixels>,
}

fn poll_focus_once(own_pid: i32) -> Option<WindowBoundsResult> {
    let opts = K_CG_WINDOW_LIST_OPTION_ON_SCREEN_ONLY | K_CG_WINDOW_LIST_EXCLUDE_DESKTOP_ELEMENTS;
    let list = unsafe { CGWindowListCopyWindowInfo(opts, 0) };
    if list.is_null() {
        #[cfg(debug_assertions)]
        eprintln!("[monitor/macos] CGWindowListCopyWindowInfo returned null");
        return None;
    }

    let key_pid = cfstr(b"kCGWindowOwnerPID");
    let key_layer = cfstr(b"kCGWindowLayer");
    let key_bounds = cfstr(b"kCGWindowBounds");
    let key_bounds_x = cfstr(b"X");
    let key_bounds_y = cfstr(b"Y");
    let key_bounds_w = cfstr(b"Width");
    let key_bounds_h = cfstr(b"Height");

    let count = unsafe { CFArrayGetCount(list) };
    let mut result = None;

    for i in 0..count {
        let dict = unsafe { CFArrayGetValueAtIndex(list, i) } as CFDictionaryRef;
        if dict.is_null() {
            continue;
        }

        let Some(layer) = dict_get_i32(dict, key_layer) else { continue };
        if layer != K_CG_WINDOW_LAYER_NORMAL {
            continue;
        }

        let Some(win_pid) = dict_get_i32(dict, key_pid) else { continue };
        if win_pid == own_pid {
            continue;
        }

        let bounds_dict = unsafe {
            CFDictionaryGetValue(dict, key_bounds as *const c_void)
        } as CFDictionaryRef;
        if bounds_dict.is_null() {
            continue;
        }

        let (Some(x), Some(y), Some(w), Some(h)) = (
            dict_get_f64(bounds_dict, key_bounds_x),
            dict_get_f64(bounds_dict, key_bounds_y),
            dict_get_f64(bounds_dict, key_bounds_w),
            dict_get_f64(bounds_dict, key_bounds_h),
        ) else { continue };

        if w > 0.0 && h > 0.0 {
            result = Some(WindowBoundsResult {
                bounds: Bounds::new(
                    point(px(x as f32), px(y as f32)),
                    size(px(w as f32), px(h as f32)),
                ),
            });
            break;
        }
    }

    unsafe {
        CFRelease(list);
        CFRelease(key_pid as *const c_void);
        CFRelease(key_layer as *const c_void);
        CFRelease(key_bounds as *const c_void);
        CFRelease(key_bounds_x as *const c_void);
        CFRelease(key_bounds_y as *const c_void);
        CFRelease(key_bounds_w as *const c_void);
        CFRelease(key_bounds_h as *const c_void);
    }

    result
}
