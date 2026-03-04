use super::CgWindow;
use super::ffi;
use super::ffi::{CFArrayRef, CFArrayGetCount, CFArrayGetValueAtIndex, CFRelease, CFRetain};
use std::collections::{HashMap, HashSet};
use std::ffi::c_void;

#[derive(Clone)]
pub(super) struct AxWindowMeta {
    pub title: String,
    pub is_minimized: bool,
}

/// Query the Accessibility API for the number of real windows an app has.
/// Returns None if AX is unavailable (no permission) or the app doesn't respond.
/// Returns (id_map, all_meta, accepted_count):
///   - id_map: keyed by _AXWindowID (empty when the app doesn't expose the attribute)
///   - all_meta: title + minimized for every window that passed subrole filter
///   - accepted_count: number of windows that passed subrole filter
pub(super) fn ax_windows(pid: i32) -> Option<(HashMap<u32, AxWindowMeta>, Vec<AxWindowMeta>, usize)> {
    #[link(name = "ApplicationServices", kind = "framework")]
    extern "C" {
        fn AXUIElementCreateApplication(pid: i32) -> *const c_void;
        fn AXUIElementCopyAttributeValue(
            element: *const c_void,
            attribute: *const c_void,
            value: *mut *const c_void,
        ) -> i32;
    }
    #[link(name = "CoreFoundation", kind = "framework")]
    extern "C" {
        static kCFBooleanTrue: *const c_void;
        static kCFBooleanFalse: *const c_void;
    }

    unsafe {
        let app = AXUIElementCreateApplication(pid);
        if app.is_null() {
            return None;
        }
        let windows_attr = ffi::cfstr(b"AXWindows");
        let mut windows_value: *const c_void = std::ptr::null();
        let windows_err =
            AXUIElementCopyAttributeValue(app, windows_attr, &mut windows_value);
        CFRelease(windows_attr as *const c_void);
        if windows_err != 0 || windows_value.is_null() {
            CFRelease(app);
            return None;
        }

        let id_attr = ffi::cfstr(b"_AXWindowID");
        let title_attr = ffi::cfstr(b"AXTitle");
        let subrole_attr = ffi::cfstr(b"AXSubrole");
        let minimized_attr = ffi::cfstr(b"AXMinimized");
        let count = CFArrayGetCount(windows_value as CFArrayRef);
        #[cfg(debug_assertions)]
        eprintln!("[alt-tab/ax] ax_windows pid={} AXWindows count={}", pid, count);
        let mut id_map = HashMap::new();
        let mut all_meta: Vec<AxWindowMeta> = Vec::new();
        let mut accepted_count: usize = 0;

        for i in 0..count {
            let win = CFArrayGetValueAtIndex(windows_value as CFArrayRef, i);
            if win.is_null() {
                continue;
            }

            // Skip windows with explicitly non-standard subroles (system overlays,
            // badges, floating panels). When AXSubrole is unavailable (query fails
            // during focus transitions), include the window — safe default.
            let mut subrole_value: *const c_void = std::ptr::null();
            let subrole_err =
                AXUIElementCopyAttributeValue(win, subrole_attr, &mut subrole_value);
            if subrole_err == 0 && !subrole_value.is_null() {
                let subrole = ffi::cfstring_to_string(subrole_value);
                CFRelease(subrole_value);
                if !matches!(subrole.as_deref(), Some("AXStandardWindow" | "AXDialog")) {
                    #[cfg(debug_assertions)]
                    eprintln!("[alt-tab/ax] FILTERED subrole={:?} pid={} (not AXStandardWindow/AXDialog)", subrole, pid);
                    continue;
                }
            } else if !subrole_value.is_null() {
                CFRelease(subrole_value);
            }

            accepted_count += 1;

            // Try _AXWindowID (may fail — -25205 is common on newer macOS)
            let mut id_value: *const c_void = std::ptr::null();
            let id_err = AXUIElementCopyAttributeValue(win, id_attr, &mut id_value);
            let window_id = if id_err == 0 && !id_value.is_null() {
                let maybe_id = ffi::cfnumber_to_u32(id_value);
                CFRelease(id_value);
                maybe_id
            } else {
                if !id_value.is_null() { CFRelease(id_value); }
                None
            };

            // Always query title + minimized so callers can match by title
            // when _AXWindowID is unavailable.
            let mut title = String::new();
            let mut title_value: *const c_void = std::ptr::null();
            let title_err = AXUIElementCopyAttributeValue(win, title_attr, &mut title_value);
            if title_err == 0 && !title_value.is_null() {
                title = ffi::cfstring_to_string(title_value).unwrap_or_default();
                CFRelease(title_value);
            }

            let mut minimized_value: *const c_void = std::ptr::null();
            let minimized_err =
                AXUIElementCopyAttributeValue(win, minimized_attr, &mut minimized_value);
            let is_minimized = minimized_err == 0
                && !minimized_value.is_null()
                && minimized_value == kCFBooleanTrue;
            if minimized_err == 0
                && !minimized_value.is_null()
                && minimized_value != kCFBooleanTrue
                && minimized_value != kCFBooleanFalse
            {
                CFRelease(minimized_value);
            }
            if minimized_err != 0 && !minimized_value.is_null() {
                CFRelease(minimized_value);
            }

            let meta = AxWindowMeta {
                title: title.trim().to_string(),
                is_minimized,
            };
            all_meta.push(meta.clone());
            if let Some(id) = window_id {
                id_map.insert(id, meta);
            }
        }

        CFRelease(id_attr as *const c_void);
        CFRelease(title_attr as *const c_void);
        CFRelease(subrole_attr as *const c_void);
        CFRelease(minimized_attr as *const c_void);
        CFRelease(windows_value);
        CFRelease(app);
        #[cfg(debug_assertions)]
        eprintln!("[alt-tab/ax] ax_windows pid={} id_map={} all_meta={} accepted={} (from {} AX windows)", pid, id_map.len(), all_meta.len(), accepted_count, count);
        Some((id_map, all_meta, accepted_count))
    }
}

