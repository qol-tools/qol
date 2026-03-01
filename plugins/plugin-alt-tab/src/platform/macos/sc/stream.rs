use super::{CFRelease, CFRetain};
use super::callback::{sc_completion_block, sc_frame_receiver_class, FRAME_STORE, FRAMES_DIRTY};
use super::prewarm::PREWARM_CACHE;
use super::setup::{
    build_sc_config, build_sc_window_map, install_crash_diagnostics, sc_fetch_content,
    CMTime, CM_TIME_5FPS, CM_TIME_30FPS, SC_CONTENT_CACHE,
};
use block2::RcBlock;
use objc2::runtime::AnyObject;
use std::collections::HashMap;
use std::ffi::c_void;
use std::sync::Mutex;
use std::time::Duration;

/// Per-window stream handle. Stores raw ObjC pointers that we own (retained).
struct ScStreamHandle {
    stream: *mut c_void,   // SCStream*
    output: *mut c_void,   // SCFrameReceiver* (our custom class instance)
    content: *mut c_void,  // SCShareableContent*
    window_id: u32,
}
unsafe impl Send for ScStreamHandle {}

impl Drop for ScStreamHandle {
    fn drop(&mut self) {
        unsafe {
            if !self.stream.is_null() { CFRelease(self.stream as *const c_void); }
            if !self.output.is_null() { CFRelease(self.output as *const c_void); }
            if !self.content.is_null() { CFRelease(self.content as *const c_void); }
        }
    }
}

extern "C" {
    fn dispatch_get_global_queue(identifier: isize, flags: usize) -> *mut c_void;
}

const DISPATCH_QUEUE_PRIORITY_DEFAULT: isize = 0;

/// Active stream handles. Protected by mutex for start/stop lifecycle.
static ACTIVE_STREAMS: Mutex<Option<Vec<ScStreamHandle>>> = Mutex::new(None);

/// Create a single SCStream + output delegate + dispatch queue for one window.
/// Does NOT start capture. Returns None if any step fails.
fn create_single_stream(
    sc_win: *const AnyObject,
    content_ptr: *mut c_void,
    wid: u32,
    max_w: usize,
    max_h: usize,
    frame_interval: CMTime,
) -> Option<ScStreamHandle> {
    use objc2::msg_send;
    use objc2::runtime::AnyClass;

    let filter: *mut AnyObject = unsafe {
        let cls = AnyClass::get(c"SCContentFilter").unwrap();
        let raw: *mut AnyObject = msg_send![cls, alloc];
        msg_send![raw, initWithDesktopIndependentWindow: sc_win]
    };
    if filter.is_null() {
        return None;
    }

    let config = build_sc_config(max_w, max_h, frame_interval);

    let receiver_cls = sc_frame_receiver_class();
    let output: *mut AnyObject = unsafe {
        let raw: *mut AnyObject = msg_send![receiver_cls, alloc];
        let obj: *mut AnyObject = msg_send![raw, init];
        let ivar = (*obj).class().instance_variable(c"_windowId").unwrap();
        let wid_ptr = ivar.load_mut::<usize>(&mut *obj);
        *wid_ptr = wid as usize;
        obj
    };

    let stream: *mut AnyObject = unsafe {
        let cls = AnyClass::get(c"SCStream").unwrap();
        let raw: *mut AnyObject = msg_send![cls, alloc];
        let delegate: *const AnyObject = output;
        msg_send![raw, initWithFilter: filter, configuration: config, delegate: delegate]
    };

    unsafe {
        CFRelease(filter as *const c_void);
        CFRelease(config as *const c_void);
    }

    if stream.is_null() {
        unsafe { CFRelease(output as *const c_void) };
        return None;
    }

    let queue = unsafe { dispatch_get_global_queue(DISPATCH_QUEUE_PRIORITY_DEFAULT, 0) };
    if queue.is_null() {
        unsafe {
            CFRelease(stream as *const c_void);
            CFRelease(output as *const c_void);
        }
        return None;
    }

    let mut error: *mut AnyObject = std::ptr::null_mut();
    let queue_obj = queue as *mut AnyObject;
    let ok: bool = unsafe {
        msg_send![
            stream,
            addStreamOutput: output,
            type: 0i64,
            sampleHandlerQueue: queue_obj,
            error: &mut error as *mut *mut AnyObject,
        ]
    };
    if !ok {
        unsafe {
            CFRelease(stream as *const c_void);
            CFRelease(output as *const c_void);
        }
        return None;
    }

    unsafe {
        if !content_ptr.is_null() {
            CFRetain(content_ptr as *const c_void);
        }
    }

    Some(ScStreamHandle {
        stream: stream as *mut c_void,
        output: output as *mut c_void,
        content: content_ptr,
        window_id: wid,
    })
}

