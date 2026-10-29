use std::cell::RefCell;
use std::collections::HashMap;
use std::ffi::{c_char, c_void};
use std::ptr::NonNull;
use std::sync::atomic::{AtomicBool, AtomicPtr, AtomicU64, Ordering};
use std::sync::{Mutex, Once, OnceLock, PoisonError};

use block2::{DynBlock, RcBlock};
use futures::channel::mpsc::{unbounded, UnboundedReceiver};
use futures::StreamExt as _;
use objc2::rc::{Retained, Weak};
use objc2::runtime::{AnyObject, NSObject};
use objc2::AnyThread;
use objc2_app_kit::{
    NSApplication, NSApplicationDidResignActiveNotification, NSColor, NSEvent, NSEventMask,
    NSEventType, NSFloatingWindowLevel, NSNormalWindowLevel, NSPopUpMenuWindowLevel, NSScreen,
    NSView, NSWindow, NSWindowAnimationBehavior, NSWindowCollectionBehavior,
    NSWindowDidResignKeyNotification, NSWindowStyleMask,
};
use objc2_foundation::{
    MainThreadMarker, NSNotification, NSNotificationCenter, NSPoint, NSRect, NSSize,
};
use raw_window_handle::{HasWindowHandle, RawWindowHandle};

use super::PopupPresentation;

type CFStringRef = *const c_void;
type CFArrayRef = *const c_void;
type CFDictionaryRef = *const c_void;
type CFNumberRef = *const c_void;
type CFIndex = isize;

const K_CG_WINDOW_LIST_OPTION_ON_SCREEN_ONLY: u32 = 1 << 0;
const K_CG_WINDOW_LIST_EXCLUDE_DESKTOP_ELEMENTS: u32 = 1 << 16;
const K_VK_ESCAPE: u16 = 53;
const K_CF_NUMBER_INT_TYPE: u64 = 3;
const K_CF_STRING_ENCODING_UTF8: u32 = 0x0800_0100;

