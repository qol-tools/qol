// ScreenCaptureKit GPU capture (macOS 14.0+)
// Raw ObjC bindings — no Swift runtime dependency.

mod buf;
mod callback;
mod prewarm;
mod setup;
mod stream;

use super::{CFRelease, CFRetain};

pub use buf::SendCVBuf;

pub(super) use callback::{sc_callback_stats, sc_has_new_frames, sc_live_frame_wids, sc_take_frames};
pub(super) use prewarm::{
    sc_clone_opener_surfaces, sc_heartbeat_snapshot, sc_prewarm_retain, sc_prewarm_wids,
    sc_take_prewarm_surfaces,
};
pub(super) use setup::sc_fetch_content;
pub(super) use stream::{
    sc_demote_stream, sc_promote_stream, sc_snapshot_window, sc_start_streams,
    sc_start_streams_with_content, sc_stop_streams, sc_streams_active,
};

use std::ffi::c_void;
use std::sync::atomic::{AtomicU8, Ordering};

/// Load ScreenCaptureKit framework at runtime (graceful fallback on older macOS).
fn ensure_sc_framework() -> bool {
    static STATE: AtomicU8 = AtomicU8::new(0);
    let v = STATE.load(Ordering::Relaxed);
    if v != 0 {
        return v == 1;
    }
    extern "C" {
        fn dlopen(path: *const i8, mode: i32) -> *mut c_void;
    }
    let handle = unsafe {
        dlopen(
            b"/System/Library/Frameworks/ScreenCaptureKit.framework/ScreenCaptureKit\0".as_ptr()
                as *const i8,
            1, // RTLD_LAZY
        )
    };
    let available =
        !handle.is_null() && objc2::runtime::AnyClass::get(c"SCStream").is_some();
    STATE.store(if available { 1 } else { 2 }, Ordering::Relaxed);
    available
}

pub(super) fn sc_available() -> bool {
    ensure_sc_framework()
}
