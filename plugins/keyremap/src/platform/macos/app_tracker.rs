use std::ffi::c_void;
use std::sync::{Arc, RwLock};
use std::time::Duration;

const POLL_INTERVAL: Duration = Duration::from_millis(100);

#[derive(Clone, Default)]
struct AppSnapshot {
    pid: i32,
    bundle_id: String,
}

pub struct AppTracker {
    snapshot: Arc<RwLock<AppSnapshot>>,
}

impl AppTracker {
    pub fn start() -> Arc<Self> {
        let snapshot = Arc::new(RwLock::new(frontmost_app().unwrap_or_default()));

        let poll_ref = Arc::clone(&snapshot);
        std::thread::spawn(move || loop {
            if let Some(snapshot) = frontmost_app() {
                if let Ok(mut guard) = poll_ref.write() {
                    *guard = snapshot;
                }
            }
            std::thread::sleep(POLL_INTERVAL);
        });

        Arc::new(Self { snapshot })
    }

    pub fn bundle_id_for_target(&self, target_pid: i32) -> String {
        self.snapshot
            .read()
            .map(|snapshot| {
                bundle_id_for_event_target(snapshot.pid, &snapshot.bundle_id, target_pid).to_owned()
            })
            .unwrap_or_default()
    }
}

fn bundle_id_for_event_target(
    frontmost_pid: i32,
    frontmost_bundle_id: &str,
    target_pid: i32,
) -> &str {
    if target_pid > 0 && target_pid != frontmost_pid {
        return "";
    }
    frontmost_bundle_id
}

fn frontmost_app() -> Option<AppSnapshot> {
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

            let bundle_id = autoreleasepool(|_| {
                NSRunningApplication::runningApplicationWithProcessIdentifier(pid)
                    .and_then(|app| app.bundleIdentifier())
                    .map(|bundle_id| bundle_id.to_string())
                    .unwrap_or_default()
            });
            result = Some(AppSnapshot { pid, bundle_id });
            break;
        }

        CFRelease(list);
        result
    }
}

#[cfg(test)]
mod tests {
    use super::bundle_id_for_event_target;

    #[test]
    fn event_target_does_not_inherit_another_process_exclusion() {
        let cases = [
            (10, "com.example.excluded", 10, "com.example.excluded"),
            (10, "com.example.excluded", 20, ""),
            (10, "com.example.excluded", 0, "com.example.excluded"),
            (20, "", 20, ""),
        ];

        for (frontmost_pid, frontmost_bundle, target_pid, expected) in cases {
            assert_eq!(
                bundle_id_for_event_target(frontmost_pid, frontmost_bundle, target_pid),
                expected
            );
        }
    }
}
