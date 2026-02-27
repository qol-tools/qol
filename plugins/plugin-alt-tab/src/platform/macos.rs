use super::cg_helpers;
use super::RgbaImage;
use super::WindowInfo;
use std::collections::{HashMap, HashSet};
use std::ffi::c_void;
use std::sync::{Mutex, OnceLock};

type CFArrayRef = *const c_void;
type CFDictionaryRef = *const c_void;
type CGImageRef = *const c_void;
type CFDataRef = *const c_void;
type CGDataProviderRef = *const c_void;

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

const CG_RECT_NULL: CGRect = CGRect {
    origin: CGPoint { x: f64::INFINITY, y: f64::INFINITY },
    size: CGSize { width: 0.0, height: 0.0 },
};
const K_CG_WINDOW_LIST_OPTION_INCLUDING_WINDOW: u32 = 1 << 3;
const K_CG_WINDOW_IMAGE_BOUNDS_IGNORE_FRAMING: u32 = 1 << 0;
const K_CG_WINDOW_IMAGE_NOMINAL_RESOLUTION: u32 = 1 << 9;

#[link(name = "CoreGraphics", kind = "framework")]
extern "C" {
    fn CGWindowListCopyWindowInfo(option: u32, relative_to: u32) -> CFArrayRef;
    fn CGWindowListCreateImage(
        screen_bounds: CGRect,
        list_option: u32,
        window_id: u32,
        image_option: u32,
    ) -> CGImageRef;
    fn CGImageGetWidth(image: CGImageRef) -> usize;
    fn CGImageGetHeight(image: CGImageRef) -> usize;
    fn CGImageGetBytesPerRow(image: CGImageRef) -> usize;
    fn CGImageGetBitsPerPixel(image: CGImageRef) -> usize;
    fn CGImageGetDataProvider(image: CGImageRef) -> CGDataProviderRef;
    fn CGDataProviderCopyData(provider: CGDataProviderRef) -> CFDataRef;
}

#[link(name = "CoreFoundation", kind = "framework")]
extern "C" {
    fn CFArrayGetCount(arr: CFArrayRef) -> isize;
    fn CFArrayGetValueAtIndex(arr: CFArrayRef, idx: isize) -> *const c_void;
    fn CFRelease(cf: *const c_void);
    fn CFDataGetBytePtr(data: CFDataRef) -> *const u8;
    fn CFDataGetLength(data: CFDataRef) -> isize;
}

const K_CG_WINDOW_LIST_OPTION_ON_SCREEN_ONLY: u32 = 1;
const K_CG_WINDOW_LIST_EXCLUDE_DESKTOP_ELEMENTS: u32 = 1 << 4;
const K_CG_NULL_WINDOW_ID: u32 = 0;
const K_CG_WINDOW_LAYER_NORMAL: i32 = 0;

const ICON_SIZE: usize = 32;

/// Parsed CG window entry.
struct CgWindow {
    id: u32,
    pid: i32,
    app_name: String,
    title: String,
    has_title: bool,
    x: f32,
    y: f32,
    w: f32,
    h: f32,
}

#[derive(Clone)]
struct AxWindowMeta {
    title: String,
    is_minimized: bool,
}

#[derive(Clone, Copy, Eq, PartialEq, Hash)]
struct ProcessIdentity {
    pid: i32,
    start_time_us: u64,
}

#[repr(C)]
struct ProcBsdInfo {
    pbi_flags: u32,
    pbi_status: u32,
    pbi_xstatus: u32,
    pbi_pid: u32,
    pbi_ppid: u32,
    pbi_uid: u32,
    pbi_ruid: u32,
    pbi_gid: u32,
    pbi_rgid: u32,
    pbi_svuid: u32,
    pbi_svgid: u32,
    rfu_1: u32,
    pbi_comm: [u8; 16],
    pbi_name: [u8; 32],
    pbi_nfiles: u32,
    pbi_pgid: u32,
    pbi_pjobc: u32,
    e_tdev: u32,
    e_tpgid: u32,
    pbi_nice: i32,
    pbi_start_tvsec: u64,
    pbi_start_tvusec: u64,
}

#[link(name = "proc")]
extern "C" {
    fn proc_pidinfo(
        pid: i32,
        flavor: i32,
        arg: u64,
        buffer: *mut c_void,
        buffersize: i32,
    ) -> i32;
}

const PROC_PIDTBSDINFO: i32 = 3;

static KNOWN_WINDOW_IDS_BY_IDENTITY: OnceLock<Mutex<HashMap<ProcessIdentity, HashSet<u32>>>> =
    OnceLock::new();