#[link(name = "CoreGraphics", kind = "framework")]
extern "C" {
    static kCGWindowOwnerPID: CFStringRef;
    static kCGWindowNumber: CFStringRef;
    static kCGWindowLayer: CFStringRef;
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

struct InputRegion {
    title: String,
    rect: NSRect,
    window: Option<Weak<NSWindow>>,
}

impl InputRegion {
    fn window(&mut self) -> Option<Retained<NSWindow>> {
        if let Some(window) = self.window.as_ref().and_then(Weak::load) {
            return Some(window);
        }
        let window = resolve_window(&self.title)?;
        self.window = Some(Weak::new(&window));
        Some(window)
    }
}

struct PointerWatch {
    tokens: [Retained<AnyObject>; 2],
    _global: RcBlock<dyn Fn(NonNull<NSEvent>)>,
    _local: RcBlock<dyn Fn(NonNull<NSEvent>) -> *mut NSEvent>,
}

impl Drop for PointerWatch {
    fn drop(&mut self) {
        for token in &self.tokens {
            unsafe { NSEvent::removeMonitor(token) };
        }
    }
}

thread_local! {
    static INPUT_REGIONS: RefCell<Vec<InputRegion>> = const { RefCell::new(Vec::new()) };
    static POINTER_WATCH: RefCell<Option<PointerWatch>> = const { RefCell::new(None) };
}

fn remember_input_region(title: &str, rect: NSRect) {
    INPUT_REGIONS.with_borrow_mut(|regions| {
        match regions.iter_mut().find(|region| region.title == title) {
            Some(region) => region.rect = rect,
            None => regions.push(InputRegion {
                title: title.to_owned(),
                rect,
                window: None,
            }),
        }
    });
}

fn input_region(title: &str) -> Option<NSRect> {
    INPUT_REGIONS.with_borrow(|regions| {
        regions
            .iter()
            .find(|region| region.title == title)
            .map(|region| region.rect)
    })
}

fn pointer_in_region(frame: NSRect, region: NSRect, pointer: NSPoint) -> bool {
    let x = pointer.x - frame.origin.x;
    let y = frame.origin.y + frame.size.height - pointer.y;
    x >= region.origin.x
        && y >= region.origin.y
        && x < region.origin.x + region.size.width
        && y < region.origin.y + region.size.height
}

fn apply_input_regions() -> bool {
    let pointer = NSEvent::mouseLocation();
    let mut shown = false;
    INPUT_REGIONS.with_borrow_mut(|regions| {
        regions.retain_mut(|region| {
            let Some(window) = region.window() else {
                return false;
            };
            if window.isVisible() && window.alphaValue() > 0.0 {
                shown = true;
                let inside = pointer_in_region(window.frame(), region.rect, pointer);
                if window.ignoresMouseEvents() == inside {
                    window.setIgnoresMouseEvents(!inside);
                    qol_runtime::probe!(
                        "INPUT_REGION",
                        "step=toggle title={} inside={inside}",
                        region.title
                    );
                }
            }
            true
        });
    });
    shown
}

const POINTER_MOVES: NSEventMask = NSEventMask(
    NSEventMask::MouseMoved.0
        | NSEventMask::LeftMouseDragged.0
        | NSEventMask::RightMouseDragged.0
        | NSEventMask::OtherMouseDragged.0,
);

fn on_pointer_moved() {
    if !apply_input_regions() {
        run_on_main(Box::new(stop_pointer_watch_when_hidden));
    }
}

fn stop_pointer_watch_when_hidden() {
    if apply_input_regions() {
        return;
    }
    if POINTER_WATCH.take().is_some() {
        qol_runtime::probe!("INPUT_REGION", "step=watch-stopped");
    }
}

fn watch_pointer_for_regions() {
    if POINTER_WATCH.with_borrow(Option::is_some) {
        return;
    }
    let global = RcBlock::new(|_event: NonNull<NSEvent>| on_pointer_moved());
    let local = RcBlock::new(|event: NonNull<NSEvent>| {
        on_pointer_moved();
        event.as_ptr()
    });
    let global_ref: &DynBlock<dyn Fn(NonNull<NSEvent>)> = &global;
    let local_ref: &DynBlock<dyn Fn(NonNull<NSEvent>) -> *mut NSEvent> = &local;
    let Some(global_token) =
        NSEvent::addGlobalMonitorForEventsMatchingMask_handler(POINTER_MOVES, global_ref)
    else {
        return;
    };
    let Some(local_token) = (unsafe {
        NSEvent::addLocalMonitorForEventsMatchingMask_handler(POINTER_MOVES, local_ref)
    }) else {
        unsafe { NSEvent::removeMonitor(&global_token) };
        return;
    };
    POINTER_WATCH.set(Some(PointerWatch {
        tokens: [global_token, local_token],
        _global: global,
        _local: local,
    }));
    qol_runtime::probe!("INPUT_REGION", "step=watch-started");
}

fn set_input_region_on_main(title: &str, x: i16, y: i16, width: u16, height: u16) -> bool {
    let rect = NSRect::new(
        NSPoint::new(f64::from(x), f64::from(y)),
        NSSize::new(f64::from(width), f64::from(height)),
    );
    let title = title.to_owned();
    let apply = move || {
        remember_input_region(&title, rect);
        apply_input_regions();
        watch_pointer_for_regions();
    };
    if MainThreadMarker::new().is_some() {
        apply();
    } else {
        run_on_main(Box::new(apply));
    }
    true
}

fn pointer_inside_input(title: &str, window: &NSWindow) -> bool {
    let pointer = NSEvent::mouseLocation();
    match input_region(title) {
        Some(region) => pointer_in_region(window.frame(), region, pointer),
        None => pointer_inside(window.frame(), pointer),
    }
}

fn pointer_inside(frame: NSRect, pointer: NSPoint) -> bool {
    let (x, y) = (pointer.x - frame.origin.x, pointer.y - frame.origin.y);
    x >= 0.0 && y >= 0.0 && x < frame.size.width && y < frame.size.height
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
    cancel_focus_return();
    set_app_frontmost(std::process::id() as i32);
}

fn set_app_frontmost(pid: i32) {
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

const FOCUS_RETURN_SETTLE: std::time::Duration = std::time::Duration::from_millis(120);

static FOCUS_RETURN_GENERATION: AtomicU64 = AtomicU64::new(0);

fn schedule_focus_return() {
    let generation = FOCUS_RETURN_GENERATION.fetch_add(1, Ordering::SeqCst) + 1;
    std::thread::spawn(move || {
        std::thread::sleep(FOCUS_RETURN_SETTLE);
        run_on_main(Box::new(move || {
            if FOCUS_RETURN_GENERATION.load(Ordering::SeqCst) == generation {
                return_focus_if_unheld();
            }
        }));
    });
}

fn cancel_focus_return() {
    FOCUS_RETURN_GENERATION.fetch_add(1, Ordering::SeqCst);
}

fn return_focus_if_unheld() {
    let Some(mtm) = MainThreadMarker::new() else {
        return;
    };
    let app = NSApplication::sharedApplication(mtm);
    let shown_key = app
        .keyWindow()
        .is_some_and(|key| key.isVisible() && key.alphaValue() > 0.0);
    if !should_return_focus(
        app.isActive(),
        shown_key,
        crate::platform::has_process_focus(),
    ) {
        return;
    }
    let target = frontmost_foreign_pid();
    qol_runtime::probe!("FOCUS_RETURN", "phase=returned target={target:?}");
    if let Some(pid) = target {
        set_app_frontmost(pid);
    }
}

fn should_return_focus(app_active: bool, shown_key: bool, process_frontmost: bool) -> bool {
    app_active && process_frontmost && !shown_key
}

fn frontmost_foreign_pid() -> Option<i32> {
    let own = std::process::id() as i32;
    unsafe {
        let list = CGWindowListCopyWindowInfo(
            K_CG_WINDOW_LIST_OPTION_ON_SCREEN_ONLY | K_CG_WINDOW_LIST_EXCLUDE_DESKTOP_ELEMENTS,
            0,
        );
        if list.is_null() {
            return None;
        }
        let target = (0..CFArrayGetCount(list))
            .map(|index| CFArrayGetValueAtIndex(list, index))
            .filter(|dict| !dict.is_null())
            .filter(|&dict| cf_int_value(dict, &kCGWindowLayer) == Some(0))
            .filter_map(|dict| cf_int_value(dict, &kCGWindowOwnerPID))
            .find(|&pid| pid > 0 && pid != own);
        CFRelease(list);
        target
    }
}

#[cfg(debug_assertions)]
use std::sync::atomic::AtomicU32;

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

    pub fn client_origin(&self) -> Option<(i32, i32)> {
        None
    }

    pub fn pointer_root(&self) -> Option<(i32, i32)> {
        None
    }

    pub fn bounds(&self) -> Option<(i32, i32, u32, u32)> {
        None
    }

    pub fn pointer_on(&self) -> Option<crate::popup_window::PointerOnWindow> {
        None
    }

    pub fn pressed(&self) -> Option<bool> {
        None
    }

    pub fn set_input_region(&self, x: i16, y: i16, width: u16, height: u16) -> bool {
        set_input_region_on_main(&self.title, x, y, width, height)
    }

    pub fn anchor_content(&self, _right: bool, _bottom: bool) {}
}

pub fn window_geometry_session(title: &str) -> Option<WindowGeometrySession> {
    Some(WindowGeometrySession {
        title: title.to_owned(),
    })
}

pub struct EscapeGrab;

impl EscapeGrab {
    pub fn take_pressed(&self) -> bool {
        false
    }
}

pub fn grab_escape() -> Option<EscapeGrab> {
    None
}

pub struct InputWatch {
    token: Retained<AnyObject>,
    _handler: RcBlock<dyn Fn(NonNull<NSEvent>)>,
    events: UnboundedReceiver<crate::popup_window::InputEvent>,
}

impl InputWatch {
    pub async fn next(&mut self) -> Option<crate::popup_window::InputEvent> {
        self.events.next().await
    }
}

impl Drop for InputWatch {
    fn drop(&mut self) {
        unsafe { NSEvent::removeMonitor(&self.token) };
    }
}

fn input_event_for(title: &str, event: &NSEvent) -> Option<crate::popup_window::InputEvent> {
    use crate::popup_window::{InputEvent, PointerOnWindow};
    let kind = event.r#type();
    if kind == NSEventType::KeyDown {
        return (event.keyCode() == K_VK_ESCAPE).then_some(InputEvent::Escape);
    }
    Some(InputEvent::Pointer(PointerOnWindow {
        inside: pointer_over_window_by_title(title),
        pressed: kind != NSEventType::MouseMoved,
    }))
}

pub fn watch_input(title: &str) -> Option<InputWatch> {
    let (sender, events) = unbounded();
    let title = title.to_owned();
    let handler = RcBlock::new(move |event: NonNull<NSEvent>| {
        if let Some(input) = input_event_for(&title, unsafe { event.as_ref() }) {
            let _ = sender.unbounded_send(input);
        }
    });
    let handler_ref: &DynBlock<dyn Fn(NonNull<NSEvent>)> = &handler;
    let token = NSEvent::addGlobalMonitorForEventsMatchingMask_handler(
        NSEventMask::MouseMoved
            | NSEventMask::LeftMouseDown
            | NSEventMask::RightMouseDown
            | NSEventMask::OtherMouseDown
            | NSEventMask::KeyDown,
        handler_ref,
    )?;
    Some(InputWatch {
        token,
        _handler: handler,
        events,
    })
}

pub fn work_area_within(
    _monitor: gpui::Bounds<gpui::Pixels>,
) -> Option<gpui::Bounds<gpui::Pixels>> {
    None
}

pub fn window_position_by_title(_title: &str) -> Option<(i32, i32)> {
    None
}

pub fn pointer_on_window_by_title(title: &str) -> Option<crate::popup_window::PointerOnWindow> {
    let window = resolve_window(title)?;
    Some(crate::popup_window::PointerOnWindow {
        inside: window.isVisible() && pointer_inside_input(title, &window),
        pressed: NSEvent::pressedMouseButtons() != 0,
    })
}

pub fn pointer_over_window_by_title(title: &str) -> bool {
    pointer_on_window_by_title(title).is_some_and(|pointer| pointer.inside)
}

pub fn set_input_region_by_title(title: &str, x: i16, y: i16, width: u16, height: u16) -> bool {
    let Some(window) = resolve_window(title) else {
        return false;
    };
    let scale = window.backingScaleFactor().max(1.0);
    let points = |pixels: f64| pixels / scale;
    set_input_region_on_main(
        title,
        points(f64::from(x)).floor() as i16,
        points(f64::from(y)).floor() as i16,
        points(f64::from(width)).ceil() as u16,
        points(f64::from(height)).ceil() as u16,
    )
}

pub fn make_override_redirect(_title: &str) -> bool {
    false
}

pub fn set_override_redirect_by_title(_title: &str) -> bool {
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
    _scale: Option<f32>,
) -> bool {
    let backing = window_backing_scale(title);
    crate::window::resize_or_sync_scale(window, size, backing);
    sync_window_layout_by_title(title, origin, size)
}

pub fn sync_window_layout_by_title(
    title: &str,
    origin: gpui::Point<gpui::Pixels>,
    size: gpui::Size<gpui::Pixels>,
) -> bool {
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
        cocoa_origin_for_primary_anchored_top_left(
            mtm,
            origin.x.to_f64(),
            origin.y.to_f64(),
            size.height,
        ),
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
    let origin = cocoa_origin_for_primary_anchored_top_left(mtm, gpui_x, gpui_y, current_height);
    window.setFrameOrigin(origin);
    sync_backing_properties(&window);
    true
}

fn cocoa_origin_for_primary_anchored_top_left(
    mtm: MainThreadMarker,
    gpui_x: f64,
    gpui_y: f64,
    frame_height: f64,
) -> NSPoint {
    cocoa_origin_from_primary_anchored(gpui_x, gpui_y, frame_height, primary_screen_height(mtm))
}

fn cocoa_origin_from_primary_anchored(
    gpui_x: f64,
    gpui_y: f64,
    frame_height: f64,
    primary_screen_height: f64,
) -> NSPoint {
    NSPoint::new(
        gpui_x,
        cocoa_bottom_edge(primary_screen_height, gpui_y, frame_height),
    )
}

fn primary_screen_height(mtm: MainThreadMarker) -> f64 {
    NSScreen::screens(mtm)
        .iter()
        .next()
        .map(|screen| screen.frame().size.height)
        .unwrap_or(1080.0)
}

fn primary_anchored_from_screen_relative(
    screen_relative_x: f64,
    screen_relative_y: f64,
    screen_origin_x: f64,
    screen_origin_y: f64,
    screen_height: f64,
    primary_screen_height: f64,
) -> (f64, f64) {
    (
        screen_relative_x + screen_origin_x,
        screen_relative_y + primary_screen_height - screen_height - screen_origin_y,
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
    let Some(mtm) = MainThreadMarker::new() else {
        return false;
    };
    let Some(window) = find_window_by_title(mtm, title) else {
        return false;
    };
    forget_window_number(title);
    let held_key = window.isKeyWindow();
    window.setLevel(NSPopUpMenuWindowLevel);
    window.setAlphaValue(0.0);
    window.setIgnoresMouseEvents(true);
    window.orderOut(None);
    if held_key {
        schedule_focus_return();
    }
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

pub fn hide_unmapped(title: &str) -> bool {
    hide_invisible(title)
}

pub fn compositor_active() -> bool {
    true
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

fn native_window_from_gpui_window(
    window: &mut gpui::Window,
) -> Result<Retained<NSWindow>, &'static str> {
    let Ok(handle) = HasWindowHandle::window_handle(window) else {
        return Err("handle_error");
    };
    let RawWindowHandle::AppKit(handle) = handle.as_raw() else {
        return Err("not_appkit");
    };
    let Some(view) = (unsafe { Retained::<NSView>::retain(handle.ns_view.as_ptr().cast()) }) else {
        return Err("view_missing");
    };
    view.window().ok_or("window_missing")
}

pub fn window_bounds_primary_anchored(window: &mut gpui::Window) -> gpui::Bounds<gpui::Pixels> {
    let bounds = window.bounds();
    let Some(mtm) = MainThreadMarker::new() else {
        return bounds;
    };
    let Ok(native_window) = native_window_from_gpui_window(window) else {
        return bounds;
    };
    let Some(screen) = native_window.screen() else {
        return bounds;
    };
    let frame = screen.frame();
    let (x, y) = primary_anchored_from_screen_relative(
        bounds.origin.x.to_f64(),
        bounds.origin.y.to_f64(),
        frame.origin.x,
        frame.origin.y,
        frame.size.height,
        primary_screen_height(mtm),
    );
    gpui::Bounds::new(
        gpui::point(gpui::px(x as f32), gpui::px(y as f32)),
        bounds.size,
    )
}

pub fn hide_for_capture(title: &str, window: &mut gpui::Window) -> bool {
    #[cfg(not(debug_assertions))]
    let _ = title;
    let native_window = match native_window_from_gpui_window(window) {
        Ok(native_window) => native_window,
        Err(reason) => {
            qol_runtime::probe!("HIDE_WIN_CAPTURE", "title={title} result={reason}");
            return false;
        }
    };
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
    let (red, green, blue) = qol_color::parse_hex_color(hex.trim())?;
    Some(qol_color::rgb24(red, green, blue))
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
    show_window_by_title_with_focus(title, true, WindowPresentation::Overlay, false)
}

pub fn show_window_passive_by_title(title: &str) -> bool {
    show_window_by_title_with_focus(title, false, WindowPresentation::Overlay, true)
}

pub fn show_window_interactive_by_title(title: &str) -> bool {
    show_window_by_title_with_focus(title, false, WindowPresentation::Overlay, false)
}

pub fn show_toast_window_by_title(title: &str) -> bool {
    show_window_by_title_with_focus(title, false, WindowPresentation::Notice, false)
}

pub fn show_normal_window_by_title(title: &str) -> bool {
    show_window_by_title_with_focus(title, true, WindowPresentation::Normal, false)
}

pub fn window_presentation_is_normal_by_title(_title: &str) -> bool {
    true
}

#[derive(Clone, Copy, Debug)]
enum WindowPresentation {
    Overlay,
    Notice,
    Normal,
}

fn show_window_by_title_with_focus(
    title: &str,
    focus: bool,
    presentation: WindowPresentation,
    input_passthrough: bool,
) -> bool {
    let Some(mtm) = MainThreadMarker::new() else {
        return false;
    };
    let Some(window) = resolve_window(title) else {
        return false;
    };
    let level = match presentation {
        WindowPresentation::Overlay if window.styleMask().contains(NSWindowStyleMask::Titled) => {
            NSFloatingWindowLevel
        }
        WindowPresentation::Overlay => NSPopUpMenuWindowLevel,
        WindowPresentation::Notice => NSPopUpMenuWindowLevel - 1,
        WindowPresentation::Normal => NSNormalWindowLevel,
    };
    window.setLevel(level);
    if window.styleMask().contains(NSWindowStyleMask::Titled) {
        window.setBackgroundColor(Some(&NSColor::windowBackgroundColor()));
    } else {
        window.setBackgroundColor(Some(&NSColor::clearColor()));
    }
    window.setAlphaValue(1.0);
    window.setIgnoresMouseEvents(input_passthrough);
    if !input_passthrough && input_region(title).is_some() {
        apply_input_regions();
        watch_pointer_for_regions();
    }
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

pub fn reassert_focus_while_current(
    title: String,
    gen: &'static std::sync::atomic::AtomicU64,
    commit_gen: u64,
) {
    run_on_main(Box::new(move || {
        if gen.load(std::sync::atomic::Ordering::SeqCst) != commit_gen {
            return;
        }
        reassert_focus_on_main(&title);
    }));
}

fn reassert_focus_on_main(title: &str) {
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

pub fn register_native_display(_window: &gpui::Window) {}

pub fn grab_keyboard(_window: &gpui::Window) -> bool {
    false
}

pub fn release_keyboard(_window: &gpui::Window) {}

static INPUT_HELD: AtomicBool = AtomicBool::new(false);
static HOLD_REASSERTS: AtomicU64 = AtomicU64::new(0);
static HOLD_LAST_REASSERT: Mutex<Option<std::time::Instant>> = Mutex::new(None);
static HOLD_RETRY_PENDING: AtomicBool = AtomicBool::new(false);
static HELD_TITLE: Mutex<Option<String>> = Mutex::new(None);
static HOLD_OBSERVER: AtomicPtr<objc2::runtime::AnyObject> = AtomicPtr::new(std::ptr::null_mut());

const HOLD_REASSERT_SPACING: std::time::Duration = std::time::Duration::from_millis(100);

objc2::define_class!(
    #[unsafe(super(NSObject))]
    struct HoldInputObserver;

    impl HoldInputObserver {
        #[unsafe(method(holdWindowDidResignKey:))]
        unsafe fn __hold_window_did_resign_key(&self, _notification: &NSNotification) {
            reassert_held_input("resign_key");
        }

        #[unsafe(method(holdApplicationDidResignActive:))]
        unsafe fn __hold_application_did_resign_active(&self, _notification: &NSNotification) {
            reassert_held_input("resign_active");
        }
    }
);

fn hold_input_observer() -> Retained<HoldInputObserver> {
    unsafe { objc2::msg_send![HoldInputObserver::alloc(), init] }
}

fn hold_observer() -> *mut objc2::runtime::AnyObject {
    static CREATE: Once = Once::new();
    CREATE.call_once(|| {
        let observer = hold_input_observer();
        HOLD_OBSERVER.store(
            Retained::into_raw(observer) as *mut objc2::runtime::AnyObject,
            Ordering::Release,
        );
    });
    HOLD_OBSERVER.load(Ordering::Acquire)
}

unsafe fn as_any_object<T>(value: &T) -> &objc2::runtime::AnyObject {
    &*(std::ptr::from_ref(value).cast::<objc2::runtime::AnyObject>())
}

pub fn hold_input(title: &str) -> bool {
    let Some(window) = resolve_window(title) else {
        return false;
    };
    window.setLevel(NSFloatingWindowLevel);
    let observer = hold_observer();
    let center = NSNotificationCenter::defaultCenter();
    unsafe {
        center.removeObserver(&*observer);
        center.addObserver_selector_name_object(
            &*observer,
            objc2::sel!(holdWindowDidResignKey:),
            Some(NSWindowDidResignKeyNotification),
            Some(as_any_object(&*window)),
        );
        center.addObserver_selector_name_object(
            &*observer,
            objc2::sel!(holdApplicationDidResignActive:),
            Some(NSApplicationDidResignActiveNotification),
            None,
        );
    }
    HELD_TITLE
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .replace(title.to_owned());
    HOLD_REASSERTS.store(0, Ordering::Relaxed);
    *HOLD_LAST_REASSERT
        .lock()
        .unwrap_or_else(PoisonError::into_inner) = None;
    INPUT_HELD.store(true, Ordering::Release);
    qol_runtime::probe!(
        "HOLD_IN",
        "title={title} level=floating key_window={} reason={}",
        window.isKeyWindow(),
        crate::popup_window::change_reason()
    );
    true
}

pub fn release_input(title: &str) {
    INPUT_HELD.store(false, Ordering::Release);
    HOLD_REASSERTS.store(0, Ordering::Relaxed);
    *HOLD_LAST_REASSERT
        .lock()
        .unwrap_or_else(PoisonError::into_inner) = None;
    HELD_TITLE
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .take();
    let observer = HOLD_OBSERVER.load(Ordering::Acquire);
    if !observer.is_null() {
        let center = NSNotificationCenter::defaultCenter();
        unsafe {
            center.removeObserver(&*observer);
        }
    }
    qol_runtime::probe!(
        "HOLD_RELEASE",
        "title={title} reason={}",
        crate::popup_window::change_reason()
    );
}

pub fn input_held() -> bool {
    INPUT_HELD.load(Ordering::Acquire)
}

fn reassert_delay(since_last: Option<std::time::Duration>) -> std::time::Duration {
    match since_last {
        Some(elapsed) if elapsed < HOLD_REASSERT_SPACING => HOLD_REASSERT_SPACING - elapsed,
        _ => std::time::Duration::ZERO,
    }
}

fn reassert_held_input(reason: &'static str) {
    if !INPUT_HELD.load(Ordering::Acquire) {
        return;
    }
    let since_last = HOLD_LAST_REASSERT
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .map(|at| at.elapsed());
    let delay = reassert_delay(since_last);
    if delay.is_zero() {
        reassert_held_input_now(reason);
        return;
    }
    if HOLD_RETRY_PENDING.swap(true, Ordering::AcqRel) {
        return;
    }
    std::thread::spawn(move || {
        std::thread::sleep(delay);
        run_on_main(Box::new(move || {
            HOLD_RETRY_PENDING.store(false, Ordering::Release);
            if held_window_lost_focus() {
                reassert_held_input(reason);
            }
        }));
    });
}

fn held_title() -> Option<String> {
    HELD_TITLE
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .as_ref()
        .cloned()
}

fn held_window_lost_focus() -> bool {
    let Some(mtm) = MainThreadMarker::new() else {
        return false;
    };
    let Some(title) = held_title() else {
        return false;
    };
    let app = NSApplication::sharedApplication(mtm);
    !app.isActive() || find_window_by_title(mtm, &title).is_some_and(|window| !window.isKeyWindow())
}

fn reassert_held_input_now(reason: &str) {
    let Some(mtm) = MainThreadMarker::new() else {
        return;
    };
    let Some(title) = held_title() else {
        return;
    };
    let Some(window) = find_window_by_title(mtm, &title) else {
        return;
    };
    *HOLD_LAST_REASSERT
        .lock()
        .unwrap_or_else(PoisonError::into_inner) = Some(std::time::Instant::now());
    let attempt = HOLD_REASSERTS.fetch_add(1, Ordering::AcqRel) + 1;
    let app = NSApplication::sharedApplication(mtm);
    #[allow(deprecated)]
    app.activateIgnoringOtherApps(true);
    window.makeKeyAndOrderFront(None);
    force_app_frontmost();
    qol_runtime::probe!(
        "HOLD_REASSERT",
        "title={title} attempt={attempt} reason={reason} key_window={}",
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
    install_process_square_corner_mask();
    true
}

static SQUARE_CORNER_MASK: AtomicPtr<objc2::runtime::AnyObject> =
    AtomicPtr::new(std::ptr::null_mut());

unsafe extern "C-unwind" fn opaque_corner_mask(
    _this: *mut objc2::runtime::AnyObject,
    _cmd: objc2::runtime::Sel,
) -> *mut objc2::runtime::AnyObject {
    SQUARE_CORNER_MASK.load(std::sync::atomic::Ordering::Acquire)
}

unsafe fn make_opaque_corner_mask() -> *mut objc2::runtime::AnyObject {
    let image: *mut objc2::runtime::AnyObject = objc2::msg_send![objc2::class!(NSImage), alloc];
    let size = NSSize::new(1.0, 1.0);
    let image: *mut objc2::runtime::AnyObject = objc2::msg_send![image, initWithSize: size];
    if image.is_null() {
        return std::ptr::null_mut();
    }
    let _: () = objc2::msg_send![image, lockFocus];
    let white: *mut objc2::runtime::AnyObject =
        objc2::msg_send![objc2::class!(NSColor), whiteColor];
    let _: () = objc2::msg_send![white, setFill];
    let rect = NSRect::new(NSPoint::new(0.0, 0.0), size);
    let _: () = objc2::msg_send![objc2::class!(NSBezierPath), fillRect: rect];
    let _: () = objc2::msg_send![image, unlockFocus];
    image
}

fn install_process_square_corner_mask() {
    static INSTALLED: Once = Once::new();
    INSTALLED.call_once(|| unsafe {
        let image = make_opaque_corner_mask();
        if image.is_null() {
            return;
        }
        SQUARE_CORNER_MASK.store(image, std::sync::atomic::Ordering::Release);
        for class_name in [c"GPUIWindow", c"GPUIPanel"] {
            let Some(cls) = objc2::runtime::AnyClass::get(class_name) else {
                continue;
            };
            let added = objc2::ffi::class_addMethod(
                cls as *const objc2::runtime::AnyClass as *mut objc2::runtime::AnyClass,
                objc2::sel!(_cornerMask),
                objc2::runtime::MethodImplementation::__imp(
                    opaque_corner_mask
                        as unsafe extern "C-unwind" fn(
                            *mut objc2::runtime::AnyObject,
                            objc2::runtime::Sel,
                        )
                            -> *mut objc2::runtime::AnyObject,
                ),
                c"@@:".as_ptr(),
            );
            qol_runtime::probe!(
                "CORNER_MASK",
                "class={} added={}",
                class_name.to_string_lossy(),
                added.as_bool()
            );
        }
    });
}

pub fn set_window_type_dock_by_title(_title: &str) -> bool {
    false
}

pub fn join_all_spaces(title: &str) -> bool {
    let Some(window) = resolve_window(title) else {
        return false;
    };
    window.setHidesOnDeactivate(false);
    window.setCollectionBehavior(
        NSWindowCollectionBehavior::CanJoinAllSpaces
            | NSWindowCollectionBehavior::FullScreenAuxiliary,
    );
    true
}

pub fn configure_overlay_window(title: &str, _resizable: bool, decorated: bool) -> bool {
    if !configure_popup_window(title) {
        return false;
    }
    let Some(window) = resolve_window(title) else {
        return false;
    };
    if !decorated {
        window.setStyleMask(NSWindowStyleMask::Borderless);
    }
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

type DispatchFunction = unsafe extern "C" fn(*mut c_void);

extern "C" {
    fn dlopen(path: *const c_char, mode: i32) -> *mut c_void;
    fn dlsym(handle: *mut c_void, symbol: *const c_char) -> *mut c_void;
}

struct DispatchSymbols {
    main_queue: *const c_void,
    async_f: unsafe extern "C" fn(*const c_void, *mut c_void, DispatchFunction),
}

unsafe impl Send for DispatchSymbols {}
unsafe impl Sync for DispatchSymbols {}

fn dispatch_symbols() -> Option<&'static DispatchSymbols> {
    static SYMBOLS: OnceLock<Option<DispatchSymbols>> = OnceLock::new();
    SYMBOLS
        .get_or_init(|| {
            let handle = unsafe { dlopen(c"libSystem.B.dylib".as_ptr(), 0x1) };
            if handle.is_null() {
                return None;
            }
            let main_queue =
                unsafe { dlsym(handle, c"_dispatch_main_q".as_ptr()) as *const c_void };
            let async_f = unsafe { dlsym(handle, c"dispatch_async_f".as_ptr()) as *const () };
            if main_queue.is_null() || async_f.is_null() {
                return None;
            }
            Some(DispatchSymbols {
                main_queue,
                async_f: unsafe {
                    std::mem::transmute::<
                        *const (),
                        unsafe extern "C" fn(*const c_void, *mut c_void, DispatchFunction),
                    >(async_f)
                },
            })
        })
        .as_ref()
}

unsafe extern "C" fn run_task_on_main(context: *mut c_void) {
    let task = Box::from_raw(context as *mut Box<dyn FnOnce() + Send>);
    task();
}

fn run_on_main(task: Box<dyn FnOnce() + Send + 'static>) {
    let Some(symbols) = dispatch_symbols() else {
        return;
    };
    unsafe {
        let boxed: Box<Box<dyn FnOnce() + Send>> = Box::new(task);
        (symbols.async_f)(
            symbols.main_queue,
            Box::into_raw(boxed) as *mut c_void,
            run_task_on_main,
        );
    }
}

#[cfg(test)]
mod tests {
    use super::{
        cocoa_bottom_edge, cocoa_origin_from_primary_anchored, pointer_in_region, pointer_inside,
        primary_anchored_from_screen_relative, reassert_delay, should_return_focus,
        HOLD_REASSERT_SPACING,
    };
    use objc2_foundation::{NSPoint, NSRect, NSSize};
    use std::time::Duration;

    const PRIMARY_HEIGHT: f64 = 1080.0;

    #[test]
    fn focus_returns_only_while_this_process_still_holds_the_front() {
        let cases = [
            (true, false, true, true),
            (true, false, false, false),
            (true, true, true, false),
            (false, false, true, false),
            (false, false, false, false),
        ];
        for (app_active, shown_key, process_frontmost, expected) in cases {
            assert_eq!(
                should_return_focus(app_active, shown_key, process_frontmost),
                expected,
                "active={app_active} shown_key={shown_key} frontmost={process_frontmost}"
            );
        }
    }

    #[test]
    fn a_held_window_reasserts_at_once_unless_it_just_did() {
        let spacing = HOLD_REASSERT_SPACING;
        let cases = [
            (None, Duration::ZERO),
            (Some(spacing), Duration::ZERO),
            (Some(spacing * 5), Duration::ZERO),
            (Some(Duration::ZERO), spacing),
            (Some(spacing / 4), spacing - spacing / 4),
        ];
        for (since_last, expected) in cases {
            assert_eq!(
                reassert_delay(since_last),
                expected,
                "since_last={since_last:?}"
            );
        }
    }

    fn window() -> NSRect {
        NSRect::new(NSPoint::new(100.0, 200.0), NSSize::new(440.0, 600.0))
    }

    #[test]
    fn only_the_painted_region_counts_as_inside_a_taller_window() {
        let card = NSRect::new(NSPoint::new(20.0, 0.0), NSSize::new(400.0, 120.0));
        let cases = [
            (NSPoint::new(300.0, 790.0), true),
            (NSPoint::new(300.0, 681.0), true),
            (NSPoint::new(300.0, 679.0), false),
            (NSPoint::new(300.0, 210.0), false),
            (NSPoint::new(110.0, 790.0), false),
            (NSPoint::new(530.0, 790.0), false),
        ];
        for (pointer, expected) in cases {
            assert_eq!(
                pointer_in_region(window(), card, pointer),
                expected,
                "pointer=({}, {})",
                pointer.x,
                pointer.y
            );
        }
    }

    #[test]
    fn a_pointer_outside_the_window_is_not_inside() {
        assert!(!pointer_inside(window(), NSPoint::new(99.0, 300.0)));
        assert!(!pointer_inside(window(), NSPoint::new(300.0, 801.0)));
        assert!(!pointer_inside(window(), NSPoint::new(540.0, 300.0)));
    }

    #[test]
    fn a_pointer_within_the_window_frame_is_inside() {
        assert!(pointer_inside(window(), NSPoint::new(101.0, 201.0)));
        assert!(pointer_inside(window(), NSPoint::new(539.0, 799.0)));
    }

    struct Screen {
        origin_x: f64,
        origin_y: f64,
        height: f64,
    }

    fn gpui_bounds_origin(
        cocoa_x: f64,
        cocoa_bottom: f64,
        frame_height: f64,
        screen: &Screen,
    ) -> (f64, f64) {
        (
            cocoa_x - screen.origin_x,
            screen.height - cocoa_bottom - frame_height + screen.origin_y,
        )
    }

    #[test]
    fn a_panel_resized_in_place_keeps_its_cocoa_frame_on_every_screen() {
        let cases = [
            (
                "primary",
                Screen {
                    origin_x: 0.0,
                    origin_y: 0.0,
                    height: 1080.0,
                },
                (100.0, 440.0),
            ),
            (
                "left",
                Screen {
                    origin_x: -1920.0,
                    origin_y: 0.0,
                    height: 1080.0,
                },
                (-1160.0, 280.0),
            ),
            (
                "above",
                Screen {
                    origin_x: 0.0,
                    origin_y: 1080.0,
                    height: 800.0,
                },
                (960.0, 1480.0),
            ),
            (
                "right taller",
                Screen {
                    origin_x: 1920.0,
                    origin_y: -300.0,
                    height: 1440.0,
                },
                (2380.0, 520.0),
            ),
        ];
        let frame_height = 600.0;
        for (name, screen, (cocoa_x, cocoa_bottom)) in cases {
            let (screen_x, screen_y) =
                gpui_bounds_origin(cocoa_x, cocoa_bottom, frame_height, &screen);
            let (anchored_x, anchored_y) = primary_anchored_from_screen_relative(
                screen_x,
                screen_y,
                screen.origin_x,
                screen.origin_y,
                screen.height,
                PRIMARY_HEIGHT,
            );
            let written = cocoa_origin_from_primary_anchored(
                anchored_x,
                anchored_y,
                frame_height,
                PRIMARY_HEIGHT,
            );
            assert_eq!(
                (written.x, written.y),
                (cocoa_x, cocoa_bottom),
                "{name}: reading and writing the same frame must not move the window"
            );
        }
    }

    #[test]
    fn a_placement_off_the_primary_screen_is_written_where_it_was_asked_for() {
        let written = cocoa_origin_from_primary_anchored(-1160.0, 240.0, 600.0, PRIMARY_HEIGHT);
        assert_eq!(
            (written.x, written.y),
            (-1160.0, 240.0),
            "a placement built from monitor bounds is already primary anchored"
        );
    }

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
