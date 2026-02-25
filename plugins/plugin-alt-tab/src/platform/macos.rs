use super::cg_helpers;
use super::preview;
use super::RgbaImage;
use super::WindowInfo;
use objc2::{AnyThread, Message};
use std::ffi::c_void;
use std::sync::mpsc;
use std::time::Duration;

type CFArrayRef = *const c_void;
type CFDictionaryRef = *const c_void;
type CGImageRef = *const c_void;
type CFDataRef = *const c_void;
type CGDataProviderRef = *const c_void;

#[repr(C)]
#[derive(Copy, Clone)]
struct CGRect {
    origin: CGPoint,
    size: CGSize,
}

#[repr(C)]
#[derive(Copy, Clone)]
struct CGPoint {
    x: f64,
    y: f64,
}

#[repr(C)]
#[derive(Copy, Clone)]
struct CGSize {
    width: f64,
    height: f64,
}

const CG_RECT_NULL: CGRect = CGRect {
    origin: CGPoint { x: f64::INFINITY, y: f64::INFINITY },
    size: CGSize { width: 0.0, height: 0.0 },
};
const K_CG_WINDOW_LIST_OPTION_INCLUDING_WINDOW: u32 = 1 << 3;
const K_CG_WINDOW_IMAGE_BOUNDS_IGNORE_FRAMING: u32 = 1 << 0;
const K_CG_WINDOW_IMAGE_NOMINAL_RESOLUTION: u32 = 1 << 9;

#[link(name = "CoreGraphics", kind = "framework")]
extern "C" {
    fn CGWindowListCopyWindowInfo(option: u32, relative_to: u32) -> CFArrayRef;
    fn CGWindowListCreateImage(
        screen_bounds: CGRect,
        list_option: u32,
        window_id: u32,
        image_option: u32,
    ) -> CGImageRef;
    fn CGImageGetWidth(image: CGImageRef) -> usize;
    fn CGImageGetHeight(image: CGImageRef) -> usize;
    fn CGImageGetBytesPerRow(image: CGImageRef) -> usize;
    fn CGImageGetBitsPerPixel(image: CGImageRef) -> usize;
    fn CGImageGetDataProvider(image: CGImageRef) -> CGDataProviderRef;
    fn CGDataProviderCopyData(provider: CGDataProviderRef) -> CFDataRef;
}

#[link(name = "CoreFoundation", kind = "framework")]
extern "C" {
    fn CFArrayGetCount(arr: CFArrayRef) -> isize;
    fn CFArrayGetValueAtIndex(arr: CFArrayRef, idx: isize) -> *const c_void;
    fn CFRelease(cf: *const c_void);
    fn CFDataGetBytePtr(data: CFDataRef) -> *const u8;
    fn CFDataGetLength(data: CFDataRef) -> isize;
}

const K_CG_WINDOW_LIST_OPTION_ON_SCREEN_ONLY: u32 = 1;
const K_CG_WINDOW_LIST_EXCLUDE_DESKTOP_ELEMENTS: u32 = 1 << 4;
const K_CG_NULL_WINDOW_ID: u32 = 0;
const K_CG_WINDOW_LAYER_NORMAL: i32 = 0;

pub fn get_open_windows() -> Vec<WindowInfo> {
    let own_pid = std::process::id() as i32;
    let opts =
        K_CG_WINDOW_LIST_OPTION_ON_SCREEN_ONLY | K_CG_WINDOW_LIST_EXCLUDE_DESKTOP_ELEMENTS;
    let list = unsafe { CGWindowListCopyWindowInfo(opts, K_CG_NULL_WINDOW_ID) };
    if list.is_null() {
        return Vec::new();
    }

    let key_layer = cg_helpers::cfstr(b"kCGWindowLayer");
    let key_pid = cg_helpers::cfstr(b"kCGWindowOwnerPID");
    let key_owner = cg_helpers::cfstr(b"kCGWindowOwnerName");
    let key_name = cg_helpers::cfstr(b"kCGWindowName");
    let key_number = cg_helpers::cfstr(b"kCGWindowNumber");
    let key_bounds = cg_helpers::cfstr(b"kCGWindowBounds");

    let count = unsafe { CFArrayGetCount(list) };
    let mut windows = Vec::with_capacity(count.max(0) as usize);

    for i in 0..count {
        let dict = unsafe { CFArrayGetValueAtIndex(list, i) } as CFDictionaryRef;
        if dict.is_null() {
            continue;
        }
        let Some(layer) = cg_helpers::dict_get_i32(dict, key_layer) else {
            continue;
        };
        if layer != K_CG_WINDOW_LAYER_NORMAL {
            continue;
        }
        let Some(pid) = cg_helpers::dict_get_i32(dict, key_pid) else {
            continue;
        };
        if pid == own_pid {
            continue;
        }
        let app_name = cg_helpers::dict_get_string(dict, key_owner)
            .unwrap_or_default()
            .trim()
            .to_string();
        let title = cg_helpers::dict_get_string(dict, key_name)
            .unwrap_or_default()
            .trim()
            .to_string();
        let Some(id) = cg_helpers::dict_get_i32(dict, key_number) else {
            continue;
        };
        if app_name.is_empty() && title.is_empty() {
            continue;
        }
        let display_title = if title.is_empty() {
            app_name.clone()
        } else {
            title
        };
        let (wx, wy, ww, wh) = cg_helpers::dict_get_rect(dict, key_bounds)
            .unwrap_or((0.0, 0.0, 0.0, 0.0));
        windows.push(WindowInfo {
            id: id as u32,
            title: display_title,
            app_name,
            preview_path: None,
            x: wx as f32,
            y: wy as f32,
            width: ww as f32,
            height: wh as f32,
        });
    }

    unsafe {
        CFRelease(list as *const c_void);
        CFRelease(key_layer as *const c_void);
        CFRelease(key_pid as *const c_void);
        CFRelease(key_owner as *const c_void);
        CFRelease(key_name as *const c_void);
        CFRelease(key_number as *const c_void);
    }

    windows
}