fn known_window_ids_by_identity() -> &'static Mutex<HashMap<ProcessIdentity, HashSet<u32>>> {
    KNOWN_WINDOW_IDS_BY_IDENTITY.get_or_init(|| Mutex::new(HashMap::new()))
}

fn process_identity(pid: i32) -> Option<ProcessIdentity> {
    if pid <= 0 {
        return None;
    }

    let mut info: ProcBsdInfo = unsafe { std::mem::zeroed() };
    let size = std::mem::size_of::<ProcBsdInfo>() as i32;
    let read = unsafe {
        proc_pidinfo(
            pid,
            PROC_PIDTBSDINFO,
            0,
            (&mut info as *mut ProcBsdInfo).cast::<c_void>(),
            size,
        )
    };
    if read != size {
        return None;
    }

    let start_time_us = info
        .pbi_start_tvsec
        .saturating_mul(1_000_000)
        .saturating_add(info.pbi_start_tvusec);

    Some(ProcessIdentity {
        pid,
        start_time_us,
    })
}

fn cached_process_identity(
    pid: i32,
    cache: &mut HashMap<i32, Option<ProcessIdentity>>,
) -> Option<ProcessIdentity> {
    if let Some(identity) = cache.get(&pid) {
        return *identity;
    }
    let identity = process_identity(pid);
    cache.insert(pid, identity);
    identity
}

#[derive(Default)]
struct WindowEnumeration {
    windows: Vec<WindowInfo>,
    on_screen_ids: HashSet<u32>,
    on_screen_pids: HashSet<i32>,
    on_screen_count_by_pid: HashMap<i32, usize>,
}

impl WindowEnumeration {
    fn on_screen_count(&self, pid: i32) -> usize {
        self.on_screen_count_by_pid.get(&pid).copied().unwrap_or(0)
    }

    fn register_on_screen_pid(&mut self, pid: i32) {
        self.on_screen_pids.insert(pid);
    }

    fn push_on_screen(&mut self, window: CgWindow) {
        self.on_screen_ids.insert(window.id);
        *self.on_screen_count_by_pid.entry(window.pid).or_insert(0) += 1;
        self.windows.push(WindowInfo {
            id: window.id,
            title: window.title,
            app_name: window.app_name,
            preview_path: None,
            icon: None,
            x: window.x,
            y: window.y,
            width: window.w,
            height: window.h,
            is_minimized: false,
        });
    }

    fn push_minimized(&mut self, window: &CgWindow, title: String) {
        self.windows.push(WindowInfo {
            id: window.id,
            title,
            app_name: window.app_name.clone(),
            preview_path: None,
            icon: None,
            x: window.x,
            y: window.y,
            width: window.w,
            height: window.h,
            is_minimized: true,
        });
    }
}

struct KnownWindowTracker {
    snapshot: HashMap<ProcessIdentity, HashSet<u32>>,
    accepted: HashMap<ProcessIdentity, HashSet<u32>>,
    seen: HashSet<ProcessIdentity>,
    identity_cache: HashMap<i32, Option<ProcessIdentity>>,
}

impl KnownWindowTracker {
    fn new() -> Self {
        let snapshot = known_window_ids_by_identity()
            .lock()
            .ok()
            .map(|cache| cache.clone())
            .unwrap_or_default();
        Self {
            snapshot,
            accepted: HashMap::new(),
            seen: HashSet::new(),
            identity_cache: HashMap::new(),
        }
    }

    fn identity_for_pid(&mut self, pid: i32) -> Option<ProcessIdentity> {
        let identity = cached_process_identity(pid, &mut self.identity_cache);
        if let Some(identity) = identity {
            self.seen.insert(identity);
        }
        identity
    }

    fn remember_window(&mut self, pid: i32, window_id: u32) {
        let Some(identity) = self.identity_for_pid(pid) else {
            return;
        };
        self.accepted.entry(identity).or_default().insert(window_id);
    }

    fn persist(self) {
        if let Ok(mut known_cache) = known_window_ids_by_identity().lock() {
            known_cache.retain(|identity, _| self.seen.contains(identity));
            for (identity, ids) in self.accepted {
                known_cache.insert(identity, ids);
            }
        }
    }
}

fn allowed_minimized_count(
    on_screen_count: usize,
    identity: Option<ProcessIdentity>,
    snapshot: &HashMap<ProcessIdentity, HashSet<u32>>,
    meta_map: &HashMap<u32, AxWindowMeta>,
) -> usize {
    if on_screen_count != 0 {
        return meta_map.values().filter(|window| window.is_minimized).count();
    }

    if let Some(identity) = identity {
        if let Some(count) = snapshot
            .get(&identity)
            .map(|ids| ids.len())
            .filter(|count| *count > 0)
        {
            return count;
        }
    }

    meta_map.values().filter(|window| window.is_minimized).count()
}

