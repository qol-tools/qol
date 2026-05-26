use qol_runtime::MonitorBounds;
use std::ffi::c_void;

use crate::desktop_state::Platform;

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
const K_AX_VALUE_CG_POINT_TYPE: u32 = 1;
const K_AX_VALUE_CG_SIZE_TYPE: u32 = 2;

#[link(name = "CoreGraphics", kind = "framework")]
extern "C" {
    fn CGGetActiveDisplayList(max: u32, displays: *mut CGDirectDisplayID, count: *mut u32) -> i32;
    fn CGDisplayBounds(display: CGDirectDisplayID) -> CGRect;
    fn CGEventCreate(source: *const c_void) -> *const c_void;
    fn CGEventGetLocation(event: *const c_void) -> CGPoint;
    fn CGWindowListCreate(list_option: u32, relative_to_window: u32) -> *const c_void;
}

#[link(name = "CoreFoundation", kind = "framework")]
extern "C" {
    fn CFRelease(cf: *const c_void);
    fn CFArrayGetCount(arr: *const c_void) -> isize;
    fn CFArrayGetValueAtIndex(arr: *const c_void, idx: isize) -> *const c_void;
    fn CFStringCreateWithBytes(
        alloc: *const c_void,
        bytes: *const u8,
        num_bytes: isize,
        encoding: u32,
        is_external: bool,
    ) -> CFStringRef;
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

struct CfGuard(*const c_void);

impl CfGuard {
    fn new(ptr: *const c_void) -> Option<Self> {
        if ptr.is_null() {
            return None;
        }
        Some(Self(ptr))
    }

    fn as_ptr(&self) -> *const c_void {
        self.0
    }
}

impl Drop for CfGuard {
    fn drop(&mut self) {
        unsafe { CFRelease(self.0) }
    }
}

fn ax_attr_str(name: &[u8]) -> CfGuard {
    let ptr = unsafe {
        CFStringCreateWithBytes(
            std::ptr::null(),
            name.as_ptr(),
            name.len() as isize,
            0x08000100,
            false,
        )
    };
    CfGuard(ptr)
}

fn ax_get_attr(element: *const c_void, attr: &CfGuard) -> Option<CfGuard> {
    let mut value: CFTypeRef = std::ptr::null();
    let err = unsafe { AXUIElementCopyAttributeValue(element, attr.as_ptr(), &mut value) };
    if err != K_AX_ERROR_SUCCESS {
        return None;
    }
    CfGuard::new(value)
}

fn ax_get_pid(element: *const c_void) -> Option<i32> {
    let mut pid: i32 = 0;
    let err = unsafe { AXUIElementGetPid(element, &mut pid) };
    (err == K_AX_ERROR_SUCCESS).then_some(pid)
}

fn ax_get_point(value: &CfGuard) -> Option<CGPoint> {
    let mut point = CGPoint { x: 0.0, y: 0.0 };
    let ok = unsafe {
        AXValueGetValue(
            value.as_ptr(),
            K_AX_VALUE_CG_POINT_TYPE,
            &mut point as *mut _ as *mut c_void,
        )
    };
    ok.then_some(point)
}

fn ax_get_size(value: &CfGuard) -> Option<CGSize> {
    let mut size = CGSize {
        width: 0.0,
        height: 0.0,
    };
    let ok = unsafe {
        AXValueGetValue(
            value.as_ptr(),
            K_AX_VALUE_CG_SIZE_TYPE,
            &mut size as *mut _ as *mut c_void,
        )
    };
    (ok && size.width > 0.0 && size.height > 0.0).then_some(size)
}

fn resolve_focused_app(own_pid: i32) -> Option<(CfGuard, Option<i32>)> {
    let system = CfGuard::new(unsafe { AXUIElementCreateSystemWide() })?;
    let app = ax_get_attr(system.as_ptr(), &ax_attr_str(b"AXFocusedApplication"))?;
    let pid = ax_get_pid(app.as_ptr());
    if let Some(pid) = pid {
        let ignored = super::super::is_ignored_pid(pid as u32);
        if pid == own_pid || ignored {
            log::debug!(
                "[runtime/ax] SKIP pid={} own={} ignored={}",
                pid,
                pid == own_pid,
                ignored
            );
            return None;
        }
    }
    Some((app, pid))
}

fn ax_window_bounds(window: &CfGuard) -> Option<MonitorBounds> {
    let pos = ax_get_point(&ax_get_attr(window.as_ptr(), &ax_attr_str(b"AXPosition"))?)?;
    let sz = ax_get_size(&ax_get_attr(window.as_ptr(), &ax_attr_str(b"AXSize"))?)?;
    Some(MonitorBounds {
        x: pos.x as f32,
        y: pos.y as f32,
        width: sz.width as f32,
        height: sz.height as f32,
    })
}

fn focused_window_bounds_ax(own_pid: i32) -> Option<MonitorBounds> {
    let (app, app_pid) = resolve_focused_app(own_pid)?;
    let focused_window =
        ax_get_attr(app.as_ptr(), &ax_attr_str(b"AXFocusedWindow")).or_else(|| {
            log::debug!("[runtime/ax] pid={:?} has no focused window", app_pid);
            None
        })?;
    let bounds = ax_window_bounds(&focused_window)?;
    log::debug!(
        "[runtime/ax] HIT pid={:?} window=({}, {}, {}x{})",
        app_pid,
        bounds.x,
        bounds.y,
        bounds.width,
        bounds.height
    );
    Some(bounds)
}

pub(super) struct MacQueries {
    own_pid: i32,
}

impl MacQueries {
    pub(super) fn new(own_pid: i32) -> Self {
        Self { own_pid }
    }
}

impl Platform for MacQueries {
    fn poll_focused_window(&self) -> bool {
        true
    }

    fn cursor_position(&self) -> Option<(f32, f32)> {
        let event = CfGuard::new(unsafe { CGEventCreate(std::ptr::null()) })?;
        let loc = unsafe { CGEventGetLocation(event.as_ptr()) };
        Some((loc.x as f32, loc.y as f32))
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

    fn window_list_fingerprint(&self) -> Option<u64> {
        const ON_SCREEN_ONLY: u32 = 1;
        const EXCLUDE_DESKTOP: u32 = 1 << 4;
        const OPTS: u32 = ON_SCREEN_ONLY | EXCLUDE_DESKTOP;
        let arr = unsafe { CGWindowListCreate(OPTS, 0) };
        let guard = CfGuard::new(arr)?;
        let n = unsafe { CFArrayGetCount(arr) };
        let mut h: u64 = 0;
        for i in 0..n {
            let id = unsafe { CFArrayGetValueAtIndex(arr, i) } as usize as u64;
            let mut x = id.wrapping_mul(0x9E3779B97F4A7C15);
            x ^= x >> 32;
            h ^= x;
        }
        let mut count_mix = n as u64;
        count_mix = count_mix.wrapping_mul(0x9E3779B97F4A7C15);
        count_mix ^= count_mix >> 32;
        h ^= count_mix;
        drop(guard);
        Some(h)
    }
}
