use std::collections::HashSet;
use std::ffi::c_void;
use std::thread::sleep;
use std::time::{Duration, Instant};

use super::objc::{
    ax_attr, ax_attr_result, cf_boolean_false, cf_boolean_true, cfstr, cfstring_to_string,
    cg_window_layer, cg_window_owner_pid, cls, dict_get_i32, msg_bool, msg_bool_usize, msg_i32,
    msg_ptr_usize, sel, AXUIElementCreateApplication, AXUIElementCreateSystemWide,
    AXUIElementGetPid, AXUIElementPerformAction, AXUIElementSetAttributeValue, AXValueCreate,
    AXValueGetValue, CFArrayGetCount, CFArrayGetValueAtIndex, CGPoint, CGSize,
    CGWindowListCopyWindowInfo, CfGuard, AX_VALUE_TYPE_CG_POINT, AX_VALUE_TYPE_CG_SIZE,
    CG_WINDOW_LIST_EXCLUDE_DESKTOP, CG_WINDOW_LIST_OPTION_ON_SCREEN_ONLY,
};
use super::trace::{timed_bool, timed_opt, timed_pid, timed_pred, trace_geometry};

const VERIFY_TIMEOUT: Duration = Duration::from_millis(120);
const VERIFY_INTERVAL: Duration = Duration::from_millis(8);
const RECT_TOLERANCE: f64 = 1.0;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum GeometryOutcome {
    Exact,
    Constrained,
    Adjusted,
    Unchanged,
    Unreadable,
}

impl GeometryOutcome {
    fn applied(self) -> bool {
        matches!(self, Self::Exact | Self::Constrained | Self::Adjusted)
    }

    fn as_str(self) -> &'static str {
        match self {
            Self::Exact => "exact",
            Self::Constrained => "constrained",
            Self::Adjusted => "adjusted",
            Self::Unchanged => "unchanged",
            Self::Unreadable => "unreadable",
        }
    }
}

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

fn activate_app(pid: i32, app: *const c_void, options: usize) -> bool {
    timed_bool("activate_app", pid, || unsafe {
        let _ = set_ax_bool_attr(app, "AXFrontmost", true);
        if wait_for_app_active(pid) {
            return true;
        }
        if let Some(ns_app) = ns_running_app(pid) {
            msg_bool_usize(ns_app, sel("activateWithOptions:"), options);
        }
        wait_for_app_active(pid)
    })
}

fn wait_for_app_active(pid: i32) -> bool {
    if app_is_active(pid) {
        return true;
    }

    let deadline = Instant::now() + VERIFY_TIMEOUT;
    while Instant::now() < deadline {
        sleep(VERIFY_INTERVAL);
        if app_is_active(pid) {
            return true;
        }
    }

    false
}

