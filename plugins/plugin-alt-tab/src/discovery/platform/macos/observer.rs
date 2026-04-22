use super::events::AxEvent;
use super::ffi::{cfstring_to_string, CFRelease, CFRetain};
use std::ffi::c_void;
use std::sync::mpsc::SyncSender;

const K_AX_ERROR_API_DISABLED: i32 = -25211;

type AXObserverRef = *const c_void;
type AXUIElementRef = *const c_void;
type CFRunLoopSourceRef = *const c_void;

type AXObserverCallback = unsafe extern "C" fn(
    observer: AXObserverRef,
    element: AXUIElementRef,
    notification: *const c_void,
    refcon: *mut c_void,
);

#[link(name = "ApplicationServices", kind = "framework")]
extern "C" {
    fn AXUIElementCreateApplication(pid: i32) -> AXUIElementRef;
    fn AXObserverCreate(
        pid: i32,
        callback: AXObserverCallback,
        out_observer: *mut AXObserverRef,
    ) -> i32;
    fn AXObserverAddNotification(
        observer: AXObserverRef,
        element: AXUIElementRef,
        notification: *const c_void,
        refcon: *mut c_void,
    ) -> i32;
    fn AXObserverRemoveNotification(
        observer: AXObserverRef,
        element: AXUIElementRef,
        notification: *const c_void,
    ) -> i32;
    fn AXObserverGetRunLoopSource(observer: AXObserverRef) -> CFRunLoopSourceRef;
}

#[link(name = "CoreFoundation", kind = "framework")]
extern "C" {
    fn CFRunLoopGetCurrent() -> *const c_void;
    fn CFRunLoopAddSource(run_loop: *const c_void, source: *const c_void, mode: *const c_void);
    static kCFRunLoopDefaultMode: *const c_void;
}

struct NotificationNames {
    application_activated: *const c_void,
    focused_window_changed: *const c_void,
    main_window_changed: *const c_void,
    window_created: *const c_void,
    ui_element_destroyed: *const c_void,
    application_hidden: *const c_void,
    application_shown: *const c_void,
    window_miniaturized: *const c_void,
    window_deminiaturized: *const c_void,
}

impl NotificationNames {
    fn new() -> Self {
        Self {
            application_activated: super::ffi::cfstr(b"AXApplicationActivated"),
            focused_window_changed: super::ffi::cfstr(b"AXFocusedWindowChanged"),
            main_window_changed: super::ffi::cfstr(b"AXMainWindowChanged"),
            window_created: super::ffi::cfstr(b"AXWindowCreated"),
            ui_element_destroyed: super::ffi::cfstr(b"AXUIElementDestroyed"),
            application_hidden: super::ffi::cfstr(b"AXApplicationHidden"),
            application_shown: super::ffi::cfstr(b"AXApplicationShown"),
            window_miniaturized: super::ffi::cfstr(b"AXWindowMiniaturized"),
            window_deminiaturized: super::ffi::cfstr(b"AXWindowDeminiaturized"),
        }
    }

    fn all(&self) -> [*const c_void; 9] {
        [
            self.application_activated,
            self.focused_window_changed,
            self.main_window_changed,
            self.window_created,
            self.ui_element_destroyed,
            self.application_hidden,
            self.application_shown,
            self.window_miniaturized,
            self.window_deminiaturized,
        ]
    }
}

impl Drop for NotificationNames {
    fn drop(&mut self) {
        unsafe {
            for name in self.all() {
                CFRelease(name);
            }
        }
    }
}

/// Owning handle for the AXObserver subscriptions of a single application pid.
/// Dropping the handle removes the notifications and releases the observer.
pub(crate) struct AppObserver {
    observer: AXObserverRef,
    element: AXUIElementRef,
    pid: i32,
    subscribed: Vec<*const c_void>,
    sender: *mut SyncSender<AxEvent>,
}

// The raw pointers are AX-framework-owned objects. Access is serialized through the
// CFRunLoop running them; we only send the handle between threads during setup/teardown.
unsafe impl Send for AppObserver {}

impl AppObserver {
    pub(crate) fn pid(&self) -> i32 {
        self.pid
    }
}

impl Drop for AppObserver {
    fn drop(&mut self) {
        unsafe {
            for name in &self.subscribed {
                AXObserverRemoveNotification(self.observer, self.element, *name);
                CFRelease(*name);
            }
            if !self.element.is_null() {
                CFRelease(self.element);
            }
            if !self.observer.is_null() {
                CFRelease(self.observer);
            }
            if !self.sender.is_null() {
                drop(Box::from_raw(self.sender));
            }
        }
    }
}

