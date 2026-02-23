use super::Platform;
use qol_runtime::MonitorBounds;
use std::ffi::c_void;

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

type CGDirectDisplayID = u32;
type AXUIElementRef = *const c_void;
type AXError = i32;
type CFTypeRef = *const c_void;
type CFStringRef = *const c_void;

const K_AX_ERROR_SUCCESS: AXError = 0;

#[link(name = "CoreGraphics", kind = "framework")]
extern "C" {
    fn CGGetActiveDisplayList(max: u32, displays: *mut CGDirectDisplayID, count: *mut u32) -> i32;
    fn CGDisplayBounds(display: CGDirectDisplayID) -> CGRect;
    fn CGEventCreate(source: *const c_void) -> *const c_void;
    fn CGEventGetLocation(event: *const c_void) -> CGPoint;
}

#[link(name = "CoreFoundation", kind = "framework")]
extern "C" {
    fn CFRelease(cf: *const c_void);
}

#[link(name = "ApplicationServices", kind = "framework")]
extern "C" {
    fn AXUIElementCreateSystemWide() -> AXUIElementRef;
    fn AXUIElementCopyAttributeValue(
        element: AXUIElementRef,
        attribute: CFStringRef,
        value: *mut CFTypeRef,
    ) -> AXError;
    fn AXValueGetValue(value: CFTypeRef, value_type: u32, value_ptr: *mut c_void) -> bool;
    fn AXUIElementGetPid(element: AXUIElementRef, pid: *mut i32) -> AXError;
}

// AXValueType constants
const K_AX_VALUE_CG_POINT_TYPE: u32 = 1;
const K_AX_VALUE_CG_SIZE_TYPE: u32 = 2;

fn ax_attr_str(name: &[u8]) -> CFStringRef {
    extern "C" {
        fn CFStringCreateWithBytes(
            alloc: *const c_void,
            bytes: *const u8,
            num_bytes: isize,
            encoding: u32,
            is_external: bool,
        ) -> CFStringRef;
    }
    unsafe {
        CFStringCreateWithBytes(
            std::ptr::null(),
            name.as_ptr(),
            name.len() as isize,
            0x08000100, // kCFStringEncodingUTF8
            false,
        )
    }
}

fn ax_get_attr(element: AXUIElementRef, attr: CFStringRef) -> Option<CFTypeRef> {
    let mut value: CFTypeRef = std::ptr::null();
    let err = unsafe { AXUIElementCopyAttributeValue(element, attr, &mut value) };
    if err != K_AX_ERROR_SUCCESS || value.is_null() {
        return None;
    }
    Some(value)
}

fn ax_get_pid(element: AXUIElementRef) -> Option<i32> {
    let mut pid: i32 = 0;
    let err = unsafe { AXUIElementGetPid(element, &mut pid) };
    if err == K_AX_ERROR_SUCCESS {
        Some(pid)
    } else {
        None
    }
}

fn focused_window_bounds_ax(own_pid: i32) -> Option<MonitorBounds> {
    let system = unsafe { AXUIElementCreateSystemWide() };
    if system.is_null() {
        eprintln!("[runtime/ax] AXUIElementCreateSystemWide returned null");
        return None;
    }

    let attr_focused_app = ax_attr_str(b"AXFocusedApplication");
    let focused_app = ax_get_attr(system, attr_focused_app);
    unsafe {
        CFRelease(attr_focused_app);
        CFRelease(system);
    }
    let focused_app = match focused_app {
        Some(app) => app,
        None => {
            eprintln!("[runtime/ax] no focused application");
            return None;
        }
    };

    let app_pid = ax_get_pid(focused_app);
    if let Some(pid) = app_pid {
        let ignored = super::is_ignored_pid(pid as u32);
        if pid == own_pid || ignored {
            unsafe { CFRelease(focused_app); }
            eprintln!("[runtime/ax] SKIP pid={} own={} ignored={}", pid, pid == own_pid, ignored);
            return None;
        }
    }

    let attr_focused_window = ax_attr_str(b"AXFocusedWindow");
    let focused_window = ax_get_attr(focused_app, attr_focused_window);
    unsafe {
        CFRelease(attr_focused_window);
        CFRelease(focused_app);
    }
    let focused_window = match focused_window {
        Some(w) => w,
        None => {
            eprintln!("[runtime/ax] pid={:?} has no focused window", app_pid);
            return None;
        }
    };

    let attr_position = ax_attr_str(b"AXPosition");
    let attr_size = ax_attr_str(b"AXSize");
    let pos_val = ax_get_attr(focused_window, attr_position);
    let size_val = ax_get_attr(focused_window, attr_size);
    unsafe {
        CFRelease(attr_position);
        CFRelease(attr_size);
        CFRelease(focused_window);
    }

    let pos_val = pos_val?;
    let size_val = size_val?;

    let mut pos = CGPoint { x: 0.0, y: 0.0 };
    let mut sz = CGSize { width: 0.0, height: 0.0 };

    let pos_ok = unsafe {
        AXValueGetValue(pos_val, K_AX_VALUE_CG_POINT_TYPE, &mut pos as *mut CGPoint as *mut c_void)
    };
    let size_ok = unsafe {
        AXValueGetValue(size_val, K_AX_VALUE_CG_SIZE_TYPE, &mut sz as *mut CGSize as *mut c_void)
    };

    unsafe {
        CFRelease(pos_val);
        CFRelease(size_val);
    }

    if !pos_ok || !size_ok || sz.width <= 0.0 || sz.height <= 0.0 {
        eprintln!("[runtime/ax] bad geometry: pos_ok={} size_ok={} sz={}x{}", pos_ok, size_ok, sz.width, sz.height);
        return None;
    }

    let result = MonitorBounds {
        x: pos.x as f32,
        y: pos.y as f32,
        width: sz.width as f32,
        height: sz.height as f32,
    };
    eprintln!("[runtime/ax] HIT pid={:?} window=({}, {}, {}x{})",
        app_pid, result.x, result.y, result.width, result.height);
    Some(result)
}

pub(super) struct MacQueries {
    own_pid: i32,
}

impl MacQueries {
    pub fn new(own_pid: i32) -> Self {
        Self { own_pid }
    }
}

impl Platform for MacQueries {
    fn poll_focused_window(&self) -> bool {
        true
    }

    fn cursor_position(&self) -> Option<(f32, f32)> {
        unsafe {
            let event = CGEventCreate(std::ptr::null());
            if event.is_null() {
                return None;
            }
            let loc = CGEventGetLocation(event);
            CFRelease(event);
            Some((loc.x as f32, loc.y as f32))
        }
    }

    fn focused_window_bounds(&self) -> Option<MonitorBounds> {
        focused_window_bounds_ax(self.own_pid)
    }

    fn physical_monitors(&self) -> Vec<MonitorBounds> {
        let mut ids = [0u32; 16];
        let mut count = 0u32;
        let ret = unsafe { CGGetActiveDisplayList(16, ids.as_mut_ptr(), &mut count) };
        if ret != 0 {
            return Vec::new();
        }
        (0..count as usize)
            .map(|i| {
                let rect = unsafe { CGDisplayBounds(ids[i]) };
                MonitorBounds {
                    x: rect.origin.x as f32,
                    y: rect.origin.y as f32,
                    width: rect.size.width as f32,
                    height: rect.size.height as f32,
                }
            })
            .collect()
    }
}
