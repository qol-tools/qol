use crate::discovery::macos::ffi;
use crate::discovery::macos::ffi::{
    CFArrayGetCount, CFArrayGetValueAtIndex, CFDataGetBytePtr, CFDataGetLength, CFDictionaryRef,
    CFRelease, CGDataProviderCopyData, CGImageGetBytesPerRow, CGImageGetDataProvider,
    CGImageGetHeight, CGImageGetWidth, CGImageRef, CGWindowListCopyWindowInfo,
    CGWindowListCreateImage, CG_RECT_NULL, K_CG_NULL_WINDOW_ID,
    K_CG_WINDOW_IMAGE_BOUNDS_IGNORE_FRAMING, K_CG_WINDOW_IMAGE_NOMINAL_RESOLUTION,
    K_CG_WINDOW_LIST_EXCLUDE_DESKTOP_ELEMENTS, K_CG_WINDOW_LIST_OPTION_INCLUDING_WINDOW,
};
use crate::discovery::WindowInfo;
use qol_plugin_api::app_icon::RgbaImage;
use std::collections::HashMap;
use std::ffi::c_void;

const ICON_SIZE: usize = 32;

pub fn get_app_icons(windows: &[WindowInfo]) -> HashMap<String, RgbaImage> {
    let needed: std::collections::HashSet<&str> =
        windows.iter().map(|w| w.app_name.as_str()).collect();
    let app_pids = resolve_app_pids();
    let mut icons = HashMap::new();
    for (name, pid) in &app_pids {
        if !needed.contains(name.as_str()) {
            continue;
        }
        let Some(icon) = qol_plugin_api::app_icon::icon_for_pid(*pid, ICON_SIZE) else {
            continue;
        };
        icons.insert(name.clone(), icon);
    }
    icons
}

fn resolve_app_pids() -> HashMap<String, i32> {
    let own_pid = std::process::id() as i32;
    let list = unsafe {
        CGWindowListCopyWindowInfo(
            K_CG_WINDOW_LIST_EXCLUDE_DESKTOP_ELEMENTS,
            K_CG_NULL_WINDOW_ID,
        )
    };
    if list.is_null() {
        return HashMap::new();
    }
    let pids = collect_pids_from_list(list, own_pid);
    unsafe { CFRelease(list) };
    pids
}

fn collect_pids_from_list(list: ffi::CFArrayRef, own_pid: i32) -> HashMap<String, i32> {
    let key_pid = ffi::cfstr(b"kCGWindowOwnerPID");
    let key_owner = ffi::cfstr(b"kCGWindowOwnerName");
    let mut pids: HashMap<String, i32> = HashMap::new();

    let count = unsafe { CFArrayGetCount(list) };
    for i in 0..count {
        if let Some((name, pid)) = extract_owner(list, i, key_pid, key_owner, own_pid) {
            pids.entry(name).or_insert(pid);
        }
    }

    unsafe {
        CFRelease(key_pid);
        CFRelease(key_owner);
    }
    pids
}

fn extract_owner(
    list: ffi::CFArrayRef,
    i: isize,
    key_pid: *const c_void,
    key_owner: *const c_void,
    own_pid: i32,
) -> Option<(String, i32)> {
    let dict = unsafe { CFArrayGetValueAtIndex(list, i) } as CFDictionaryRef;
    if dict.is_null() {
        return None;
    }
    let pid = ffi::dict_get_i32(dict, key_pid)?;
    if pid == own_pid {
        return None;
    }
    let name = ffi::dict_get_string(dict, key_owner)
        .unwrap_or_default()
        .trim()
        .to_string();
    if name.is_empty() {
        return None;
    }
    Some((name, pid))
}

