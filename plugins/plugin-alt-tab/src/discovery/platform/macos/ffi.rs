use std::ffi::c_void;

// ── Type aliases ─────────────────────────────────────────────────

pub(crate) type CFArrayRef = *const c_void;
pub(crate) type CFDictionaryRef = *const c_void;
pub(crate) type CGImageRef = *const c_void;
pub(crate) type CFDataRef = *const c_void;
pub(crate) type CGDataProviderRef = *const c_void;

// ── Geometry ─────────────────────────────────────────────────────

#[repr(C)]
#[derive(Copy, Clone)]
pub(crate) struct CGRect {
    pub origin: CGPoint,
    pub size: CGSize,
}

#[repr(C)]
#[derive(Copy, Clone)]
pub(crate) struct CGPoint {
    pub x: f64,
    pub y: f64,
}

#[repr(C)]
#[derive(Copy, Clone)]
pub(crate) struct CGSize {
    pub width: f64,
    pub height: f64,
}

// ── Constants ────────────────────────────────────────────────────

pub(crate) const CG_RECT_NULL: CGRect = CGRect {
    origin: CGPoint { x: f64::INFINITY, y: f64::INFINITY },
    size: CGSize { width: 0.0, height: 0.0 },
};
pub(crate) const K_CG_WINDOW_LIST_OPTION_INCLUDING_WINDOW: u32 = 1 << 3;
pub(crate) const K_CG_WINDOW_IMAGE_BOUNDS_IGNORE_FRAMING: u32 = 1 << 0;
pub(crate) const K_CG_WINDOW_IMAGE_NOMINAL_RESOLUTION: u32 = 1 << 9;
pub(crate) const K_CG_WINDOW_LIST_OPTION_ON_SCREEN_ONLY: u32 = 1;
pub(crate) const K_CG_WINDOW_LIST_EXCLUDE_DESKTOP_ELEMENTS: u32 = 1 << 4;
pub(crate) const K_CG_NULL_WINDOW_ID: u32 = 0;
pub(crate) const K_CG_WINDOW_LAYER_NORMAL: i32 = 0;

// ── CoreGraphics FFI ─────────────────────────────────────────────

#[link(name = "CoreGraphics", kind = "framework")]
extern "C" {
    pub(crate) fn CGWindowListCopyWindowInfo(option: u32, relative_to: u32) -> CFArrayRef;
    pub(crate) fn CGWindowListCreateImage(
        screen_bounds: CGRect,
        list_option: u32,
        window_id: u32,
        image_option: u32,
    ) -> CGImageRef;
    pub(crate) fn CGImageGetWidth(image: CGImageRef) -> usize;
    pub(crate) fn CGImageGetHeight(image: CGImageRef) -> usize;
    pub(crate) fn CGImageGetBytesPerRow(image: CGImageRef) -> usize;
    #[allow(dead_code)]
    pub(crate) fn CGImageGetBitsPerPixel(image: CGImageRef) -> usize;
    pub(crate) fn CGImageGetDataProvider(image: CGImageRef) -> CGDataProviderRef;
    pub(crate) fn CGDataProviderCopyData(provider: CGDataProviderRef) -> CFDataRef;
}

// ── CoreFoundation FFI ───────────────────────────────────────────

#[link(name = "CoreFoundation", kind = "framework")]
extern "C" {
    pub(crate) fn CFArrayGetCount(arr: CFArrayRef) -> isize;
    pub(crate) fn CFArrayGetValueAtIndex(arr: CFArrayRef, idx: isize) -> *const c_void;
    pub(crate) fn CFRelease(cf: *const c_void);
    pub(crate) fn CFRetain(cf: *const c_void) -> *const c_void;
    pub(crate) fn CFDataGetBytePtr(data: CFDataRef) -> *const u8;
    pub(crate) fn CFDataGetLength(data: CFDataRef) -> isize;
}

// ── Dictionary / string helpers ──────────────────────────────────

type CFStringRef = *const c_void;
type CFNumberRef = *const c_void;

const K_CF_NUMBER_INT32_TYPE: isize = 3;
const K_CF_NUMBER_FLOAT64_TYPE: isize = 13;