fn collect_on_screen_windows(
    own_pid: i32,
    state: &mut WindowEnumeration,
    tracker: &mut KnownWindowTracker,
) {
    let options =
        K_CG_WINDOW_LIST_OPTION_ON_SCREEN_ONLY | K_CG_WINDOW_LIST_EXCLUDE_DESKTOP_ELEMENTS;
    let list = unsafe { CGWindowListCopyWindowInfo(options, K_CG_NULL_WINDOW_ID) };
    if list.is_null() {
        return;
    }

    let parsed = parse_cg_window_list(list, own_pid);
    unsafe { CFRelease(list as *const c_void) };

    for window in &parsed {
        state.register_on_screen_pid(window.pid);
    }

    for window in dedup_by_ax(parsed) {
        tracker.remember_window(window.pid, window.id);
        state.push_on_screen(window);
    }
}

fn collect_minimized_windows(
    own_pid: i32,
    state: &mut WindowEnumeration,
    tracker: &mut KnownWindowTracker,
) {
    let options = K_CG_WINDOW_LIST_EXCLUDE_DESKTOP_ELEMENTS;
    let list = unsafe { CGWindowListCopyWindowInfo(options, K_CG_NULL_WINDOW_ID) };
    if list.is_null() {
        return;
    }

    let mut regular_app_cache: HashMap<i32, bool> = HashMap::new();
    let mut ax_windows_cache: HashMap<i32, Option<HashMap<u32, AxWindowMeta>>> = HashMap::new();
    let mut minimized_count_by_pid: HashMap<i32, usize> = HashMap::new();
    let mut seen_minimized_ids = HashSet::new();

    for window in parse_cg_window_list(list, own_pid) {
        if state.on_screen_ids.contains(&window.id) {
            continue;
        }
        if !seen_minimized_ids.insert(window.id) {
            continue;
        }
        if state.on_screen_pids.contains(&window.pid)
            && !ax_is_window_minimized(window.pid, window.id, &window.title)
        {
            continue;
        }
        if !window.has_title || window.w < 1.0 || window.h < 1.0 {
            continue;
        }
        let is_regular = *regular_app_cache
            .entry(window.pid)
            .or_insert_with(|| is_regular_app(window.pid));
        if !is_regular {
            continue;
        }

        let on_screen_count = state.on_screen_count(window.pid);
        let identity = tracker.identity_for_pid(window.pid);
        let known_ids = if on_screen_count == 0 {
            identity.and_then(|id| tracker.snapshot.get(&id))
        } else {
            None
        };
        let known_budget = known_ids
            .map(|ids| ids.len())
            .filter(|count| *count > 0);

        let ax_windows = ax_windows_cache
            .entry(window.pid)
            .or_insert_with(|| ax_windows(window.pid));
        let mut title = window.title.clone();
        let mut allowed_count = known_budget.unwrap_or(usize::MAX);
        let mut ax_has_window = false;
        let mut ax_is_minimized = false;

        if let Some(meta_map) = ax_windows.as_ref() {
            if let Some(meta) = meta_map.get(&window.id) {
                ax_has_window = true;
                ax_is_minimized = meta.is_minimized;
                if !meta.title.is_empty() {
                    title = meta.title.clone();
                }
            }
            if known_budget.is_none() {
                allowed_count =
                    allowed_minimized_count(on_screen_count, identity, &tracker.snapshot, meta_map);
            }
        }

        if known_budget.is_none() && ax_has_window && !ax_is_minimized {
            continue;
        }

        if let Some(known_ids) = known_ids {
            if !known_ids.is_empty() && !ax_has_window && !known_ids.contains(&window.id) {
                continue;
            }
        }

        if allowed_count == 0 {
            continue;
        }
        let current_count = minimized_count_by_pid.entry(window.pid).or_insert(0);
        if *current_count >= allowed_count {
            continue;
        }
        *current_count += 1;

        tracker.remember_window(window.pid, window.id);
        state.push_minimized(&window, title);
    }

    unsafe { CFRelease(list as *const c_void) };
}