pub fn capture_previews_cg(
    targets: &[(usize, u32)],
    max_w: usize,
    max_h: usize,
) -> Vec<(usize, Option<RgbaImage>)> {
    #[cfg(debug_assertions)]
    let t_all = std::time::Instant::now();
    let results: Vec<_> = std::thread::scope(|s| {
        let handles: Vec<_> = targets
            .iter()
            .map(|&(idx, wid)| {
                s.spawn(move || {
                    #[cfg(debug_assertions)]
                    let t = std::time::Instant::now();
                    let result = cg_capture_window(wid, max_w, max_h);
                    #[cfg(debug_assertions)]
                    {
                        let ms = t.elapsed().as_millis();
                        if ms >= 100 {
                            eprintln!(
                                "[alt-tab/capture] SLOW cg_capture_window wid={} {}ms ok={}",
                                wid,
                                ms,
                                result.is_some()
                            );
                        }
                    }
                    (idx, result)
                })
            })
            .collect();
        handles.into_iter().filter_map(|h| h.join().ok()).collect()
    });
    #[cfg(debug_assertions)]
    eprintln!(
        "[alt-tab/capture] capture_previews_cg targets={} total={}ms",
        targets.len(),
        t_all.elapsed().as_millis()
    );
    results
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
    let result = extract_cgimage_pixels(img, max_w, max_h);
    unsafe { CFRelease(img) };
    result
}

fn extract_cgimage_pixels(img: CGImageRef, max_w: usize, max_h: usize) -> Option<RgbaImage> {
    let (src_w, src_h) = unsafe { (CGImageGetWidth(img), CGImageGetHeight(img)) };
    if src_w == 0 || src_h == 0 {
        return None;
    }
    let raw = copy_cgimage_data(img)?;
    let src = BlitSource {
        data: &raw,
        bytes_per_row: unsafe { CGImageGetBytesPerRow(img) },
        w: src_w,
        h: src_h,
    };
    let scaled = compute_scaled_rect(src_w, src_h, max_w, max_h);
    Some(blit_scaled(&src, &scaled))
}

fn copy_cgimage_data(img: CGImageRef) -> Option<Vec<u8>> {
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
    let bytes = unsafe { std::slice::from_raw_parts(ptr, len) }.to_vec();
    unsafe { CFRelease(cf_data) };
    Some(bytes)
}

struct BlitSource<'a> {
    data: &'a [u8],
    bytes_per_row: usize,
    w: usize,
    h: usize,
}

struct ScaledRect {
    w: usize,
    h: usize,
    offset_x: usize,
    offset_y: usize,
    canvas_w: usize,
    canvas_h: usize,
}

fn compute_scaled_rect(src_w: usize, src_h: usize, max_w: usize, max_h: usize) -> ScaledRect {
    let scale = (max_w as f32 / src_w as f32)
        .min(max_h as f32 / src_h as f32)
        .min(1.0);
    let w = ((src_w as f32 * scale).round() as usize).max(1).min(max_w);
    let h = ((src_h as f32 * scale).round() as usize).max(1).min(max_h);
    ScaledRect {
        w,
        h,
        offset_x: (max_w - w) / 2,
        offset_y: (max_h - h) / 2,
        canvas_w: max_w,
        canvas_h: max_h,
    }
}

fn blit_scaled(src: &BlitSource, rect: &ScaledRect) -> RgbaImage {
    let mut bgra = vec![0u8; rect.canvas_w * rect.canvas_h * 4];
    for y in 0..rect.h {
        let src_row = (y * src.h) / rect.h * src.bytes_per_row;
        let dst_row = (rect.offset_y + y) * rect.canvas_w + rect.offset_x;
        blit_row(src, src_row, rect.w, &mut bgra, dst_row);
    }
    RgbaImage {
        data: bgra,
        width: rect.canvas_w,
        height: rect.canvas_h,
    }
}

fn blit_row(src: &BlitSource, src_row: usize, dst_w: usize, dst: &mut [u8], dst_row: usize) {
    for x in 0..dst_w {
        let s = src_row + (x * src.w / dst_w) * 4;
        if s + 4 > src.data.len() {
            continue;
        }
        let d = (dst_row + x) * 4;
        dst[d..d + 4].copy_from_slice(&src.data[s..s + 4]);
    }
}
