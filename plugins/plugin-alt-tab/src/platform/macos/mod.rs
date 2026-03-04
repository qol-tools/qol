use super::WindowInfo;
use std::ffi::c_void;

pub(super) type CFArrayRef = *const c_void;
pub(super) type CFDictionaryRef = *const c_void;
pub(super) type CGImageRef = *const c_void;
pub(super) type CFDataRef = *const c_void;
pub(super) type CGDataProviderRef = *const c_void;

#[repr(C)]
#[derive(Copy, Clone)]
pub(super) struct CGRect {
    pub origin: CGPoint,
    pub size: CGSize,
}

#[repr(C)]
#[derive(Copy, Clone)]
pub(super) struct CGPoint {
    pub x: f64,
    pub y: f64,
}

#[repr(C)]
#[derive(Copy, Clone)]
pub(super) struct CGSize {
    pub width: f64,
    pub height: f64,
}

pub(super) const CG_RECT_NULL: CGRect = CGRect {
    origin: CGPoint { x: f64::INFINITY, y: f64::INFINITY },
    size: CGSize { width: 0.0, height: 0.0 },
};
pub(super) const K_CG_WINDOW_LIST_OPTION_INCLUDING_WINDOW: u32 = 1 << 3;
pub(super) const K_CG_WINDOW_IMAGE_BOUNDS_IGNORE_FRAMING: u32 = 1 << 0;
pub(super) const K_CG_WINDOW_IMAGE_NOMINAL_RESOLUTION: u32 = 1 << 9;
pub(super) const K_CG_WINDOW_LIST_OPTION_ON_SCREEN_ONLY: u32 = 1;
pub(super) const K_CG_WINDOW_LIST_EXCLUDE_DESKTOP_ELEMENTS: u32 = 1 << 4;
pub(super) const K_CG_NULL_WINDOW_ID: u32 = 0;
pub(super) const K_CG_WINDOW_LAYER_NORMAL: i32 = 0;

#[link(name = "CoreGraphics", kind = "framework")]
extern "C" {
    pub(super) fn CGWindowListCopyWindowInfo(option: u32, relative_to: u32) -> CFArrayRef;
    pub(super) fn CGWindowListCreateImage(
        screen_bounds: CGRect,
        list_option: u32,
        window_id: u32,
        image_option: u32,
    ) -> CGImageRef;
    pub(super) fn CGImageGetWidth(image: CGImageRef) -> usize;
    pub(super) fn CGImageGetHeight(image: CGImageRef) -> usize;
    pub(super) fn CGImageGetBytesPerRow(image: CGImageRef) -> usize;
    #[allow(dead_code)]
    pub(super) fn CGImageGetBitsPerPixel(image: CGImageRef) -> usize;
    pub(super) fn CGImageGetDataProvider(image: CGImageRef) -> CGDataProviderRef;
    pub(super) fn CGDataProviderCopyData(provider: CGDataProviderRef) -> CFDataRef;
}