use std::sync::atomic::{AtomicBool, Ordering};

static SCK_AVAILABLE: AtomicBool = AtomicBool::new(true);

pub fn capture_preview(window_id: u32, max_w: usize, max_h: usize) -> Option<String> {
    if !SCK_AVAILABLE.load(Ordering::Relaxed) {
        return None;
    }

    let (tx, rx) = mpsc::channel::<Option<RgbaImage>>();

    std::thread::spawn(move || {
        let result = sck_capture_on_thread(window_id, max_w, max_h);
        let _ = tx.send(result);
    });

    match rx.recv_timeout(Duration::from_millis(2000)) {
        Ok(Some(rgba)) => {
            preview::downscale_and_save_preview(
                window_id, &rgba.data, rgba.width, rgba.height, max_w, max_h,
            )
        }
        Ok(None) => None,
        Err(_) => {
            eprintln!("[alt-tab/macos] SCK capture timed out — disabling for this session");
            SCK_AVAILABLE.store(false, Ordering::Relaxed);
            None
        }
    }
}

pub fn capture_previews_batch(
    targets: &[(usize, u32)],
    max_w: usize,
    max_h: usize,
) -> Vec<(usize, Option<String>)> {
    if !SCK_AVAILABLE.load(Ordering::Relaxed) {
        return targets.iter().map(|&(idx, _)| (idx, None)).collect();
    }
    std::thread::scope(|s| {
        let handles: Vec<_> = targets
            .iter()
            .map(|&(idx, wid)| {
                s.spawn(move || {
                    let path = sck_capture_on_thread(wid, max_w, max_h).and_then(|rgba| {
                        preview::downscale_and_save_preview(
                            wid, &rgba.data, rgba.width, rgba.height, max_w, max_h,
                        )
                    });
                    (idx, path)
                })
            })
            .collect();
        handles
            .into_iter()
            .filter_map(|h| h.join().ok())
            .collect()
    })
}

pub fn capture_preview_rgba(window_id: u32, max_w: usize, max_h: usize) -> Option<RgbaImage> {
    if !SCK_AVAILABLE.load(Ordering::Relaxed) {
        return None;
    }

    let (tx, rx) = mpsc::channel::<Option<RgbaImage>>();
    std::thread::spawn(move || {
        let result = sck_capture_on_thread(window_id, max_w, max_h);
        let _ = tx.send(result);
    });

    match rx.recv_timeout(Duration::from_millis(2000)) {
        Ok(result) => result,
        Err(_) => {
            eprintln!("[alt-tab/macos] SCK capture timed out — disabling for this session");
            SCK_AVAILABLE.store(false, Ordering::Relaxed);
            None
        }
    }
}

