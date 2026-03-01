use super::{CFRelease, CFRetain};
use super::callback::SendPtr;
use super::ensure_sc_framework;
use objc2::runtime::AnyObject;
use std::collections::HashMap;
use std::ffi::c_void;
use std::sync::Mutex;
use std::time::Duration;

/// Extract a human-readable description from an NSError* (or any ObjC object with localizedDescription).
/// Returns None if the pointer is null or the description can't be read.
pub(crate) unsafe fn objc_error_string(error: *mut AnyObject) -> Option<String> {
    use objc2::msg_send;
    if error.is_null() { return None; }
    let desc: *const AnyObject = msg_send![error, localizedDescription];
    if desc.is_null() { return None; }
    let cstr: *const i8 = msg_send![desc, UTF8String];
    if cstr.is_null() { return None; }
    Some(std::ffi::CStr::from_ptr(cstr).to_string_lossy().into_owned())
}

/// CMTime for minimumFrameInterval.
#[repr(C)]
#[derive(Copy, Clone)]
pub(crate) struct CMTime {
    pub value: i64,
    pub timescale: i32,
    pub flags: u32,
    pub epoch: i64,
}

unsafe impl objc2::Encode for CMTime {
    const ENCODING: objc2::Encoding = objc2::Encoding::Struct(
        "?",
        &[
            objc2::Encoding::LongLong,  // value: i64
            objc2::Encoding::Int,       // timescale: i32
            objc2::Encoding::UInt,      // flags: u32
            objc2::Encoding::LongLong,  // epoch: i64
        ],
    );
}

unsafe impl objc2::RefEncode for CMTime {
    const ENCODING_REF: objc2::Encoding =
        objc2::Encoding::Pointer(&<Self as objc2::Encode>::ENCODING);
}

pub(crate) const CM_TIME_5FPS: CMTime = CMTime {
    value: 1,
    timescale: 5,
    flags: 1, // kCMTimeFlags_Valid
    epoch: 0,
};

pub(crate) const CM_TIME_30FPS: CMTime = CMTime {
    value: 1,
    timescale: 30,
    flags: 1, // kCMTimeFlags_Valid
    epoch: 0,
};

/// Cached SCShareableContent from the latest heartbeat fetch.
pub(crate) static SC_CONTENT_CACHE: Mutex<Option<SendPtr>> = Mutex::new(None);

pub(crate) fn install_crash_diagnostics() {
    use std::sync::Once;
    static INIT_DIAG: Once = Once::new();
    INIT_DIAG.call_once(|| {
        std::panic::set_hook(Box::new(|info| {
            eprintln!("[alt-tab] PANIC: {}", info);
        }));
        extern "C" fn sig_handler(sig: i32) {
            extern "C" {
                fn write(fd: i32, buf: *const u8, count: usize) -> isize;
                fn _exit(status: i32);
            }
            let msg: &[u8] = match sig {
                11 => b"[alt-tab] CRASH: SIGSEGV\n",
                6 => b"[alt-tab] CRASH: SIGABRT\n",
                10 => b"[alt-tab] CRASH: SIGBUS\n",
                5 => b"[alt-tab] CRASH: SIGTRAP\n",
                _ => b"[alt-tab] CRASH: unknown signal\n",
            };
            unsafe {
                write(2, msg.as_ptr(), msg.len());
                _exit(128 + sig);
            }
        }
        extern "C" {
            fn signal(sig: i32, handler: extern "C" fn(i32)) -> usize;
        }
        unsafe {
            signal(11, sig_handler);
            signal(6, sig_handler);
            signal(10, sig_handler);
            signal(5, sig_handler);
        }
        eprintln!("[alt-tab/sc] crash diagnostics installed");
    });
}

/// Fetch SCShareableContent asynchronously and cache it.
/// Returns a retained pointer — caller MUST CFRelease when done.
/// Returns null on failure or timeout.
pub(crate) fn sc_fetch_content() -> *mut c_void {
    use objc2::msg_send;
    use objc2::runtime::AnyClass;
    use std::sync::mpsc;

    if !ensure_sc_framework() {
        return std::ptr::null_mut();
    }

    let (tx, rx) = mpsc::sync_channel(1);
    let block = block2::RcBlock::new(move |obj: *mut AnyObject, _error: *mut AnyObject| {
        let ptr = obj as *mut c_void;
        if !ptr.is_null() {
            unsafe { CFRetain(ptr as *const c_void) };
        }
        if tx.send(ptr).is_err() && !ptr.is_null() {
            unsafe { CFRelease(ptr as *const c_void) };
        }
    });
    unsafe {
        let cls = AnyClass::get(c"SCShareableContent").unwrap();
        let _: () = msg_send![
            cls,
            getShareableContentExcludingDesktopWindows: false,
            onScreenWindowsOnly: false,
            completionHandler: &*block,
        ];
    }
    let content_ptr = match rx.recv_timeout(Duration::from_secs(3)) {
        Ok(p) if !p.is_null() => p,
        _ => return std::ptr::null_mut(),
    };

    // Store a second retained copy in the global cache
    unsafe { CFRetain(content_ptr as *const c_void) };
    if let Some(old) = SC_CONTENT_CACHE
        .lock()
        .ok()
        .and_then(|mut cache| cache.replace(SendPtr(content_ptr)))
        .filter(|p| !p.0.is_null())
    {
        unsafe { CFRelease(old.0 as *const c_void) };
    }

    content_ptr
}

/// Build CGWindowID → SCWindow* map from SCShareableContent.
pub(crate) fn build_sc_window_map(content_ptr: *const c_void) -> HashMap<u32, *const AnyObject> {
    use objc2::msg_send;

    let content = content_ptr as *const AnyObject;
    let windows: *const AnyObject = unsafe { msg_send![content, windows] };
    let count: usize = unsafe { msg_send![windows, count] };
    let mut sc_map: HashMap<u32, *const AnyObject> = HashMap::with_capacity(count);
    for i in 0..count {
        let win: *const AnyObject = unsafe { msg_send![windows, objectAtIndex: i] };
        let wid: u32 = unsafe { msg_send![win, windowID] };
        sc_map.insert(wid, win);
    }
    sc_map
}

/// Build a configured SCStreamConfiguration object. Caller must CFRelease.
pub(crate) fn build_sc_config(max_w: usize, max_h: usize, frame_interval: CMTime) -> *mut AnyObject {
    use objc2::msg_send;
    use objc2::runtime::AnyClass;
    unsafe {
        let cls = AnyClass::get(c"SCStreamConfiguration").unwrap();
        let obj: *mut AnyObject = msg_send![cls, new];
        let _: () = msg_send![obj, setWidth: max_w];
        let _: () = msg_send![obj, setHeight: max_h];
        let _: () = msg_send![obj, setScalesToFit: true];
        let _: () = msg_send![obj, setPixelFormat: 0x3432_3066u32]; // YUV420 BiPlanar Full Range
        let _: () = msg_send![obj, setMinimumFrameInterval: frame_interval];
        let _: () = msg_send![obj, setShowsCursor: false];
        obj
    }
}
