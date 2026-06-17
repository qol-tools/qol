use std::collections::HashSet;
use std::ffi::c_void;
use std::thread::sleep;
use std::time::{Duration, Instant};

use super::objc::{
    ax_attr, cf_boolean_false, cf_boolean_true, cfstr, cfstring_to_string, cg_window_layer,
    cg_window_owner_pid, cls, dict_get_i32, msg_bool, msg_bool_usize, msg_i32, msg_ptr,
    msg_ptr_usize, sel, AXUIElementCreateApplication, AXUIElementPerformAction,
    AXUIElementSetAttributeValue, AXValueCreate, AXValueGetValue, CFArrayGetCount,
    CFArrayGetValueAtIndex, CFRelease, CGPoint, CGSize, CGWindowListCopyWindowInfo, CfGuard,
    AX_VALUE_TYPE_CG_POINT, AX_VALUE_TYPE_CG_SIZE, CG_WINDOW_LIST_EXCLUDE_DESKTOP,
    CG_WINDOW_LIST_OPTION_ON_SCREEN_ONLY,
};
use super::trace::{timed_bool, timed_opt, timed_pred, timed_unit};

const VERIFY_TIMEOUT: Duration = Duration::from_millis(120);
const VERIFY_INTERVAL: Duration = Duration::from_millis(8);

/// App element + the best target window (focused, or AXWindows[0] fallback).
/// `_keeper` owns the CF reference that `win` points into.
struct FrontTarget {
    app: CfGuard,
    _keeper: CfGuard,
    win: *const c_void,
}

fn front_target(pid: i32) -> Option<FrontTarget> {
    timed_opt("front_target", pid, || {
        let app = CfGuard::new(unsafe { AXUIElementCreateApplication(pid) })?;
        if let Some(focused) = ax_attr(app.as_ptr(), "AXFocusedWindow") {
            if is_targetable_window(focused.as_ptr()) {
                let win = focused.as_ptr();
                return Some(FrontTarget {
                    app,
                    _keeper: focused,
                    win,
                });
            }
        }
        let windows = ax_attr(app.as_ptr(), "AXWindows")?;
        let count = unsafe { CFArrayGetCount(windows.as_ptr()) };
        if count == 0 {
            return None;
        }

        for i in 0..count {
            let win = unsafe { CFArrayGetValueAtIndex(windows.as_ptr(), i) };
            if is_targetable_window(win) {
                return Some(FrontTarget {
                    app,
                    _keeper: windows,
                    win,
                });
            }
        }

        let win = unsafe { CFArrayGetValueAtIndex(windows.as_ptr(), 0) };
        Some(FrontTarget {
            app,
            _keeper: windows,
            win,
        })
    })
}

fn ns_running_app(pid: i32) -> Option<*mut c_void> {
    let ns_app = unsafe {
        msg_ptr_usize(
            cls("NSRunningApplication"),
            sel("runningApplicationWithProcessIdentifier:"),
            pid as usize,
        )
    };
    if ns_app.is_null() {
        return None;
    }
    Some(ns_app)
}

