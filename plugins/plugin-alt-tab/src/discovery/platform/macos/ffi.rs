use std::ffi::c_void;

pub(crate) type CFArrayRef = *const c_void;
pub(crate) type CFDictionaryRef = *const c_void;
pub(crate) type CGImageRef = *const c_void;
pub(crate) type CFDataRef = *const c_void;
pub(crate) type CGDataProviderRef = *const c_void;

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

pub(crate) const CG_RECT_NULL: CGRect = CGRect {
    origin: CGPoint {
        x: f64::INFINITY,
        y: f64::INFINITY,
    },
    size: CGSize {
        width: 0.0,
        height: 0.0,
    },
};
pub(crate) const K_CG_WINDOW_LIST_OPTION_INCLUDING_WINDOW: u32 = 1 << 3;
pub(crate) const K_CG_WINDOW_IMAGE_BOUNDS_IGNORE_FRAMING: u32 = 1 << 0;
pub(crate) const K_CG_WINDOW_IMAGE_NOMINAL_RESOLUTION: u32 = 1 << 9;
pub(crate) const K_CG_WINDOW_LIST_OPTION_ON_SCREEN_ONLY: u32 = 1;
pub(crate) const K_CG_WINDOW_LIST_EXCLUDE_DESKTOP_ELEMENTS: u32 = 1 << 4;
pub(crate) const K_CG_NULL_WINDOW_ID: u32 = 0;
pub(crate) const K_CG_WINDOW_LAYER_NORMAL: i32 = 0;

const K_CF_NUMBER_INT32_TYPE: isize = 3;
const K_CF_NUMBER_FLOAT64_TYPE: isize = 13;
const K_CF_NUMBER_SINT64_TYPE: isize = 4;
const K_CF_STRING_ENCODING_UTF8: u32 = 0x08000100;

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
    fn CGRectMakeWithDictionaryRepresentation(dict: CFDictionaryRef, rect: *mut CGRect) -> bool;
}

#[link(name = "CoreFoundation", kind = "framework")]
extern "C" {
    pub(crate) fn CFArrayGetCount(arr: CFArrayRef) -> isize;
    pub(crate) fn CFArrayGetValueAtIndex(arr: CFArrayRef, idx: isize) -> *const c_void;
    pub(crate) fn CFRelease(cf: *const c_void);
    pub(crate) fn CFRetain(cf: *const c_void) -> *const c_void;
    pub(crate) fn CFDataGetBytePtr(data: CFDataRef) -> *const u8;
    pub(crate) fn CFDataGetLength(data: CFDataRef) -> isize;
    fn CFDictionaryGetValue(dict: CFDictionaryRef, key: *const c_void) -> *const c_void;
    fn CFNumberGetValue(num: *const c_void, the_type: isize, value_ptr: *mut c_void) -> bool;
    fn CFBooleanGetValue(boolean: *const c_void) -> bool;
    fn CFStringCreateWithBytes(
        alloc: *const c_void,
        bytes: *const u8,
        num_bytes: isize,
        encoding: u32,
        is_external: bool,
    ) -> *const c_void;
    fn CFStringGetLength(s: *const c_void) -> isize;
    fn CFStringGetMaximumSizeForEncoding(length: isize, encoding: u32) -> isize;
    fn CFStringGetCString(
        s: *const c_void,
        buffer: *mut u8,
        buffer_size: isize,
        encoding: u32,
    ) -> bool;
}

pub(crate) fn cfstr(s: &[u8]) -> *const c_void {
    unsafe {
        CFStringCreateWithBytes(
            std::ptr::null(),
            s.as_ptr(),
            s.len() as isize,
            K_CF_STRING_ENCODING_UTF8,
            false,
        )
    }
}

fn dict_get_value(dict: CFDictionaryRef, key: *const c_void) -> Option<*const c_void> {
    let val = unsafe { CFDictionaryGetValue(dict, key) };
    if val.is_null() {
        None
    } else {
        Some(val)
    }
}

pub(crate) fn dict_get_i32(dict: CFDictionaryRef, key: *const c_void) -> Option<i32> {
    let val = dict_get_value(dict, key)?;
    let mut result: i32 = 0;
    let ok = unsafe {
        CFNumberGetValue(
            val,
            K_CF_NUMBER_INT32_TYPE,
            &mut result as *mut i32 as *mut c_void,
        )
    };
    if ok {
        Some(result)
    } else {
        None
    }
}

pub(crate) fn dict_get_f64(dict: CFDictionaryRef, key: *const c_void) -> Option<f64> {
    let val = dict_get_value(dict, key)?;
    let mut result: f64 = 0.0;
    let ok = unsafe {
        CFNumberGetValue(
            val,
            K_CF_NUMBER_FLOAT64_TYPE,
            &mut result as *mut f64 as *mut c_void,
        )
    };
    if ok {
        Some(result)
    } else {
        None
    }
}

pub(crate) fn dict_get_rect(
    dict: CFDictionaryRef,
    key: *const c_void,
) -> Option<(f64, f64, f64, f64)> {
    let val = dict_get_value(dict, key)?;
    let mut rect = CGRect {
        origin: CGPoint { x: 0.0, y: 0.0 },
        size: CGSize {
            width: 0.0,
            height: 0.0,
        },
    };
    let ok = unsafe { CGRectMakeWithDictionaryRepresentation(val as CFDictionaryRef, &mut rect) };
    if ok {
        Some((
            rect.origin.x,
            rect.origin.y,
            rect.size.width,
            rect.size.height,
        ))
    } else {
        None
    }
}

pub(crate) fn dict_get_bool(dict: CFDictionaryRef, key: *const c_void) -> Option<bool> {
    let val = dict_get_value(dict, key)?;
    Some(unsafe { CFBooleanGetValue(val) })
}

pub(crate) fn dict_get_string(dict: CFDictionaryRef, key: *const c_void) -> Option<String> {
    let val = dict_get_value(dict, key)?;
    cfstring_to_string(val)
}

pub(crate) fn cfstring_to_string(s: *const c_void) -> Option<String> {
    if s.is_null() {
        return None;
    }
    let len = unsafe { CFStringGetLength(s) };
    if len <= 0 {
        return Some(String::new());
    }
    let max_bytes = unsafe { CFStringGetMaximumSizeForEncoding(len, K_CF_STRING_ENCODING_UTF8) };
    if max_bytes <= 0 {
        return None;
    }
    let mut buf = vec![0u8; (max_bytes + 1) as usize];
    if !unsafe {
        CFStringGetCString(
            s,
            buf.as_mut_ptr(),
            buf.len() as isize,
            K_CF_STRING_ENCODING_UTF8,
        )
    } {
        return None;
    }
    let end = buf.iter().position(|&b| b == 0).unwrap_or(buf.len());
    String::from_utf8(buf[..end].to_vec()).ok()
}

pub(crate) fn cfnumber_to_u32(num: *const c_void) -> Option<u32> {
    if num.is_null() {
        return None;
    }
    let mut val: i64 = 0;
    let ok = unsafe {
        CFNumberGetValue(
            num,
            K_CF_NUMBER_SINT64_TYPE,
            &mut val as *mut i64 as *mut c_void,
        )
    };
    if ok {
        Some(val as u32)
    } else {
        None
    }
}
