use std::process::Command;

use crate::restore::WindowSystem;

const LAUNCHER_MATCH_MARKERS: [&str; 4] = [
    "qol-tray-launcher",
    "plugin-launcher",
    "qol-launcher",
    "qol launcher",
];

pub struct MacWindowSystem;

impl WindowSystem for MacWindowSystem {
    fn active_window_id(&self) -> Result<Option<String>, String> {
        match ax::frontmost_pid() {
            Some(pid) if pid > 0 => Ok(Some(format!("pid:{pid}:0"))),
            _ => Ok(None),
        }
    }

    fn minimize_window(&self, window_id: &str) -> Result<bool, String> {
        let pid = parse_pid(window_id).ok_or_else(|| format!("Invalid window ID: {window_id}"))?;
        Ok(ax::minimize_front_window(pid as i32))
    }

    fn stacking_window_ids(&self) -> Result<Vec<String>, String> {
        // System Events can't enumerate minimized windows on macOS.
        Ok(vec![])
    }

    fn is_window_id(&self, id: &str) -> bool {
        id.starts_with("pid:")
    }

    fn normalize_window_id(&self, window_id: &str) -> Option<String> {
        if self.is_window_id(window_id) {
            Some(window_id.to_string())
        } else {
            None
        }
    }

    fn is_excluded_window_type(&self, _window_id: &str) -> Result<bool, String> {
        Ok(false)
    }

    fn is_hidden_window(&self, _window_id: &str) -> Result<bool, String> {
        // System Events cannot read miniaturized state of minimized windows on macOS.
        // Return true so try_restore_window proceeds to activate_window.
        Ok(true)
    }

    fn is_launcher_window(&self, window_id: &str) -> bool {
        let Some(pid) = parse_pid(window_id) else {
            return false;
        };
        process_name(pid as u32)
            .map(|name| {
                let lower = name.to_ascii_lowercase();
                LAUNCHER_MATCH_MARKERS.iter().any(|m| lower.contains(m))
            })
            .unwrap_or(false)
    }

    fn activate_window(&self, window_id: &str) -> Result<bool, String> {
        let pid = parse_pid(window_id).ok_or_else(|| format!("Invalid window ID: {window_id}"))?;
        Ok(ax::unminimize_and_raise(pid as i32))
    }

    fn window_pid(&self, window_id: &str) -> Result<Option<u32>, String> {
        Ok(parse_pid(window_id).map(|p| p as u32))
    }

    fn process_start_ticks(&self, pid: u32) -> Option<u64> {
        let output = Command::new("ps")
            .args(["-o", "lstart=", "-p", &pid.to_string()])
            .output()
            .ok()?;
        if !output.status.success() {
            return None;
        }
        let raw = String::from_utf8_lossy(&output.stdout);
        let trimmed = raw.trim();
        if trimmed.is_empty() {
            return None;
        }
        Some(fnv1a(trimmed.as_bytes()))
    }
}

// -- Geometric actions (pure Rust via AX + ObjC runtime — no osascript) --

fn frontmost_screen() -> Result<(i32, ax::Rect), String> {
    let pid = ax::frontmost_pid().ok_or("No frontmost application")?;
    let (wx, wy, ww, wh) =
        ax::front_window_rect(pid).ok_or("Cannot read window geometry")?;
    let scr = ax::screen_for_point(wx + ww / 2.0, wy + wh / 2.0)
        .ok_or("Cannot determine screen")?;
    Ok((pid, scr))
}

fn ax_set(pid: i32, x: f64, y: f64, w: f64, h: f64) -> Result<(), String> {
    if !ax::set_position_and_size(pid, x, y, w, h) {
        return Err("Failed to set window geometry".into());
    }
    Ok(())
}

pub fn snap_left() -> Result<(), String> {
    let (pid, s) = frontmost_screen()?;
    ax_set(pid, s.x, s.y, s.w / 2.0, s.h)
}

pub fn snap_right() -> Result<(), String> {
    let (pid, s) = frontmost_screen()?;
    ax_set(pid, s.x + s.w / 2.0, s.y, s.w / 2.0, s.h)
}

pub fn snap_bottom() -> Result<(), String> {
    let (pid, s) = frontmost_screen()?;
    ax_set(pid, s.x, s.y + s.h / 2.0, s.w, s.h / 2.0)
}