fn activate_app(pid: i32, options: usize) {
    timed_unit("activate_app", pid, || unsafe {
        let Some(ns_app) = ns_running_app(pid) else {
            return;
        };
        msg_bool_usize(ns_app, sel("activateWithOptions:"), options);
    });
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

fn plain_minimize(win: *const c_void) -> bool {
    let _ = set_ax_bool_attr(win, "AXMinimized", true);
    if wait_for_bool_attr(win, "AXMinimized", true) {
        return true;
    }

    false
}

fn verified_minimize(pid: i32, win: *const c_void) -> bool {
    timed_bool("set_minimized", pid, || {
        if plain_minimize(win) {
            return true;
        }
        let _ = press_minimize_button(pid, win);
        wait_for_bool_attr(win, "AXMinimized", true)
    })
}

fn press_minimize_button(pid: i32, win: *const c_void) -> bool {
    timed_bool("press_minimize", pid, || {
        let Some(button) = ax_attr(win, "AXMinimizeButton") else {
            return false;
        };
        let action = CfGuard::new(cfstr("AXPress")).unwrap();
        unsafe { AXUIElementPerformAction(button.as_const(), action.as_const()) == 0 }
    })
}

fn set_ax_bool_attr(element: *const c_void, name: &str, value: bool) -> bool {
    let attr = CfGuard::new(cfstr(name)).unwrap();
    let value = if value {
        cf_boolean_true()
    } else {
        cf_boolean_false()
    };
    unsafe { AXUIElementSetAttributeValue(element, attr.as_const(), value) == 0 }
}

fn ax_bool_attr(element: *const c_void, name: &str) -> Option<bool> {
    ax_attr(element, name).map(|v| std::ptr::eq(v.as_ptr(), cf_boolean_true() as *mut c_void))
}

fn ax_bool_attr_is(element: *const c_void, name: &str, expected: bool) -> bool {
    ax_bool_attr(element, name).is_some_and(|actual| actual == expected)
}

fn wait_for_bool_attr(element: *const c_void, name: &str, expected: bool) -> bool {
    if ax_bool_attr_is(element, name, expected) {
        return true;
    }

    let deadline = Instant::now() + VERIFY_TIMEOUT;
    while Instant::now() < deadline {
        sleep(VERIFY_INTERVAL);
        if ax_bool_attr_is(element, name, expected) {
            return true;
        }
    }

    false
}

fn is_targetable_window(win: *const c_void) -> bool {
    window_is_normal(win) && !window_is_minimized(win)
}

fn window_is_normal(win: *const c_void) -> bool {
    let Some(subrole_ref) = ax_attr(win, "AXSubrole") else {
        return false;
    };
    let subrole = cfstring_to_string(subrole_ref.as_const());
    matches!(subrole.as_deref(), Some("AXStandardWindow" | "AXDialog"))
}

fn window_is_minimized(win: *const c_void) -> bool {
    ax_bool_attr_is(win, "AXMinimized", true)
}

/// Returns true for real application windows (standard windows and dialogs),
/// false for transient overlays, badges, floating panels, etc.
pub(super) fn is_normal_window(pid: i32) -> bool {
    timed_pred("is_normal_window", pid, || {
        let Some(ft) = front_target(pid) else {
            return false;
        };
        window_is_normal(ft.win)
    })
}

pub(super) fn frontmost_pid() -> Option<i32> {
    timed_opt("frontmost_pid", 0, || unsafe {
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
    })
}

pub(super) fn find_normal_window_pid() -> Option<i32> {
    timed_opt("find_pid", 0, || {
        let frontmost = frontmost_pid();
        if frontmost.is_some_and(is_normal_window) {
            return frontmost;
        }

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
            if is_normal_window(pid) {
                result = Some(pid);
                break;
            }
        }

        unsafe { CFRelease(list) };
        result.or(frontmost)
    })
}

pub(super) fn front_window_rect(pid: i32) -> Option<super::screen::Rect> {
    timed_opt("front_window_rect", pid, || {
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
    })
}

pub(super) fn set_position_and_size(pid: i32, rect: super::screen::Rect) -> bool {
    timed_bool("set_pos_size", pid, || {
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
        unsafe {
            let e1 = AXUIElementSetAttributeValue(ft.win, ax_size.as_const(), size_val.as_const());
            let e2 = AXUIElementSetAttributeValue(ft.win, ax_pos.as_const(), pos_val.as_const());
            AXUIElementSetAttributeValue(ft.win, ax_size.as_const(), size_val.as_const());
            e1 == 0 && e2 == 0
        }
    })
}

/// Minimize the focused window.
///   - Single window of a regular app: `AXHidden=true` on the app (instant, no animation).
///   - Multiple windows, or a non-regular owner (e.g. Steam's Chromium helper, which
///     ignores app-level hide and only flashes): `AXMinimized=true` on the window.
pub(super) fn instant_minimize(pid: i32) -> bool {
    timed_bool("minimize", pid, || {
        let Some(ft) = front_target(pid) else {
            return false;
        };

        let visible = visible_window_count(ft.app.as_ptr());
        let regular = app_is_regular(pid);
        let use_minimize = visible > 1 || !regular;

        let (branch, ok) = if use_minimize {
            minimize_then_hide(pid, &ft)
        } else {
            hide_then_minimize(pid, &ft)
        };

        trace_minimize_branch(pid, branch, visible, regular, ok);
        ok
    })
}

fn minimize_then_hide(pid: i32, ft: &FrontTarget) -> (&'static str, bool) {
    if verified_minimize(pid, ft.win) {
        return ("minimize", true);
    }
    ("hide-fallback", hide_app(pid, ft.app.as_ptr()))
}

