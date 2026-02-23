use super::cg_helpers;
use super::preview;
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

#[link(name = "CoreGraphics", kind = "framework")]
extern "C" {
    fn CGWindowListCopyWindowInfo(option: u32, relative_to: u32) -> CFArrayRef;
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
        windows.push(WindowInfo {
            id: id as u32,
            title: display_title,
            app_name,
            preview_path: None,
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
    targets
        .iter()
        .map(|&(idx, wid)| (idx, capture_preview(wid, max_w, max_h)))
        .collect()
}

struct RgbaImage {
    data: Vec<u8>,
    width: usize,
    height: usize,
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
                let w = unsafe { windows.objectAtIndex(i) };
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
        config.setScalesToFit(true);
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
    extract_rgba_from_cgimage(&cg_image)
}

fn extract_rgba_from_cgimage(
    cg_image: &objc2_core_graphics::CGImage,
) -> Option<RgbaImage> {
    let img_ptr = cg_image as *const objc2_core_graphics::CGImage as CGImageRef;

    let width = unsafe { CGImageGetWidth(img_ptr) };
    let height = unsafe { CGImageGetHeight(img_ptr) };
    let bytes_per_row = unsafe { CGImageGetBytesPerRow(img_ptr) };
    let bits_per_pixel = unsafe { CGImageGetBitsPerPixel(img_ptr) };

    if width == 0 || height == 0 {
        return None;
    }

    let provider = unsafe { CGImageGetDataProvider(img_ptr) };
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

    let bytes_per_pixel = bits_per_pixel / 8;
    let mut rgba = Vec::with_capacity(width * height * 4);

    for y in 0..height {
        let row_start = y * bytes_per_row;
        for x in 0..width {
            let offset = row_start + x * bytes_per_pixel;
            if offset + 4 > raw.len() {
                rgba.extend_from_slice(&[0, 0, 0, 255]);
                continue;
            }
            let b = raw[offset];
            let g = raw[offset + 1];
            let r = raw[offset + 2];
            let a = raw[offset + 3];
            rgba.extend_from_slice(&[r, g, b, a]);
        }
    }

    unsafe { CFRelease(cf_data) };

    Some(RgbaImage {
        data: rgba,
        width,
        height,
    })
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

pub fn move_app_window(_title: &str, _x: i32, _y: i32) {
    eprintln!(
        "[alt-tab/macos] move_app_window({:?}, {}, {}) not implemented",
        _title, _x, _y
    );
}

pub fn picker_window_kind() -> gpui::WindowKind {
    gpui::WindowKind::Normal
}

pub fn dismiss_picker(window: &mut gpui::Window) {
    // Move offscreen + shrink instead of remove_window() so the handle stays
    // alive for the reuse fast-path in open_picker().
    window.resize(gpui::size(gpui::px(1.0), gpui::px(1.0)));
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