pub fn maximize() -> Result<(), String> {
    let (pid, s) = frontmost_screen()?;
    ax_set(pid, s.x, s.y, s.w, s.h)
}

pub fn center() -> Result<(), String> {
    let (pid, s) = frontmost_screen()?;
    let w = 1152.0_f64.min(s.w);
    let h = 892.0_f64.min(s.h);
    ax_set(pid, s.x + (s.w - w) / 2.0, s.y + (s.h - h) / 2.0, w, h)
}

pub fn move_monitor_left() -> Result<(), String> {
    move_monitor(-1)
}

pub fn move_monitor_right() -> Result<(), String> {
    move_monitor(1)
}

fn move_monitor(delta: i32) -> Result<(), String> {
    let pid = ax::frontmost_pid().ok_or("No frontmost application")?;
    let (wx, wy, ww, wh) =
        ax::front_window_rect(pid).ok_or("Cannot read window geometry")?;
    let cx = wx + ww / 2.0;
    let cy = wy + wh / 2.0;

    let screens = ax::all_screens_sorted();
    if screens.len() < 2 {
        return Ok(());
    }

    let from_idx = screens
        .iter()
        .position(|s| cx >= s.x && cx < s.x + s.w && cy >= s.y && cy < s.y + s.h)
        .unwrap_or(0);

    let to_idx = ((from_idx as i32 + delta).rem_euclid(screens.len() as i32)) as usize;
    let from = &screens[from_idx];
    let to = &screens[to_idx];

    let x_ratio = (wx - from.x) / from.w;
    let y_ratio = (wy - from.y) / from.h;
    let w_ratio = ww / from.w;
    let h_ratio = wh / from.h;

    ax_set(
        pid,
        (to.x + x_ratio * to.w).round(),
        (to.y + y_ratio * to.h).round(),
        (w_ratio * to.w).round(),
        (h_ratio * to.h).round(),
    )
}

// -- Accessibility + ObjC runtime FFI --

mod ax {
    use std::ffi::{c_void, CString};

    #[repr(C)]
    struct CGPoint {
        x: f64,
        y: f64,
    }

    #[repr(C)]
    struct CGSize {
        width: f64,
        height: f64,
    }

    #[repr(C)]
    struct CGRect {
        origin: CGPoint,
        size: CGSize,
    }

    #[derive(Clone, Copy)]
    pub struct Rect {
        pub x: f64,
        pub y: f64,
        pub w: f64,
        pub h: f64,
    }

    const AX_VALUE_TYPE_CG_POINT: u32 = 1;
    const AX_VALUE_TYPE_CG_SIZE: u32 = 2;
    const UTF8: u32 = 0x08000100;

    // -- Framework bindings --

    #[link(name = "ApplicationServices", kind = "framework")]
    extern "C" {
        fn AXUIElementCreateApplication(pid: i32) -> *mut c_void;
        fn AXUIElementCopyAttributeValue(
            element: *const c_void,
            attribute: *const c_void,
            value: *mut *mut c_void,
        ) -> i32;
        fn AXUIElementSetAttributeValue(
            element: *const c_void,
            attribute: *const c_void,
            value: *const c_void,
        ) -> i32;
        fn AXValueCreate(value_type: u32, value: *const c_void) -> *mut c_void;
        fn AXValueGetValue(
            value: *const c_void,
            value_type: u32,
            value_ptr: *mut c_void,
        ) -> u8;
    }

    #[link(name = "CoreFoundation", kind = "framework")]
    extern "C" {
        fn CFStringCreateWithCString(
            allocator: *const c_void,
            c_str: *const i8,
            encoding: u32,
        ) -> *mut c_void;
        fn CFArrayGetCount(array: *const c_void) -> isize;
        fn CFArrayGetValueAtIndex(array: *const c_void, idx: isize) -> *const c_void;
        fn CFRelease(cf: *const c_void);
        static kCFBooleanFalse: *const c_void;
        static kCFBooleanTrue: *const c_void;
    }

    #[link(name = "objc", kind = "dylib")]
    extern "C" {
        fn objc_getClass(name: *const i8) -> *mut c_void;
        fn sel_registerName(name: *const i8) -> *mut c_void;
        fn objc_msgSend(obj: *mut c_void, sel: *mut c_void, ...) -> *mut c_void;
    }

