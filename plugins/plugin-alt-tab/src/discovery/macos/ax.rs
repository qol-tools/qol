use super::ffi;
use super::ffi::{CFArrayGetCount, CFArrayGetValueAtIndex, CFArrayRef, CFRelease, CFRetain};
use super::CgWindow;
use std::collections::{HashMap, HashSet};
use std::ffi::c_void;
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};

const AX_MESSAGING_TIMEOUT_SECONDS: f32 = 0.05;

#[link(name = "ApplicationServices", kind = "framework")]
extern "C" {
    fn AXUIElementCreateSystemWide() -> *const c_void;
    fn AXUIElementCreateApplication(pid: i32) -> *const c_void;
    fn AXUIElementCopyAttributeValue(
        el: *const c_void,
        attr: *const c_void,
        val: *mut *const c_void,
    ) -> i32;
    fn AXUIElementSetMessagingTimeout(el: *const c_void, timeout: f32) -> i32;
    fn _AXUIElementCreateWithRemoteToken(token: *const c_void) -> *const c_void;
    fn _AXUIElementGetWindow(el: *const c_void, window_id: *mut u32) -> i32;
}

#[link(name = "CoreFoundation", kind = "framework")]
extern "C" {
    fn CFDataCreate(allocator: *const c_void, bytes: *const u8, length: isize) -> *const c_void;
}

pub(crate) fn init_messaging_timeout() {
    unsafe {
        let system = AXUIElementCreateSystemWide();
        if system.is_null() {
            return;
        }
        AXUIElementSetMessagingTimeout(system, AX_MESSAGING_TIMEOUT_SECONDS);
        CFRelease(system);
    }
}

unsafe fn cap_messaging_timeout(el: *const c_void) {
    if el.is_null() {
        return;
    }
    AXUIElementSetMessagingTimeout(el, AX_MESSAGING_TIMEOUT_SECONDS);
}

#[link(name = "CoreFoundation", kind = "framework")]
extern "C" {
    static kCFBooleanTrue: *const c_void;
}

#[derive(Clone)]
pub(super) struct AxWindowMeta {
    pub title: String,
    pub is_minimized: bool,
}

unsafe fn ax_open_window_list(pid: i32) -> *const c_void {
    let app = AXUIElementCreateApplication(pid);
    if app.is_null() {
        return std::ptr::null();
    }
    cap_messaging_timeout(app);
    let attr = ffi::cfstr(b"AXWindows");
    let mut val: *const c_void = std::ptr::null();
    let err = AXUIElementCopyAttributeValue(app, attr, &mut val);
    CFRelease(attr);
    CFRelease(app);
    if err != 0 || val.is_null() {
        return std::ptr::null();
    }
    val
}

unsafe fn ax_copy_attr(el: *const c_void, attr: *const c_void) -> *const c_void {
    cap_messaging_timeout(el);
    let mut val: *const c_void = std::ptr::null();
    let err = AXUIElementCopyAttributeValue(el, attr, &mut val);
    if err != 0 || val.is_null() {
        if !val.is_null() {
            CFRelease(val);
        }
        return std::ptr::null();
    }
    val
}

unsafe fn ax_check_subrole(win: *const c_void, attr: *const c_void) -> bool {
    let val = ax_copy_attr(win, attr);
    if val.is_null() {
        return true;
    } // include when AX unavailable — safe default
    let subrole = ffi::cfstring_to_string(val);
    CFRelease(val);
    matches!(subrole.as_deref(), Some("AXStandardWindow" | "AXDialog"))
}

unsafe fn ax_has_window_subrole(win: *const c_void, attr: *const c_void) -> bool {
    let val = ax_copy_attr(win, attr);
    if val.is_null() {
        return false;
    }
    let subrole = ffi::cfstring_to_string(val);
    CFRelease(val);
    matches!(subrole.as_deref(), Some("AXStandardWindow" | "AXDialog"))
}

