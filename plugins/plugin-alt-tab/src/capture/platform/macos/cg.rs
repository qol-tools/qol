use super::extract_cgimage_pixels;
use crate::discovery::macos::ffi::{
    CFRelease, CGWindowListCreateImage, CG_RECT_NULL, K_CG_WINDOW_IMAGE_BOUNDS_IGNORE_FRAMING,
    K_CG_WINDOW_IMAGE_NOMINAL_RESOLUTION, K_CG_WINDOW_LIST_OPTION_INCLUDING_WINDOW,
};
use qol_app_icon::RgbaImage;

pub(super) fn capture(
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
                    let result = capture_window(wid, max_w, max_h);
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
        "backend=cg targets={} total={}ms",
        targets.len(),
        t_all.elapsed().as_millis()
    );
    results
}

fn capture_window(wid: u32, max_w: usize, max_h: usize) -> Option<RgbaImage> {
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
