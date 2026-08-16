use objc2::rc::Retained;
use objc2_app_kit::NSView;
use raw_window_handle::{HasWindowHandle, RawWindowHandle};

pub fn set_accessory_policy() {
    use objc2_app_kit::{NSApplication, NSApplicationActivationPolicy};
    use objc2_foundation::MainThreadMarker;
    let mtm = MainThreadMarker::new().expect("must be on main thread");
    let app = NSApplication::sharedApplication(mtm);
    app.setActivationPolicy(NSApplicationActivationPolicy::Accessory);
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

pub fn is_escape_held() -> bool {
    false
}

pub fn ghost_window_kind() -> gpui::WindowKind {
    gpui::WindowKind::Normal
}

pub fn ghost_window_decorations(transparent: bool) -> gpui::WindowDecorations {
    if transparent {
        gpui::WindowDecorations::Server
    } else {
        gpui::WindowDecorations::Client
    }
}

pub fn adjust_ghost_bounds(bounds: gpui::Bounds<gpui::Pixels>) -> gpui::Bounds<gpui::Pixels> {
    bounds
}

pub fn should_poll_focus() -> bool {
    true
}

#[repr(C)]
#[derive(Clone, Copy)]
struct ProcessSerialNumber {
    high: u32,
    low: u32,
}

#[link(name = "ApplicationServices", kind = "framework")]
extern "C" {
    fn GetFrontProcess(psn: *mut ProcessSerialNumber) -> i32;
    fn GetProcessPID(psn: *const ProcessSerialNumber, pid: *mut i32) -> i32;
}

fn frontmost_pid() -> Option<i32> {
    unsafe {
        #[allow(deprecated)]
        {
            let mut psn = ProcessSerialNumber { high: 0, low: 0 };
            if GetFrontProcess(&mut psn) != 0 {
                return None;
            }
            let mut pid = 0;
            if GetProcessPID(&psn, &mut pid) != 0 {
                return None;
            }
            Some(pid)
        }
    }
}

pub fn has_process_focus() -> bool {
    frontmost_pid() == Some(std::process::id() as i32)
}

use std::ffi::{c_char, c_void};
use std::sync::OnceLock;

type DispatchFunction = unsafe extern "C" fn(*mut c_void);

extern "C" {
    fn dlopen(path: *const c_char, mode: i32) -> *mut c_void;
    fn dlsym(handle: *mut c_void, symbol: *const c_char) -> *mut c_void;
}

struct DispatchSymbols {
    get_main_queue: unsafe extern "C" fn() -> *const c_void,
    async_f: unsafe extern "C" fn(*const c_void, *mut c_void, DispatchFunction),
}

fn dispatch_symbols() -> Option<&'static DispatchSymbols> {
    static SYMBOLS: OnceLock<Option<DispatchSymbols>> = OnceLock::new();
    SYMBOLS
        .get_or_init(|| {
            let handle = unsafe { dlopen(c"libSystem.B.dylib".as_ptr(), 0x1) };
            if handle.is_null() {
                return None;
            }
            let get_main_queue =
                unsafe { dlsym(handle, c"_dispatch_get_main_queue".as_ptr()) as *const () };
            let async_f = unsafe { dlsym(handle, c"_dispatch_async_f".as_ptr()) as *const () };
            if get_main_queue.is_null() || async_f.is_null() {
                return None;
            }
            Some(DispatchSymbols {
                get_main_queue: unsafe {
                    std::mem::transmute::<*const (), unsafe extern "C" fn() -> *const c_void>(
                        get_main_queue,
                    )
                },
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

pub fn run_on_main(task: Box<dyn FnOnce() + Send + 'static>) {
    let Some(symbols) = dispatch_symbols() else {
        return;
    };
    unsafe {
        let queue = (symbols.get_main_queue)();
        if queue.is_null() {
            return;
        }
        let boxed: Box<Box<dyn FnOnce() + Send>> = Box::new(task);
        (symbols.async_f)(queue, Box::into_raw(boxed) as *mut c_void, run_task_on_main);
    }
}

pub fn start_window_move(window: &mut gpui::Window) {
    let Ok(handle) = HasWindowHandle::window_handle(window) else {
        window.start_window_move();
        return;
    };
    let RawWindowHandle::AppKit(handle) = handle.as_raw() else {
        window.start_window_move();
        return;
    };
    let Some(view) = (unsafe { Retained::<NSView>::retain(handle.ns_view.as_ptr().cast()) }) else {
        window.start_window_move();
        return;
    };
    let Some(native_window) = view.window() else {
        window.start_window_move();
        return;
    };
    let Some(event) = native_window.currentEvent() else {
        window.start_window_move();
        return;
    };
    native_window.performWindowDragWithEvent(&event);
}