/// Build an AXObserver for `pid` and subscribe to the nine window/application
/// notifications the picker cares about. Returns `None` if accessibility is
/// unavailable (`kAXErrorAPIDisabled`) or the pid rejects observation.
pub(crate) fn register_app(pid: i32, tx: SyncSender<AxEvent>) -> Option<AppObserver> {
    let sender = Box::into_raw(Box::new(tx));
    let observer = unsafe { create_observer(pid, sender)? };
    let element = unsafe { AXUIElementCreateApplication(pid) };
    if element.is_null() {
        unsafe {
            CFRelease(observer);
            drop(Box::from_raw(sender));
        }
        return None;
    }
    let names = NotificationNames::new();
    let subscribed = unsafe { subscribe_all(observer, element, &names, sender) };
    if subscribed.is_empty() {
        unsafe {
            CFRelease(element);
            CFRelease(observer);
            drop(Box::from_raw(sender));
        }
        return None;
    }
    unsafe { attach_run_loop_source(observer) };
    Some(AppObserver {
        observer,
        element,
        pid,
        subscribed,
        sender,
    })
}

unsafe fn create_observer(pid: i32, sender: *mut SyncSender<AxEvent>) -> Option<AXObserverRef> {
    let mut observer: AXObserverRef = std::ptr::null();
    let err = AXObserverCreate(pid, ax_notification_callback, &mut observer);
    if err == K_AX_ERROR_API_DISABLED {
        drop(Box::from_raw(sender));
        return None;
    }
    if err != 0 || observer.is_null() {
        drop(Box::from_raw(sender));
        return None;
    }
    Some(observer)
}

unsafe fn subscribe_all(
    observer: AXObserverRef,
    element: AXUIElementRef,
    names: &NotificationNames,
    sender: *mut SyncSender<AxEvent>,
) -> Vec<*const c_void> {
    let mut subscribed = Vec::with_capacity(9);
    for name in names.all() {
        let retained = CFRetain(name);
        let err = AXObserverAddNotification(observer, element, retained, sender.cast::<c_void>());
        if err != 0 {
            CFRelease(retained);
            continue;
        }
        subscribed.push(retained);
    }
    subscribed
}

unsafe fn attach_run_loop_source(observer: AXObserverRef) {
    let source = AXObserverGetRunLoopSource(observer);
    if source.is_null() {
        return;
    }
    let run_loop = CFRunLoopGetCurrent();
    if run_loop.is_null() {
        return;
    }
    CFRunLoopAddSource(run_loop, source, kCFRunLoopDefaultMode);
}

unsafe extern "C" fn ax_notification_callback(
    _observer: AXObserverRef,
    element: AXUIElementRef,
    notification: *const c_void,
    refcon: *mut c_void,
) {
    if refcon.is_null() || notification.is_null() {
        return;
    }
    let sender = &*(refcon as *const SyncSender<AxEvent>);
    let Some(name) = cfstring_to_string(notification) else {
        return;
    };
    let Some(pid) = pid_for_element(element) else {
        return;
    };
    let Some(event) = classify(&name, pid) else {
        return;
    };
    let _ = sender.try_send(event);
}

unsafe fn pid_for_element(element: AXUIElementRef) -> Option<i32> {
    if element.is_null() {
        return None;
    }
    #[link(name = "ApplicationServices", kind = "framework")]
    extern "C" {
        fn AXUIElementGetPid(element: AXUIElementRef, pid: *mut i32) -> i32;
    }
    let mut pid: i32 = 0;
    let err = AXUIElementGetPid(element, &mut pid);
    if err != 0 {
        return None;
    }
    Some(pid)
}

fn classify(name: &str, pid: i32) -> Option<AxEvent> {
    match name {
        "AXApplicationActivated" => Some(AxEvent::ApplicationActivated),
        "AXFocusedWindowChanged" => Some(AxEvent::FocusedWindowChanged),
        "AXMainWindowChanged" => Some(AxEvent::MainWindowChanged),
        "AXWindowCreated" => Some(AxEvent::WindowCreated),
        "AXUIElementDestroyed" => Some(AxEvent::WindowDestroyed),
        "AXApplicationHidden" => Some(AxEvent::ApplicationHidden { pid }),
        "AXApplicationShown" => Some(AxEvent::ApplicationShown { pid }),
        "AXWindowMiniaturized" => Some(AxEvent::WindowMiniaturized { pid }),
        "AXWindowDeminiaturized" => Some(AxEvent::WindowDeminiaturized { pid }),
        _ => None,
    }
}
