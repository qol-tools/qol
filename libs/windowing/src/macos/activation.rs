use std::ffi::c_void;
use std::sync::OnceLock;

extern "C" {
    fn dlopen(path: *const std::ffi::c_char, mode: i32) -> *mut c_void;
    fn dlsym(handle: *mut c_void, symbol: *const std::ffi::c_char) -> *mut c_void;
}

const RTLD_NOW: i32 = 2;

#[repr(C)]
#[derive(Clone, Copy)]
struct ProcessSerialNumber {
    high: u32,
    low: u32,
}

type SetFrontFn = unsafe extern "C" fn(*const ProcessSerialNumber, u32, u32) -> i32;
type PostEventFn = unsafe extern "C" fn(*const ProcessSerialNumber, *const u8) -> i32;

struct SkyLight {
    set_front: SetFrontFn,
    post_event: PostEventFn,
}

#[link(name = "ApplicationServices", kind = "framework")]
extern "C" {
    fn GetProcessForPID(pid: i32, psn: *mut ProcessSerialNumber) -> i32;
    fn AXUIElementCreateApplication(pid: i32) -> *const c_void;
    fn AXUIElementSetAttributeValue(
        el: *const c_void,
        attr: *const c_void,
        val: *const c_void,
    ) -> i32;
}

#[link(name = "CoreFoundation", kind = "framework")]
extern "C" {
    fn CFStringCreateWithBytes(
        alloc: *const c_void,
        bytes: *const u8,
        num_bytes: isize,
        encoding: u32,
        is_external_representation: bool,
    ) -> *const c_void;
    fn CFRelease(cf: *const c_void);
    static kCFBooleanTrue: *const c_void;
}

const K_CF_STRING_ENCODING_UTF8: u32 = 0x08000100;

fn cfstr(bytes: &[u8]) -> *const c_void {
    unsafe {
        CFStringCreateWithBytes(
            std::ptr::null(),
            bytes.as_ptr(),
            bytes.len() as isize,
            K_CF_STRING_ENCODING_UTF8,
            false,
        )
    }
}

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

pub fn set_front(pid: i32, wid: u32) -> bool {
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

pub fn ax_app_frontmost(pid: i32) {
    unsafe {
        let app = AXUIElementCreateApplication(pid);
        if app.is_null() {
            return;
        }
        let attr = cfstr(b"AXFrontmost");
        let _ = AXUIElementSetAttributeValue(app, attr, kCFBooleanTrue);
        CFRelease(attr);
        CFRelease(app);
    }
}

pub fn ns_activate_app(pid: i32) -> bool {
    objc2::rc::autoreleasepool(|_| {
        use objc2_app_kit::{NSApplicationActivationOptions, NSRunningApplication};
        #[allow(deprecated)]
        NSRunningApplication::runningApplicationWithProcessIdentifier(pid).is_some_and(|app| {
            app.activateWithOptions(NSApplicationActivationOptions::ActivateIgnoringOtherApps)
        })
    })
}
