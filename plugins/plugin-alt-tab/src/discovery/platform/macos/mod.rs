use ffi::{
    CFArrayGetCount, CFArrayGetValueAtIndex, CFRelease, CFDictionaryRef,
    CGWindowListCopyWindowInfo,
    K_CG_NULL_WINDOW_ID, K_CG_WINDOW_LAYER_NORMAL,
    K_CG_WINDOW_LIST_EXCLUDE_DESKTOP_ELEMENTS, K_CG_WINDOW_LIST_OPTION_ON_SCREEN_ONLY,
};
use crate::discovery::WindowInfo;
use std::ffi::c_void;
use window_enum::{
    collect_minimized_windows, collect_on_screen_windows, KnownWindowTracker, WindowEnumeration,
};

pub(crate) mod ax;
pub(crate) mod ffi;
mod process;
mod window_enum;

/// Parsed CG window entry.
pub(super) struct CgWindow {
    pub id: u32,
    pub pid: i32,
    pub app_name: String,
    pub title: String,
    pub has_title: bool,
    pub sharing_state: i32,
    pub is_onscreen: bool,
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
}

impl CgWindow {
    pub fn into_window_info(self, is_minimized: bool) -> WindowInfo {
        WindowInfo {
            id: self.id,
            title: self.title,
            app_name: self.app_name,
            preview_path: None,
            icon: None,
            x: self.x,
            y: self.y,
            width: self.w,
            height: self.h,
            is_minimized,
        }
    }

    pub fn to_window_info(&self, is_minimized: bool, title: String) -> WindowInfo {
        WindowInfo {
            id: self.id,
            title,
            app_name: self.app_name.clone(),
            preview_path: None,
            icon: None,
            x: self.x,
            y: self.y,
            width: self.w,
            height: self.h,
            is_minimized,
        }
    }
}

/// Fast pre-filter for known system process windows.
fn is_system_process(app_name: &str) -> bool {
    let app = app_name.to_ascii_lowercase();
    app.contains("screencapturekit")
        || app.contains("screensharingindicator")
        || app.contains("registerassistantservice")
}

/// Shared helper: parse normal-layer windows from a CG window list.
fn parse_cg_window_list(list: *const c_void, own_pid: i32) -> Vec<CgWindow> {
    let key_layer = ffi::cfstr(b"kCGWindowLayer");
    let key_pid = ffi::cfstr(b"kCGWindowOwnerPID");
    let key_owner = ffi::cfstr(b"kCGWindowOwnerName");
    let key_name = ffi::cfstr(b"kCGWindowName");
    let key_number = ffi::cfstr(b"kCGWindowNumber");
    let key_bounds = ffi::cfstr(b"kCGWindowBounds");
    let key_sharing = ffi::cfstr(b"kCGWindowSharingState");
    let key_is_onscreen = ffi::cfstr(b"kCGWindowIsOnscreen");

    let count = unsafe { CFArrayGetCount(list) };
    let mut result: Vec<CgWindow> = Vec::with_capacity(count.max(0) as usize);

    for i in 0..count {
        let dict = unsafe { CFArrayGetValueAtIndex(list, i) } as CFDictionaryRef;
        if dict.is_null() {
            continue;
        }
        let Some(layer) = ffi::dict_get_i32(dict, key_layer) else {
            continue;
        };
        if layer != K_CG_WINDOW_LAYER_NORMAL {
            continue;
        }
        let Some(pid) = ffi::dict_get_i32(dict, key_pid) else {
            continue;
        };
        if pid == own_pid {
            continue;
        }
        let app_name = ffi::dict_get_string(dict, key_owner)
            .unwrap_or_default()
            .trim()
            .to_string();
        let title = ffi::dict_get_string(dict, key_name)
            .unwrap_or_default()
            .trim()
            .to_string();
        let Some(id) = ffi::dict_get_i32(dict, key_number) else {
            continue;
        };
        if app_name.is_empty() && title.is_empty() {
            continue;
        }
        let (wx, wy, ww, wh) = ffi::dict_get_rect(dict, key_bounds)
            .unwrap_or((0.0, 0.0, 0.0, 0.0));
        if is_system_process(&app_name) {
            continue;
        }
        let sharing_state = ffi::dict_get_i32(dict, key_sharing).unwrap_or(0);
        let is_onscreen = ffi::dict_get_bool(dict, key_is_onscreen).unwrap_or(false);
        let has_title = !title.is_empty();
        let display_title = if title.is_empty() { app_name.clone() } else { title };
        result.push(CgWindow {
            id: id as u32, pid, app_name, title: display_title, has_title, sharing_state,
            is_onscreen, x: wx as f32, y: wy as f32, w: ww as f32, h: wh as f32,
        });
    }

    unsafe {
        CFRelease(key_layer as *const c_void);
        CFRelease(key_pid as *const c_void);
        CFRelease(key_owner as *const c_void);
        CFRelease(key_name as *const c_void);
        CFRelease(key_number as *const c_void);
        CFRelease(key_bounds as *const c_void);
        CFRelease(key_sharing as *const c_void);
        CFRelease(key_is_onscreen as *const c_void);
    }

    result
}

