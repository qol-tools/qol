//! CoreGraphics dictionary utilities for macOS platform modules.
//! Only compiled on macOS.

#![cfg(target_os = "macos")]

use std::ffi::c_void;

type CFDictionaryRef = *const c_void;
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
    struct CGPoint { x: f64, y: f64 }
    #[repr(C)]
    struct CGSize { width: f64, height: f64 }
    #[repr(C)]
    struct CGRect { origin: CGPoint, size: CGSize }

    #[link(name = "CoreGraphics", kind = "framework")]
    extern "C" {
        fn CGRectMakeWithDictionaryRepresentation(dict: CFDictionaryRef, rect: *mut CGRect) -> bool;
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
        let mut rect = CGRect {
            origin: CGPoint { x: 0.0, y: 0.0 },
            size: CGSize { width: 0.0, height: 0.0 },
        };
        if CGRectMakeWithDictionaryRepresentation(val as CFDictionaryRef, &mut rect) {
            Some((rect.origin.x, rect.origin.y, rect.size.width, rect.size.height))
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
