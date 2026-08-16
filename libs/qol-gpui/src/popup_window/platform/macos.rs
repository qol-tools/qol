use std::collections::HashMap;
use std::ffi::{c_char, c_void};
use std::sync::{Mutex, PoisonError};

use objc2::rc::Retained;
use objc2_app_kit::{
    NSApplication, NSColor, NSPopUpMenuWindowLevel, NSScreen, NSView, NSWindow,
    NSWindowAnimationBehavior, NSWindowStyleMask,
};
use objc2_foundation::{MainThreadMarker, NSPoint, NSRect, NSSize};
use raw_window_handle::{HasWindowHandle, RawWindowHandle};

use super::PopupPresentation;

type CFStringRef = *const c_void;
type CFArrayRef = *const c_void;
type CFDictionaryRef = *const c_void;
type CFNumberRef = *const c_void;
type CFIndex = isize;

const K_CG_WINDOW_LIST_OPTION_ON_SCREEN_ONLY: u32 = 1 << 0;
const K_CG_WINDOW_LIST_EXCLUDE_DESKTOP_ELEMENTS: u32 = 1 << 16;
const K_CF_NUMBER_INT_TYPE: u64 = 3;
const K_CF_STRING_ENCODING_UTF8: u32 = 0x0800_0100;

#[link(name = "CoreGraphics", kind = "framework")]
extern "C" {
    static kCGWindowOwnerPID: CFStringRef;
    static kCGWindowNumber: CFStringRef;
    fn CGWindowListCopyWindowInfo(option: u32, relative_to_window: u32) -> CFArrayRef;
}

#[link(name = "CoreFoundation", kind = "framework")]
extern "C" {
    static kCFBooleanTrue: *const c_void;
    fn CFArrayGetCount(array: CFArrayRef) -> CFIndex;
    fn CFArrayGetValueAtIndex(array: CFArrayRef, index: CFIndex) -> *const c_void;
    fn CFDictionaryGetValue(dict: CFDictionaryRef, key: *const c_void) -> *const c_void;
    fn CFNumberGetValue(number: CFNumberRef, the_type: u64, value_ptr: *mut c_void) -> u8;
    fn CFStringCreateWithCString(
        alloc: *const c_void,
        c_str: *const c_char,
        encoding: u32,
    ) -> CFStringRef;
    fn CFRelease(value: *const c_void);
}

#[link(name = "ApplicationServices", kind = "framework")]
extern "C" {
    fn AXUIElementCreateApplication(pid: i32) -> *const c_void;
    fn AXUIElementSetAttributeValue(
        element: *const c_void,
        attribute: CFStringRef,
        value: *const c_void,
    ) -> i32;
}

static NATIVE_WINDOW_NUMBERS: Mutex<Option<HashMap<String, u32>>> = Mutex::new(None);

fn note_window_number(title: &str, window: &NSWindow) {
    let mut registry = NATIVE_WINDOW_NUMBERS
        .lock()
        .unwrap_or_else(PoisonError::into_inner);
    registry
        .get_or_insert_with(HashMap::new)
        .insert(title.to_owned(), window.windowNumber() as u32);
}

fn forget_window_number(title: &str) {
    let mut registry = NATIVE_WINDOW_NUMBERS
        .lock()
        .unwrap_or_else(PoisonError::into_inner);
    if let Some(map) = registry.as_mut() {
        map.remove(title);
    }
}

fn onscreen_window_numbers() -> Vec<u32> {
    let pid = std::process::id() as i32;
    unsafe {
        let list = CGWindowListCopyWindowInfo(
            K_CG_WINDOW_LIST_OPTION_ON_SCREEN_ONLY | K_CG_WINDOW_LIST_EXCLUDE_DESKTOP_ELEMENTS,
            0,
        );
        if list.is_null() {
            return Vec::new();
        }
        let count = CFArrayGetCount(list);
        let mut numbers = Vec::new();
        for index in 0..count {
            let dict = CFArrayGetValueAtIndex(list, index);
            if dict.is_null() {
                continue;
            }
            if cf_int_value(dict, &kCGWindowOwnerPID) != Some(pid) {
                continue;
            }
            if let Some(number) = cf_int_value(dict, &kCGWindowNumber) {
                numbers.push(number as u32);
            }
        }
        CFRelease(list);
        numbers
    }
}

