use crate::discovery::macos::ffi;
use crate::discovery::macos::ffi::{
    CFArrayGetCount, CFArrayGetValueAtIndex, CFDataGetBytePtr, CFDataGetLength, CFDictionaryRef,
    CFRelease, CGDataProviderCopyData, CGImageGetBytesPerRow, CGImageGetDataProvider,
    CGImageGetHeight, CGImageGetWidth, CGImageRef, CGWindowListCreateImage, CG_RECT_NULL,
    K_CG_NULL_WINDOW_ID, K_CG_WINDOW_IMAGE_BOUNDS_IGNORE_FRAMING,
    K_CG_WINDOW_IMAGE_NOMINAL_RESOLUTION, K_CG_WINDOW_LIST_EXCLUDE_DESKTOP_ELEMENTS,
    K_CG_WINDOW_LIST_OPTION_INCLUDING_WINDOW,
};
use crate::discovery::WindowInfo;
use qol_app_icon::RgbaImage;
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
        let Some(icon) = qol_app_icon::icon_for_pid(*pid, ICON_SIZE) else {
            continue;
        };
        icons.insert(name.clone(), icon);
    }
    icons
}

fn resolve_app_pids() -> HashMap<String, i32> {
    let own_pid = std::process::id() as i32;
    let list = ffi::copy_window_list_timed(
        K_CG_WINDOW_LIST_EXCLUDE_DESKTOP_ELEMENTS,
        K_CG_NULL_WINDOW_ID,
        "icon_pids",
    );
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
                            qol_runtime::probe!(
                                "CAPTURE_SLOW",
                                "wid={wid} ms={ms} ok={}",
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
    qol_runtime::probe!(
        "CAPTURE",
        "targets={} total={}ms",
        targets.len(),
        t_all.elapsed().as_millis()
    );
    results
}

fn cg_capture_window(wid: u32, max_w: usize, max_h: usize) -> Option<RgbaImage> {
    #[cfg(debug_assertions)]
    let t_create = std::time::Instant::now();
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
    #[cfg(debug_assertions)]
    let create_ms = t_create.elapsed().as_millis();
    let result = extract_cgimage_pixels(img, max_w, max_h);
    #[cfg(debug_assertions)]
    {
        let total_ms = t_create.elapsed().as_millis();
        if total_ms >= 100 {
            qol_runtime::probe!(
                "CAPTURE_SPLIT",
                "wid={wid} create={create_ms}ms extract={}ms",
                total_ms - create_ms
            );
        }
    }
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
    };
    let cover = compute_cover_rect(src_w, src_h, max_w, max_h);
    Some(blit_cover(&src, &cover))
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
}

struct CoverRect {
    src_x: usize,
    src_y: usize,
    src_w: usize,
    src_h: usize,
    dst_w: usize,
    dst_h: usize,
}

fn compute_cover_rect(src_w: usize, src_h: usize, dst_w: usize, dst_h: usize) -> CoverRect {
    let src_aspect = src_w as f64 / src_h as f64;
    let dst_aspect = dst_w as f64 / dst_h as f64;
    if src_aspect > dst_aspect {
        let crop_w = ((src_h as f64 * dst_aspect).round() as usize)
            .max(1)
            .min(src_w);
        return CoverRect {
            src_x: (src_w - crop_w) / 2,
            src_y: 0,
            src_w: crop_w,
            src_h,
            dst_w,
            dst_h,
        };
    }

    let crop_h = ((src_w as f64 / dst_aspect).round() as usize)
        .max(1)
        .min(src_h);
    CoverRect {
        src_x: 0,
        src_y: (src_h - crop_h) / 2,
        src_w,
        src_h: crop_h,
        dst_w,
        dst_h,
    }
}

fn blit_cover(src: &BlitSource, rect: &CoverRect) -> RgbaImage {
    let mut bgra = vec![0u8; rect.dst_w * rect.dst_h * 4];
    for y in 0..rect.dst_h {
        let src_y = rect.src_y + (y * rect.src_h) / rect.dst_h;
        let src_row = src_y * src.bytes_per_row;
        let dst_row = y * rect.dst_w;
        blit_cover_row(src, rect, src_row, &mut bgra, dst_row);
    }
    RgbaImage {
        data: bgra,
        width: rect.dst_w,
        height: rect.dst_h,
    }
}

fn blit_cover_row(
    src: &BlitSource,
    rect: &CoverRect,
    src_row: usize,
    dst: &mut [u8],
    dst_row: usize,
) {
    for x in 0..rect.dst_w {
        let src_x = rect.src_x + (x * rect.src_w) / rect.dst_w;
        let s = src_row + src_x * 4;
        if s + 4 > src.data.len() {
            continue;
        }
        let d = (dst_row + x) * 4;
        dst[d..d + 4].copy_from_slice(&src.data[s..s + 4]);
    }
}

#[cfg(test)]
mod tests {
    use super::compute_cover_rect;

    #[test]
    fn cover_rect_crops_height_for_tall_source() {
        let rect = compute_cover_rect(1600, 1000, 542, 305);
        assert_eq!(rect.src_x, 0);
        assert!(rect.src_h < 1000);
        assert_eq!(rect.dst_w, 542);
        assert_eq!(rect.dst_h, 305);
    }

    #[test]
    fn cover_rect_crops_width_for_wide_source() {
        let rect = compute_cover_rect(2400, 1000, 542, 305);
        assert_eq!(rect.src_y, 0);
        assert!(rect.src_w < 2400);
        assert_eq!(rect.dst_w, 542);
        assert_eq!(rect.dst_h, 305);
    }
}