/// Shared helper: parse normal-layer windows from a CG window list.
fn parse_cg_window_list(list: *const c_void, own_pid: i32) -> Vec<CgWindow> {
    let key_layer = cg_helpers::cfstr(b"kCGWindowLayer");
    let key_pid = cg_helpers::cfstr(b"kCGWindowOwnerPID");
    let key_owner = cg_helpers::cfstr(b"kCGWindowOwnerName");
    let key_name = cg_helpers::cfstr(b"kCGWindowName");
    let key_number = cg_helpers::cfstr(b"kCGWindowNumber");
    let key_bounds = cg_helpers::cfstr(b"kCGWindowBounds");

    let count = unsafe { CFArrayGetCount(list) };
    let mut result: Vec<CgWindow> = Vec::with_capacity(count.max(0) as usize);

    for i in 0..count {
        let dict = unsafe { CFArrayGetValueAtIndex(list, i) } as CFDictionaryRef;
        if dict.is_null() {
            continue;
        }
        let Some(layer) = cg_helpers::dict_get_i32(dict, key_layer) else {
            continue;
        };
        if layer != K_CG_WINDOW_LAYER_NORMAL {
            continue;
        }
        let Some(pid) = cg_helpers::dict_get_i32(dict, key_pid) else {
            continue;
        };
        if pid == own_pid {
            continue;
        }
        let app_name = cg_helpers::dict_get_string(dict, key_owner)
            .unwrap_or_default()
            .trim()
            .to_string();
        let title = cg_helpers::dict_get_string(dict, key_name)
            .unwrap_or_default()
            .trim()
            .to_string();
        let Some(id) = cg_helpers::dict_get_i32(dict, key_number) else {
            continue;
        };
        if app_name.is_empty() && title.is_empty() {
            continue;
        }
        let has_title = !title.is_empty();
        let display_title = if title.is_empty() {
            app_name.clone()
        } else {
            title
        };
        let (wx, wy, ww, wh) = cg_helpers::dict_get_rect(dict, key_bounds)
            .unwrap_or((0.0, 0.0, 0.0, 0.0));
        result.push(CgWindow {
            id: id as u32, pid, app_name, title: display_title, has_title,
            x: wx as f32, y: wy as f32, w: ww as f32, h: wh as f32,
        });
    }

    unsafe {
        CFRelease(key_layer as *const c_void);
        CFRelease(key_pid as *const c_void);
        CFRelease(key_owner as *const c_void);
        CFRelease(key_name as *const c_void);
        CFRelease(key_number as *const c_void);
        CFRelease(key_bounds as *const c_void);
    }

    result
}

pub fn get_open_windows() -> Vec<WindowInfo> {
    let own_pid = std::process::id() as i32;
    let mut state = WindowEnumeration::default();
    let mut tracker = KnownWindowTracker::new();

    collect_on_screen_windows(own_pid, &mut state, &mut tracker);
    collect_minimized_windows(own_pid, &mut state, &mut tracker);
    tracker.persist();

    state.windows
}

/// Query the Accessibility API for the number of real windows an app has.
/// Returns None if AX is unavailable (no permission) or the app doesn't respond.
fn ax_windows(pid: i32) -> Option<HashMap<u32, AxWindowMeta>> {
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
        let windows_attr = cg_helpers::cfstr(b"AXWindows");
        let mut windows_value: *const c_void = std::ptr::null();
        let windows_err =
            AXUIElementCopyAttributeValue(app, windows_attr, &mut windows_value);
        CFRelease(windows_attr as *const c_void);
        if windows_err != 0 || windows_value.is_null() {
            CFRelease(app);
            return None;
        }

        let id_attr = cg_helpers::cfstr(b"_AXWindowID");
        let title_attr = cg_helpers::cfstr(b"AXTitle");
        let minimized_attr = cg_helpers::cfstr(b"AXMinimized");
        let count = CFArrayGetCount(windows_value as CFArrayRef);
        let mut out = HashMap::new();

        for i in 0..count {
            let win = CFArrayGetValueAtIndex(windows_value as CFArrayRef, i);
            if win.is_null() {
                continue;
            }

            let mut id_value: *const c_void = std::ptr::null();
            let id_err = AXUIElementCopyAttributeValue(win, id_attr, &mut id_value);
            if id_err != 0 || id_value.is_null() {
                if !id_value.is_null() {
                    CFRelease(id_value);
                }
                continue;
            }
            let Some(id) = cg_helpers::cfnumber_to_u32(id_value) else {
                CFRelease(id_value);
                continue;
            };
            CFRelease(id_value);

            let mut title = String::new();
            let mut title_value: *const c_void = std::ptr::null();
            let title_err = AXUIElementCopyAttributeValue(win, title_attr, &mut title_value);
            if title_err == 0 && !title_value.is_null() {
                title = cg_helpers::cfstring_to_string(title_value).unwrap_or_default();
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

            out.insert(
                id,
                AxWindowMeta {
                    title: title.trim().to_string(),
                    is_minimized,
                },
            );
        }

        CFRelease(id_attr as *const c_void);
        CFRelease(title_attr as *const c_void);
        CFRelease(minimized_attr as *const c_void);
        CFRelease(windows_value);
        CFRelease(app);
        if out.is_empty() {
            None
        } else {
            Some(out)
        }
    }
}