fn hide_then_minimize(pid: i32, ft: &FrontTarget) -> (&'static str, bool) {
    if hide_app(pid, ft.app.as_ptr()) {
        return ("hide", true);
    }
    ("minimize-fallback", verified_minimize(pid, ft.win))
}

fn hide_app(pid: i32, app: *const c_void) -> bool {
    timed_bool("hide_app", pid, || {
        let _ = set_ax_bool_attr(app, "AXHidden", true);
        if wait_for_app_hidden(pid, app, true) {
            return true;
        }
        let _ = ns_hide_app(pid);
        wait_for_app_hidden(pid, app, true)
    })
}

fn show_app(pid: i32, app: *const c_void) -> bool {
    timed_bool("show_app", pid, || {
        let _ = set_ax_bool_attr(app, "AXHidden", false);
        if wait_for_app_hidden(pid, app, false) {
            return true;
        }
        let _ = ns_unhide_app(pid);
        wait_for_app_hidden(pid, app, false)
    })
}

fn wait_for_app_hidden(pid: i32, app: *const c_void, expected: bool) -> bool {
    if app_is_hidden(pid, app) == expected {
        return true;
    }

    let deadline = Instant::now() + VERIFY_TIMEOUT;
    while Instant::now() < deadline {
        sleep(VERIFY_INTERVAL);
        if app_is_hidden(pid, app) == expected {
            return true;
        }
    }

    false
}

fn app_is_hidden(pid: i32, app: *const c_void) -> bool {
    ax_bool_attr_is(app, "AXHidden", true) || ns_app_is_hidden(pid)
}

fn ns_hide_app(pid: i32) -> bool {
    timed_bool("ns_hide", pid, || ns_app_bool(pid, "hide"))
}

fn ns_unhide_app(pid: i32) -> bool {
    timed_bool("ns_unhide", pid, || ns_app_bool(pid, "unhide"))
}

fn ns_app_is_hidden(pid: i32) -> bool {
    ns_app_bool(pid, "isHidden")
}

fn ns_app_bool(pid: i32, selector: &str) -> bool {
    let Some(ns_app) = ns_running_app(pid) else {
        return false;
    };
    unsafe { msg_bool(ns_app, sel(selector)) }
}

fn trace_minimize_branch(pid: i32, branch: &str, visible: usize, regular: bool, ok: bool) {
    #[cfg(debug_assertions)]
    qol_runtime::probe!(
        "WINACT_MINIMIZE",
        "pid={pid} branch={branch} visible={visible} regular={regular} outcome={}",
        if ok { "ok" } else { "fail" },
    );
    #[cfg(not(debug_assertions))]
    let _ = (pid, branch, visible, regular, ok);
}

fn app_is_regular(pid: i32) -> bool {
    timed_pred("app_is_regular", pid, || unsafe {
        let Some(ns_app) = ns_running_app(pid) else {
            return false;
        };
        msg_i32(ns_app, sel("activationPolicy")) == 0
    })
}

fn visible_window_count(app: *const c_void) -> usize {
    let Some(windows) = ax_attr(app, "AXWindows") else {
        return 0;
    };
    let count = unsafe { CFArrayGetCount(windows.as_ptr()) };
    let mut visible = 0;
    for i in 0..count {
        let win = unsafe { CFArrayGetValueAtIndex(windows.as_ptr(), i) };
        if is_targetable_window(win) {
            visible += 1;
        }
    }
    visible
}

/// Restore a previously minimized window. Matches the strategy used by `instant_minimize`:
///   - If the app was hidden (`AXHidden`): unhide and activate all windows.
///   - If an individual window was minimized (`AXMinimized`): unminimize only the
///     first minimized window and raise it, without bringing other app windows forward.
pub(super) fn unminimize_and_raise(pid: i32) -> bool {
    timed_bool("unminimize", pid, || {
        let app = CfGuard::new(unsafe { AXUIElementCreateApplication(pid) });
        let Some(app) = app else {
            return false;
        };

        // Check if app was hidden (AXHidden) vs individual window minimized (AXMinimized).
        let was_hidden = app_is_hidden(pid, app.as_ptr());

        if was_hidden {
            let _ = show_app(pid, app.as_ptr());
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
                if !ax_bool_attr_is(win, "AXMinimized", true) {
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
    })
}
