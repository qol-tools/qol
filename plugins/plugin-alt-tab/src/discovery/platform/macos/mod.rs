use crate::discovery::WindowInfo;
use ffi::{
    CFArrayGetCount, CFArrayGetValueAtIndex, CFDictionaryRef, CFRelease,
    CGWindowListCopyWindowInfo, K_CG_NULL_WINDOW_ID, K_CG_WINDOW_LAYER_NORMAL,
    K_CG_WINDOW_LIST_EXCLUDE_DESKTOP_ELEMENTS, K_CG_WINDOW_LIST_OPTION_ON_SCREEN_ONLY,
};
use std::ffi::c_void;
use window_enum::{
    collect_minimized_windows, collect_on_screen_windows, KnownWindowTracker, WindowEnumeration,
};

pub(crate) mod ax;
pub(crate) mod ffi;
mod process;
mod window_enum;

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

fn is_system_process(app_name: &str) -> bool {
    let app = app_name.to_ascii_lowercase();
    app.contains("screencapturekit")
        || app.contains("screensharingindicator")
        || app.contains("registerassistantservice")
}

struct CgKeys {
    layer: *const c_void,
    pid: *const c_void,
    owner: *const c_void,
    name: *const c_void,
    number: *const c_void,
    bounds: *const c_void,
    sharing: *const c_void,
    onscreen: *const c_void,
}

impl CgKeys {
    fn new() -> Self {
        Self {
            layer: ffi::cfstr(b"kCGWindowLayer"),
            pid: ffi::cfstr(b"kCGWindowOwnerPID"),
            owner: ffi::cfstr(b"kCGWindowOwnerName"),
            name: ffi::cfstr(b"kCGWindowName"),
            number: ffi::cfstr(b"kCGWindowNumber"),
            bounds: ffi::cfstr(b"kCGWindowBounds"),
            sharing: ffi::cfstr(b"kCGWindowSharingState"),
            onscreen: ffi::cfstr(b"kCGWindowIsOnscreen"),
        }
    }
}

impl Drop for CgKeys {
    fn drop(&mut self) {
        unsafe {
            CFRelease(self.layer);
            CFRelease(self.pid);
            CFRelease(self.owner);
            CFRelease(self.name);
            CFRelease(self.number);
            CFRelease(self.bounds);
            CFRelease(self.sharing);
            CFRelease(self.onscreen);
        }
    }
}

fn fetch_cg_windows(options: u32, own_pid: i32) -> Vec<CgWindow> {
    let list = unsafe { CGWindowListCopyWindowInfo(options, K_CG_NULL_WINDOW_ID) };
    if list.is_null() {
        return Vec::new();
    }
    let result = parse_cg_window_list(list, own_pid);
    unsafe { CFRelease(list as *const c_void) };
    result
}

fn parse_cg_window_list(list: *const c_void, own_pid: i32) -> Vec<CgWindow> {
    let keys = CgKeys::new();
    let count = unsafe { CFArrayGetCount(list) };
    let mut result = Vec::with_capacity(count.max(0) as usize);
    for i in 0..count {
        let dict = unsafe { CFArrayGetValueAtIndex(list, i) } as CFDictionaryRef;
        if let Some(win) = parse_cg_entry(dict, own_pid, &keys) {
            result.push(win);
        }
    }
    result
}

fn parse_cg_entry(dict: CFDictionaryRef, own_pid: i32, keys: &CgKeys) -> Option<CgWindow> {
    if dict.is_null() {
        return None;
    }
    let layer = ffi::dict_get_i32(dict, keys.layer)?;
    if layer != K_CG_WINDOW_LAYER_NORMAL {
        return None;
    }
    let pid = ffi::dict_get_i32(dict, keys.pid)?;
    if pid == own_pid {
        return None;
    }
    let app_name = ffi::dict_get_string(dict, keys.owner)
        .unwrap_or_default()
        .trim()
        .to_string();
    let title = ffi::dict_get_string(dict, keys.name)
        .unwrap_or_default()
        .trim()
        .to_string();
    let id = ffi::dict_get_i32(dict, keys.number)?;
    if app_name.is_empty() && title.is_empty() {
        return None;
    }
    if is_system_process(&app_name) {
        return None;
    }
    let (wx, wy, ww, wh) = ffi::dict_get_rect(dict, keys.bounds).unwrap_or((0.0, 0.0, 0.0, 0.0));
    let sharing_state = ffi::dict_get_i32(dict, keys.sharing).unwrap_or(0);
    let is_onscreen = ffi::dict_get_bool(dict, keys.onscreen).unwrap_or(false);
    let has_title = !title.is_empty();
    let display_title = if title.is_empty() {
        app_name.clone()
    } else {
        title
    };
    Some(CgWindow {
        id: id as u32,
        pid,
        app_name,
        title: display_title,
        has_title,
        sharing_state,
        is_onscreen,
        x: wx as f32,
        y: wy as f32,
        w: ww as f32,
        h: wh as f32,
    })
}

pub fn on_screen_window_ids() -> Vec<u32> {
    let own_pid = std::process::id() as i32;
    let opts = K_CG_WINDOW_LIST_OPTION_ON_SCREEN_ONLY | K_CG_WINDOW_LIST_EXCLUDE_DESKTOP_ELEMENTS;
    let parsed = fetch_cg_windows(opts, own_pid);
    let mut ids: Vec<u32> = parsed.into_iter().map(|w| w.id).collect();
    ids.sort_unstable();
    ids
}

pub fn get_open_windows() -> Vec<WindowInfo> {
    get_open_windows_impl(true)
}

pub fn get_on_screen_windows() -> Vec<WindowInfo> {
    let own_pid = std::process::id() as i32;
    let opts = K_CG_WINDOW_LIST_OPTION_ON_SCREEN_ONLY | K_CG_WINDOW_LIST_EXCLUDE_DESKTOP_ELEMENTS;
    let parsed = fetch_cg_windows(opts, own_pid);
    let mut state = WindowEnumeration::default();
    let mut tracker = KnownWindowTracker::new();
    let mut ax_cache = std::collections::HashMap::new();
    collect_on_screen_windows(parsed, &mut state, &mut tracker, &mut ax_cache);
    state.windows
}

fn get_open_windows_impl(include_minimized: bool) -> Vec<WindowInfo> {
    let own_pid = std::process::id() as i32;

    // ON_SCREEN_ONLY is required for correct z-ordering (most-recently-focused first).
    let on_screen_opts =
        K_CG_WINDOW_LIST_OPTION_ON_SCREEN_ONLY | K_CG_WINDOW_LIST_EXCLUDE_DESKTOP_ELEMENTS;
    let on_screen = fetch_cg_windows(on_screen_opts, own_pid);

    let mut state = WindowEnumeration::default();
    let mut tracker = KnownWindowTracker::new();
    let mut ax_cache = std::collections::HashMap::new();
    collect_on_screen_windows(on_screen, &mut state, &mut tracker, &mut ax_cache);

    if !include_minimized {
        tracker.persist();
        return state.windows;
    }
    let all_windows = fetch_cg_windows(K_CG_WINDOW_LIST_EXCLUDE_DESKTOP_ELEMENTS, own_pid);
    if all_windows.is_empty() {
        tracker.persist();
        return state.windows;
    }
    let off_screen: Vec<_> = all_windows.into_iter().filter(|w| !w.is_onscreen).collect();
    collect_minimized_windows(off_screen, &mut state, &mut tracker, &mut ax_cache);
    tracker.persist();
    state.windows
}
