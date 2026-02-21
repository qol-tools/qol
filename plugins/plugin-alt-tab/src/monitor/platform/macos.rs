use super::PlatformQueries;
use gpui::*;
use std::ffi::c_void;

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
    fn CGEventCreate(source: *const c_void) -> *const c_void;
    fn CGEventGetLocation(event: *const c_void) -> CGPoint;
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
    unsafe {
        CFStringCreateWithBytes(
            std::ptr::null(),
            s.as_ptr(),
            s.len() as isize,
            0x08000100,
            false,
        )
    }
}

fn dict_get_i32(dict: CFDictionaryRef, key: CFStringRef) -> Option<i32> {
    unsafe {
        let val = CFDictionaryGetValue(dict, key as *const c_void);
        if val.is_null() {
            return None;
        }
        let mut result: i32 = 0;
        if CFNumberGetValue(
            val as CFNumberRef,
            K_CF_NUMBER_INT32_TYPE,
            &mut result as *mut i32 as *mut c_void,
        ) {
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
        if CFNumberGetValue(
            val as CFNumberRef,
            K_CF_NUMBER_FLOAT64_TYPE,
            &mut result as *mut f64 as *mut c_void,
        ) {
            Some(result)
        } else {
            None
        }
    }
}

pub(super) struct MacQueries {
    own_pid: i32,
}

impl MacQueries {
    pub fn new(own_pid: i32) -> Self {
        Self { own_pid }
    }
}

impl PlatformQueries for MacQueries {
    fn poll_focused_window(&self) -> bool {
        // CGWindowListCopyWindowInfo deadlocks when called from a background
        // thread while AppKit is rendering on the main thread.
        false
    }

    fn cursor_position(&self) -> Option<(f32, f32)> {
        unsafe {
            let event = CGEventCreate(std::ptr::null());
            if event.is_null() {
                return None;
            }
            let loc = CGEventGetLocation(event);
            CFRelease(event);
            Some((loc.x as f32, loc.y as f32))
        }
    }

    fn focused_window_bounds(&self) -> Option<Bounds<Pixels>> {
        let opts =
            K_CG_WINDOW_LIST_OPTION_ON_SCREEN_ONLY | K_CG_WINDOW_LIST_EXCLUDE_DESKTOP_ELEMENTS;
        let list = unsafe { CGWindowListCopyWindowInfo(opts, 0) };
        if list.is_null() {
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
            let Some(layer) = dict_get_i32(dict, key_layer) else {
                continue;
            };
            if layer != K_CG_WINDOW_LAYER_NORMAL {
                continue;
            }
            let Some(win_pid) = dict_get_i32(dict, key_pid) else {
                continue;
            };
            if win_pid == self.own_pid {
                continue;
            }
            let bounds_dict = unsafe { CFDictionaryGetValue(dict, key_bounds as *const c_void) }
                as CFDictionaryRef;
            if bounds_dict.is_null() {
                continue;
            }
            let (Some(x), Some(y), Some(w), Some(h)) = (
                dict_get_f64(bounds_dict, key_bounds_x),
                dict_get_f64(bounds_dict, key_bounds_y),
                dict_get_f64(bounds_dict, key_bounds_w),
                dict_get_f64(bounds_dict, key_bounds_h),
            ) else {
                continue;
            };
            if w > 0.0 && h > 0.0 {
                result = Some(Bounds::new(
                    point(px(x as f32), px(y as f32)),
                    size(px(w as f32), px(h as f32)),
                ));
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

    fn physical_monitors(&self) -> Vec<Bounds<Pixels>> {
        let mut ids = [0u32; 16];
        let mut count = 0u32;
        let ret = unsafe { CGGetActiveDisplayList(16, ids.as_mut_ptr(), &mut count) };
        if ret != 0 {
            return Vec::new();
        }
        (0..count as usize)
            .map(|i| {
                let rect = unsafe { CGDisplayBounds(ids[i]) };
                Bounds::new(
                    point(px(rect.origin.x as f32), px(rect.origin.y as f32)),
                    size(px(rect.size.width as f32), px(rect.size.height as f32)),
                )
            })
            .collect()
    }
}