#[link(name = "CoreFoundation", kind = "framework")]
extern "C" {
    pub(super) fn CFArrayGetCount(arr: CFArrayRef) -> isize;
    pub(super) fn CFArrayGetValueAtIndex(arr: CFArrayRef, idx: isize) -> *const c_void;
    pub(super) fn CFRelease(cf: *const c_void);
    pub(super) fn CFRetain(cf: *const c_void) -> *const c_void;
    pub(super) fn CFDataGetBytePtr(data: CFDataRef) -> *const u8;
    pub(super) fn CFDataGetLength(data: CFDataRef) -> isize;
}

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
    pub fn into_window_info(self, is_minimized: bool) -> crate::platform::WindowInfo {
        crate::platform::WindowInfo {
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

    pub fn to_window_info(&self, is_minimized: bool, title: String) -> crate::platform::WindowInfo {
        crate::platform::WindowInfo {
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

#[derive(Clone)]
pub(super) struct AxWindowMeta {
    pub title: String,
    pub is_minimized: bool,
}

mod ax;
mod capture;
mod picker;
mod process;
mod sc;
mod window_actions;
mod window_enum;
mod window_list;

pub use sc::SendCVBuf;

pub fn get_open_windows() -> Vec<WindowInfo> {
    window_list::get_open_windows()
}

pub fn on_screen_window_ids() -> Vec<u32> {
    window_list::on_screen_window_ids()
}

pub fn get_on_screen_windows() -> Vec<WindowInfo> {
    window_list::get_on_screen_windows()
}

pub fn capture_previews_cg(
    targets: &[(usize, u32)],
    max_w: usize,
    max_h: usize,
) -> Vec<(usize, Option<super::RgbaImage>)> {
    capture::capture_previews_cg(targets, max_w, max_h)
}

pub fn get_app_icons(
    windows: &[WindowInfo],
) -> std::collections::HashMap<String, super::RgbaImage> {
    capture::get_app_icons(windows)
}

pub fn sc_available() -> bool {
    sc::sc_available()
}

pub fn sc_start_streams(targets: &[(usize, u32)], max_w: usize, max_h: usize) {
    sc::sc_start_streams(targets, max_w, max_h)
}

pub fn sc_start_streams_with_content(targets: &[(usize, u32)], max_w: usize, max_h: usize) {
    sc::sc_start_streams_with_content(targets, max_w, max_h)
}

pub fn sc_fetch_content() -> *mut std::ffi::c_void {
    sc::sc_fetch_content()
}

pub fn sc_snapshot_window(content_ptr: *mut std::ffi::c_void, wid: u32, max_w: usize, max_h: usize) -> bool {
    sc::sc_snapshot_window(content_ptr, wid, max_w, max_h)
}

pub fn sc_prewarm_wids() -> std::collections::HashSet<u32> {
    sc::sc_prewarm_wids()
}

pub fn sc_live_frame_wids() -> std::collections::HashSet<u32> {
    sc::sc_live_frame_wids()
}

pub fn sc_clone_opener_surfaces() -> std::collections::HashMap<u32, SendCVBuf> {
    sc::sc_clone_opener_surfaces()
}

pub fn sc_heartbeat_snapshot(wids: &[u32], max_w: usize, max_h: usize) {
    sc::sc_heartbeat_snapshot(wids, max_w, max_h)
}

pub fn sc_stop_streams() {
    sc::sc_stop_streams()
}

pub fn sc_streams_active() -> bool {
    sc::sc_streams_active()
}

pub fn sc_promote_stream(wid: u32, max_w: usize, max_h: usize) {
    sc::sc_promote_stream(wid, max_w, max_h)
}

pub fn sc_demote_stream(wid: u32, max_w: usize, max_h: usize) {
    sc::sc_demote_stream(wid, max_w, max_h)
}

pub fn sc_has_new_frames() -> bool {
    sc::sc_has_new_frames()
}

pub fn sc_callback_stats() -> (u64, u64, u64) {
    sc::sc_callback_stats()
}

pub fn sc_take_prewarm_surfaces() -> std::collections::HashMap<u32, SendCVBuf> {
    sc::sc_take_prewarm_surfaces()
}

pub fn sc_prewarm_retain(live_ids: &std::collections::HashSet<u32>) {
    sc::sc_prewarm_retain(live_ids)
}

pub fn sc_take_frames() -> std::collections::HashMap<u32, SendCVBuf> {
    sc::sc_take_frames()
}

pub fn activate_window(window_id: u32) {
    window_actions::activate_window(window_id)
}

pub fn close_window(window_id: u32) {
    window_actions::close_window(window_id)
}

pub fn quit_app(window_id: u32) {
    window_actions::quit_app(window_id)
}

pub fn minimize_window_by_id(window_id: u32) {
    window_actions::minimize_window_by_id(window_id)
}

pub fn move_app_window(title: &str, x: i32, y: i32) -> bool {
    use crate::platform::cg_helpers;

    #[link(name = "ApplicationServices", kind = "framework")]
    extern "C" {
        fn AXUIElementCreateApplication(pid: i32) -> *const c_void;
        fn AXUIElementCopyAttributeValue(
            el: *const c_void,
            attr: *const c_void,
            val: *mut *const c_void,
        ) -> i32;
        fn AXUIElementSetAttributeValue(
            el: *const c_void,
            attr: *const c_void,
            val: *const c_void,
        ) -> i32;
        fn AXValueCreate(value_type: u32, value: *const c_void) -> *const c_void;
    }

    const AX_VALUE_TYPE_CG_POINT: u32 = 1;
    let own_pid = std::process::id() as i32;

    unsafe {
        let app = AXUIElementCreateApplication(own_pid);
        if app.is_null() {
            return false;
        }

        let windows_attr = cg_helpers::cfstr(b"AXWindows");
        let mut windows_value: *const c_void = std::ptr::null();
        let err = AXUIElementCopyAttributeValue(app, windows_attr, &mut windows_value);
        CFRelease(windows_attr as *const c_void);
        if err != 0 || windows_value.is_null() {
            CFRelease(app);
            return false;
        }

        let title_attr = cg_helpers::cfstr(b"AXTitle");
        let count = CFArrayGetCount(windows_value as CFArrayRef);
        let mut target_win: *const c_void = std::ptr::null();

        for i in 0..count {
            let win = CFArrayGetValueAtIndex(windows_value as CFArrayRef, i);
            if win.is_null() {
                continue;
            }
            let mut title_val: *const c_void = std::ptr::null();
            let terr = AXUIElementCopyAttributeValue(win, title_attr, &mut title_val);
            if terr != 0 || title_val.is_null() {
                if !title_val.is_null() { CFRelease(title_val); }
                continue;
            }
            let ax_title = cg_helpers::cfstring_to_string(title_val).unwrap_or_default();
            CFRelease(title_val);
            if ax_title != title {
                continue;
            }
            target_win = CFRetain(win);
            break;
        }

        CFRelease(title_attr as *const c_void);
        CFRelease(windows_value);
        CFRelease(app);

        if target_win.is_null() {
            return false;
        }

        let pos = CGPoint { x: x as f64, y: y as f64 };
        let pos_val = AXValueCreate(AX_VALUE_TYPE_CG_POINT, &pos as *const _ as *const c_void);
        if pos_val.is_null() {
            CFRelease(target_win);
            return false;
        }

        let ax_pos = cg_helpers::cfstr(b"AXPosition");
        let result = AXUIElementSetAttributeValue(target_win, ax_pos, pos_val);
        CFRelease(ax_pos as *const c_void);
        CFRelease(pos_val);
        CFRelease(target_win);

        #[cfg(debug_assertions)]
        eprintln!("[alt-tab/move] move_app_window({:?}, {}, {}) = {}", title, x, y, result == 0);

        result == 0
    }
}

pub fn picker_window_kind() -> gpui::WindowKind {
    picker::picker_window_kind()
}

pub fn dismiss_picker(window: &mut gpui::Window) {
    picker::dismiss_picker(window)
}

pub fn reposition_picker(gpui_x: f64, gpui_y: f64) -> bool {
    picker::reposition_picker(gpui_x, gpui_y)
}

pub fn is_modifier_held() -> bool {
    picker::is_modifier_held()
}

pub fn is_shift_held() -> bool {
    picker::is_shift_held()
}

pub fn disable_window_shadow() {
    picker::disable_window_shadow()
}