/// Synchronous start: calls startCaptureWithCompletionHandler: and waits for the
/// completion callback (up to 2s). Returns Ok(()) on success, Err(msg) on failure.
/// SCKit requires each stream to fully start before the next one begins.
fn sc_start_capture_sync(stream_ptr: *mut c_void) -> Result<(), String> {
    use objc2::msg_send;
    use std::sync::mpsc;

    let (tx, rx) = mpsc::sync_channel::<Option<String>>(1);
    let block = RcBlock::new(move |error: *mut AnyObject| {
        let msg = unsafe { super::setup::objc_error_string(error) };
        let _ = tx.send(msg);
    });

    unsafe {
        let stream = stream_ptr as *mut AnyObject;
        let _: () = msg_send![stream, startCaptureWithCompletionHandler: &*block];
    }

    match rx.recv_timeout(Duration::from_secs(2)) {
        Ok(None) => Ok(()),
        Ok(Some(msg)) => Err(msg),
        Err(_) => Err("timeout waiting for startCapture completion".into()),
    }
}

// ---------------------------------------------------------------------------
// Stream lifecycle: start, stop, promote, demote, snapshot
// ---------------------------------------------------------------------------

/// Start persistent SCStream for each target window. Frames arrive via callback.
pub(crate) fn sc_start_streams(targets: &[(usize, u32)], max_w: usize, max_h: usize) {
    install_crash_diagnostics();

    let content_ptr = sc_fetch_content();
    if content_ptr.is_null() {
        eprintln!("[alt-tab/sc] getShareableContent failed");
        return;
    }

    start_streams_with_ptr(targets, max_w, max_h, content_ptr);
    unsafe { CFRelease(content_ptr as *const c_void) };
}

/// Start persistent SCStreams using cached SCShareableContent (falls back to fresh fetch).
pub(crate) fn sc_start_streams_with_content(targets: &[(usize, u32)], max_w: usize, max_h: usize) {
    install_crash_diagnostics();

    // Try cached content, fall back to fresh fetch
    let cached = SC_CONTENT_CACHE.lock().unwrap()
        .as_ref()
        .filter(|sp| !sp.0.is_null())
        .map(|sp| { unsafe { CFRetain(sp.0 as *const c_void) }; sp.0 });
    let content_ptr = cached.unwrap_or_else(|| sc_fetch_content());
    if content_ptr.is_null() {
        eprintln!("[alt-tab/sc] no SC content available");
        return;
    }

    start_streams_with_ptr(targets, max_w, max_h, content_ptr);
    unsafe { CFRelease(content_ptr as *const c_void) };
}

/// Internal: start streams given a retained content pointer.
fn start_streams_with_ptr(
    targets: &[(usize, u32)],
    max_w: usize,
    max_h: usize,
    content_ptr: *mut c_void,
) {
    sc_stop_streams();
    *FRAME_STORE.lock().unwrap() = Some(HashMap::new());

    eprintln!(
        "[alt-tab/sc] start_streams: {} targets, {}x{}",
        targets.len(),
        max_w,
        max_h
    );

    let handles = objc2::rc::autoreleasepool(|_| {
        let sc_map = build_sc_window_map(content_ptr);
        let mut handles = Vec::new();

        for &(_idx, wid) in targets {
            let Some(&sc_win) = sc_map.get(&wid) else { continue };
            let Some(handle) = create_single_stream(sc_win, content_ptr, wid, max_w, max_h, CM_TIME_5FPS) else { continue };

            match sc_start_capture_sync(handle.stream) {
                Ok(()) => {
                    eprintln!("[alt-tab/sc] stream wid={} started OK", wid);
                    handles.push(handle);
                }
                Err(msg) => {
                    eprintln!("[alt-tab/sc] startCapture FAILED wid={}: {}", wid, msg);
                }
            }
        }
        eprintln!("[alt-tab/sc] started {} YUV420 streams at 5fps (spotlight promotes selected to 30fps)", handles.len());
        handles
    });

    let mut active = ACTIVE_STREAMS.lock().unwrap();
    *active = Some(handles);
}

/// Returns true if there are active SCStreams running.
pub(crate) fn sc_streams_active() -> bool {
    ACTIVE_STREAMS.lock().unwrap().as_ref().is_some_and(|v| !v.is_empty())
}

/// Stop all active SCStreams and clear the frame store.
pub(crate) fn sc_stop_streams() {
    use objc2::msg_send;

    if let Some(handles) = ACTIVE_STREAMS.lock().unwrap().take() {
        let completion = sc_completion_block();
        for h in &handles {
            unsafe {
                let stream = h.stream as *mut AnyObject;
                let _: () = msg_send![stream, stopCaptureWithCompletionHandler: &*completion];
            }
        }
        // Give streams time to fully stop before releasing handles.
        // Too short → new streams for the same windows fail to start.
        std::thread::sleep(Duration::from_millis(50));
        // handles drop here → CFRelease stream/output/queue
    }

    // Move stored frames to prewarm cache (for instant open next time)
    let map = FRAME_STORE.lock().unwrap().take();
    FRAMES_DIRTY.store(false, std::sync::atomic::Ordering::Relaxed);
    let Some(map) = map else { return };
    let Ok(mut cache) = PREWARM_CACHE.lock() else { return };
    for (wid, sp) in map {
        if sp.0.is_null() { continue; }
        if let Some(old) = cache.insert(wid, sp).filter(|p| !p.0.is_null()) {
            unsafe { CFRelease(old.0 as *const c_void) };
        }
    }
}