pub fn capture_previews_cg(
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

pub fn capture_previews_batch_rgba(
    targets: &[(usize, u32)],
    max_w: usize,
    max_h: usize,
) -> Vec<(usize, Option<RgbaImage>)> {
    if !SCK_AVAILABLE.load(Ordering::Relaxed) {
        return targets.iter().map(|&(idx, _)| (idx, None)).collect();
    }
    std::thread::scope(|s| {
        let handles: Vec<_> = targets
            .iter()
            .map(|&(idx, wid)| {
                s.spawn(move || {
                    let result = sck_capture_on_thread(wid, max_w, max_h);
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

fn sck_capture_on_thread(window_id: u32, max_w: usize, max_h: usize) -> Option<RgbaImage> {
    use objc2_screen_capture_kit::{
        SCContentFilter, SCScreenshotManager, SCShareableContent, SCStreamConfiguration,
    };

    let (content_tx, content_rx) = mpsc::channel();
    let content_completion = block2::RcBlock::new(
        move |content: *mut SCShareableContent, err: *mut objc2_foundation::NSError| {
            if !err.is_null() {
                let desc = unsafe { (*err).localizedDescription() };
                eprintln!("[alt-tab/macos] SCShareableContent error: {}", desc);
                let _ = content_tx.send(None);
                return;
            }
            if content.is_null() {
                let _ = content_tx.send(None);
                return;
            }
            let content = unsafe { &*content };
            let windows = unsafe { content.windows() };
            let count = windows.count();
            let mut found = None;
            for i in 0..count {
                let w = windows.objectAtIndex(i);
                if unsafe { w.windowID() } == window_id {
                    found = Some(w.retain());
                    break;
                }
            }
            let _ = content_tx.send(found);
        },
    );

    unsafe {
        SCShareableContent::getShareableContentWithCompletionHandler(&content_completion);
    }

    let sc_window = content_rx.recv_timeout(Duration::from_millis(1500)).ok()??;

    let filter = unsafe {
        SCContentFilter::initWithDesktopIndependentWindow(SCContentFilter::alloc(), &sc_window)
    };

    let config = unsafe { SCStreamConfiguration::new() };
    unsafe {
        config.setWidth(max_w);
        config.setHeight(max_h);
        config.setScalesToFit(false);
    }

    let (img_tx, img_rx) =
        mpsc::channel::<Option<objc2::rc::Retained<objc2_core_graphics::CGImage>>>();
    let img_completion = block2::RcBlock::new(
        move |image: *mut objc2_core_graphics::CGImage, err: *mut objc2_foundation::NSError| {
            if !err.is_null() {
                let desc = unsafe { (*err).localizedDescription() };
                eprintln!("[alt-tab/macos] SCScreenshotManager error: {}", desc);
                let _ = img_tx.send(None);
                return;
            }
            if image.is_null() {
                let _ = img_tx.send(None);
            } else {
                let retained = unsafe { objc2::rc::Retained::retain(image) };
                let _ = img_tx.send(retained);
            }
        },
    );

    unsafe {
        SCScreenshotManager::captureImageWithFilter_configuration_completionHandler(
            &filter,
            &config,
            Some(&img_completion),
        );
    }

    let cg_image = img_rx.recv_timeout(Duration::from_millis(1500)).ok()??;
    let img_ptr = &*cg_image as *const objc2_core_graphics::CGImage as CGImageRef;
    let result = extract_bgra_from_raw_cgimage(img_ptr, max_w, max_h);
    result
}


pub fn activate_window(window_id: u32) {
    let opts =
        K_CG_WINDOW_LIST_OPTION_ON_SCREEN_ONLY | K_CG_WINDOW_LIST_EXCLUDE_DESKTOP_ELEMENTS;
    let list = unsafe { CGWindowListCopyWindowInfo(opts, K_CG_NULL_WINDOW_ID) };
    if list.is_null() {
        return;
    }

    let key_pid = cg_helpers::cfstr(b"kCGWindowOwnerPID");
    let key_number = cg_helpers::cfstr(b"kCGWindowNumber");

    let count = unsafe { CFArrayGetCount(list) };
    let mut target_pid: Option<i32> = None;

    for i in 0..count {
        let dict = unsafe { CFArrayGetValueAtIndex(list, i) } as CFDictionaryRef;
        if dict.is_null() {
            continue;
        }
        let Some(num) = cg_helpers::dict_get_i32(dict, key_number) else {
            continue;
        };
        if num as u32 == window_id {
            target_pid = cg_helpers::dict_get_i32(dict, key_pid);
            break;
        }
    }

    unsafe {
        CFRelease(list as *const c_void);
        CFRelease(key_pid as *const c_void);
        CFRelease(key_number as *const c_void);
    }

    let Some(pid) = target_pid else {
        return;
    };

    objc2::rc::autoreleasepool(|_pool| {
        use objc2_app_kit::{NSApplicationActivationOptions, NSRunningApplication};

        if let Some(app) = NSRunningApplication::runningApplicationWithProcessIdentifier(pid) {
            #[allow(deprecated)]
            let _ = app.activateWithOptions(NSApplicationActivationOptions::ActivateIgnoringOtherApps);
        }
    });
}

pub fn move_app_window(_title: &str, _x: i32, _y: i32) -> bool {
    // macOS GPUI windows can't be reliably repositioned via AX after creation.
    // Return false so the caller closes and recreates on the correct monitor.
    false
}

pub fn picker_window_kind() -> gpui::WindowKind {
    gpui::WindowKind::Normal
}

pub fn dismiss_picker(window: &mut gpui::Window) {
    window.remove_window();
}

fn cg_event_flags() -> u64 {
    const K_CG_EVENT_SOURCE_STATE_COMBINED: i32 = 0;

    #[link(name = "CoreGraphics", kind = "framework")]
    extern "C" {
        fn CGEventSourceFlagsState(state_id: i32) -> u64;
    }

    unsafe { CGEventSourceFlagsState(K_CG_EVENT_SOURCE_STATE_COMBINED) }
}

pub fn is_modifier_held() -> bool {
    const K_CG_EVENT_FLAG_MASK_ALTERNATE: u64 = 0x0008_0000;
    cg_event_flags() & K_CG_EVENT_FLAG_MASK_ALTERNATE != 0
}

pub fn is_shift_held() -> bool {
    const K_CG_EVENT_FLAG_MASK_SHIFT: u64 = 0x0002_0000;
    cg_event_flags() & K_CG_EVENT_FLAG_MASK_SHIFT != 0
}