fn app_is_active(pid: i32) -> bool {
    ns_app_bool(pid, "isActive") || frontmost_pid() == Some(pid)
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

fn read_ax_size(win: *const c_void) -> Option<CGSize> {
    let size_ref = ax_attr(win, "AXSize")?;
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
    Some(size)
}

fn read_window_rect(win: *const c_void) -> Option<super::screen::Rect> {
    let pos = read_ax_position(win)?;
    let size = read_ax_size(win)?;
    Some(super::screen::Rect {
        x: pos.x,
        y: pos.y,
        w: size.width,
        h: size.height,
    })
}

fn verified_minimize(pid: i32, win: *const c_void) -> bool {
    timed_bool("set_minimized", pid, || {
        if set_ax_bool_attr(win, "AXMinimized", true) {
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
    timed_pid("frontmost_pid", "window_server_then_ax", || {
        let ax = system_focused_pid();
        let window_server = window_server_front_pid();
        let chosen = window_server.or_else(|| ax.pid());
        trace_focus_sources("frontmost_pid", ax, window_server, chosen);
        chosen
    })
}

pub(super) fn find_normal_window_pid() -> Option<i32> {
    timed_pid("find_pid", "window_server_then_ax", || {
        let ax = system_focused_pid();
        let Some(list) = on_screen_window_list() else {
            trace_focus_sources("find_pid", ax, None, ax.pid());
            return ax.pid();
        };
        let window_server = layer_zero_pids(window_entries(&list)).next();
        let chosen = select_normal_window_pid(ax.pid(), window_entries(&list), is_normal_window);
        trace_focus_sources("find_pid", ax, window_server, chosen);
        chosen
    })
}

/// The owner of the frontmost normal-layer window, straight from the window server.
/// Unlike `AXFocusedApplication` and `NSWorkspace.frontmostApplication`, this needs no
/// run loop, so it is the only focus source that stays correct inside a plugin daemon.
fn window_server_front_pid() -> Option<i32> {
    let list = on_screen_window_list()?;
    let front = layer_zero_pids(window_entries(&list)).next();
    front
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum AxFocus {
    Pid(i32),
    Error(i32),
    NoSystemElement,
    InvalidPid(i32),
}

impl AxFocus {
    fn pid(self) -> Option<i32> {
        match self {
            Self::Pid(pid) => Some(pid),
            Self::Error(_) | Self::NoSystemElement | Self::InvalidPid(_) => None,
        }
    }
}

fn system_focused_pid() -> AxFocus {
    let Some(system) = CfGuard::new(unsafe { AXUIElementCreateSystemWide() }) else {
        return AxFocus::NoSystemElement;
    };
    let app = match ax_attr_result(system.as_ptr(), "AXFocusedApplication") {
        Ok(app) => app,
        Err(code) => return AxFocus::Error(code),
    };
    let mut pid = 0;
    let result = unsafe { AXUIElementGetPid(app.as_const(), &mut pid) };
    if result != 0 {
        return AxFocus::Error(result);
    }
    if pid <= 0 {
        return AxFocus::InvalidPid(pid);
    }
    AxFocus::Pid(pid)
}

fn trace_focus_sources(op: &str, ax: AxFocus, window_server: Option<i32>, chosen: Option<i32>) {
    #[cfg(debug_assertions)]
    {
        let ax = match ax {
            AxFocus::Pid(pid) => format!("{pid}"),
            AxFocus::Error(code) => format!("err:{code}"),
            AxFocus::NoSystemElement => "no_system_element".to_string(),
            AxFocus::InvalidPid(pid) => format!("invalid_pid:{pid}"),
        };
        qol_runtime::probe!(
            "WINACT_AX",
            "op=focus_sources caller={op} ax={ax} window_server={} chosen={}",
            window_server.map_or_else(|| "none".to_string(), |pid| pid.to_string()),
            chosen.map_or_else(|| "none".to_string(), |pid| pid.to_string()),
        );
    }
    #[cfg(not(debug_assertions))]
    let _ = (op, ax, window_server, chosen);
}

fn select_normal_window_pid(
    preferred: impl IntoIterator<Item = i32>,
    entries: impl IntoIterator<Item = (i32, i32)>,
    mut is_normal: impl FnMut(i32) -> bool,
) -> Option<i32> {
    let mut fallback = None;
    for pid in preferred {
        fallback.get_or_insert(pid);
        if is_normal(pid) {
            return Some(pid);
        }
    }
    layer_zero_pids(entries)
        .find(|pid| is_normal(*pid))
        .or(fallback)
}

fn on_screen_window_list() -> Option<CfGuard> {
    CfGuard::new(unsafe {
        CGWindowListCopyWindowInfo(
            CG_WINDOW_LIST_OPTION_ON_SCREEN_ONLY | CG_WINDOW_LIST_EXCLUDE_DESKTOP,
            0,
        )
    })
}

fn window_entries(list: &CfGuard) -> impl Iterator<Item = (i32, i32)> + '_ {
    let count = unsafe { CFArrayGetCount(list.as_ptr()) };
    (0..count).map(move |index| {
        let dict = unsafe { CFArrayGetValueAtIndex(list.as_ptr(), index) };
        (
            dict_get_i32(dict, cg_window_layer()).unwrap_or(-1),
            dict_get_i32(dict, cg_window_owner_pid()).unwrap_or(0),
        )
    })
}

fn layer_zero_pids(entries: impl IntoIterator<Item = (i32, i32)>) -> impl Iterator<Item = i32> {
    let mut seen = HashSet::new();
    entries
        .into_iter()
        .filter_map(|(layer, pid)| (layer == 0 && pid > 0).then_some(pid))
        .filter(move |pid| seen.insert(*pid))
}

pub(super) fn front_window_rect(pid: i32) -> Option<super::screen::Rect> {
    timed_opt("front_window_rect", pid, || {
        let ft = front_target(pid)?;
        read_window_rect(ft.win)
    })
}

pub(super) fn set_position_and_size(pid: i32, rect: super::screen::Rect) -> bool {
    timed_bool("set_pos_size", pid, || {
        let Some(ft) = front_target(pid) else {
            return false;
        };
        let before = read_window_rect(ft.win);
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

        let (size_before, position, size_after) = unsafe {
            (
                AXUIElementSetAttributeValue(ft.win, ax_size.as_const(), size_val.as_const()),
                AXUIElementSetAttributeValue(ft.win, ax_pos.as_const(), pos_val.as_const()),
                AXUIElementSetAttributeValue(ft.win, ax_size.as_const(), size_val.as_const()),
            )
        };
        if size_before != 0 || position != 0 || size_after != 0 {
            return false;
        }
        let actual = read_window_rect(ft.win);
        let outcome = geometry_outcome(before, actual, rect);
        trace_geometry(pid, rect, actual, outcome.as_str());
        outcome.applied()
    })
}

fn geometry_outcome(
    before: Option<super::screen::Rect>,
    actual: Option<super::screen::Rect>,
    expected: super::screen::Rect,
) -> GeometryOutcome {
    let Some(actual) = actual else {
        return GeometryOutcome::Unreadable;
    };
    if rect_matches(actual, expected) {
        return GeometryOutcome::Exact;
    }
    if constrained_rect_matches(actual, expected) {
        return GeometryOutcome::Constrained;
    }
    if before.is_none_or(|before| !rect_matches(actual, before)) {
        return GeometryOutcome::Adjusted;
    }
    GeometryOutcome::Unchanged
}

fn rect_matches(actual: super::screen::Rect, expected: super::screen::Rect) -> bool {
    (actual.x - expected.x).abs() <= RECT_TOLERANCE
        && (actual.y - expected.y).abs() <= RECT_TOLERANCE
        && (actual.w - expected.w).abs() <= RECT_TOLERANCE
        && (actual.h - expected.h).abs() <= RECT_TOLERANCE
}

fn constrained_rect_matches(actual: super::screen::Rect, expected: super::screen::Rect) -> bool {
    let left = (actual.x - expected.x).abs() <= RECT_TOLERANCE;
    let top = (actual.y - expected.y).abs() <= RECT_TOLERANCE;
    let right = (actual.x + actual.w - expected.x - expected.w).abs() <= RECT_TOLERANCE;
    let bottom = (actual.y + actual.h - expected.y - expected.h).abs() <= RECT_TOLERANCE;
    let width = (actual.w - expected.w).abs() <= RECT_TOLERANCE;
    let height = (actual.h - expected.h).abs() <= RECT_TOLERANCE;
    actual.w <= expected.w + RECT_TOLERANCE
        && actual.h <= expected.h + RECT_TOLERANCE
        && (left || right)
        && (top || bottom)
        && (width || height)
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

fn focus_window(win: *const c_void) {
    let _ = set_ax_bool_attr(win, "AXMain", true);
    let _ = set_ax_bool_attr(win, "AXFocused", true);
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
            return activate_app(pid, app.as_ptr(), 3); // IgnoringOtherApps | AllWindows
        }

        let app_was_frontmost = frontmost_pid() == Some(pid);
        let mut restored = false;

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
                }
                if !app_was_frontmost {
                    unsafe {
                        AXUIElementPerformAction(win, ax_raise.as_const());
                    }
                    if !activate_app(pid, app.as_ptr(), 2) {
                        return false;
                    }
                } else {
                    focus_window(win);
                }
                restored = true;
                break;
            }
        }

        // Only raise the restored window, not all app windows.
        restored && wait_for_app_active(pid)
    })
}