/// Promote an active stream to 30fps via updateConfiguration:completionHandler:.
/// No stream teardown — instant framerate change on a running stream.
pub(crate) fn sc_promote_stream(wid: u32, max_w: usize, max_h: usize) {
    sc_update_stream_framerate(wid, max_w, max_h, CM_TIME_30FPS);
}

/// Demote an active stream to 5fps via updateConfiguration:completionHandler:.
/// No stream teardown — instant framerate change on a running stream.
pub(crate) fn sc_demote_stream(wid: u32, max_w: usize, max_h: usize) {
    sc_update_stream_framerate(wid, max_w, max_h, CM_TIME_5FPS);
}

/// Update the minimumFrameInterval of an active stream via updateConfiguration:completionHandler:.
fn sc_update_stream_framerate(wid: u32, max_w: usize, max_h: usize, interval: CMTime) {
    use objc2::msg_send;

    let active = ACTIVE_STREAMS.lock().unwrap();
    let Some(handles) = active.as_ref() else {
        eprintln!("[alt-tab/sc] updateConfig: no active streams");
        return;
    };
    let Some(handle) = handles.iter().find(|h| h.window_id == wid) else { return };
    eprintln!("[alt-tab/sc] updateConfig wid={} timescale={}", wid, interval.timescale);

    let config = build_sc_config(max_w, max_h, interval);
    let completion = sc_completion_block();
    unsafe {
        let stream = handle.stream as *mut AnyObject;
        let _: () = msg_send![
            stream,
            updateConfiguration: config,
            completionHandler: &*completion,
        ];
        CFRelease(config as *const c_void);
    }
}

/// One-shot capture: start temp stream, wait for 1 frame (up to 500ms), stop.
/// Stores result directly in PREWARM_CACHE. Returns true if frame captured.
/// Safe to call only while picker is hidden (uses FRAME_STORE, no conflict with live streams).
pub(crate) fn sc_snapshot_window(
    content_ptr: *mut c_void,
    wid: u32,
    max_w: usize,
    max_h: usize,
) -> bool {
    use objc2::msg_send;

    if content_ptr.is_null() {
        return false;
    }

    let sc_map = build_sc_window_map(content_ptr);
    let Some(&sc_win) = sc_map.get(&wid) else {
        return false;
    };

    // Init FRAME_STORE for the snapshot (may already exist from a prior snapshot)
    FRAME_STORE.lock().unwrap().get_or_insert_with(HashMap::new);

    let handle = objc2::rc::autoreleasepool(|_| {
        create_single_stream(sc_win, content_ptr, wid, max_w, max_h, CM_TIME_30FPS)
    });
    let Some(handle) = handle else {
        return false;
    };

    // Start capture synchronously
    if let Err(msg) = sc_start_capture_sync(handle.stream) {
        eprintln!("[alt-tab/sc] snapshot startCapture failed wid={}: {}", wid, msg);
        return false;
    }

    // Poll FRAME_STORE for up to 500ms until a frame arrives
    let deadline = std::time::Instant::now() + Duration::from_millis(500);
    let mut got_frame = false;
    while std::time::Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(16));
        let store = FRAME_STORE.lock().unwrap();
        let Some(map) = store.as_ref() else {
            break; // sc_stop_streams was called — abort
        };
        let Some(sp) = map.get(&wid) else { continue };
        if sp.0.is_null() {
            continue;
        }
        got_frame = true;
        break;
    }

    // Stop capture
    let completion = sc_completion_block();
    unsafe {
        let stream = handle.stream as *mut AnyObject;
        let _: () = msg_send![stream, stopCaptureWithCompletionHandler: &*completion];
    }
    std::thread::sleep(Duration::from_millis(20));

    drop(handle); // releases stream/output/queue

    if !got_frame {
        return false;
    }

    // Move frame from FRAME_STORE to PREWARM_CACHE
    let mut store = FRAME_STORE.lock().unwrap();
    let Some(map) = store.as_mut() else { return false };
    let Some(sp) = map.remove(&wid) else { return false };
    if sp.0.is_null() {
        return false;
    }
    let Ok(mut cache) = PREWARM_CACHE.lock() else { return true };
    if let Some(old) = cache.insert(wid, sp).filter(|p| !p.0.is_null()) {
        unsafe { CFRelease(old.0 as *const c_void) };
    }
    true
}
