use crate::discovery::platform::macos::ffi::{self, CFRelease, CFRetain};
use futures::channel::mpsc::{unbounded, UnboundedReceiver, UnboundedSender};
use std::cell::RefCell;
use std::collections::HashMap;
use std::ffi::c_void;
use std::sync::OnceLock;

type AxObserverCallback = extern "C" fn(*const c_void, *const c_void, *const c_void, *mut c_void);

#[link(name = "ApplicationServices", kind = "framework")]
extern "C" {
    fn AXObserverCreate(pid: i32, callback: AxObserverCallback, out: *mut *const c_void) -> i32;
    fn AXObserverAddNotification(
        observer: *const c_void,
        element: *const c_void,
        notification: *const c_void,
        refcon: *mut c_void,
    ) -> i32;
    fn AXObserverRemoveNotification(
        observer: *const c_void,
        element: *const c_void,
        notification: *const c_void,
    ) -> i32;
    fn AXObserverGetRunLoopSource(observer: *const c_void) -> *const c_void;
}

#[link(name = "CoreFoundation", kind = "framework")]
extern "C" {
    static kCFRunLoopCommonModes: *const c_void;
    fn CFRunLoopGetMain() -> *const c_void;
    fn CFRunLoopAddSource(rl: *const c_void, source: *const c_void, mode: *const c_void);
    fn CFRunLoopRemoveSource(rl: *const c_void, source: *const c_void, mode: *const c_void);
}

const DESTROYED: &[u8] = b"AXUIElementDestroyed";

static DESTROYED_TX: OnceLock<UnboundedSender<u32>> = OnceLock::new();

struct Watch {
    observer: *const c_void,
    window: *const c_void,
}

thread_local! {
    static WATCHES: RefCell<HashMap<u32, Watch>> = RefCell::new(HashMap::new());
}

pub fn destroyed_windows() -> Option<UnboundedReceiver<u32>> {
    let (tx, rx) = unbounded();
    DESTROYED_TX.set(tx).ok()?;
    Some(rx)
}

pub(super) unsafe fn watch(pid: i32, window: *const c_void, window_id: u32) -> bool {
    if DESTROYED_TX.get().is_none() {
        return false;
    }
    let mut observer: *const c_void = std::ptr::null();
    if AXObserverCreate(pid, on_destroyed, &mut observer) != 0 || observer.is_null() {
        return false;
    }
    let name = ffi::cfstr(DESTROYED);
    let added =
        AXObserverAddNotification(observer, window, name, window_id as usize as *mut c_void);
    CFRelease(name);
    if added != 0 {
        CFRelease(observer);
        return false;
    }
    CFRunLoopAddSource(
        CFRunLoopGetMain(),
        AXObserverGetRunLoopSource(observer),
        kCFRunLoopCommonModes,
    );
    let watch = Watch {
        observer,
        window: CFRetain(window),
    };
    if let Some(previous) = WATCHES.with(|w| w.borrow_mut().insert(window_id, watch)) {
        stop(previous);
    }
    true
}

extern "C" fn on_destroyed(
    _observer: *const c_void,
    _element: *const c_void,
    _notification: *const c_void,
    refcon: *mut c_void,
) {
    let window_id = refcon as usize as u32;
    if let Some(watch) = WATCHES.with(|w| w.borrow_mut().remove(&window_id)) {
        unsafe { stop(watch) };
    }
    qol_runtime::probe!("CLOSE_WINDOW", "wid={window_id} result=destroyed");
    if let Some(tx) = DESTROYED_TX.get() {
        let _ = tx.unbounded_send(window_id);
    }
}

unsafe fn stop(watch: Watch) {
    let name = ffi::cfstr(DESTROYED);
    AXObserverRemoveNotification(watch.observer, watch.window, name);
    CFRelease(name);
    CFRunLoopRemoveSource(
        CFRunLoopGetMain(),
        AXObserverGetRunLoopSource(watch.observer),
        kCFRunLoopCommonModes,
    );
    CFRelease(watch.window);
    CFRelease(watch.observer);
}