pub(super) fn ax_is_window_real(pid: i32, cg_window_id: u32, title: &str) -> bool {
    let win = unsafe { ax_find_window(pid, cg_window_id, title) };
    if win.is_null() {
        return false;
    }
    unsafe { CFRelease(win) };
    true
}

/// Check if a specific off-screen CG window is truly minimized via AX.
pub(super) fn ax_is_window_minimized(pid: i32, cg_window_id: u32, title: &str) -> bool {
    #[link(name = "ApplicationServices", kind = "framework")]
    extern "C" {
        fn AXUIElementCopyAttributeValue(
            el: *const c_void,
            attr: *const c_void,
            val: *mut *const c_void,
        ) -> i32;
    }
    #[link(name = "CoreFoundation", kind = "framework")]
    extern "C" {
        static kCFBooleanTrue: *const c_void;
    }

    let win = unsafe { ax_find_window(pid, cg_window_id, title) };
    if win.is_null() {
        return false;
    }
    let minimized_attr = ffi::cfstr(b"AXMinimized");
    let mut value: *const c_void = std::ptr::null();
    let result = unsafe {
        let err = AXUIElementCopyAttributeValue(win, minimized_attr, &mut value);
        let is_min = err == 0 && !value.is_null() && value == kCFBooleanTrue;
        if !value.is_null() && err == 0 {
            // CFBooleans are singletons, don't release them
        }
        CFRelease(minimized_attr as *const c_void);
        CFRelease(win);
        is_min
    };
    result
}

/// Deduplicate CG windows using the Accessibility API while preserving z-order.
/// Apps like Kitty create multiple CG windows per visual window (one per tab).
/// AX reports the real user-visible window count. For each PID, keep only
/// that many CG windows, but each kept window stays at its original z-position
/// so that windows from different apps remain correctly interleaved.
pub(super) type AxCache = HashMap<i32, Option<(HashMap<u32, AxWindowMeta>, Vec<AxWindowMeta>, usize)>>;

pub(super) fn dedup_by_ax(windows: Vec<CgWindow>, ax_cache: &mut AxCache) -> Vec<CgWindow> {
    let mut cg_count_by_pid: HashMap<i32, usize> = HashMap::new();
    for w in &windows {
        *cg_count_by_pid.entry(w.pid).or_insert(0) += 1;
    }

    let multi_pids: Vec<i32> = cg_count_by_pid
        .iter()
        .filter(|(_, count)| **count > 1)
        .map(|(pid, _)| *pid)
        .collect();

    struct PidDedup {
        ax_ids: HashSet<u32>,
        ax_meta: HashMap<u32, AxWindowMeta>,
        budget: usize,
    }

    // Pre-populate ax_cache for multi-window PIDs so collect_minimized_windows
    // can reuse these results instead of re-querying the same PIDs.
    for &pid in &multi_pids {
        ax_cache.entry(pid).or_insert_with(|| ax_windows(pid));
    }

    let mut dedup_info: HashMap<i32, PidDedup> = HashMap::new();
    for pid in multi_pids {
        let ax_result = ax_cache.get(&pid).unwrap(); // just populated above
        let dedup = match ax_result {
            Some((id_map, _, accepted_count)) if !id_map.is_empty() => {
                let budget = (*accepted_count).max(id_map.len());
                let ax_ids = id_map.keys().copied().collect();
                PidDedup { ax_ids, ax_meta: id_map.clone(), budget }
            }
            Some((_, _, accepted_count)) => {
                // AX responded but _AXWindowID failed for all windows.
                // Use accepted_count (windows that passed subrole filter) as budget.
                PidDedup {
                    ax_ids: HashSet::new(),
                    ax_meta: HashMap::new(),
                    budget: (*accepted_count).max(1),
                }
            }
            // AX unavailable: keep 1 window per PID (safe default —
            // avoids leaking system-injected overlays when AX times out).
            None => PidDedup {
                ax_ids: HashSet::new(),
                ax_meta: HashMap::new(),
                budget: 1,
            },
        };
        dedup_info.insert(pid, dedup);
    }

    let mut result = Vec::with_capacity(windows.len());
    let mut emitted_by_pid: HashMap<i32, usize> = HashMap::new();

    for mut win in windows {
        let Some(dedup) = dedup_info.get_mut(&win.pid) else {
            result.push(win);
            continue;
        };

        let emitted = emitted_by_pid.entry(win.pid).or_insert(0);
        if *emitted >= dedup.budget {
            #[cfg(debug_assertions)]
            eprintln!("[alt-tab/ax] DEDUP budget exhausted: wid={} app={:?} budget={}", win.id, win.app_name, dedup.budget);
            continue;
        }

        let ax_ids_complete = dedup.ax_ids.len() >= dedup.budget;
        if ax_ids_complete && !dedup.ax_ids.contains(&win.id) {
            #[cfg(debug_assertions)]
            eprintln!("[alt-tab/ax] DEDUP not in AX ids: wid={} app={:?}", win.id, win.app_name);
            continue;
        }

        if let Some(meta) = dedup.ax_meta.get(&win.id).filter(|m| !m.title.is_empty()) {
            win.title = meta.title.clone();
        }

        *emitted += 1;
        result.push(win);
    }

    result
}