unsafe fn ax_read_window_id(win: *const c_void, attr: *const c_void) -> Option<u32> {
    let mut id = 0;
    if _AXUIElementGetWindow(win, &mut id) == 0 && id > 0 {
        return Some(id);
    }
    let val = ax_copy_attr(win, attr);
    if val.is_null() {
        return None;
    }
    let id = ffi::cfnumber_to_u32(val);
    CFRelease(val);
    id
}

#[cfg(debug_assertions)]
pub(crate) unsafe fn ax_focused_window_id(pid: i32) -> Option<u32> {
    let app = AXUIElementCreateApplication(pid);
    if app.is_null() {
        return None;
    }
    cap_messaging_timeout(app);
    let focused_attr = ffi::cfstr(b"AXFocusedWindow");
    let win = ax_copy_attr(app, focused_attr);
    CFRelease(focused_attr);
    CFRelease(app);
    if win.is_null() {
        return None;
    }
    let id_attr = ffi::cfstr(b"_AXWindowID");
    let id = ax_read_window_id(win, id_attr);
    CFRelease(id_attr);
    CFRelease(win);
    id
}

unsafe fn ax_read_title(win: *const c_void, attr: *const c_void) -> String {
    let val = ax_copy_attr(win, attr);
    if val.is_null() {
        return String::new();
    }
    let title = ffi::cfstring_to_string(val).unwrap_or_default();
    CFRelease(val);
    title
}

unsafe fn ax_read_bool(win: *const c_void, attr: *const c_void) -> bool {
    let val = ax_copy_attr(win, attr);
    if val.is_null() {
        return false;
    }
    let result = val == kCFBooleanTrue;
    CFRelease(val);
    result
}

struct AxAttrs {
    subrole: *const c_void,
    id: *const c_void,
    title: *const c_void,
    minimized: *const c_void,
}

impl AxAttrs {
    fn new() -> Self {
        Self {
            subrole: ffi::cfstr(b"AXSubrole"),
            id: ffi::cfstr(b"_AXWindowID"),
            title: ffi::cfstr(b"AXTitle"),
            minimized: ffi::cfstr(b"AXMinimized"),
        }
    }
}

impl Drop for AxAttrs {
    fn drop(&mut self) {
        unsafe {
            CFRelease(self.subrole);
            CFRelease(self.id);
            CFRelease(self.title);
            CFRelease(self.minimized);
        }
    }
}

pub(super) type AxWindowsResult = Option<(HashMap<u32, AxWindowMeta>, Vec<AxWindowMeta>, usize)>;

/// Skip re-querying the same PID within this window. Activity Monitor and other
/// background helpers can stall for 400ms+ on AXWindows lookups; caching empty/slow
/// results means repeated Alt+Tab presses don't pay that cost every time.
const AX_CACHE_TTL: Duration = Duration::from_millis(2000);

struct CachedAxEntry {
    captured_at: Instant,
    value: AxWindowsResult,
}

static AX_WINDOWS_CACHE: OnceLock<Mutex<HashMap<i32, CachedAxEntry>>> = OnceLock::new();

fn ax_windows_cache() -> &'static Mutex<HashMap<i32, CachedAxEntry>> {
    AX_WINDOWS_CACHE.get_or_init(|| Mutex::new(HashMap::new()))
}

fn cached_ax_windows(pid: i32) -> Option<AxWindowsResult> {
    let cache = ax_windows_cache().lock().ok()?;
    let entry = cache.get(&pid)?;
    if entry.captured_at.elapsed() >= AX_CACHE_TTL {
        return None;
    }
    Some(entry.value.clone())
}

fn remember_ax_windows(pid: i32, value: &AxWindowsResult) {
    let Ok(mut cache) = ax_windows_cache().lock() else {
        return;
    };
    cache.insert(
        pid,
        CachedAxEntry {
            captured_at: Instant::now(),
            value: value.clone(),
        },
    );
}

pub(super) fn ax_windows(pid: i32) -> AxWindowsResult {
    if let Some(cached) = cached_ax_windows(pid) {
        #[cfg(debug_assertions)]
        eprintln!("[alt-tab/ax] cache hit pid={}", pid);
        return cached;
    }
    let computed = ax_windows_compute(pid);
    remember_ax_windows(pid, &computed);
    computed
}