fn cf_int_value(dict: CFDictionaryRef, key: &CFStringRef) -> Option<i32> {
    unsafe {
        let value = CFDictionaryGetValue(dict, *key);
        if value.is_null() {
            return None;
        }
        let mut out: i32 = 0;
        if CFNumberGetValue(
            value as CFNumberRef,
            K_CF_NUMBER_INT_TYPE,
            &mut out as *mut i32 as *mut c_void,
        ) != 0
        {
            Some(out)
        } else {
            None
        }
    }
}

fn force_app_frontmost() {
    let pid = std::process::id() as i32;
    unsafe {
        let app = AXUIElementCreateApplication(pid);
        if app.is_null() {
            return;
        }
        let attr = CFStringCreateWithCString(
            std::ptr::null(),
            c"AXFrontmost".as_ptr(),
            K_CF_STRING_ENCODING_UTF8,
        );
        let result = if attr.is_null() {
            -1
        } else {
            let outcome = AXUIElementSetAttributeValue(app, attr, kCFBooleanTrue);
            CFRelease(attr);
            outcome
        };
        CFRelease(app);
        qol_runtime::probe!("AX_FRONT", "pid={pid} result={result}");
    }
}

#[cfg(debug_assertions)]
use std::sync::atomic::{AtomicU32, Ordering};

#[cfg(debug_assertions)]
const GHOST_COLOR_UNSET: u32 = u32::MAX;

#[cfg(debug_assertions)]
static GHOST_DEBUG_ALPHA: AtomicU32 = AtomicU32::new(0);
#[cfg(debug_assertions)]
static GHOST_DEBUG_COLOR: AtomicU32 = AtomicU32::new(GHOST_COLOR_UNSET);

pub struct Platform;

impl PopupPresentation for Platform {
    fn present_topmost(_title: &str) {}

    fn restore_composite(_title: &str) {}
}

#[derive(Clone)]
pub struct WindowGeometrySession {
    title: String,
}

impl WindowGeometrySession {
    pub fn set_bounds(&self, _x: i32, _y: i32, _width: u32, _height: u32) {}

    pub fn set_position(&self, _x: i32, _y: i32) {}

    pub fn reposition(&self, x: i32, y: i32) -> bool {
        reposition_window_by_title(&self.title, f64::from(x), f64::from(y))
    }

    pub fn pointer_root(&self) -> Option<(i32, i32)> {
        None
    }

    pub fn bounds(&self) -> Option<(i32, i32, u32, u32)> {
        None
    }

    pub fn anchor_content(&self, _right: bool, _bottom: bool) {}
}

pub fn window_geometry_session(title: &str) -> Option<WindowGeometrySession> {
    Some(WindowGeometrySession {
        title: title.to_owned(),
    })
}

pub fn window_position_by_title(_title: &str) -> Option<(i32, i32)> {
    None
}

pub fn make_override_redirect(_title: &str) -> bool {
    false
}

pub fn focus_window_by_title(_title: &str) -> bool {
    false
}

pub fn release_focus_by_title(_title: &str) {}

pub fn sync_window_layout(
    title: &str,
    window: &mut gpui::Window,
    origin: gpui::Point<gpui::Pixels>,
    size: gpui::Size<gpui::Pixels>,
) -> bool {
    let backing = window_backing_scale(title);
    crate::window::resize_or_sync_scale(window, size, backing);
    set_window_frame_by_title(title, origin, size)
}

