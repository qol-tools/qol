use std::collections::HashSet;
use std::ffi::c_void;

use super::objc::{
    ax_attr, cf_boolean_false, cf_boolean_true, cfstr, cfstring_to_string, cg_window_layer,
    cg_window_owner_pid, cls, dict_get_i32, msg_bool_usize, msg_i32, msg_ptr, msg_ptr_usize, sel,
    AXUIElementCreateApplication, AXUIElementPerformAction, AXUIElementSetAttributeValue,
    AXValueCreate, AXValueGetValue, CFArrayGetCount, CFArrayGetValueAtIndex, CFRelease, CGPoint,
    CGSize, CGWindowListCopyWindowInfo, CfGuard, AX_VALUE_TYPE_CG_POINT, AX_VALUE_TYPE_CG_SIZE,
    CG_WINDOW_LIST_EXCLUDE_DESKTOP, CG_WINDOW_LIST_OPTION_ON_SCREEN_ONLY,
};

/// App element + the best target window (focused, or AXWindows[0] fallback).
/// `_keeper` owns the CF reference that `win` points into.
struct FrontTarget {
    app: CfGuard,
    _keeper: CfGuard,
    win: *const c_void,
}

fn front_target(pid: i32) -> Option<FrontTarget> {
    let app = CfGuard::new(unsafe { AXUIElementCreateApplication(pid) })?;
    if let Some(focused) = ax_attr(app.as_ptr(), "AXFocusedWindow") {
        let win = focused.as_ptr();
        return Some(FrontTarget {
            app,
            _keeper: focused,
            win,
        });
    }
    let windows = ax_attr(app.as_ptr(), "AXWindows")?;
    if unsafe { CFArrayGetCount(windows.as_ptr()) } == 0 {
        return None;
    }
    let win = unsafe { CFArrayGetValueAtIndex(windows.as_ptr(), 0) };
    Some(FrontTarget {
        app,
        _keeper: windows,
        win,
    })
}

fn activate_app(pid: i32, options: usize) {
    unsafe {
        let ns_app = msg_ptr_usize(
            cls("NSRunningApplication"),
            sel("runningApplicationWithProcessIdentifier:"),
            pid as usize,
        );
        if ns_app.is_null() {
            return;
        }
        msg_bool_usize(ns_app, sel("activateWithOptions:"), options);
    }
}

fn read_ax_position(win: *const c_void) -> Option<CGPoint> {
    let pos_ref = ax_attr(win, "AXPosition")?;
    let mut pos = CGPoint { x: 0.0, y: 0.0 };
    unsafe {
        AXValueGetValue(
            pos_ref.as_const(),
            AX_VALUE_TYPE_CG_POINT,
            &mut pos as *mut _ as *mut c_void,
        );
    }
    Some(pos)
}

fn ax_set_position(win: *const c_void, pos: &CGPoint) {
    let val = CfGuard::new(unsafe {
        AXValueCreate(AX_VALUE_TYPE_CG_POINT, pos as *const _ as *const c_void)
    })
    .unwrap();
    let attr = CfGuard::new(cfstr("AXPosition")).unwrap();
    unsafe {
        AXUIElementSetAttributeValue(win, attr.as_const(), val.as_const());
    }
}

/// Nudge window position +1px then back to force WindowServer to re-register input.
fn nudge_position(win: *const c_void) {
    let Some(pos) = read_ax_position(win) else {
        return;
    };
    ax_set_position(
        win,
        &CGPoint {
            x: pos.x + 1.0,
            y: pos.y,
        },
    );
    ax_set_position(win, &pos);
}

fn plain_minimize(win: *const c_void) {
    let ax_minimized = CfGuard::new(cfstr("AXMinimized")).unwrap();
    unsafe {
        AXUIElementSetAttributeValue(win, ax_minimized.as_const(), cf_boolean_true());
    }
}

