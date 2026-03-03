use std::ffi::{c_void, CStr, CString};

// -- C struct types --

#[repr(C)]
pub(super) struct CGPoint {
    pub x: f64,
    pub y: f64,
}

#[repr(C)]
pub(super) struct CGSize {
    pub width: f64,
    pub height: f64,
}

#[repr(C)]
pub(super) struct CGRect {
    pub origin: CGPoint,
    pub size: CGSize,
}

// -- Constants --

pub(super) const AX_VALUE_TYPE_CG_POINT: u32 = 1;
pub(super) const AX_VALUE_TYPE_CG_SIZE: u32 = 2;
const UTF8: u32 = 0x08000100;

pub(super) const CG_WINDOW_LIST_OPTION_ON_SCREEN_ONLY: u32 = 1;
pub(super) const CG_WINDOW_LIST_EXCLUDE_DESKTOP: u32 = 1 << 4;
pub(super) const CF_NUMBER_SINT32_TYPE: i32 = 3;

// -- Framework bindings --

#[link(name = "ApplicationServices", kind = "framework")]
extern "C" {
    pub(super) fn AXUIElementCreateApplication(pid: i32) -> *mut c_void;
    pub(super) fn AXUIElementCopyAttributeValue(
        element: *const c_void,
        attribute: *const c_void,
        value: *mut *mut c_void,
    ) -> i32;
    pub(super) fn AXUIElementSetAttributeValue(
        element: *const c_void,
        attribute: *const c_void,
        value: *const c_void,
    ) -> i32;
    pub(super) fn AXUIElementPerformAction(
        element: *const c_void,
        action: *const c_void,
    ) -> i32;
    pub(super) fn AXValueCreate(value_type: u32, value: *const c_void) -> *mut c_void;
    pub(super) fn AXValueGetValue(
        value: *const c_void,
        value_type: u32,
        value_ptr: *mut c_void,
    ) -> u8;
}

#[link(name = "CoreFoundation", kind = "framework")]
extern "C" {
    fn CFStringCreateWithCString(
        allocator: *const c_void,
        c_str: *const i8,
        encoding: u32,
    ) -> *mut c_void;
    fn CFStringGetLength(string: *const c_void) -> isize;
    fn CFStringGetCString(
        string: *const c_void,
        buffer: *mut i8,
        buffer_size: isize,
        encoding: u32,
    ) -> u8;
    fn CFGetTypeID(cf: *const c_void) -> usize;
    fn CFStringGetTypeID() -> usize;
    pub(super) fn CFArrayGetCount(array: *const c_void) -> isize;
    pub(super) fn CFArrayGetValueAtIndex(array: *const c_void, idx: isize) -> *const c_void;
    pub(super) fn CFDictionaryGetValue(
        dict: *const c_void,
        key: *const c_void,
    ) -> *const c_void;
    pub(super) fn CFNumberGetValue(
        number: *const c_void,
        the_type: i32,
        value_ptr: *mut c_void,
    ) -> u8;
    pub(super) fn CFRelease(cf: *const c_void);
    #[link_name = "kCFBooleanFalse"]
    static CF_BOOLEAN_FALSE: *const c_void;
    #[link_name = "kCFBooleanTrue"]
    static CF_BOOLEAN_TRUE: *const c_void;
}

pub(super) fn cf_boolean_false() -> *const c_void { unsafe { CF_BOOLEAN_FALSE } }
pub(super) fn cf_boolean_true() -> *const c_void { unsafe { CF_BOOLEAN_TRUE } }

#[link(name = "CoreGraphics", kind = "framework")]
extern "C" {
    pub(super) fn CGWindowListCopyWindowInfo(option: u32, relative_to: u32) -> *mut c_void;
    #[link_name = "kCGWindowLayer"]
    static CG_WINDOW_LAYER: *const c_void;
    #[link_name = "kCGWindowOwnerPID"]
    static CG_WINDOW_OWNER_PID: *const c_void;
}

pub(super) fn cg_window_layer() -> *const c_void { unsafe { CG_WINDOW_LAYER } }
pub(super) fn cg_window_owner_pid() -> *const c_void { unsafe { CG_WINDOW_OWNER_PID } }

#[link(name = "objc", kind = "dylib")]
extern "C" {
    fn objc_getClass(name: *const i8) -> *mut c_void;
    fn sel_registerName(name: *const i8) -> *mut c_void;
    fn objc_msgSend(obj: *mut c_void, sel: *mut c_void, ...) -> *mut c_void;
}

// Link AppKit for NSScreen / NSWorkspace.
#[link(name = "AppKit", kind = "framework")]
extern "C" {}

#[cfg(target_arch = "x86_64")]
extern "C" {
    fn objc_msgSend_stret(stret: *mut c_void, obj: *mut c_void, sel: *mut c_void, ...);
}

// -- RAII guard for CF types --

