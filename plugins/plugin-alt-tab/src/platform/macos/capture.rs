use crate::platform::cg_helpers;
use crate::platform::{RgbaImage, WindowInfo};
use std::collections::HashMap;
use std::ffi::c_void;

use super::{
    CFArrayGetCount, CFArrayGetValueAtIndex, CFDataGetBytePtr, CFDataGetLength, CFRelease,
    CGDataProviderCopyData, CGImageGetBytesPerRow, CGImageGetDataProvider, CGImageGetHeight,
    CGImageGetWidth, CGWindowListCreateImage, CGWindowListCopyWindowInfo,
    CFDictionaryRef, CGImageRef,
    CG_RECT_NULL, K_CG_NULL_WINDOW_ID,
    K_CG_WINDOW_IMAGE_BOUNDS_IGNORE_FRAMING, K_CG_WINDOW_IMAGE_NOMINAL_RESOLUTION,
    K_CG_WINDOW_LIST_EXCLUDE_DESKTOP_ELEMENTS, K_CG_WINDOW_LIST_OPTION_INCLUDING_WINDOW,
};

const ICON_SIZE: usize = 32;

pub(super) fn get_app_icons(windows: &[WindowInfo]) -> HashMap<String, RgbaImage> {
    let own_pid = std::process::id() as i32;
    let opts = K_CG_WINDOW_LIST_EXCLUDE_DESKTOP_ELEMENTS;
    let list = unsafe { CGWindowListCopyWindowInfo(opts, K_CG_NULL_WINDOW_ID) };
    if list.is_null() {
        return HashMap::new();
    }

    let key_pid = cg_helpers::cfstr(b"kCGWindowOwnerPID");
    let key_owner = cg_helpers::cfstr(b"kCGWindowOwnerName");

    let mut app_pids: HashMap<String, i32> = HashMap::new();
    let count = unsafe { CFArrayGetCount(list) };
    for i in 0..count {
        let dict = unsafe { CFArrayGetValueAtIndex(list, i) } as CFDictionaryRef;
        if dict.is_null() {
            continue;
        }
        let Some(pid) = cg_helpers::dict_get_i32(dict, key_pid) else { continue };
        if pid == own_pid {
            continue;
        }
        let name = cg_helpers::dict_get_string(dict, key_owner)
            .unwrap_or_default()
            .trim()
            .to_string();
        if !name.is_empty() {
            app_pids.entry(name).or_insert(pid);
        }
    }

    unsafe {
        CFRelease(list as *const c_void);
        CFRelease(key_pid as *const c_void);
        CFRelease(key_owner as *const c_void);
    }

    let needed: std::collections::HashSet<&str> =
        windows.iter().map(|w| w.app_name.as_str()).collect();

    let mut icons = HashMap::new();
    for (name, pid) in &app_pids {
        if !needed.contains(name.as_str()) {
            continue;
        }
        if let Some(icon) = qol_plugin_api::app_icon::icon_for_pid(*pid, ICON_SIZE) {
            icons.insert(name.clone(), icon);
        }
    }
    icons
}

pub(super) fn capture_previews_cg(
    targets: &[(usize, u32)],
    max_w: usize,
    max_h: usize,
) -> Vec<(usize, Option<RgbaImage>)> {
    std::thread::scope(|s| {
        let handles: Vec<_> = targets
            .iter()
            .map(|&(idx, wid)| {
                s.spawn(move || {
                    let result = cg_capture_window(wid, max_w, max_h);
                    (idx, result)
                })
            })
            .collect();
        handles
            .into_iter()
            .filter_map(|h| h.join().ok())
            .collect()
    })
}

fn cg_capture_window(wid: u32, max_w: usize, max_h: usize) -> Option<RgbaImage> {
    let img = unsafe {
        CGWindowListCreateImage(
            CG_RECT_NULL,
            K_CG_WINDOW_LIST_OPTION_INCLUDING_WINDOW,
            wid,
            K_CG_WINDOW_IMAGE_BOUNDS_IGNORE_FRAMING | K_CG_WINDOW_IMAGE_NOMINAL_RESOLUTION,
        )
    };
    if img.is_null() {
        return None;
    }
    let result = extract_bgra_from_raw_cgimage(img, max_w, max_h);
    unsafe { CFRelease(img) };
    result
}

fn extract_bgra_from_raw_cgimage(img: CGImageRef, max_w: usize, max_h: usize) -> Option<RgbaImage> {
    let src_w = unsafe { CGImageGetWidth(img) };
    let src_h = unsafe { CGImageGetHeight(img) };
    if src_w == 0 || src_h == 0 {
        return None;
    }

    let provider = unsafe { CGImageGetDataProvider(img) };
    if provider.is_null() {
        return None;
    }

    let cf_data = unsafe { CGDataProviderCopyData(provider) };
    if cf_data.is_null() {
        return None;
    }

    let ptr = unsafe { CFDataGetBytePtr(cf_data) };
    let len = unsafe { CFDataGetLength(cf_data) } as usize;
    let raw = unsafe { std::slice::from_raw_parts(ptr, len) };
    let bytes_per_row = unsafe { CGImageGetBytesPerRow(img) };

    let scale = (max_w as f32 / src_w as f32).min(max_h as f32 / src_h as f32).min(1.0);
    let scaled_w = ((src_w as f32 * scale).round() as usize).max(1).min(max_w);
    let scaled_h = ((src_h as f32 * scale).round() as usize).max(1).min(max_h);
    let offset_x = (max_w - scaled_w) / 2;
    let offset_y = (max_h - scaled_h) / 2;

    let mut bgra = vec![0u8; max_w * max_h * 4];
    for y in 0..scaled_h {
        let src_y = (y * src_h) / scaled_h;
        let row_start = src_y * bytes_per_row;
        for x in 0..scaled_w {
            let src_x = (x * src_w) / scaled_w;
            let src_off = row_start + src_x * 4;
            if src_off + 4 > len {
                continue;
            }
            let dst_off = ((offset_y + y) * max_w + offset_x + x) * 4;
            bgra[dst_off..dst_off + 4].copy_from_slice(&raw[src_off..src_off + 4]);
        }
    }

    unsafe { CFRelease(cf_data) };
    Some(RgbaImage { data: bgra, width: max_w, height: max_h })
}
