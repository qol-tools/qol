use super::cg_helpers;
use super::RgbaImage;
use super::WindowInfo;
use objc2::{AnyThread, Message};
use std::collections::HashMap;
use std::ffi::c_void;

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
    fn CGColorSpaceCreateDeviceRGB() -> *const c_void;
    fn CGBitmapContextCreate(
        data: *mut c_void,
        width: usize,
        height: usize,
        bits_per_component: usize,
        bytes_per_row: usize,
        space: *const c_void,
        bitmap_info: u32,
    ) -> *const c_void;
    fn CGContextDrawImage(ctx: *const c_void, rect: CGRect, image: CGImageRef);
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

const ICON_SIZE: usize = 32;

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
            icon: None,
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

pub fn get_app_icons(windows: &[WindowInfo]) -> HashMap<String, RgbaImage> {
    let own_pid = std::process::id() as i32;
    let opts =
        K_CG_WINDOW_LIST_OPTION_ON_SCREEN_ONLY | K_CG_WINDOW_LIST_EXCLUDE_DESKTOP_ELEMENTS;
    let list = unsafe { CGWindowListCopyWindowInfo(opts, K_CG_NULL_WINDOW_ID) };
    if list.is_null() {
        return HashMap::new();
    }

    let key_pid = cg_helpers::cfstr(b"kCGWindowOwnerPID");
    let key_owner = cg_helpers::cfstr(b"kCGWindowOwnerName");

    // Build app_name → pid mapping from CG list
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

    // Only extract icons for apps that are in our window list
    let needed: std::collections::HashSet<&str> =
        windows.iter().map(|w| w.app_name.as_str()).collect();

    let mut icons = HashMap::new();
    for (name, pid) in &app_pids {
        if !needed.contains(name.as_str()) {
            continue;
        }
        if let Some(icon) = extract_app_icon(*pid) {
            icons.insert(name.clone(), icon);
        }
    }
    icons
}

fn extract_app_icon(pid: i32) -> Option<RgbaImage> {
    use objc2_app_kit::NSRunningApplication;

    objc2::rc::autoreleasepool(|_pool| {
        let app = NSRunningApplication::runningApplicationWithProcessIdentifier(pid)?;
        let ns_image = app.icon()?;

        let cg_image = unsafe {
            ns_image.CGImageForProposedRect_context_hints(
                std::ptr::null_mut(),
                None,
                None,
            )
        }?;

        let img_ptr = &*cg_image as *const objc2_core_graphics::CGImage as CGImageRef;

        // Draw into a CGBitmapContext with known BGRA format.
        // This normalises any source pixel format (wide color, different alpha,
        // varying bpp) into the 32-bit BGRA that gpui expects.
        let sz = ICON_SIZE;
        let row_bytes = sz * 4;
        let mut buf = vec![0u8; sz * row_bytes];

        let color_space = unsafe { CGColorSpaceCreateDeviceRGB() };
        if color_space.is_null() {
            return None;
        }

        // kCGImageAlphaPremultipliedFirst (2) | kCGBitmapByteOrder32Little (2 << 12 = 8192)
        // = BGRA premultiplied, little-endian 32-bit — matches gpui/CG window captures.
        let bitmap_info: u32 = 2 | (2 << 12);

        let ctx = unsafe {
            CGBitmapContextCreate(
                buf.as_mut_ptr() as *mut c_void,
                sz,
                sz,
                8,
                row_bytes,
                color_space,
                bitmap_info,
            )
        };
        unsafe { CFRelease(color_space) };

        if ctx.is_null() {
            return None;
        }

        let draw_rect = CGRect {
            origin: CGPoint { x: 0.0, y: 0.0 },
            size: CGSize { width: sz as f64, height: sz as f64 },
        };
        unsafe { CGContextDrawImage(ctx, draw_rect, img_ptr) };
        unsafe { CFRelease(ctx) };

        Some(RgbaImage { data: buf, width: sz, height: sz })
    })
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

pub fn disable_window_shadow() {
    use objc2_app_kit::{NSApplication, NSColor};
    use objc2_foundation::MainThreadMarker;

    let mtm = MainThreadMarker::new().expect("must be on main thread");
    let app = NSApplication::sharedApplication(mtm);
    let clear = unsafe { NSColor::clearColor() };
    for window in app.windows().iter() {
        window.setHasShadow(false);
        window.setBackgroundColor(Some(&clear));
    }
}