pub fn set_window_frame_by_title(
    title: &str,
    origin: gpui::Point<gpui::Pixels>,
    size: gpui::Size<gpui::Pixels>,
) -> bool {
    let Some(mtm) = MainThreadMarker::new() else {
        return false;
    };
    let Some(window) = find_window_by_title(mtm, title) else {
        return false;
    };
    let size = NSSize::new(size.width.to_f64(), size.height.to_f64());
    let frame = NSRect::new(
        cocoa_origin_for_top_left(mtm, origin.x.to_f64(), origin.y.to_f64(), size.height),
        size,
    );
    window.setFrame_display(frame, true);
    sync_backing_properties(&window);
    true
}

pub fn reposition_window_by_title(title: &str, gpui_x: f64, gpui_y: f64) -> bool {
    let Some(mtm) = MainThreadMarker::new() else {
        return false;
    };
    let Some(window) = find_window_by_title(mtm, title) else {
        return false;
    };
    let current_height = window.frame().size.height;
    let origin = cocoa_origin_for_top_left(mtm, gpui_x, gpui_y, current_height);
    window.setFrameOrigin(origin);
    sync_backing_properties(&window);
    true
}

fn cocoa_origin_for_top_left(
    mtm: MainThreadMarker,
    gpui_x: f64,
    gpui_y: f64,
    frame_height: f64,
) -> NSPoint {
    let primary_screen_height = NSScreen::screens(mtm)
        .iter()
        .next()
        .map(|screen| screen.frame().size.height)
        .unwrap_or(1080.0);
    NSPoint::new(
        gpui_x,
        cocoa_bottom_edge(primary_screen_height, gpui_y, frame_height),
    )
}

fn cocoa_bottom_edge(primary_screen_height: f64, gpui_top: f64, frame_height: f64) -> f64 {
    primary_screen_height - gpui_top - frame_height
}

fn resolve_window(title: &str) -> Option<Retained<NSWindow>> {
    let mtm = MainThreadMarker::new()?;
    find_window_by_title(mtm, title)
}

pub fn hide_window_by_title(title: &str) -> bool {
    let Some(window) = resolve_window(title) else {
        return false;
    };
    window.setLevel(NSPopUpMenuWindowLevel);
    #[cfg(debug_assertions)]
    {
        let alpha = f32::from_bits(GHOST_DEBUG_ALPHA.load(Ordering::Relaxed));
        if alpha > 0.0 {
            match debug_ghost_color() {
                Some(color) => window.setBackgroundColor(Some(&color)),
                None => window.setBackgroundColor(Some(&NSColor::clearColor())),
            }
            window.setAlphaValue(alpha as f64);
            window.setIgnoresMouseEvents(true);
            qol_runtime::probe!(
                "HIDE_WIN",
                "title={title} path=rest alpha={alpha} reason={}",
                crate::popup_window::change_reason()
            );
            return true;
        }
    }
    window.setAlphaValue(0.0);
    window.setIgnoresMouseEvents(true);
    qol_runtime::probe!(
        "HIDE_WIN",
        "title={title} path=hidden reason={}",
        crate::popup_window::change_reason()
    );
    true
}

pub fn capture_focus_return() {}

pub fn hide_invisible(title: &str) -> bool {
    let Some(window) = resolve_window(title) else {
        return false;
    };
    forget_window_number(title);
    window.setLevel(NSPopUpMenuWindowLevel);
    window.setAlphaValue(0.0);
    window.setIgnoresMouseEvents(true);
    window.orderOut(None);
    qol_runtime::probe!(
        "HIDE_WIN",
        "title={title} path=ordered_out reason={}",
        crate::popup_window::change_reason()
    );
    true
}

pub fn park_window_by_title(title: &str) -> bool {
    hide_invisible(title)
}

pub fn prepare_window_reveal_by_title(title: &str) -> bool {
    let Some(window) = resolve_window(title) else {
        return false;
    };
    window.setLevel(NSPopUpMenuWindowLevel);
    window.setAlphaValue(0.0);
    window.setIgnoresMouseEvents(true);
    window.orderFrontRegardless();
    qol_runtime::probe!(
        "PREPARE_WIN",
        "title={title} prepared=true reason={}",
        crate::popup_window::change_reason()
    );
    true
}