    // Link AppKit for NSScreen / NSWorkspace.
    #[link(name = "AppKit", kind = "framework")]
    extern "C" {}

    #[cfg(target_arch = "x86_64")]
    extern "C" {
        fn objc_msgSend_stret(stret: *mut c_void, obj: *mut c_void, sel: *mut c_void, ...);
    }

    // -- ObjC helpers --

    fn cfstr(s: &str) -> *mut c_void {
        let c = CString::new(s).unwrap();
        unsafe { CFStringCreateWithCString(std::ptr::null(), c.as_ptr(), UTF8) }
    }

    fn sel(name: &str) -> *mut c_void {
        let c = CString::new(name).unwrap();
        unsafe { sel_registerName(c.as_ptr()) }
    }

    fn cls(name: &str) -> *mut c_void {
        let c = CString::new(name).unwrap();
        unsafe { objc_getClass(c.as_ptr()) }
    }

    unsafe fn msg_ptr(obj: *mut c_void, sel: *mut c_void) -> *mut c_void {
        objc_msgSend(obj, sel)
    }

    unsafe fn msg_i32(obj: *mut c_void, sel: *mut c_void) -> i32 {
        let f: unsafe extern "C" fn(*mut c_void, *mut c_void) -> i32 =
            std::mem::transmute(objc_msgSend as usize);
        f(obj, sel)
    }

    unsafe fn msg_usize(obj: *mut c_void, sel: *mut c_void) -> usize {
        let f: unsafe extern "C" fn(*mut c_void, *mut c_void) -> usize =
            std::mem::transmute(objc_msgSend as usize);
        f(obj, sel)
    }

    unsafe fn msg_ptr_usize(
        obj: *mut c_void,
        sel: *mut c_void,
        arg: usize,
    ) -> *mut c_void {
        let f: unsafe extern "C" fn(*mut c_void, *mut c_void, usize) -> *mut c_void =
            std::mem::transmute(objc_msgSend as usize);
        f(obj, sel, arg)
    }

    #[cfg(target_arch = "aarch64")]
    unsafe fn msg_rect(obj: *mut c_void, sel: *mut c_void) -> CGRect {
        let f: unsafe extern "C" fn(*mut c_void, *mut c_void) -> CGRect =
            std::mem::transmute(objc_msgSend as usize);
        f(obj, sel)
    }

    #[cfg(target_arch = "x86_64")]
    unsafe fn msg_rect(obj: *mut c_void, sel: *mut c_void) -> CGRect {
        let mut rect: CGRect = std::mem::zeroed();
        objc_msgSend_stret(
            &mut rect as *mut _ as *mut c_void,
            obj,
            sel,
        );
        rect
    }

    // -- Public API --

    pub fn frontmost_pid() -> Option<i32> {
        unsafe {
            let workspace = msg_ptr(cls("NSWorkspace"), sel("sharedWorkspace"));
            if workspace.is_null() {
                return None;
            }
            let app = msg_ptr(workspace, sel("frontmostApplication"));
            if app.is_null() {
                return None;
            }
            let pid = msg_i32(app, sel("processIdentifier"));
            if pid <= 0 { None } else { Some(pid) }
        }
    }

    pub fn front_window_rect(pid: i32) -> Option<(f64, f64, f64, f64)> {
        unsafe {
            let app = AXUIElementCreateApplication(pid);
            if app.is_null() {
                return None;
            }

            let ax_windows = cfstr("AXWindows");
            let mut windows_ref: *mut c_void = std::ptr::null_mut();
            let err =
                AXUIElementCopyAttributeValue(app, ax_windows, &mut windows_ref);
            CFRelease(ax_windows);

            if err != 0 || windows_ref.is_null() {
                CFRelease(app);
                return None;
            }

            if CFArrayGetCount(windows_ref) == 0 {
                CFRelease(windows_ref);
                CFRelease(app);
                return None;
            }

            let win = CFArrayGetValueAtIndex(windows_ref, 0);

            let ax_pos = cfstr("AXPosition");
            let ax_size = cfstr("AXSize");
            let mut pos_ref: *mut c_void = std::ptr::null_mut();
            let mut size_ref: *mut c_void = std::ptr::null_mut();

            let e1 = AXUIElementCopyAttributeValue(win, ax_pos, &mut pos_ref);
            let e2 = AXUIElementCopyAttributeValue(win, ax_size, &mut size_ref);
            CFRelease(ax_pos);
            CFRelease(ax_size);

            let result =
                if e1 == 0 && e2 == 0 && !pos_ref.is_null() && !size_ref.is_null() {
                    let mut pos = CGPoint { x: 0.0, y: 0.0 };
                    let mut size = CGSize { width: 0.0, height: 0.0 };
                    AXValueGetValue(
                        pos_ref,
                        AX_VALUE_TYPE_CG_POINT,
                        &mut pos as *mut _ as *mut c_void,
                    );
                    AXValueGetValue(
                        size_ref,
                        AX_VALUE_TYPE_CG_SIZE,
                        &mut size as *mut _ as *mut c_void,
                    );
                    Some((pos.x, pos.y, size.width, size.height))
                } else {
                    None
                };

            if !pos_ref.is_null() {
                CFRelease(pos_ref);
            }
            if !size_ref.is_null() {
                CFRelease(size_ref);
            }
            CFRelease(windows_ref);
            CFRelease(app);
            result
        }
    }

