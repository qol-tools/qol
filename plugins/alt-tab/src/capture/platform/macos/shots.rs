use crate::discovery::platform::macos::ffi::{CFRelease, CFRetain};
use objc2::msg_send;
use objc2::rc::autoreleasepool;
use objc2::runtime::{AnyClass, AnyObject};
use std::collections::HashMap;
use std::ffi::c_void;
use std::sync::atomic::{AtomicU8, Ordering};
use std::sync::{mpsc, Arc, Mutex};
use std::time::{Duration, Instant};

pub(crate) type LiveFrame = core_video::pixel_buffer::CVPixelBuffer;

pub(crate) fn live_frame_element(frame: &LiveFrame, width: f32, height: f32) -> gpui::AnyElement {
    use gpui::{px, surface, IntoElement, ObjectFit, Styled};
    surface(frame.clone())
        .w(px(width))
        .h(px(height))
        .object_fit(ObjectFit::Cover)
        .into_any_element()
}

pub(crate) type ShotReply = (u32, Option<SendCVBuf>);

pub(crate) const PIXEL_FORMAT_420F: u32 = 0x3432_3066;
const FETCH_TIMEOUT: Duration = Duration::from_secs(3);

#[repr(C)]
#[derive(Copy, Clone)]
struct CMTime {
    value: i64,
    timescale: i32,
    flags: u32,
    epoch: i64,
}

unsafe impl objc2::Encode for CMTime {
    const ENCODING: objc2::Encoding = objc2::Encoding::Struct(
        "?",
        &[
            objc2::Encoding::LongLong,
            objc2::Encoding::Int,
            objc2::Encoding::UInt,
            objc2::Encoding::LongLong,
        ],
    );
}

unsafe impl objc2::RefEncode for CMTime {
    const ENCODING_REF: objc2::Encoding =
        objc2::Encoding::Pointer(&<Self as objc2::Encode>::ENCODING);
}

const CM_TIME_60FPS: CMTime = CMTime {
    value: 1,
    timescale: 60,
    flags: 1,
    epoch: 0,
};

#[link(name = "CoreMedia", kind = "framework")]
extern "C" {
    fn CMSampleBufferGetImageBuffer(sbuf: *mut AnyObject) -> *mut c_void;
}

#[link(name = "CoreVideo", kind = "framework")]
extern "C" {
    fn CVPixelBufferGetPixelFormatType(buf: *const c_void) -> u32;
}

struct SendPtr(*mut c_void);
unsafe impl Send for SendPtr {}

pub(crate) struct SendCVBuf(*mut c_void);
unsafe impl Send for SendCVBuf {}

impl Clone for SendCVBuf {
    fn clone(&self) -> Self {
        if !self.0.is_null() {
            unsafe { CFRetain(self.0 as *const c_void) };
        }
        SendCVBuf(self.0)
    }
}

impl SendCVBuf {
    pub(crate) fn pixel_format(&self) -> u32 {
        unsafe { CVPixelBufferGetPixelFormatType(self.0 as *const c_void) }
    }

    pub(crate) fn into_live_frame(self) -> Option<LiveFrame> {
        use core_foundation::base::TCFType;
        let buf = unsafe {
            LiveFrame::wrap_under_create_rule(self.0 as core_video::pixel_buffer::CVPixelBufferRef)
        };
        std::mem::forget(self);
        Some(buf)
    }
}

impl Drop for SendCVBuf {
    fn drop(&mut self) {
        if !self.0.is_null() {
            unsafe { CFRelease(self.0 as *const c_void) };
        }
    }
}

pub(crate) struct ShotsSession {
    content: SendPtr,
    windows: HashMap<u32, SendPtr>,
}

unsafe impl Send for ShotsSession {}
unsafe impl Sync for ShotsSession {}

impl Drop for ShotsSession {
    fn drop(&mut self) {
        for window in self.windows.values() {
            if !window.0.is_null() {
                unsafe { CFRelease(window.0 as *const c_void) };
            }
        }
        if !self.content.0.is_null() {
            unsafe { CFRelease(self.content.0 as *const c_void) };
        }
    }
}

fn sc_framework_ready() -> bool {
    static STATE: AtomicU8 = AtomicU8::new(0);
    let cached = STATE.load(Ordering::Relaxed);
    if cached != 0 {
        return cached == 1;
    }
    extern "C" {
        fn dlopen(path: *const i8, mode: i32) -> *mut c_void;
    }
    let handle = unsafe {
        dlopen(
            c"/System/Library/Frameworks/ScreenCaptureKit.framework/ScreenCaptureKit".as_ptr(),
            1,
        )
    };
    let available = !handle.is_null() && AnyClass::get(c"SCScreenshotManager").is_some();
    STATE.store(if available { 1 } else { 2 }, Ordering::Relaxed);
    available
}

pub(crate) fn live_shots_available() -> bool {
    sc_framework_ready()
}

static WARM_SESSION: Mutex<Option<(Instant, Arc<ShotsSession>)>> = Mutex::new(None);
const WARM_SESSION_TTL: Duration = Duration::from_secs(5);

pub(crate) fn cached_shots_session(required: &[u32]) -> Option<Arc<ShotsSession>> {
    let slot = WARM_SESSION.lock().ok()?;
    let (fetched, session) = slot.as_ref()?;
    let covers = required.iter().all(|wid| session.windows.contains_key(wid));
    if fetched.elapsed() < WARM_SESSION_TTL && covers {
        return Some(session.clone());
    }
    None
}

