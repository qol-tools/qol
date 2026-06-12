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
        #[cfg(debug_assertions)]
        let t = std::time::Instant::now();
        #[cfg(debug_assertions)]
        focus_probe::log_focus_change(self.own_pid);
        #[cfg(debug_assertions)]
        let cg_ms = t.elapsed().as_millis();
        let bounds = focused_window_bounds_ax(self.own_pid);
        #[cfg(debug_assertions)]
        {
            let total_ms = t.elapsed().as_millis();
            if total_ms >= 100 {
                qol_runtime::probe!("FOCUS_POLL_SLOW", "cg={cg_ms}ms ax={}ms", total_ms - cg_ms);
            }
        }
        bounds
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

#[cfg(debug_assertions)]
mod focus_probe {
    use std::ffi::c_void;
    use std::sync::atomic::{AtomicU32, AtomicU64, Ordering};

    use super::{
        ax_attr_str, CFArrayGetCount, CFArrayGetValueAtIndex, CGPoint, CGRect, CGSize, CfGuard,
    };

    #[link(name = "CoreGraphics", kind = "framework")]
    extern "C" {
        fn CGWindowListCopyWindowInfo(list_option: u32, relative_to_window: u32) -> *const c_void;
        fn CGRectMakeWithDictionaryRepresentation(dict: *const c_void, rect: *mut CGRect) -> bool;
    }

    #[link(name = "CoreFoundation", kind = "framework")]
    extern "C" {
        fn CFDictionaryGetValue(dict: *const c_void, key: *const c_void) -> *const c_void;
        fn CFNumberGetValue(number: *const c_void, number_type: isize, out: *mut c_void) -> bool;
        fn CFStringGetCString(
            string: *const c_void,
            buffer: *mut u8,
            buffer_size: isize,
            encoding: u32,
        ) -> bool;
    }

    const K_CF_NUMBER_SINT32: isize = 3;
    const UTF8: u32 = 0x0800_0100;

    static LAST_FOCUS_WID: AtomicU32 = AtomicU32::new(0);
    static LAST_POLL_TS: AtomicU64 = AtomicU64::new(0);

    struct FrontWindow {
        wid: u32,
        pid: i32,
        bounds: Option<CGRect>,
        title: String,
    }

    pub(super) fn log_focus_change(own_pid: i32) {
        let now_ms = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_millis() as u64)
            .unwrap_or(0);
        let prev_poll_ms = LAST_POLL_TS.swap(now_ms, Ordering::Relaxed);
        let Some(win) = frontmost_normal_window() else {
            return;
        };
        if LAST_FOCUS_WID.swap(win.wid, Ordering::Relaxed) == win.wid {
            return;
        }
        let detect_lag_ms = if prev_poll_ms == 0 {
            0
        } else {
            now_ms.saturating_sub(prev_poll_ms)
        };
        let ignored = win.pid == own_pid || crate::desktop_state::is_ignored_pid(win.pid as u32);
        let winpos = win
            .bounds
            .map(|r| {
                format!(
                    "({},{},{}x{})",
                    r.origin.x as i32, r.origin.y as i32, r.size.width as i32, r.size.height as i32
                )
            })
            .unwrap_or_else(|| "none".to_string());
        qol_runtime::probe!(
            "FOCUS_WIN",
            "wid={} pid={} ignored={} detect_lag_ms={} winpos={} title={:?}",
            win.wid,
            win.pid,
            ignored,
            detect_lag_ms,
            winpos,
            win.title
        );
    }

    fn frontmost_normal_window() -> Option<FrontWindow> {
        const ON_SCREEN_ONLY: u32 = 1;
        const EXCLUDE_DESKTOP: u32 = 1 << 4;
        let list = CfGuard::new(unsafe {
            CGWindowListCopyWindowInfo(ON_SCREEN_ONLY | EXCLUDE_DESKTOP, 0)
        })?;
        let key_layer = ax_attr_str(b"kCGWindowLayer");
        let key_num = ax_attr_str(b"kCGWindowNumber");
        let key_pid = ax_attr_str(b"kCGWindowOwnerPID");
        let key_name = ax_attr_str(b"kCGWindowName");
        let key_owner = ax_attr_str(b"kCGWindowOwnerName");
        let key_bounds = ax_attr_str(b"kCGWindowBounds");
        let n = unsafe { CFArrayGetCount(list.as_ptr()) };
        for i in 0..n {
            let dict = unsafe { CFArrayGetValueAtIndex(list.as_ptr(), i) };
            if dict.is_null() {
                continue;
            }
            if dict_i32(dict, &key_layer) != Some(0) {
                continue;
            }
            let wid = dict_i32(dict, &key_num)? as u32;
            let pid = dict_i32(dict, &key_pid)?;
            let title = dict_string(dict, &key_name)
                .or_else(|| dict_string(dict, &key_owner))
                .unwrap_or_else(|| "?".to_string());
            return Some(FrontWindow {
                wid,
                pid,
                bounds: dict_rect(dict, &key_bounds),
                title,
            });
        }
        None
    }

    fn dict_i32(dict: *const c_void, key: &CfGuard) -> Option<i32> {
        let value = unsafe { CFDictionaryGetValue(dict, key.as_ptr()) };
        if value.is_null() {
            return None;
        }
        let mut out: i32 = 0;
        unsafe { CFNumberGetValue(value, K_CF_NUMBER_SINT32, &mut out as *mut _ as *mut c_void) }
            .then_some(out)
    }

    fn dict_string(dict: *const c_void, key: &CfGuard) -> Option<String> {
        let value = unsafe { CFDictionaryGetValue(dict, key.as_ptr()) };
        if value.is_null() {
            return None;
        }
        let mut buf = [0u8; 256];
        let ok = unsafe { CFStringGetCString(value, buf.as_mut_ptr(), buf.len() as isize, UTF8) };
        if !ok {
            return None;
        }
        let len = buf.iter().position(|&b| b == 0).unwrap_or(0);
        (len > 0).then(|| String::from_utf8_lossy(&buf[..len]).into_owned())
    }

    fn dict_rect(dict: *const c_void, key: &CfGuard) -> Option<CGRect> {
        let value = unsafe { CFDictionaryGetValue(dict, key.as_ptr()) };
        if value.is_null() {
            return None;
        }
        let mut rect = CGRect {
            origin: CGPoint { x: 0.0, y: 0.0 },
            size: CGSize {
                width: 0.0,
                height: 0.0,
            },
        };
        unsafe { CGRectMakeWithDictionaryRepresentation(value, &mut rect) }.then_some(rect)
    }
}