/// Run `ax_windows` for every PID concurrently. One hung process no longer serializes
/// with the rest — total wall time collapses from `sum(per-pid)` to `max(per-pid)`.
pub(super) fn prefetch_ax_parallel(pids: HashSet<i32>) -> HashMap<i32, AxWindowsResult> {
    #[cfg(debug_assertions)]
    let t = Instant::now();
    #[cfg(debug_assertions)]
    let pid_count = pids.len();
    let result: HashMap<i32, AxWindowsResult> = std::thread::scope(|s| {
        let handles: Vec<_> = pids
            .into_iter()
            .map(|pid| s.spawn(move || (pid, ax_windows(pid))))
            .collect();
        handles.into_iter().filter_map(|h| h.join().ok()).collect()
    });
    #[cfg(debug_assertions)]
    eprintln!(
        "[alt-tab/ax] prefetch_ax_parallel pids={} {}ms",
        pid_count,
        t.elapsed().as_millis()
    );
    result
}

fn ax_windows_compute(pid: i32) -> AxWindowsResult {
    #[cfg(debug_assertions)]
    let t_open = std::time::Instant::now();
    let wins_val = unsafe { ax_open_window_list(pid) };
    if wins_val.is_null() {
        #[cfg(debug_assertions)]
        {
            let open_ms = t_open.elapsed().as_millis();
            if open_ms >= 50 {
                eprintln!(
                    "[alt-tab/ax] SLOW ax_open_window_list pid={} {}ms (null result)",
                    pid, open_ms
                );
            }
        }
        return None;
    }
    let attrs = AxAttrs::new();
    let count = unsafe { CFArrayGetCount(wins_val as CFArrayRef) };
    #[cfg(debug_assertions)]
    eprintln!(
        "[alt-tab/ax] ax_windows pid={} AXWindows count={}",
        pid, count
    );

    #[cfg(debug_assertions)]
    let t_collect = std::time::Instant::now();
    let (id_map, all_meta, accepted) = collect_ax_window_meta(wins_val, count, &attrs);
    unsafe { CFRelease(wins_val) };
    drop(attrs);
    #[cfg(debug_assertions)]
    {
        let open_ms = t_open.elapsed().as_millis();
        let collect_ms = t_collect.elapsed().as_millis();
        if open_ms >= 100 {
            eprintln!(
                "[alt-tab/ax] SLOW ax_windows pid={} total={}ms collect={}ms count={}",
                pid, open_ms, collect_ms, count
            );
        }
    }
    #[cfg(debug_assertions)]
    eprintln!(
        "[alt-tab/ax] ax_windows pid={} id_map={} all_meta={} accepted={}",
        pid,
        id_map.len(),
        all_meta.len(),
        accepted
    );
    Some((id_map, all_meta, accepted))
}

fn collect_ax_window_meta(
    wins_val: *const c_void,
    count: isize,
    attrs: &AxAttrs,
) -> (HashMap<u32, AxWindowMeta>, Vec<AxWindowMeta>, usize) {
    let mut id_map = HashMap::new();
    let mut all_meta = Vec::new();
    let mut accepted: usize = 0;

    for i in 0..count {
        let win = unsafe { CFArrayGetValueAtIndex(wins_val as CFArrayRef, i) };
        if win.is_null() {
            continue;
        }
        if !unsafe { ax_check_subrole(win, attrs.subrole) } {
            continue;
        }
        accepted += 1;
        let meta = AxWindowMeta {
            title: unsafe { ax_read_title(win, attrs.title) }
                .trim()
                .to_string(),
            is_minimized: unsafe { ax_read_bool(win, attrs.minimized) },
        };
        all_meta.push(meta.clone());
        if let Some(id) = unsafe { ax_read_window_id(win, attrs.id) } {
            id_map.insert(id, meta);
        }
    }
    (id_map, all_meta, accepted)
}