pub fn configure_keepalive_window(title: &str) -> bool {
    hide_invisible(title)
}

pub fn hide_windows_by_title_prefix(prefix: &str) -> usize {
    let Some(mtm) = MainThreadMarker::new() else {
        return 0;
    };
    let app = NSApplication::sharedApplication(mtm);
    let mut hidden = 0;
    for window in app.windows().iter() {
        let title = window.title().to_string();
        if !title.starts_with(prefix) {
            continue;
        }
        forget_window_number(&title);
        window.setLevel(NSPopUpMenuWindowLevel);
        window.setAlphaValue(0.0);
        window.setIgnoresMouseEvents(true);
        window.orderOut(None);
        hidden += 1;
        qol_runtime::probe!(
            "HIDE_WIN_PREFIX",
            "prefix={prefix} title={title} path=ordered_out reason={}",
            crate::popup_window::change_reason()
        );
    }
    hidden
}

pub fn visible_windows_by_title_prefix(prefix: &str) -> usize {
    if let Some(mtm) = MainThreadMarker::new() {
        let app = NSApplication::sharedApplication(mtm);
        return app
            .windows()
            .iter()
            .filter(|window| {
                window.title().to_string().starts_with(prefix)
                    && window.isVisible()
                    && window.alphaValue() > 0.01
            })
            .count();
    }
    let onscreen = onscreen_window_numbers();
    let registry = NATIVE_WINDOW_NUMBERS
        .lock()
        .unwrap_or_else(PoisonError::into_inner);
    let Some(map) = registry.as_ref() else {
        return 0;
    };
    map.iter()
        .filter(|(title, number)| title.starts_with(prefix) && onscreen.contains(number))
        .count()
}

pub fn hide_for_capture(title: &str, window: &mut gpui::Window) -> bool {
    #[cfg(not(debug_assertions))]
    let _ = title;
    let Ok(handle) = HasWindowHandle::window_handle(window) else {
        qol_runtime::probe!("HIDE_WIN_CAPTURE", "title={title} result=handle_error");
        return false;
    };
    let RawWindowHandle::AppKit(handle) = handle.as_raw() else {
        qol_runtime::probe!("HIDE_WIN_CAPTURE", "title={title} result=not_appkit");
        return false;
    };
    let Some(view) = (unsafe { Retained::<NSView>::retain(handle.ns_view.as_ptr().cast()) }) else {
        qol_runtime::probe!("HIDE_WIN_CAPTURE", "title={title} result=view_missing");
        return false;
    };
    let Some(native_window) = view.window() else {
        qol_runtime::probe!("HIDE_WIN_CAPTURE", "title={title} result=window_missing");
        return false;
    };
    native_window.setAnimationBehavior(NSWindowAnimationBehavior::None);
    native_window.setLevel(NSPopUpMenuWindowLevel);
    native_window.setAlphaValue(0.0);
    native_window.setIgnoresMouseEvents(true);
    native_window.orderOut(None);
    qol_runtime::probe!(
        "HIDE_WIN_CAPTURE",
        "title={title} result=ordered_out reason={}",
        crate::popup_window::change_reason()
    );
    true
}

pub fn set_ghost_debug(opacity: Option<f32>, color_hex: Option<&str>) {
    #[cfg(debug_assertions)]
    {
        GHOST_DEBUG_ALPHA.store(opacity.unwrap_or(0.0).to_bits(), Ordering::Relaxed);
        let color = color_hex
            .and_then(parse_hex_rgb)
            .unwrap_or(GHOST_COLOR_UNSET);
        GHOST_DEBUG_COLOR.store(color, Ordering::Relaxed);
    }
    #[cfg(not(debug_assertions))]
    let _ = (opacity, color_hex);
}

pub fn set_unmap_hide(_enabled: bool) {}