pub(crate) fn warm_shots_session(required: &[u32]) -> Option<Arc<ShotsSession>> {
    if let Some(session) = cached_shots_session(required) {
        return Some(session);
    }
    let session = Arc::new(fetch_shots_session()?);
    *WARM_SESSION.lock().ok()? = Some((Instant::now(), session.clone()));
    Some(session)
}

pub(crate) fn fetch_shots_session() -> Option<ShotsSession> {
    if !sc_framework_ready() {
        return None;
    }
    let (tx, rx) = mpsc::sync_channel(1);
    let block = block2::RcBlock::new(move |content: *mut AnyObject, _error: *mut AnyObject| {
        let ptr = content as *mut c_void;
        if !ptr.is_null() {
            unsafe { CFRetain(ptr as *const c_void) };
        }
        if tx.send(SendPtr(ptr)).is_err() && !ptr.is_null() {
            unsafe { CFRelease(ptr as *const c_void) };
        }
    });
    unsafe {
        let cls = AnyClass::get(c"SCShareableContent")?;
        let _: () = msg_send![
            cls,
            getShareableContentExcludingDesktopWindows: false,
            onScreenWindowsOnly: false,
            completionHandler: &*block,
        ];
    }
    let content = match rx.recv_timeout(FETCH_TIMEOUT) {
        Ok(ptr) if !ptr.0.is_null() => ptr,
        _ => {
            qol_runtime::probe!("PREVIEW_LIVE", "source=shots outcome=fetch_failed");
            return None;
        }
    };
    let windows = build_window_map(content.0);
    Some(ShotsSession { content, windows })
}

fn build_window_map(content: *mut c_void) -> HashMap<u32, SendPtr> {
    let content = content as *const AnyObject;
    let mut map = HashMap::new();
    autoreleasepool(|_| unsafe {
        let windows: *const AnyObject = msg_send![content, windows];
        let count: usize = msg_send![windows, count];
        for i in 0..count {
            let win: *mut AnyObject = msg_send![windows, objectAtIndex: i];
            let wid: u32 = msg_send![win, windowID];
            CFRetain(win as *const c_void);
            map.insert(wid, SendPtr(win as *mut c_void));
        }
    });
    map
}

impl ShotsSession {
    pub(crate) fn request_capture(
        &self,
        wid: u32,
        max_w: usize,
        max_h: usize,
        reply: &mpsc::Sender<ShotReply>,
    ) -> bool {
        let Some(sc_window) = self.windows.get(&wid) else {
            return false;
        };
        let sc_window = sc_window.0 as *mut AnyObject;
        let reply = reply.clone();
        let block = block2::RcBlock::new(move |sample: *mut AnyObject, error: *mut AnyObject| {
            let buf = if sample.is_null() {
                std::ptr::null_mut()
            } else {
                unsafe { CMSampleBufferGetImageBuffer(sample) }
            };
            if buf.is_null() {
                qol_runtime::probe!(
                    "PREVIEW_LIVE",
                    "source=shots outcome=shot_failed wid={} err={}",
                    wid,
                    unsafe { error_description(error) }.unwrap_or_default()
                );
                let _ = reply.send((wid, None));
                return;
            }
            unsafe { CFRetain(buf as *const c_void) };
            let _ = reply.send((wid, Some(SendCVBuf(buf))));
        });
        autoreleasepool(|_| unsafe {
            let filter_cls = AnyClass::get(c"SCContentFilter")?;
            let filter: *mut AnyObject = msg_send![filter_cls, alloc];
            let filter: *mut AnyObject =
                msg_send![filter, initWithDesktopIndependentWindow: sc_window];
            if filter.is_null() {
                return None;
            }
            let config = build_shot_config(max_w, max_h)?;
            let manager = AnyClass::get(c"SCScreenshotManager")?;
            let _: () = msg_send![
                manager,
                captureSampleBufferWithFilter: filter,
                configuration: config,
                completionHandler: &*block,
            ];
            CFRelease(filter as *const c_void);
            CFRelease(config as *const c_void);
            Some(())
        })
        .is_some()
    }
}

unsafe fn build_shot_config(max_w: usize, max_h: usize) -> Option<*mut AnyObject> {
    let cls = AnyClass::get(c"SCStreamConfiguration")?;
    let config: *mut AnyObject = msg_send![cls, new];
    if config.is_null() {
        return None;
    }
    let _: () = msg_send![config, setWidth: max_w];
    let _: () = msg_send![config, setHeight: max_h];
    let _: () = msg_send![config, setScalesToFit: true];
    let _: () = msg_send![config, setPixelFormat: PIXEL_FORMAT_420F];
    let _: () = msg_send![config, setShowsCursor: false];
    let _: () = msg_send![config, setMinimumFrameInterval: CM_TIME_60FPS];
    let _: () = msg_send![config, setQueueDepth: 3usize];
    Some(config)
}

unsafe fn error_description(error: *mut AnyObject) -> Option<String> {
    if error.is_null() {
        return None;
    }
    let description: *const AnyObject = msg_send![error, localizedDescription];
    if description.is_null() {
        return None;
    }
    let cstr: *const i8 = msg_send![description, UTF8String];
    if cstr.is_null() {
        return None;
    }
    Some(
        std::ffi::CStr::from_ptr(cstr)
            .to_string_lossy()
            .into_owned(),
    )
}