    fn primary_screen_height() -> f64 {
        unsafe {
            let screens = msg_ptr(cls("NSScreen"), sel("screens"));
            if screens.is_null() {
                return 0.0;
            }
            let count = msg_usize(screens, sel("count"));
            if count == 0 {
                return 0.0;
            }
            let primary = msg_ptr_usize(screens, sel("objectAtIndex:"), 0);
            let frame = msg_rect(primary, sel("frame"));
            frame.size.height
        }
    }

    fn cocoa_to_ax(frame: CGRect, primary_h: f64) -> Rect {
        Rect {
            x: frame.origin.x,
            y: primary_h - frame.origin.y - frame.size.height,
            w: frame.size.width,
            h: frame.size.height,
        }
    }

    pub fn screen_for_point(cx: f64, cy: f64) -> Option<Rect> {
        unsafe {
            let primary_h = primary_screen_height();
            if primary_h == 0.0 {
                return None;
            }

            let screens = msg_ptr(cls("NSScreen"), sel("screens"));
            if screens.is_null() {
                return None;
            }

            let count = msg_usize(screens, sel("count"));
            let mut fallback = None;

            for i in 0..count {
                let screen =
                    msg_ptr_usize(screens, sel("objectAtIndex:"), i);
                let vf = msg_rect(screen, sel("visibleFrame"));
                let ax = cocoa_to_ax(vf, primary_h);
                if fallback.is_none() {
                    fallback = Some(ax);
                }
                if cx >= ax.x && cx < ax.x + ax.w && cy >= ax.y && cy < ax.y + ax.h
                {
                    return Some(ax);
                }
            }

            fallback
        }
    }

    pub fn all_screens_sorted() -> Vec<Rect> {
        unsafe {
            let primary_h = primary_screen_height();
            if primary_h == 0.0 {
                return vec![];
            }

            let screens = msg_ptr(cls("NSScreen"), sel("screens"));
            if screens.is_null() {
                return vec![];
            }

            let count = msg_usize(screens, sel("count"));
            let mut result = Vec::with_capacity(count);

            for i in 0..count {
                let screen =
                    msg_ptr_usize(screens, sel("objectAtIndex:"), i);
                let vf = msg_rect(screen, sel("visibleFrame"));
                result.push(cocoa_to_ax(vf, primary_h));
            }

            result.sort_by(|a, b| {
                a.x.partial_cmp(&b.x).unwrap_or(std::cmp::Ordering::Equal)
            });
            result
        }
    }