/// Check if a specific off-screen CG window is truly minimized via AX.
fn ax_is_window_minimized(pid: i32, cg_window_id: u32, title: &str) -> bool {
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
    let minimized_attr = cg_helpers::cfstr(b"AXMinimized");
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
fn dedup_by_ax(windows: Vec<CgWindow>) -> Vec<CgWindow> {
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

    let mut dedup_info: HashMap<i32, Option<PidDedup>> = HashMap::new();
    for pid in multi_pids {
        let info = ax_windows(pid).map(|meta| {
            let budget = meta.len().max(1);
            let ax_ids = meta.keys().copied().collect();
            PidDedup { ax_ids, ax_meta: meta, budget }
        });
        dedup_info.insert(pid, info);
    }

    let mut result = Vec::with_capacity(windows.len());
    let mut emitted_by_pid: HashMap<i32, usize> = HashMap::new();

    for mut win in windows {
        let Some(info) = dedup_info.get_mut(&win.pid) else {
            result.push(win);
            continue;
        };

        let Some(dedup) = info.as_mut() else {
            result.push(win);
            continue;
        };

        let emitted = emitted_by_pid.entry(win.pid).or_insert(0);
        if *emitted >= dedup.budget {
            continue;
        }

        if !dedup.ax_ids.is_empty() && !dedup.ax_ids.contains(&win.id) {
            continue;
        }

        if let Some(meta) = dedup.ax_meta.get(&win.id) {
            if !meta.title.is_empty() {
                win.title = meta.title.clone();
            }
        }

        *emitted += 1;
        result.push(win);
    }

    result
}

/// Check if a PID belongs to a regular app (appears in Dock / Cmd+Tab).
/// Returns false for menu bar apps, background agents, and system services.
fn is_regular_app(pid: i32) -> bool {
    use objc2_app_kit::{NSApplicationActivationPolicy, NSRunningApplication};

    objc2::rc::autoreleasepool(|_pool| {
        let Some(app) = NSRunningApplication::runningApplicationWithProcessIdentifier(pid) else {
            return false;
        };
        app.activationPolicy() == NSApplicationActivationPolicy::Regular
    })
}

pub fn get_app_icons(windows: &[WindowInfo]) -> HashMap<String, RgbaImage> {
    let own_pid = std::process::id() as i32;
    let opts = K_CG_WINDOW_LIST_EXCLUDE_DESKTOP_ELEMENTS;
    let list = unsafe { CGWindowListCopyWindowInfo(opts, K_CG_NULL_WINDOW_ID) };
    if list.is_null() {
        return HashMap::new();
    }

    let key_pid = cg_helpers::cfstr(b"kCGWindowOwnerPID");
    let key_owner = cg_helpers::cfstr(b"kCGWindowOwnerName");

    // Build app_name → pid mapping from CG list
    let mut app_pids: HashMap<String, i32> = HashMap::new();
    let count = unsafe { CFArrayGetCount(list) };
    for i in 0..count {
        let dict = unsafe { CFArrayGetValueAtIndex(list, i) } as CFDictionaryRef;
        if dict.is_null() {
            continue;
        }
        let Some(pid) = cg_helpers::dict_get_i32(dict, key_pid) else { continue };
        if pid == own_pid {
            continue;
        }
        let name = cg_helpers::dict_get_string(dict, key_owner)
            .unwrap_or_default()
            .trim()
            .to_string();
        if !name.is_empty() {
            app_pids.entry(name).or_insert(pid);
        }
    }

    unsafe {
        CFRelease(list as *const c_void);
        CFRelease(key_pid as *const c_void);
        CFRelease(key_owner as *const c_void);
    }

    // Only extract icons for apps that are in our window list
    let needed: std::collections::HashSet<&str> =
        windows.iter().map(|w| w.app_name.as_str()).collect();

    let mut icons = HashMap::new();
    for (name, pid) in &app_pids {
        if !needed.contains(name.as_str()) {
            continue;
        }
        if let Some(icon) = qol_plugin_api::app_icon::icon_for_pid(*pid, ICON_SIZE) {
            icons.insert(name.clone(), icon);
        }
    }
    icons
}

pub fn capture_previews_cg(
    targets: &[(usize, u32)],
    max_w: usize,
    max_h: usize,
) -> Vec<(usize, Option<RgbaImage>)> {
    std::thread::scope(|s| {
        let handles: Vec<_> = targets
            .iter()
            .map(|&(idx, wid)| {
                s.spawn(move || {
                    let result = cg_capture_window(wid, max_w, max_h);
                    (idx, result)
                })
            })
            .collect();
        handles
            .into_iter()
            .filter_map(|h| h.join().ok())
            .collect()
    })
}

fn cg_capture_window(wid: u32, max_w: usize, max_h: usize) -> Option<RgbaImage> {
    let img = unsafe {
        CGWindowListCreateImage(
            CG_RECT_NULL,
            K_CG_WINDOW_LIST_OPTION_INCLUDING_WINDOW,
            wid,
            K_CG_WINDOW_IMAGE_BOUNDS_IGNORE_FRAMING | K_CG_WINDOW_IMAGE_NOMINAL_RESOLUTION,
        )
    };
    if img.is_null() {
        return None;
    }
    let result = extract_bgra_from_raw_cgimage(img, max_w, max_h);
    unsafe { CFRelease(img) };
    result
}

fn extract_bgra_from_raw_cgimage(img: CGImageRef, max_w: usize, max_h: usize) -> Option<RgbaImage> {
    let src_w = unsafe { CGImageGetWidth(img) };
    let src_h = unsafe { CGImageGetHeight(img) };
    if src_w == 0 || src_h == 0 {
        return None;
    }

    let provider = unsafe { CGImageGetDataProvider(img) };
    if provider.is_null() {
        return None;
    }

    let cf_data = unsafe { CGDataProviderCopyData(provider) };
    if cf_data.is_null() {
        return None;
    }

    let ptr = unsafe { CFDataGetBytePtr(cf_data) };
    let len = unsafe { CFDataGetLength(cf_data) } as usize;
    let raw = unsafe { std::slice::from_raw_parts(ptr, len) };
    let bytes_per_row = unsafe { CGImageGetBytesPerRow(img) };

    let scale = (max_w as f32 / src_w as f32).min(max_h as f32 / src_h as f32).min(1.0);
    let scaled_w = ((src_w as f32 * scale).round() as usize).max(1).min(max_w);
    let scaled_h = ((src_h as f32 * scale).round() as usize).max(1).min(max_h);
    let offset_x = (max_w - scaled_w) / 2;
    let offset_y = (max_h - scaled_h) / 2;

    let mut bgra = vec![0u8; max_w * max_h * 4];
    for y in 0..scaled_h {
        let src_y = (y * src_h) / scaled_h;
        let row_start = src_y * bytes_per_row;
        for x in 0..scaled_w {
            let src_x = (x * src_w) / scaled_w;
            let src_off = row_start + src_x * 4;
            if src_off + 4 > len {
                continue;
            }
            let dst_off = ((offset_y + y) * max_w + offset_x + x) * 4;
            bgra[dst_off..dst_off + 4].copy_from_slice(&raw[src_off..src_off + 4]);
        }
    }

    unsafe { CFRelease(cf_data) };
    Some(RgbaImage { data: bgra, width: max_w, height: max_h })
}

pub fn activate_window(window_id: u32) {
    let Some((pid, title)) = cg_window_pid_and_title(window_id) else {
        return;
    };

    // Raise the specific AX window so the correct window comes to front,
    // not just whichever window macOS picks for the app.
    // Also unminimize if needed — AXRaise alone won't restore a minimized window.
    let win = unsafe { ax_find_window(pid, window_id, &title) };
    if !win.is_null() {
        unsafe {
            #[link(name = "ApplicationServices", kind = "framework")]
            extern "C" {
                fn AXUIElementPerformAction(el: *const c_void, action: *const c_void) -> i32;
                fn AXUIElementSetAttributeValue(
                    el: *const c_void,
                    attr: *const c_void,
                    val: *const c_void,
                ) -> i32;
            }
            #[link(name = "CoreFoundation", kind = "framework")]
            extern "C" {
                static kCFBooleanFalse: *const c_void;
            }
            let minimized_attr = cg_helpers::cfstr(b"AXMinimized");
            let _ = AXUIElementSetAttributeValue(win, minimized_attr, kCFBooleanFalse);
            CFRelease(minimized_attr as *const c_void);

            let raise = cg_helpers::cfstr(b"AXRaise");
            let _ = AXUIElementPerformAction(win, raise);
            CFRelease(raise as *const c_void);
            CFRelease(win);
        }
    }

    // Activate the app so it comes to the foreground.
    objc2::rc::autoreleasepool(|_pool| {
        use objc2_app_kit::{NSApplicationActivationOptions, NSRunningApplication};

        if let Some(app) = NSRunningApplication::runningApplicationWithProcessIdentifier(pid) {
            #[allow(deprecated)]
            let _ = app.activateWithOptions(NSApplicationActivationOptions::ActivateIgnoringOtherApps);
        }
    });
}

pub fn close_window(window_id: u32) {
    let Some((pid, title)) = cg_window_pid_and_title(window_id) else {
        return;
    };
    let win = unsafe { ax_find_window(pid, window_id, &title) };
    if win.is_null() {
        return;
    }
    unsafe {
        ax_press_window_button(win, b"AXCloseButton");
        CFRelease(win);
    }
}

pub fn quit_app(window_id: u32) {
    let Some((pid, _title)) = cg_window_pid_and_title(window_id) else {
        return;
    };
    objc2::rc::autoreleasepool(|_pool| {
        use objc2_app_kit::NSRunningApplication;
        if let Some(app) = NSRunningApplication::runningApplicationWithProcessIdentifier(pid) {
            let _ = app.terminate();
        }
    });
}

pub fn minimize_window_by_id(window_id: u32) {
    let Some((pid, title)) = cg_window_pid_and_title(window_id) else {
        return;
    };
    let win = unsafe { ax_find_window(pid, window_id, &title) };
    if win.is_null() {
        return;
    }
    unsafe {
        ax_set_minimized(win);
        CFRelease(win);
    }
}

/// Look up a CG window's owning pid and title by its window ID.
fn cg_window_pid_and_title(window_id: u32) -> Option<(i32, String)> {
    let opts = K_CG_WINDOW_LIST_EXCLUDE_DESKTOP_ELEMENTS;
    let list = unsafe { CGWindowListCopyWindowInfo(opts, K_CG_NULL_WINDOW_ID) };
    if list.is_null() {
        return None;
    }
    let key_pid = cg_helpers::cfstr(b"kCGWindowOwnerPID");
    let key_number = cg_helpers::cfstr(b"kCGWindowNumber");
    let key_name = cg_helpers::cfstr(b"kCGWindowName");

    let count = unsafe { CFArrayGetCount(list) };
    let mut result: Option<(i32, String)> = None;

    for i in 0..count {
        let dict = unsafe { CFArrayGetValueAtIndex(list, i) } as CFDictionaryRef;
        if dict.is_null() {
            continue;
        }
        let Some(num) = cg_helpers::dict_get_i32(dict, key_number) else {
            continue;
        };
        if num as u32 == window_id {
            if let Some(pid) = cg_helpers::dict_get_i32(dict, key_pid) {
                let title =
                    cg_helpers::dict_get_string(dict, key_name).unwrap_or_default();
                result = Some((pid, title));
            }
            break;
        }
    }

    unsafe {
        CFRelease(list as *const c_void);
        CFRelease(key_pid as *const c_void);
        CFRelease(key_number as *const c_void);
        CFRelease(key_name as *const c_void);
    }
    result
}

/// Find an AX window element for `pid`.
/// Tries (in order): `_AXWindowID` match → `AXTitle` match → first window if only one.
/// Returns a CFRetained pointer; caller must CFRelease it.
unsafe fn ax_find_window(pid: i32, cg_window_id: u32, title_hint: &str) -> *const c_void {
    #[link(name = "ApplicationServices", kind = "framework")]
    extern "C" {
        fn AXUIElementCreateApplication(pid: i32) -> *const c_void;
        fn AXUIElementCopyAttributeValue(
            el: *const c_void,
            attr: *const c_void,
            val: *mut *const c_void,
        ) -> i32;
        fn CFRetain(cf: *const c_void) -> *const c_void;
    }

    let app_el = AXUIElementCreateApplication(pid);
    if app_el.is_null() {
        return std::ptr::null();
    }

    let windows_attr = cg_helpers::cfstr(b"AXWindows");
    let mut wins_val: *const c_void = std::ptr::null();
    let err = AXUIElementCopyAttributeValue(app_el, windows_attr, &mut wins_val);
    CFRelease(windows_attr as *const c_void);
    CFRelease(app_el);
    if err != 0 || wins_val.is_null() {
        return std::ptr::null();
    }

    let id_attr = cg_helpers::cfstr(b"_AXWindowID");
    let title_attr = cg_helpers::cfstr(b"AXTitle");
    let count = CFArrayGetCount(wins_val as CFArrayRef);
    #[cfg(debug_assertions)]
    eprintln!("[alt-tab/ax_find_window] pid={} count={} cg_id={} title_hint={:?}", pid, count, cg_window_id, title_hint);

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

        // Try _AXWindowID
        if id_match.is_null() {
            let mut id_val: *const c_void = std::ptr::null();
            let err = AXUIElementCopyAttributeValue(win_el, id_attr, &mut id_val);
            if err == 0 && !id_val.is_null() {
                let maybe_id = cg_helpers::cfnumber_to_u32(id_val);
                CFRelease(id_val);
                if maybe_id == Some(cg_window_id) {
                    id_match = CFRetain(win_el);
                }
            } else {
                if !id_val.is_null() { CFRelease(id_val); }
            }
        }

        // Try AXTitle fallback
        if title_match.is_null() && !title_hint.is_empty() {
            let mut title_val: *const c_void = std::ptr::null();
            let err = AXUIElementCopyAttributeValue(win_el, title_attr, &mut title_val);
            if err == 0 && !title_val.is_null() {
                let ax_title = cg_helpers::cfstring_to_string(title_val).unwrap_or_default();
                CFRelease(title_val);
                if ax_title == title_hint {
                    title_match = CFRetain(win_el);
                }
            } else {
                if !title_val.is_null() { CFRelease(title_val); }
            }
        }
    }

    CFRelease(id_attr as *const c_void);
    CFRelease(title_attr as *const c_void);
    CFRelease(wins_val);

    // Pick best match: ID > title > first (only if exactly one window)
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

/// Press a named button (e.g. `AXCloseButton`) on an AX window element.
/// `win_el` must be a valid, retained AX element.
unsafe fn ax_press_window_button(win_el: *const c_void, button_attr_name: &[u8]) {
    #[link(name = "ApplicationServices", kind = "framework")]
    extern "C" {
        fn AXUIElementCopyAttributeValue(
            el: *const c_void,
            attr: *const c_void,
            val: *mut *const c_void,
        ) -> i32;
        fn AXUIElementPerformAction(el: *const c_void, action: *const c_void) -> i32;
    }

    let button_attr = cg_helpers::cfstr(button_attr_name);
    let press_action = cg_helpers::cfstr(b"AXPress");

    let mut button_val: *const c_void = std::ptr::null();
    let err = AXUIElementCopyAttributeValue(win_el, button_attr, &mut button_val);
    if err == 0 && !button_val.is_null() {
        let _ = AXUIElementPerformAction(button_val, press_action);
        CFRelease(button_val as *const c_void);
    }

    CFRelease(button_attr as *const c_void);
    CFRelease(press_action as *const c_void);
}

/// Set the `AXMinimized` attribute to true on an AX window element.
/// `win_el` must be a valid, retained AX element.
unsafe fn ax_set_minimized(win_el: *const c_void) {
    #[link(name = "ApplicationServices", kind = "framework")]
    extern "C" {
        fn AXUIElementSetAttributeValue(
            el: *const c_void,
            attr: *const c_void,
            val: *const c_void,
        ) -> i32;
    }
    #[link(name = "CoreFoundation", kind = "framework")]
    extern "C" {
        static kCFBooleanTrue: *const c_void;
    }

    let minimized_attr = cg_helpers::cfstr(b"AXMinimized");
    let _ = AXUIElementSetAttributeValue(win_el, minimized_attr, kCFBooleanTrue);
    CFRelease(minimized_attr as *const c_void);
}

pub fn move_app_window(_title: &str, _x: i32, _y: i32) -> bool {
    // macOS GPUI windows can't be reliably repositioned via AX after creation.
    // Return false so the caller closes and recreates on the correct monitor.
    false
}

pub fn picker_window_kind() -> gpui::WindowKind {
    gpui::WindowKind::Normal
}

pub fn dismiss_picker(_window: &mut gpui::Window) {
    // Hide the NSWindow via orderOut: instead of remove_window() or resize(1x1).
    // orderOut: hides the window completely (no visible artifact, no shadow dot)
    // while keeping the GPUI handle alive for the reuse fast-path.
    // activate_window() -> makeKeyAndOrderFront: will bring it back correctly sized.
    use objc2_app_kit::NSApplication;
    use objc2_foundation::MainThreadMarker;
    let mtm = MainThreadMarker::new().expect("must be on main thread");
    let app = NSApplication::sharedApplication(mtm);
    for win in app.windows().iter() {
        if win.title().to_string() == "qol-alt-tab-picker" {
            win.orderOut(None);
            return;
        }
    }
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

pub fn disable_window_shadow() {
    use objc2_app_kit::{NSApplication, NSColor};
    use objc2_foundation::MainThreadMarker;

    let mtm = MainThreadMarker::new().expect("must be on main thread");
    let app = NSApplication::sharedApplication(mtm);
    let clear = unsafe { NSColor::clearColor() };
    for window in app.windows().iter() {
        window.setHasShadow(false);
        window.setBackgroundColor(Some(&clear));
    }
}