#[cfg(test)]
mod tests {
    use super::{
        geometry_outcome, layer_zero_pids, rect_matches, select_normal_window_pid, GeometryOutcome,
    };
    use crate::platform::macos::screen::Rect;

    #[test]
    fn layer_zero_pids_selects_first_valid_normal_layer_window() {
        let cases = [
            ("first normal window", vec![(0, 42), (0, 43)], Some(42)),
            (
                "skip elevated and desktop layers",
                vec![(25, 7), (-1, 8), (0, 9)],
                Some(9),
            ),
            (
                "skip invalid process identifiers",
                vec![(0, 0), (0, -1), (0, 17)],
                Some(17),
            ),
            ("no normal window", vec![(3, 42), (-1, 43)], None),
        ];

        for (name, entries, expected) in cases {
            assert_eq!(layer_zero_pids(entries).next(), expected, "{name}");
        }
    }

    #[test]
    fn normal_window_selection_prefers_workspace_and_validates_fallbacks() {
        let cases = [
            (
                "accessibility focus wins over workspace and global order",
                vec![42, 77],
                vec![(0, 99), (0, 42)],
                vec![42, 77, 99],
                Some(42),
            ),
            (
                "workspace wins when accessibility focus has no normal window",
                vec![42, 77],
                vec![(0, 99), (0, 77)],
                vec![77, 99],
                Some(77),
            ),
            (
                "non-window preferred owners fall back to normal layer zero window",
                vec![42, 77],
                vec![(25, 7), (0, 99), (0, 100)],
                vec![99, 100],
                Some(99),
            ),
            (
                "first preferred owner survives when no normal fallback exists",
                vec![42, 77],
                vec![(25, 7), (0, 99)],
                vec![],
                Some(42),
            ),
            (
                "missing preferred owner uses first normal layer zero window",
                vec![],
                vec![(0, 99), (0, 100)],
                vec![100],
                Some(100),
            ),
            (
                "missing preferred and normal windows returns none",
                vec![],
                vec![(3, 99), (0, 100)],
                vec![],
                None,
            ),
        ];

        for (name, preferred, entries, normal, expected) in cases {
            assert_eq!(
                select_normal_window_pid(preferred, entries, |pid| normal.contains(&pid)),
                expected,
                "{name}"
            );
        }
    }

