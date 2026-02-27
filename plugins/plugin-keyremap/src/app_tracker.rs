use std::ffi::c_void;
use std::sync::{Arc, RwLock};
use std::time::Duration;

const POLL_INTERVAL: Duration = Duration::from_millis(100);

pub struct AppTracker {
    bundle_id: Arc<RwLock<String>>,
}

impl AppTracker {
    pub fn start() -> Arc<Self> {
        let bundle_id = Arc::new(RwLock::new(
            frontmost_bundle_id().unwrap_or_default(),
        ));

        let poll_ref = Arc::clone(&bundle_id);
        std::thread::spawn(move || loop {
            if let Some(id) = frontmost_bundle_id() {
                if let Ok(mut guard) = poll_ref.write() {
                    *guard = id;
                }
            }
            std::thread::sleep(POLL_INTERVAL);
        });

        Arc::new(Self { bundle_id })
    }

    pub fn bundle_id(&self) -> String {
        self.bundle_id.read().map(|g| g.clone()).unwrap_or_default()
    }
}

/// Get the frontmost app's bundle ID via the window server.
///
/// `NSWorkspace.frontmostApplication()` caches state and only refreshes when
/// the thread's Cocoa run loop processes workspace notifications — a plain
/// background thread never receives those updates so the value goes stale.
///
/// `CGWindowListCopyWindowInfo` queries the window server directly and always
/// returns a live result regardless of thread or run-loop context.
fn frontmost_bundle_id() -> Option<String> {
    use core_foundation::base::TCFType;
    use core_foundation::string::CFString;
    use objc2::rc::autoreleasepool;
    use objc2_app_kit::NSRunningApplication;

    extern "C" {
        fn CGWindowListCopyWindowInfo(option: u32, relative_to: u32) -> *const c_void;
        fn CFArrayGetCount(array: *const c_void) -> isize;
        fn CFArrayGetValueAtIndex(array: *const c_void, idx: isize) -> *const c_void;
        fn CFDictionaryGetValue(dict: *const c_void, key: *const c_void) -> *const c_void;
        fn CFNumberGetValue(num: *const c_void, num_type: u32, out: *mut c_void) -> bool;
        fn CFRelease(cf: *const c_void);
    }

    const ON_SCREEN_ONLY: u32 = 1;
    const EXCLUDE_DESKTOP: u32 = 1 << 4;
    const CF_NUMBER_INT_TYPE: u32 = 9;

    unsafe {
        let list = CGWindowListCopyWindowInfo(ON_SCREEN_ONLY | EXCLUDE_DESKTOP, 0);
        if list.is_null() {
            return None;
        }

        let layer_key = CFString::new("kCGWindowLayer");
        let pid_key = CFString::new("kCGWindowOwnerPID");
        let count = CFArrayGetCount(list);
        let mut result = None;

        for i in 0..count {
            let dict = CFArrayGetValueAtIndex(list, i);
            if dict.is_null() {
                continue;
            }

            let layer_val =
                CFDictionaryGetValue(dict, layer_key.as_concrete_TypeRef() as *const c_void);
            if layer_val.is_null() {
                continue;
            }
            let mut layer: i32 = -1;
            CFNumberGetValue(
                layer_val,
                CF_NUMBER_INT_TYPE,
                &mut layer as *mut i32 as *mut c_void,
            );
            if layer != 0 {
                continue;
            }

            let pid_val =
                CFDictionaryGetValue(dict, pid_key.as_concrete_TypeRef() as *const c_void);
            if pid_val.is_null() {
                continue;
            }
            let mut pid: i32 = 0;
            CFNumberGetValue(
                pid_val,
                CF_NUMBER_INT_TYPE,
                &mut pid as *mut i32 as *mut c_void,
            );
            if pid <= 0 {
                continue;
            }

            result = autoreleasepool(|_| {
                let app = NSRunningApplication::runningApplicationWithProcessIdentifier(pid)?;
                let bundle_id = app.bundleIdentifier()?;
                Some(bundle_id.to_string())
            });
            break;
        }

        CFRelease(list);
        result
    }
}
