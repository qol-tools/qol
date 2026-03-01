use super::{CFRelease, CFRetain};
use super::buf::SendCVBuf;
use super::callback::SendPtr;
use super::setup::sc_fetch_content;
use super::stream::sc_snapshot_window;
use std::collections::HashMap;
use std::ffi::c_void;
use std::sync::{LazyLock, Mutex};

/// Cached YUV420 frames from last session. Populated on sc_stop_streams(),
/// consumed on picker open for instant surface() rendering.
pub(crate) static PREWARM_CACHE: LazyLock<Mutex<HashMap<u32, SendPtr>>> =
    LazyLock::new(|| Mutex::new(HashMap::new()));

/// Clone prewarm cache entries as SendCVBuf (retains each buffer).
/// Cache persists — entries are not removed.
pub(crate) fn sc_take_prewarm_surfaces() -> HashMap<u32, SendCVBuf> {
    let cache = PREWARM_CACHE.lock().unwrap();
    let mut result = HashMap::with_capacity(cache.len());
    for (&wid, sp) in cache.iter() {
        if sp.0.is_null() {
            continue;
        }
        unsafe { CFRetain(sp.0 as *const c_void) };
        result.insert(wid, SendCVBuf(sp.0));
    }
    result
}

/// Heartbeat: fetch content + snapshot all new (non-cached, non-minimized) windows.
/// Call from the background prewarm loop. Handles content lifecycle internally.
pub(crate) fn sc_heartbeat_snapshot(wids: &[u32], max_w: usize, max_h: usize) {
    let content_ptr = sc_fetch_content();
    if content_ptr.is_null() {
        return;
    }
    let cached = sc_prewarm_wids();
    for &wid in wids {
        if cached.contains(&wid) {
            continue;
        }
        sc_snapshot_window(content_ptr, wid, max_w, max_h);
    }
    unsafe { CFRelease(content_ptr as *const c_void) };
}

/// Return the set of window IDs currently in the prewarm cache (no buffer copies).
pub(crate) fn sc_prewarm_wids() -> std::collections::HashSet<u32> {
    PREWARM_CACHE
        .lock()
        .map(|cache| cache.keys().copied().collect())
        .unwrap_or_default()
}

/// Remove prewarm entries for windows no longer in `live_ids`.
pub(crate) fn sc_prewarm_retain(live_ids: &std::collections::HashSet<u32>) {
    let Ok(mut cache) = PREWARM_CACHE.lock() else { return };
    cache.retain(|wid, sp| {
        if live_ids.contains(wid) { return true; }
        if !sp.0.is_null() { unsafe { CFRelease(sp.0 as *const c_void) }; }
        false
    });
}