#[cfg(debug_assertions)]
fn parse_hex_rgb(hex: &str) -> Option<u32> {
    let hex = hex.trim().trim_start_matches('#');
    if hex.len() != 6 {
        return None;
    }
    u32::from_str_radix(hex, 16).ok()
}

#[cfg(debug_assertions)]
fn debug_ghost_color() -> Option<Retained<NSColor>> {
    let rgb = GHOST_DEBUG_COLOR.load(Ordering::Relaxed);
    if rgb == GHOST_COLOR_UNSET {
        return None;
    }
    let r = ((rgb >> 16) & 0xFF) as f64 / 255.0;
    let g = ((rgb >> 8) & 0xFF) as f64 / 255.0;
    let b = (rgb & 0xFF) as f64 / 255.0;
    Some(NSColor::colorWithSRGBRed_green_blue_alpha(r, g, b, 1.0))
}

pub fn show_window_by_title(title: &str) -> bool {
    show_window_by_title_with_focus(title, true)
}

pub fn show_window_passive_by_title(title: &str) -> bool {
    show_window_by_title_with_focus(title, false)
}

pub fn show_normal_window_by_title(title: &str) -> bool {
    show_window_by_title_with_focus(title, true)
}

pub fn set_window_fixed_size_by_title(title: &str, size: gpui::Size<gpui::Pixels>) -> bool {
    let Some(window) = resolve_window(title) else {
        return false;
    };
    let size = NSSize::new(size.width.to_f64(), size.height.to_f64());
    window.setContentMinSize(size);
    window.setContentMaxSize(size);
    true
}

fn show_window_by_title_with_focus(title: &str, focus: bool) -> bool {
    let Some(mtm) = MainThreadMarker::new() else {
        return false;
    };
    let Some(window) = resolve_window(title) else {
        return false;
    };
    window.setLevel(NSPopUpMenuWindowLevel);
    window.setBackgroundColor(Some(&NSColor::clearColor()));
    window.setAlphaValue(1.0);
    window.setIgnoresMouseEvents(!focus);
    if focus {
        let app = NSApplication::sharedApplication(mtm);
        #[allow(deprecated)]
        app.activateIgnoringOtherApps(true);
        window.makeKeyAndOrderFront(None);
        force_app_frontmost();
    }
    if !focus {
        window.orderFrontRegardless();
    }
    qol_runtime::probe!(
        "SHOW_WIN",
        "title={title} focus_requested={focus} key_window={} reason={}",
        window.isKeyWindow(),
        crate::popup_window::change_reason()
    );
    true
}

pub fn reassert_focus_on_main(title: &str) {
    let Some(mtm) = MainThreadMarker::new() else {
        return;
    };
    let Some(window) = find_window_by_title(mtm, title) else {
        return;
    };
    let app = NSApplication::sharedApplication(mtm);
    #[allow(deprecated)]
    app.activateIgnoringOtherApps(true);
    window.makeKeyAndOrderFront(None);
    force_app_frontmost();
    qol_runtime::probe!(
        "FOCUS_REASSERT",
        "title={title} step=reassert-on-main key_window={}",
        window.isKeyWindow()
    );
}

pub fn disable_window_shadow(title: &str) -> bool {
    let Some(window) = resolve_window(title) else {
        return false;
    };
    window.setHasShadow(false);
    #[cfg(debug_assertions)]
    {
        let alpha = f32::from_bits(GHOST_DEBUG_ALPHA.load(Ordering::Relaxed));
        if alpha > 0.0 {
            if let Some(color) = debug_ghost_color() {
                window.setBackgroundColor(Some(&color));
                return true;
            }
        }
    }
    window.setBackgroundColor(Some(&NSColor::clearColor()));
    true
}

pub fn configure_popup_window(title: &str) -> bool {
    let Some(window) = resolve_window(title) else {
        return false;
    };
    window.setAnimationBehavior(NSWindowAnimationBehavior::None);
    window.setLevel(NSPopUpMenuWindowLevel);
    true
}