pub(crate) fn cfstr(s: &[u8]) -> CFStringRef {
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

pub(crate) fn dict_get_i32(dict: CFDictionaryRef, key: CFStringRef) -> Option<i32> {
    #[link(name = "CoreFoundation", kind = "framework")]
    extern "C" {
        fn CFDictionaryGetValue(dict: CFDictionaryRef, key: *const c_void) -> *const c_void;
        fn CFNumberGetValue(num: CFNumberRef, the_type: isize, value_ptr: *mut c_void) -> bool;
    }
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

pub(crate) fn dict_get_f64(dict: CFDictionaryRef, key: CFStringRef) -> Option<f64> {
    #[link(name = "CoreFoundation", kind = "framework")]
    extern "C" {
        fn CFDictionaryGetValue(dict: CFDictionaryRef, key: *const c_void) -> *const c_void;
        fn CFNumberGetValue(num: CFNumberRef, the_type: isize, value_ptr: *mut c_void) -> bool;
    }
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

pub(crate) fn dict_get_rect(dict: CFDictionaryRef, key: CFStringRef) -> Option<(f64, f64, f64, f64)> {
    #[repr(C)]
    struct CgPoint { x: f64, y: f64 }
    #[repr(C)]
    struct CgSize { width: f64, height: f64 }
    #[repr(C)]
    struct CgRect { origin: CgPoint, size: CgSize }

    #[link(name = "CoreGraphics", kind = "framework")]
    extern "C" {
        fn CGRectMakeWithDictionaryRepresentation(dict: CFDictionaryRef, rect: *mut CgRect) -> bool;
    }
    #[link(name = "CoreFoundation", kind = "framework")]
    extern "C" {
        fn CFDictionaryGetValue(dict: CFDictionaryRef, key: *const c_void) -> *const c_void;
    }
    unsafe {
        let val = CFDictionaryGetValue(dict, key as *const c_void);
        if val.is_null() {
            return None;
        }
        let mut rect = CgRect {
            origin: CgPoint { x: 0.0, y: 0.0 },
            size: CgSize { width: 0.0, height: 0.0 },
        };
        if CGRectMakeWithDictionaryRepresentation(val as CFDictionaryRef, &mut rect) {
            Some((rect.origin.x, rect.origin.y, rect.size.width, rect.size.height))
        } else {
            None
        }
    }
}

pub(crate) fn dict_get_bool(dict: CFDictionaryRef, key: CFStringRef) -> Option<bool> {
    #[link(name = "CoreFoundation", kind = "framework")]
    extern "C" {
        fn CFDictionaryGetValue(dict: CFDictionaryRef, key: *const c_void) -> *const c_void;
        fn CFBooleanGetValue(boolean: *const c_void) -> bool;
    }
    unsafe {
        let val = CFDictionaryGetValue(dict, key as *const c_void);
        if val.is_null() {
            return None;
        }
        Some(CFBooleanGetValue(val))
    }
}

pub(crate) fn cfstring_to_string(s: *const c_void) -> Option<String> {
    #[link(name = "CoreFoundation", kind = "framework")]
    extern "C" {
        fn CFStringGetLength(s: *const c_void) -> isize;
        fn CFStringGetMaximumSizeForEncoding(length: isize, encoding: u32) -> isize;
        fn CFStringGetCString(
            s: *const c_void,
            buffer: *mut u8,
            buffer_size: isize,
            encoding: u32,
        ) -> bool;
    }
    unsafe {
        if s.is_null() {
            return None;
        }
        let len = CFStringGetLength(s);
        if len <= 0 {
            return Some(String::new());
        }
        let max_bytes = CFStringGetMaximumSizeForEncoding(len, 0x08000100);
        if max_bytes <= 0 {
            return None;
        }
        let mut buf = vec![0u8; (max_bytes + 1) as usize];
        if !CFStringGetCString(s, buf.as_mut_ptr(), buf.len() as isize, 0x08000100) {
            return None;
        }
        let len = buf.iter().position(|&b| b == 0).unwrap_or(buf.len());
        String::from_utf8(buf[..len].to_vec()).ok()
    }
}

pub(crate) fn cfnumber_to_u32(num: *const c_void) -> Option<u32> {
    const K_CF_NUMBER_SINT64_TYPE: isize = 4;
    #[link(name = "CoreFoundation", kind = "framework")]
    extern "C" {
        fn CFNumberGetValue(num: *const c_void, the_type: isize, value_ptr: *mut c_void) -> bool;
    }
    unsafe {
        if num.is_null() {
            return None;
        }
        let mut val: i64 = 0;
        let ok = CFNumberGetValue(
            num,
            K_CF_NUMBER_SINT64_TYPE,
            &mut val as *mut i64 as *mut c_void,
        );
        if ok {
            Some(val as u32)
        } else {
            None
        }
    }
}

pub(crate) fn dict_get_string(dict: CFDictionaryRef, key: CFStringRef) -> Option<String> {
    #[link(name = "CoreFoundation", kind = "framework")]
    extern "C" {
        fn CFDictionaryGetValue(dict: CFDictionaryRef, key: *const c_void) -> *const c_void;
        fn CFStringGetLength(s: CFStringRef) -> isize;
        fn CFStringGetMaximumSizeForEncoding(length: isize, encoding: u32) -> isize;
        fn CFStringGetCString(
            s: CFStringRef,
            buffer: *mut u8,
            buffer_size: isize,
            encoding: u32,
        ) -> bool;
    }
    unsafe {
        let val = CFDictionaryGetValue(dict, key as *const c_void);
        if val.is_null() {
            return None;
        }
        let s = val as CFStringRef;
        let len = CFStringGetLength(s);
        if len <= 0 {
            return Some(String::new());
        }
        let max_bytes = CFStringGetMaximumSizeForEncoding(len, 0x08000100);
        if max_bytes <= 0 {
            return None;
        }
        let mut buf = vec![0u8; (max_bytes + 1) as usize];
        if !CFStringGetCString(s, buf.as_mut_ptr(), buf.len() as isize, 0x08000100) {
            return None;
        }
        let len = buf.iter().position(|&b| b == 0).unwrap_or(buf.len());
        String::from_utf8(buf[..len].to_vec()).ok()
    }
}