/// Find an AX window element for `pid`.
/// Tries (in order): `_AXWindowID` match → `AXTitle` match → first window if only one.
/// Returns a CFRetained pointer; caller must CFRelease it.
pub(crate) unsafe fn ax_find_window(
    pid: i32,
    cg_window_id: u32,
    title_hint: &str,
) -> *const c_void {
    #[link(name = "ApplicationServices", kind = "framework")]
    extern "C" {
        fn AXUIElementCreateApplication(pid: i32) -> *const c_void;
        fn AXUIElementCopyAttributeValue(
            el: *const c_void,
            attr: *const c_void,
            val: *mut *const c_void,
        ) -> i32;
    }

    let app_el = AXUIElementCreateApplication(pid);
    if app_el.is_null() {
        return std::ptr::null();
    }

    let windows_attr = ffi::cfstr(b"AXWindows");
    let mut wins_val: *const c_void = std::ptr::null();
    let err = AXUIElementCopyAttributeValue(app_el, windows_attr, &mut wins_val);
    CFRelease(windows_attr as *const c_void);
    CFRelease(app_el);
    if err != 0 || wins_val.is_null() {
        return std::ptr::null();
    }

    let id_attr = ffi::cfstr(b"_AXWindowID");
    let title_attr = ffi::cfstr(b"AXTitle");
    let count = CFArrayGetCount(wins_val as CFArrayRef);
    #[cfg(debug_assertions)]
    eprintln!(
        "[alt-tab/ax_find_window] pid={} count={} cg_id={} title_hint={:?}",
        pid, count, cg_window_id, title_hint
    );

    let mut id_match: *const c_void = std::ptr::null();
    let mut title_match: *const c_void = std::ptr::null();
    let mut first_win: *const c_void = std::ptr::null();

    for i in 0..count {
        let win_el = CFArrayGetValueAtIndex(wins_val as CFArrayRef, i);
        if win_el.is_null() {
            continue;
        }

        if first_win.is_null() {
            first_win = CFRetain(win_el);
        }

        if id_match.is_null() {
            let mut id_val: *const c_void = std::ptr::null();
            let err = AXUIElementCopyAttributeValue(win_el, id_attr, &mut id_val);
            if err == 0 && !id_val.is_null() {
                let maybe_id = ffi::cfnumber_to_u32(id_val);
                CFRelease(id_val);
                if maybe_id == Some(cg_window_id) {
                    id_match = CFRetain(win_el);
                }
            } else if !id_val.is_null() {
                CFRelease(id_val);
            }
        }

        if title_match.is_null() && !title_hint.is_empty() {
            let mut title_val: *const c_void = std::ptr::null();
            let err = AXUIElementCopyAttributeValue(win_el, title_attr, &mut title_val);
            if err == 0 && !title_val.is_null() {
                let ax_title = ffi::cfstring_to_string(title_val).unwrap_or_default();
                CFRelease(title_val);
                if ax_title == title_hint {
                    title_match = CFRetain(win_el);
                }
            } else if !title_val.is_null() {
                CFRelease(title_val);
            }
        }
    }

    CFRelease(id_attr as *const c_void);
    CFRelease(title_attr as *const c_void);
    CFRelease(wins_val);

    if !id_match.is_null() {
        if !title_match.is_null() { CFRelease(title_match); }
        if !first_win.is_null() { CFRelease(first_win); }
        return id_match;
    }
    if !title_match.is_null() {
        if !first_win.is_null() { CFRelease(first_win); }
        return title_match;
    }
    if count == 1 && !first_win.is_null() {
        return first_win;
    }
    if !first_win.is_null() { CFRelease(first_win); }
    std::ptr::null()
}