pub fn set_window_type_dock_by_title(_title: &str) -> bool {
    false
}

pub fn configure_overlay_window(title: &str) -> bool {
    if !configure_popup_window(title) {
        return false;
    }
    let Some(window) = resolve_window(title) else {
        return false;
    };
    // gpui opens ghost windows with NSTitledWindowMask, and AppKit runs
    // constrainFrameRect:toScreen: on titled windows, which pushes a display-sized
    // overlay down by the menu bar height. Borderless windows are exempt, and gpui's
    // window class overrides canBecomeKeyWindow, so the overlay still takes focus.
    window.setStyleMask(NSWindowStyleMask::Borderless);
    true
}

pub fn configure_pinned_window(title: &str) -> bool {
    configure_popup_window(title)
}

pub fn pinned_window_kind() -> gpui::WindowKind {
    gpui::WindowKind::Normal
}

pub fn window_backing_scale(title: &str) -> Option<f32> {
    let mtm = MainThreadMarker::new()?;
    let window = find_window_by_title(mtm, title)?;
    Some(window.backingScaleFactor() as f32)
}

#[cfg(not(debug_assertions))]
pub fn dump_ghost_windows(_context: &str) {}

#[cfg(debug_assertions)]
pub fn dump_ghost_windows(context: &str) {
    let Some(mtm) = MainThreadMarker::new() else {
        return;
    };
    let app = NSApplication::sharedApplication(mtm);
    for window in app.windows().iter() {
        let frame = window.frame();
        qol_runtime::probe!(
            "GHOST_DUMP",
            "ctx=({context}) title={:?} alpha={:.2} level={} mouse_ignored={} frame={}x{}@{},{}",
            window.title().to_string(),
            window.alphaValue(),
            window.level(),
            window.ignoresMouseEvents(),
            frame.size.width,
            frame.size.height,
            frame.origin.x,
            frame.origin.y
        );
    }
}

fn find_window_by_title(mtm: MainThreadMarker, title: &str) -> Option<Retained<NSWindow>> {
    let app = NSApplication::sharedApplication(mtm);
    let found = app
        .windows()
        .iter()
        .filter(|win| win.title().to_string() == title)
        .last();
    if let Some(window) = &found {
        note_window_number(title, window);
    } else {
        qol_runtime::probe!(
            "WIN_MISS",
            "title={title} reason={}",
            crate::popup_window::change_reason()
        );
    }
    found
}

fn sync_backing_properties(window: &NSWindow) {
    let Some(view) = window.contentView() else {
        return;
    };
    let Some(gpui_view) = view.subviews().firstObject() else {
        return;
    };
    gpui_view.viewDidChangeBackingProperties();
}

pub fn window_holds_input_focus(title: &str) -> Option<bool> {
    if !crate::platform::has_process_focus() {
        return Some(false);
    }
    let Some(frontmost) = onscreen_window_numbers().first().copied() else {
        return Some(false);
    };
    let registry = NATIVE_WINDOW_NUMBERS
        .lock()
        .unwrap_or_else(PoisonError::into_inner);
    let Some(&number) = registry.as_ref().and_then(|map| map.get(title)) else {
        return Some(false);
    };
    Some(frontmost == number)
}

#[cfg(test)]
mod tests {
    use super::cocoa_bottom_edge;

    #[test]
    fn top_edge_survives_every_height_change() {
        let screen = 1080.0;
        let requested_top = 40.0;
        for height in [720.0, 480.0, 320.0, 32.0] {
            let bottom = cocoa_bottom_edge(screen, requested_top, height);
            let resulting_top = screen - (bottom + height);
            assert_eq!(resulting_top, requested_top, "height {height}");
        }
    }

    #[test]
    fn bottom_edge_sits_on_the_screen_floor_for_a_full_height_window() {
        assert_eq!(cocoa_bottom_edge(1080.0, 0.0, 1080.0), 0.0);
    }
}