pub(super) struct CfGuard(*mut c_void);

impl CfGuard {
    pub fn new(ptr: *mut c_void) -> Option<Self> {
        if ptr.is_null() {
            return None;
        }
        Some(Self(ptr))
    }

    pub fn as_ptr(&self) -> *mut c_void {
        self.0
    }

    pub fn as_const(&self) -> *const c_void {
        self.0
    }
}

impl Drop for CfGuard {
    fn drop(&mut self) {
        unsafe { CFRelease(self.0) }
    }
}

// -- AX attribute helper --

pub(super) fn ax_attr(element: *const c_void, name: &str) -> Option<CfGuard> {
    unsafe {
        let attr = cfstr(name);
        let mut value: *mut c_void = std::ptr::null_mut();
        let err = AXUIElementCopyAttributeValue(element, attr, &mut value);
        CFRelease(attr);
        if err != 0 {
            return None;
        }
        CfGuard::new(value)
    }
}

// -- ObjC runtime helpers --

pub(super) fn cfstr(s: &str) -> *mut c_void {
    let c = CString::new(s).unwrap();
    unsafe { CFStringCreateWithCString(std::ptr::null(), c.as_ptr(), UTF8) }
}

pub(super) fn sel(name: &str) -> *mut c_void {
    let c = CString::new(name).unwrap();
    unsafe { sel_registerName(c.as_ptr()) }
}

pub(super) fn cls(name: &str) -> *mut c_void {
    let c = CString::new(name).unwrap();
    unsafe { objc_getClass(c.as_ptr()) }
}

pub(super) unsafe fn msg_ptr(obj: *mut c_void, sel: *mut c_void) -> *mut c_void {
    objc_msgSend(obj, sel)
}

pub(super) unsafe fn msg_i32(obj: *mut c_void, sel: *mut c_void) -> i32 {
    let f: unsafe extern "C" fn(*mut c_void, *mut c_void) -> i32 =
        std::mem::transmute(objc_msgSend as usize);
    f(obj, sel)
}

pub(super) unsafe fn msg_usize(obj: *mut c_void, sel: *mut c_void) -> usize {
    let f: unsafe extern "C" fn(*mut c_void, *mut c_void) -> usize =
        std::mem::transmute(objc_msgSend as usize);
    f(obj, sel)
}

pub(super) unsafe fn msg_ptr_usize(
    obj: *mut c_void,
    sel: *mut c_void,
    arg: usize,
) -> *mut c_void {
    let f: unsafe extern "C" fn(*mut c_void, *mut c_void, usize) -> *mut c_void =
        std::mem::transmute(objc_msgSend as usize);
    f(obj, sel, arg)
}

pub(super) unsafe fn msg_bool_usize(
    obj: *mut c_void,
    sel: *mut c_void,
    arg: usize,
) -> bool {
    let f: unsafe extern "C" fn(*mut c_void, *mut c_void, usize) -> i8 =
        std::mem::transmute(objc_msgSend as usize);
    f(obj, sel, arg) != 0
}

#[cfg(target_arch = "aarch64")]
pub(super) unsafe fn msg_rect(obj: *mut c_void, sel: *mut c_void) -> CGRect {
    let f: unsafe extern "C" fn(*mut c_void, *mut c_void) -> CGRect =
        std::mem::transmute(objc_msgSend as usize);
    f(obj, sel)
}

#[cfg(target_arch = "x86_64")]
pub(super) unsafe fn msg_rect(obj: *mut c_void, sel: *mut c_void) -> CGRect {
    let mut rect: CGRect = std::mem::zeroed();
    objc_msgSend_stret(
        &mut rect as *mut _ as *mut c_void,
        obj,
        sel,
    );
    rect
}

// -- CF conversion helpers --

pub(super) fn cfstring_to_string(cf: *const c_void) -> Option<String> {
    if cf.is_null() {
        return None;
    }
    unsafe {
        if CFGetTypeID(cf) != CFStringGetTypeID() {
            return None;
        }
        let len = CFStringGetLength(cf);
        let buf_size = len * 4 + 1;
        let mut buf = vec![0i8; buf_size as usize];
        if CFStringGetCString(cf, buf.as_mut_ptr(), buf_size, UTF8) == 0 {
            return None;
        }
        let cstr = CStr::from_ptr(buf.as_ptr());
        cstr.to_str().ok().map(|s| s.to_string())
    }
}

pub(super) fn dict_get_i32(dict: *const c_void, key: *const c_void) -> Option<i32> {
    unsafe {
        let val = CFDictionaryGetValue(dict, key);
        if val.is_null() {
            return None;
        }
        let mut out: i32 = 0;
        if CFNumberGetValue(val, CF_NUMBER_SINT32_TYPE, &mut out as *mut _ as *mut c_void) == 0 {
            return None;
        }
        Some(out)
    }
}