pub(super) type AxCache =
    HashMap<i32, Option<(HashMap<u32, AxWindowMeta>, Vec<AxWindowMeta>, usize)>>;

pub(super) fn dedup_by_ax(windows: Vec<CgWindow>, ax_cache: &mut AxCache) -> Vec<CgWindow> {
    let dedup_info = build_dedup_info(&windows, ax_cache);
    emit_deduped(windows, &dedup_info)
}

struct PidDedup {
    ax_ids: HashSet<u32>,
    ax_meta: HashMap<u32, AxWindowMeta>,
    budget: usize,
}

fn build_dedup_info(windows: &[CgWindow], ax_cache: &mut AxCache) -> HashMap<i32, PidDedup> {
    let mut cg_count: HashMap<i32, usize> = HashMap::new();
    for w in windows {
        *cg_count.entry(w.pid).or_insert(0) += 1;
    }
    let mut info = HashMap::new();
    for (pid, count) in cg_count {
        if count <= 1 {
            continue;
        }
        let ax_result = ax_cache.entry(pid).or_insert_with(|| ax_windows(pid));
        info.insert(pid, pid_dedup_from_ax(ax_result));
    }
    info
}

fn pid_dedup_from_ax(
    ax: &Option<(HashMap<u32, AxWindowMeta>, Vec<AxWindowMeta>, usize)>,
) -> PidDedup {
    match ax {
        Some((id_map, _, accepted)) if !id_map.is_empty() => PidDedup {
            budget: (*accepted).max(id_map.len()),
            ax_ids: id_map.keys().copied().collect(),
            ax_meta: id_map.clone(),
        },
        Some((_, _, accepted)) => PidDedup {
            ax_ids: HashSet::new(),
            ax_meta: HashMap::new(),
            budget: (*accepted).max(1),
        },
        // AX unavailable: keep 1 window per PID (safe default —
        // avoids leaking system-injected overlays when AX times out).
        None => PidDedup {
            ax_ids: HashSet::new(),
            ax_meta: HashMap::new(),
            budget: 1,
        },
    }
}

fn emit_deduped(windows: Vec<CgWindow>, dedup_info: &HashMap<i32, PidDedup>) -> Vec<CgWindow> {
    let mut emitted: HashMap<i32, usize> = HashMap::new();
    let mut result = Vec::with_capacity(windows.len());
    for mut win in windows {
        let Some(dedup) = dedup_info.get(&win.pid) else {
            result.push(win);
            continue;
        };
        let count = emitted.entry(win.pid).or_insert(0);
        if !should_keep(&win, dedup, *count) {
            continue;
        }
        if let Some(meta) = dedup.ax_meta.get(&win.id).filter(|m| !m.title.is_empty()) {
            win.title = meta.title.clone();
        }
        *count += 1;
        result.push(win);
    }
    result
}

fn should_keep(win: &CgWindow, dedup: &PidDedup, emitted: usize) -> bool {
    if emitted >= dedup.budget {
        #[cfg(debug_assertions)]
        eprintln!(
            "[alt-tab/ax] DEDUP budget exhausted: wid={} app={:?} budget={}",
            win.id, win.app_name, dedup.budget
        );
        return false;
    }
    if dedup.ax_ids.len() >= dedup.budget && !dedup.ax_ids.contains(&win.id) {
        #[cfg(debug_assertions)]
        eprintln!(
            "[alt-tab/ax] DEDUP not in AX ids: wid={} app={:?}",
            win.id, win.app_name
        );
        return false;
    }
    true
}

