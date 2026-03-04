use super::{CFRelease, CFRetain};
use super::buf::SendCVBuf;
use block2::RcBlock;
use objc2::runtime::AnyObject;
use std::collections::HashMap;
use std::ffi::c_void;
use std::sync::Mutex;

#[link(name = "CoreMedia", kind = "framework")]
extern "C" {
    fn CMSampleBufferGetImageBuffer(sbuf: *const c_void) -> *const c_void;
}

/// Wrapper so `*mut c_void` (CVPixelBufferRef) can live in a static Mutex.
pub(crate) struct SendPtr(pub(crate) *mut c_void);
unsafe impl Send for SendPtr {}

/// Latest frame per window_id. Written by ObjC callback, read by main thread.
pub(crate) static FRAME_STORE: Mutex<Option<HashMap<u32, SendPtr>>> = Mutex::new(None);

/// Set by SCKit callback when a new frame arrives. Checked by repaint timer.
pub(crate) static FRAMES_DIRTY: std::sync::atomic::AtomicBool =
    std::sync::atomic::AtomicBool::new(false);

/// Diagnostic counters for callback invocations.
static CB_CALLS: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
static CB_STORED: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
static CB_NULL_IMG: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

/// Register the SCFrameReceiver ObjC class (once). Returns the class pointer.
pub(crate) fn sc_frame_receiver_class() -> &'static objc2::runtime::AnyClass {
    use objc2::runtime::{AnyClass, AnyProtocol, ClassBuilder, Sel};
    use std::sync::Once;

    static REGISTER: Once = Once::new();
    static mut CLS: *const AnyClass = std::ptr::null();

    REGISTER.call_once(|| {
        let superclass = AnyClass::get(c"NSObject").unwrap();
        let mut builder = ClassBuilder::new(c"QoLSCFrameReceiver", superclass).unwrap();

        if let Some(proto) = AnyProtocol::get(c"SCStreamOutput") {
            builder.add_protocol(proto);
        }
        if let Some(proto) = AnyProtocol::get(c"SCStreamDelegate") {
            builder.add_protocol(proto);
        }

        // ivar: window_id (u32 stored as usize for alignment)
        builder.add_ivar::<usize>(c"_windowId");

        // SCStreamOutput protocol method: stream:didOutputSampleBuffer:ofType:
        // All pointer args must be *mut AnyObject (encoding @) to match protocol signature.
        unsafe {
            builder.add_method(
                Sel::register(c"stream:didOutputSampleBuffer:ofType:"),
                sc_frame_callback
                    as unsafe extern "C" fn(*mut AnyObject, Sel, *mut AnyObject, *mut AnyObject, i64),
            );
            builder.add_method(
                Sel::register(c"stream:didStopWithError:"),
                sc_stream_stopped_callback
                    as unsafe extern "C" fn(*mut AnyObject, Sel, *mut AnyObject, *mut AnyObject),
            );
        }

        let cls = builder.register();
        unsafe { CLS = cls as *const AnyClass; }
    });

    unsafe { &*CLS }
}

unsafe extern "C" fn sc_stream_stopped_callback(
    this: *mut AnyObject,
    _sel: objc2::runtime::Sel,
    _stream: *mut AnyObject,
    error: *mut AnyObject,
) {
    let Some(ivar) = (*this).class().instance_variable(c"_windowId") else { return };
    let wid = *ivar.load::<usize>(&*this) as u32;
    let msg = super::setup::objc_error_string(error).unwrap_or_else(|| "no error".into());
    eprintln!("[alt-tab/sc] stream stopped wid={} err={}", wid, msg);
}

/// ObjC callback: stream:didOutputSampleBuffer:ofType:
/// Extracts CVPixelBuffer from CMSampleBuffer, stores in FRAME_STORE.
unsafe extern "C" fn sc_frame_callback(
    this: *mut AnyObject,
    _sel: objc2::runtime::Sel,
    _stream: *mut AnyObject,
    sample_buffer: *mut AnyObject,
    _output_type: i64,
) {
    CB_CALLS.fetch_add(1, std::sync::atomic::Ordering::Relaxed);

    if sample_buffer.is_null() {
        return;
    }
    let cv_buf = CMSampleBufferGetImageBuffer(sample_buffer as *const c_void);
    if cv_buf.is_null() {
        CB_NULL_IMG.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        return;
    }

    // Read window_id from ivar
    let Some(ivar) = (*this).class().instance_variable(c"_windowId") else {
        return;
    };
    let wid = *ivar.load::<usize>(&*this) as u32;

    // Retain the new CVPixelBuffer, swap into store, release old
    CFRetain(cv_buf);
    let Ok(mut store) = FRAME_STORE.lock() else {
        CFRelease(cv_buf);
        return;
    };
    let Some(map) = store.as_mut() else {
        CFRelease(cv_buf);
        return;
    };
    let old = map.insert(wid, SendPtr(cv_buf as *mut c_void));
    drop(store);
    if let Some(old) = old.filter(|p| !p.0.is_null()) {
        CFRelease(old.0 as *const c_void);
    }
    CB_STORED.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    FRAMES_DIRTY.store(true, std::sync::atomic::Ordering::Release);
}

/// Fire-and-forget completion block for stopCapture / updateConfiguration.
pub(crate) fn sc_completion_block() -> RcBlock<dyn Fn(*mut AnyObject)> {
    RcBlock::new(move |error: *mut AnyObject| {
        let Some(msg) = (unsafe { super::setup::objc_error_string(error) }) else { return };
        // Benign race conditions during rapid cycling — suppress
        if msg.contains("already stopped") || msg.contains("does not exist") { return; }
        eprintln!("[alt-tab/sc] completion error: {}", msg);
    })
}

/// Take the latest frames from all active streams. Returns (window_id, SendCVBuf) pairs.
/// Each frame is removed from the store (caller owns the reference).
pub(crate) fn sc_take_frames() -> HashMap<u32, SendCVBuf> {
    let mut store = FRAME_STORE.lock().unwrap();
    let Some(map) = store.as_mut() else {
        return HashMap::new();
    };
    let mut result = HashMap::with_capacity(map.len());
    for (&wid, sp) in map.iter_mut() {
        if sp.0.is_null() {
            continue;
        }
        result.insert(wid, SendCVBuf(sp.0));
        sp.0 = std::ptr::null_mut();
    }
    FRAMES_DIRTY.store(false, std::sync::atomic::Ordering::Relaxed);
    result
}

/// Check if any new frames arrived since last check. Resets the flag.
pub(crate) fn sc_has_new_frames() -> bool {
    FRAMES_DIRTY.swap(false, std::sync::atomic::Ordering::Acquire)
}

/// Return wids that have a non-null frame in FRAME_STORE (read-only, no drain).
pub(crate) fn sc_live_frame_wids() -> std::collections::HashSet<u32> {
    FRAME_STORE
        .lock()
        .ok()
        .and_then(|store| {
            store.as_ref().map(|m| {
                m.iter()
                    .filter(|(_, sp)| !sp.0.is_null())
                    .map(|(wid, _)| *wid)
                    .collect()
            })
        })
        .unwrap_or_default()
}

/// Read and reset callback diagnostic counters: (calls, stored, null_img).
pub(crate) fn sc_callback_stats() -> (u64, u64, u64) {
    let calls = CB_CALLS.swap(0, std::sync::atomic::Ordering::Relaxed);
    let stored = CB_STORED.swap(0, std::sync::atomic::Ordering::Relaxed);
    let null_img = CB_NULL_IMG.swap(0, std::sync::atomic::Ordering::Relaxed);
    (calls, stored, null_img)
}