    #[test]
    fn geometry_verification_allows_only_rounding_tolerance() {
        let expected = Rect {
            x: 100.0,
            y: 200.0,
            w: 1152.0,
            h: 800.0,
        };
        let cases = [
            ("exact", expected, true),
            (
                "subpixel rounding",
                Rect {
                    x: 100.5,
                    y: 199.5,
                    w: 1151.5,
                    h: 800.5,
                },
                true,
            ),
            (
                "wrong position",
                Rect {
                    x: 102.0,
                    ..expected
                },
                false,
            ),
            (
                "wrong size",
                Rect {
                    w: 1150.0,
                    ..expected
                },
                false,
            ),
        ];

        for (name, actual, matches) in cases {
            assert_eq!(rect_matches(actual, expected), matches, "{name}");
        }
    }

    #[test]
    fn geometry_outcome_accepts_mac_constraints_without_accepting_noops() {
        let expected = Rect {
            x: 4360.0,
            y: -716.0,
            w: 1920.0,
            h: 1080.0,
        };
        let before = Rect {
            x: 1800.0,
            y: -716.0,
            w: 2560.0,
            h: 1440.0,
        };
        let cases = [
            (
                "exact",
                Some(before),
                Some(expected),
                GeometryOutcome::Exact,
            ),
            (
                "menu bar clamp",
                Some(before),
                Some(Rect {
                    y: -685.0,
                    h: 1049.0,
                    ..expected
                }),
                GeometryOutcome::Constrained,
            ),
            (
                "application adjustment",
                Some(before),
                Some(Rect {
                    x: 4400.0,
                    y: -680.0,
                    w: 1800.0,
                    h: 1000.0,
                }),
                GeometryOutcome::Adjusted,
            ),
            (
                "unchanged unrelated frame",
                Some(before),
                Some(before),
                GeometryOutcome::Unchanged,
            ),
            (
                "unreadable",
                Some(before),
                None,
                GeometryOutcome::Unreadable,
            ),
        ];
        for (name, before, actual, outcome) in cases {
            assert_eq!(
                geometry_outcome(before, actual, expected),
                outcome,
                "{name}"
            );
        }
    }

    #[test]
    fn layer_zero_pids_yields_each_owner_once_in_front_to_back_order() {
        let cases = [
            (
                "repeated owner collapses to first position",
                vec![(0, 42), (0, 42), (0, 43)],
                vec![42, 43],
            ),
            (
                "interleaved owners keep first sighting",
                vec![(0, 7), (0, 9), (0, 7)],
                vec![7, 9],
            ),
            (
                "rejected entries never occupy a slot",
                vec![(25, 7), (0, 0), (-1, 8), (0, 9)],
                vec![9],
            ),
        ];

        for (name, entries, expected) in cases {
            assert_eq!(
                layer_zero_pids(entries).collect::<Vec<_>>(),
                expected,
                "{name}"
            );
        }
    }
}