/// Returns true for real application windows (standard windows and dialogs),
/// false for transient overlays, badges, floating panels, etc.
pub(super) fn is_normal_window(pid: i32) -> bool {
    let Some(ft) = front_target(pid) else {
        #[cfg(debug_assertions)]
        eprintln!("[window-actions:dbg] is_normal_window pid={pid}: no windows");
        return false;
    };
    let Some(subrole_ref) = ax_attr(ft.win, "AXSubrole") else {
        #[cfg(debug_assertions)]
        eprintln!("[window-actions:dbg] is_normal_window pid={pid}: no AXSubrole");
        return false;
    };
    let subrole = cfstring_to_string(subrole_ref.as_const());
    #[cfg(debug_assertions)]
    eprintln!("[window-actions:dbg] is_normal_window pid={pid}: subrole={subrole:?}");
    matches!(subrole.as_deref(), Some("AXStandardWindow" | "AXDialog"))
}

pub(super) fn frontmost_pid() -> Option<i32> {
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
        if pid <= 0 {
            return None;
        }
        Some(pid)
    }
}

pub(super) fn find_normal_window_pid() -> Option<i32> {
    let frontmost = frontmost_pid();
    #[cfg(debug_assertions)]
    eprintln!("[window-actions:dbg] find_normal_window_pid: frontmost={frontmost:?}");
    if frontmost.is_some_and(is_normal_window) {
        #[cfg(debug_assertions)]
        eprintln!("[window-actions:dbg] find_normal_window_pid: fast path");
        return frontmost;
    }

    #[cfg(debug_assertions)]
    eprintln!("[window-actions:dbg] find_normal_window_pid: walking CGWindowList");

    let list = unsafe {
        CGWindowListCopyWindowInfo(
            CG_WINDOW_LIST_OPTION_ON_SCREEN_ONLY | CG_WINDOW_LIST_EXCLUDE_DESKTOP,
            0,
        )
    };
    if list.is_null() {
        return frontmost;
    }

    let count = unsafe { CFArrayGetCount(list) };
    let mut seen = HashSet::new();
    let mut result = None;

    for i in 0..count {
        let dict = unsafe { CFArrayGetValueAtIndex(list, i) };
        let layer = dict_get_i32(dict, cg_window_layer()).unwrap_or(-1);
        if layer != 0 {
            continue;
        }
        let pid = match dict_get_i32(dict, cg_window_owner_pid()) {
            Some(p) if p > 0 => p,
            _ => continue,
        };
        if !seen.insert(pid) {
            continue;
        }
        #[cfg(debug_assertions)]
        eprintln!("[window-actions:dbg] find_normal_window_pid: checking pid={pid}");
        if is_normal_window(pid) {
            result = Some(pid);
            break;
        }
    }

    unsafe { CFRelease(list) };
    #[cfg(debug_assertions)]
    eprintln!(
        "[window-actions:dbg] find_normal_window_pid: result={result:?} (fallback={frontmost:?})"
    );
    result.or(frontmost)
}

pub(super) fn front_window_rect(pid: i32) -> Option<super::screen::Rect> {
    let ft = front_target(pid)?;
    let pos = read_ax_position(ft.win)?;
    let size_ref = ax_attr(ft.win, "AXSize")?;

    let mut size = CGSize {
        width: 0.0,
        height: 0.0,
    };
    unsafe {
        AXValueGetValue(
            size_ref.as_const(),
            AX_VALUE_TYPE_CG_SIZE,
            &mut size as *mut _ as *mut c_void,
        );
    }
    Some(super::screen::Rect {
        x: pos.x,
        y: pos.y,
        w: size.width,
        h: size.height,
    })
}