pub fn get_open_windows() -> Vec<WindowInfo> {
    get_open_windows_impl(true)
}

/// Cheap CG-only snapshot of on-screen window IDs (no AX, no proc_pidinfo).
pub fn on_screen_window_ids() -> Vec<u32> {
    let own_pid = std::process::id() as i32;
    let options =
        K_CG_WINDOW_LIST_OPTION_ON_SCREEN_ONLY | K_CG_WINDOW_LIST_EXCLUDE_DESKTOP_ELEMENTS;
    let list = unsafe { CGWindowListCopyWindowInfo(options, K_CG_NULL_WINDOW_ID) };
    if list.is_null() {
        return Vec::new();
    }

    let key_layer = ffi::cfstr(b"kCGWindowLayer");
    let key_pid = ffi::cfstr(b"kCGWindowOwnerPID");
    let key_number = ffi::cfstr(b"kCGWindowNumber");

    let count = unsafe { CFArrayGetCount(list) };
    let mut ids = Vec::with_capacity(count.max(0) as usize);

    for i in 0..count {
        let dict = unsafe { CFArrayGetValueAtIndex(list, i) } as CFDictionaryRef;
        if dict.is_null() {
            continue;
        }
        let Some(layer) = ffi::dict_get_i32(dict, key_layer) else {
            continue;
        };
        if layer != K_CG_WINDOW_LAYER_NORMAL {
            continue;
        }
        let Some(pid) = ffi::dict_get_i32(dict, key_pid) else {
            continue;
        };
        if pid == own_pid {
            continue;
        }
        let Some(id) = ffi::dict_get_i32(dict, key_number) else {
            continue;
        };
        ids.push(id as u32);
    }

    unsafe {
        CFRelease(key_layer as *const c_void);
        CFRelease(key_pid as *const c_void);
        CFRelease(key_number as *const c_void);
        CFRelease(list as *const c_void);
    }

    ids.sort_unstable();
    ids
}

/// Fast path: CG window list + regular-app filter + AX dedup.
pub fn get_on_screen_windows() -> Vec<WindowInfo> {
    let own_pid = std::process::id() as i32;
    let options =
        K_CG_WINDOW_LIST_OPTION_ON_SCREEN_ONLY | K_CG_WINDOW_LIST_EXCLUDE_DESKTOP_ELEMENTS;
    let list = unsafe { CGWindowListCopyWindowInfo(options, K_CG_NULL_WINDOW_ID) };
    if list.is_null() {
        return Vec::new();
    }
    let parsed = parse_cg_window_list(list, own_pid);
    unsafe { CFRelease(list as *const c_void) };

    let mut state = WindowEnumeration::default();
    let mut tracker = KnownWindowTracker::new();
    let mut ax_cache = std::collections::HashMap::new();
    collect_on_screen_windows(parsed, &mut state, &mut tracker, &mut ax_cache);
    state.windows
}

fn get_open_windows_impl(include_minimized: bool) -> Vec<WindowInfo> {
    let own_pid = std::process::id() as i32;

    // ON_SCREEN_ONLY is required for correct z-ordering (most-recently-focused first).
    // Without it, CGWindowListCopyWindowInfo returns a stable but non-z-ordered list.
    let on_screen_opts =
        K_CG_WINDOW_LIST_OPTION_ON_SCREEN_ONLY | K_CG_WINDOW_LIST_EXCLUDE_DESKTOP_ELEMENTS;
    let list = unsafe { CGWindowListCopyWindowInfo(on_screen_opts, K_CG_NULL_WINDOW_ID) };
    if list.is_null() {
        return Vec::new();
    }
    let on_screen = parse_cg_window_list(list, own_pid);
    unsafe { CFRelease(list as *const c_void) };

    let mut state = WindowEnumeration::default();
    let mut tracker = KnownWindowTracker::new();
    let mut ax_cache = std::collections::HashMap::new();
    collect_on_screen_windows(on_screen, &mut state, &mut tracker, &mut ax_cache);

    if include_minimized {
        let all_opts = K_CG_WINDOW_LIST_EXCLUDE_DESKTOP_ELEMENTS;
        let list = unsafe { CGWindowListCopyWindowInfo(all_opts, K_CG_NULL_WINDOW_ID) };
        if list.is_null() {
            tracker.persist();
            return state.windows;
        }
        let all_windows = parse_cg_window_list(list, own_pid);
        unsafe { CFRelease(list as *const c_void) };
        let off_screen: Vec<_> = all_windows.into_iter()
            .filter(|w| !w.is_onscreen)
            .collect();
        collect_minimized_windows(off_screen, &mut state, &mut tracker, &mut ax_cache);
    }
    tracker.persist();

    state.windows
}