pub(crate) unsafe fn ax_find_window(
    pid: i32,
    cg_window_id: u32,
    title_hint: &str,
) -> *const c_void {
    let wins_val = ax_open_window_list(pid);
    if wins_val.is_null() {
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

    let result = scan_for_match(
        wins_val,
        count,
        cg_window_id,
        title_hint,
        id_attr,
        title_attr,
    );

    CFRelease(id_attr);
    CFRelease(title_attr);
    CFRelease(wins_val);
    result
}

pub(crate) unsafe fn ax_find_window_brute_force(pid: i32, cg_window_id: u32) -> *const c_void {
    let started = Instant::now();
    let deadline = started + Duration::from_millis(100);
    let id_attr = ffi::cfstr(b"_AXWindowID");
    let subrole_attr = ffi::cfstr(b"AXSubrole");
    let mut result = std::ptr::null();
    #[cfg(debug_assertions)]
    let mut scanned = 0;
    #[cfg(debug_assertions)]
    let mut readable = 0;
    #[cfg(debug_assertions)]
    let mut windowish = 0;
    for element_id in 0..1000i64 {
        if Instant::now() >= deadline {
            break;
        }
        #[cfg(debug_assertions)]
        {
            scanned += 1;
        }
        let el = ax_element_from_remote_token(pid, element_id);
        if el.is_null() {
            continue;
        }
        let id = ax_read_window_id(el, id_attr);
        #[cfg(debug_assertions)]
        {
            if id.is_some() {
                readable += 1;
            }
        }
        if id == Some(cg_window_id) {
            if !ax_has_window_subrole(el, subrole_attr) {
                CFRelease(el);
                continue;
            }
            #[cfg(debug_assertions)]
            {
                windowish += 1;
            }
            result = el;
            break;
        }
        CFRelease(el);
    }
    CFRelease(id_attr);
    CFRelease(subrole_attr);
    #[cfg(debug_assertions)]
    qol_runtime::probe!(
        "ACTIVATE_BRUTE_SCAN",
        "wid={cg_window_id} scanned={scanned} readable={readable} windowish={windowish} elapsed_ms={} hit={}",
        started.elapsed().as_millis(),
        !result.is_null()
    );
    result
}

unsafe fn ax_element_from_remote_token(pid: i32, element_id: i64) -> *const c_void {
    let mut token = [0u8; 20];
    token[0..4].copy_from_slice(&pid.to_le_bytes());
    token[8..12].copy_from_slice(&0x636f636f_i32.to_le_bytes());
    token[12..20].copy_from_slice(&element_id.to_le_bytes());
    let data = CFDataCreate(std::ptr::null(), token.as_ptr(), token.len() as isize);
    if data.is_null() {
        return std::ptr::null();
    }
    let el = _AXUIElementCreateWithRemoteToken(data);
    CFRelease(data);
    el
}

unsafe fn scan_for_match(
    wins_val: *const c_void,
    count: isize,
    cg_window_id: u32,
    title_hint: &str,
    id_attr: *const c_void,
    title_attr: *const c_void,
) -> *const c_void {
    let mut id_match: *const c_void = std::ptr::null();
    let mut title_match: *const c_void = std::ptr::null();
    let mut first_win: *const c_void = std::ptr::null();

    for i in 0..count {
        let win = CFArrayGetValueAtIndex(wins_val as CFArrayRef, i);
        if win.is_null() {
            continue;
        }
        if first_win.is_null() {
            first_win = CFRetain(win);
        }
        if id_match.is_null() && ax_read_window_id(win, id_attr) == Some(cg_window_id) {
            id_match = CFRetain(win);
        }
        if title_match.is_null()
            && !title_hint.is_empty()
            && ax_read_title(win, title_attr) == title_hint
        {
            title_match = CFRetain(win);
        }
    }

    pick_best_match(id_match, title_match, first_win, count)
}

unsafe fn pick_best_match(
    id_match: *const c_void,
    title_match: *const c_void,
    first_win: *const c_void,
    count: isize,
) -> *const c_void {
    if !id_match.is_null() {
        if !title_match.is_null() {
            CFRelease(title_match);
        }
        if !first_win.is_null() {
            CFRelease(first_win);
        }
        return id_match;
    }
    if !title_match.is_null() {
        if !first_win.is_null() {
            CFRelease(first_win);
        }
        return title_match;
    }
    if count == 1 && !first_win.is_null() {
        return first_win;
    }
    if !first_win.is_null() {
        CFRelease(first_win);
    }
    std::ptr::null()
}