pub(super) fn set_position_and_size(pid: i32, rect: super::screen::Rect) -> bool {
    let Some(ft) = front_target(pid) else {
        return false;
    };
    let pos = CGPoint {
        x: rect.x,
        y: rect.y,
    };
    let size = CGSize {
        width: rect.w,
        height: rect.h,
    };
    let pos_val = CfGuard::new(unsafe {
        AXValueCreate(AX_VALUE_TYPE_CG_POINT, &pos as *const _ as *const c_void)
    })
    .unwrap();
    let size_val = CfGuard::new(unsafe {
        AXValueCreate(AX_VALUE_TYPE_CG_SIZE, &size as *const _ as *const c_void)
    })
    .unwrap();

    let ax_pos = CfGuard::new(cfstr("AXPosition")).unwrap();
    let ax_size = CfGuard::new(cfstr("AXSize")).unwrap();

    // size → position → size: macOS clamps windows to screen edges,
    // so neither order works alone. Second size corrects clamping.
    #[cfg(debug_assertions)]
    eprintln!(
        "[window-actions:dbg] set_position_and_size: pos({},{}) size({},{})",
        rect.x, rect.y, rect.w, rect.h
    );
    unsafe {
        let e1 = AXUIElementSetAttributeValue(ft.win, ax_size.as_const(), size_val.as_const());
        let e2 = AXUIElementSetAttributeValue(ft.win, ax_pos.as_const(), pos_val.as_const());
        AXUIElementSetAttributeValue(ft.win, ax_size.as_const(), size_val.as_const());
        e1 == 0 && e2 == 0
    }
}

/// Minimize the focused window. Strategy depends on window count:
///   - Single visible window: AXHidden=true on the app (instant, no animation).
///   - Multiple visible windows: AXMinimized=true on the focused window only (animated but fine-grained).
pub(super) fn instant_minimize(pid: i32) -> bool {
    let Some(ft) = front_target(pid) else {
        return false;
    };

    if visible_window_count(ft.app.as_ptr()) > 1 {
        plain_minimize(ft.win);
        return true;
    }

    let ax_hidden = CfGuard::new(cfstr("AXHidden")).unwrap();
    unsafe {
        AXUIElementSetAttributeValue(ft.app.as_ptr(), ax_hidden.as_const(), cf_boolean_true());
    }
    true
}

fn visible_window_count(app: *const c_void) -> usize {
    let Some(windows) = ax_attr(app, "AXWindows") else {
        return 0;
    };
    let count = unsafe { CFArrayGetCount(windows.as_ptr()) };
    let mut visible = 0;
    for i in 0..count {
        let win = unsafe { CFArrayGetValueAtIndex(windows.as_ptr(), i) };
        let minimized = ax_attr(win, "AXMinimized")
            .is_some_and(|v| std::ptr::eq(v.as_ptr(), cf_boolean_true() as *mut c_void));
        if !minimized {
            visible += 1;
        }
    }
    visible
}

pub(super) fn unminimize_and_raise(pid: i32) -> bool {
    let app = CfGuard::new(unsafe { AXUIElementCreateApplication(pid) });
    let Some(app) = app else {
        return false;
    };

    // Check if app was hidden (AXHidden) vs individual window minimized (AXMinimized).
    let was_hidden = ax_attr(app.as_ptr(), "AXHidden")
        .is_some_and(|v| std::ptr::eq(v.as_ptr(), cf_boolean_true() as *mut c_void));

    if was_hidden {
        // App-level hide: unhide and activate all windows (they were all hidden together).
        let ax_hidden = CfGuard::new(cfstr("AXHidden")).unwrap();
        unsafe {
            AXUIElementSetAttributeValue(app.as_ptr(), ax_hidden.as_const(), cf_boolean_false());
        }
        activate_app(pid, 3); // IgnoringOtherApps | AllWindows
        return true;
    }

    // Individual window minimize: only restore one window.
    if let Some(windows) = ax_attr(app.as_ptr(), "AXWindows") {
        let ax_minimized = CfGuard::new(cfstr("AXMinimized")).unwrap();
        let ax_raise = CfGuard::new(cfstr("AXRaise")).unwrap();
        let count = unsafe { CFArrayGetCount(windows.as_ptr()) };
        for i in 0..count {
            let win = unsafe { CFArrayGetValueAtIndex(windows.as_ptr(), i) };
            let Some(val) = ax_attr(win, "AXMinimized") else {
                continue;
            };
            if !std::ptr::eq(val.as_ptr(), cf_boolean_true() as *mut c_void) {
                continue;
            }
            unsafe {
                AXUIElementSetAttributeValue(win, ax_minimized.as_const(), cf_boolean_false());
                AXUIElementPerformAction(win, ax_raise.as_const());
            }
            nudge_position(win);
            break;
        }
    }

    // Only raise the restored window, not all app windows.
    activate_app(pid, 2); // IgnoringOtherApps (without AllWindows)
    true
}