    pub fn set_position_and_size(
        pid: i32,
        x: f64,
        y: f64,
        w: f64,
        h: f64,
    ) -> bool {
        unsafe {
            let app = AXUIElementCreateApplication(pid);
            if app.is_null() {
                return false;
            }

            let ax_windows = cfstr("AXWindows");
            let mut windows_ref: *mut c_void = std::ptr::null_mut();
            let err =
                AXUIElementCopyAttributeValue(app, ax_windows, &mut windows_ref);
            CFRelease(ax_windows);

            if err != 0 || windows_ref.is_null() {
                CFRelease(app);
                return false;
            }

            let ok = if CFArrayGetCount(windows_ref) > 0 {
                let win = CFArrayGetValueAtIndex(windows_ref, 0);

                let pos = CGPoint { x, y };
                let size = CGSize { width: w, height: h };

                let ax_pos = cfstr("AXPosition");
                let ax_size = cfstr("AXSize");

                let pos_val = AXValueCreate(
                    AX_VALUE_TYPE_CG_POINT,
                    &pos as *const _ as *const c_void,
                );
                let size_val = AXValueCreate(
                    AX_VALUE_TYPE_CG_SIZE,
                    &size as *const _ as *const c_void,
                );

                // Position → size → position: macOS may adjust position
                // when resizing across monitors, so we correct it after.
                let e1 = AXUIElementSetAttributeValue(win, ax_pos, pos_val);
                let e2 = AXUIElementSetAttributeValue(win, ax_size, size_val);
                AXUIElementSetAttributeValue(win, ax_pos, pos_val);

                CFRelease(pos_val);
                CFRelease(size_val);
                CFRelease(ax_pos);
                CFRelease(ax_size);

                e1 == 0 && e2 == 0
            } else {
                false
            };

            CFRelease(windows_ref);
            CFRelease(app);
            ok
        }
    }

    pub fn minimize_front_window(pid: i32) -> bool {
        unsafe {
            let app = AXUIElementCreateApplication(pid);
            if app.is_null() {
                return false;
            }

            let ax_windows = cfstr("AXWindows");
            let mut windows_ref: *mut c_void = std::ptr::null_mut();
            let err =
                AXUIElementCopyAttributeValue(app, ax_windows, &mut windows_ref);
            CFRelease(ax_windows);

            if err != 0 || windows_ref.is_null() {
                CFRelease(app);
                return false;
            }

            let ok = if CFArrayGetCount(windows_ref) > 0 {
                let win = CFArrayGetValueAtIndex(windows_ref, 0);
                let ax_minimized = cfstr("AXMinimized");
                let err = AXUIElementSetAttributeValue(
                    win,
                    ax_minimized,
                    kCFBooleanTrue,
                );
                CFRelease(ax_minimized);
                err == 0
            } else {
                false
            };

            CFRelease(windows_ref);
            CFRelease(app);
            ok
        }
    }

    pub fn unminimize_and_raise(pid: i32) -> bool {
        unsafe {
            let app = AXUIElementCreateApplication(pid);
            if app.is_null() {
                return false;
            }

            let ax_windows = cfstr("AXWindows");
            let mut windows_ref: *mut c_void = std::ptr::null_mut();
            let err =
                AXUIElementCopyAttributeValue(app, ax_windows, &mut windows_ref);
            CFRelease(ax_windows);

            if err != 0 || windows_ref.is_null() {
                CFRelease(app);
                return false;
            }

            // Only unminimize the first minimized window, not all of them.
            let ax_minimized = cfstr("AXMinimized");
            let count = CFArrayGetCount(windows_ref);
            for i in 0..count {
                let win = CFArrayGetValueAtIndex(windows_ref, i);
                let mut val: *mut c_void = std::ptr::null_mut();
                let err = AXUIElementCopyAttributeValue(
                    win,
                    ax_minimized,
                    &mut val,
                );
                if err == 0
                    && !val.is_null()
                    && val == kCFBooleanTrue as *mut c_void
                {
                    AXUIElementSetAttributeValue(
                        win,
                        ax_minimized,
                        kCFBooleanFalse,
                    );
                    break;
                }
            }
            CFRelease(ax_minimized);

            // Bring app to front.
            let ax_frontmost = cfstr("AXFrontmost");
            AXUIElementSetAttributeValue(app, ax_frontmost, kCFBooleanTrue);
            CFRelease(ax_frontmost);

            CFRelease(windows_ref);
            CFRelease(app);
            true
        }
    }
}

// -- Helpers --

fn parse_pid(window_id: &str) -> Option<i64> {
    window_id.split(':').nth(1)?.parse().ok()
}

fn fnv1a(bytes: &[u8]) -> u64 {
    let mut hash: u64 = 0xcbf29ce484222325;
    for &b in bytes {
        hash ^= b as u64;
        hash = hash.wrapping_mul(0x100000001b3);
    }
    hash
}

fn process_name(pid: u32) -> Option<String> {
    let output = Command::new("ps")
        .args(["-o", "comm=", "-p", &pid.to_string()])
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    let name = String::from_utf8_lossy(&output.stdout).trim().to_string();
    if name.is_empty() { None } else { Some(name) }
}
