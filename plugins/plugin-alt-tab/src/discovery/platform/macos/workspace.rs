use super::events::AxEvent;
use super::observer::{register_app, AppObserver};
use super::process::is_regular_app;
use objc2::rc::Retained;
use objc2_app_kit::{
    NSRunningApplication, NSWorkspace, NSWorkspaceDidLaunchApplicationNotification,
    NSWorkspaceDidTerminateApplicationNotification,
};
use objc2_foundation::{NSNotification, NSNotificationName};
use std::collections::HashMap;
use std::ptr::NonNull;
use std::sync::mpsc::SyncSender;
use std::sync::{Arc, Mutex};

type ObserverToken = objc2::runtime::ProtocolObject<dyn objc2::runtime::NSObjectProtocol>;

type AppObserverMap = Arc<Mutex<HashMap<i32, AppObserver>>>;

/// Tracks `NSWorkspace` application lifecycle and maintains one [`AppObserver`]
/// per regular (dock-visible) process. Dropping the watcher unsubscribes all
/// notifications and releases every owned observer.
pub(crate) struct WorkspaceWatcher {
    observers: AppObserverMap,
    launch_token: Option<Retained<ObserverToken>>,
    terminate_token: Option<Retained<ObserverToken>>,
}

impl WorkspaceWatcher {
    pub(crate) fn start(tx: SyncSender<AxEvent>) -> Self {
        let observers: AppObserverMap = Arc::new(Mutex::new(HashMap::new()));
        register_initial_apps(&observers, &tx);
        let launch_token = subscribe_launch(&observers, tx.clone());
        let terminate_token = subscribe_terminate(&observers);
        Self {
            observers,
            launch_token,
            terminate_token,
        }
    }
}

impl Drop for WorkspaceWatcher {
    fn drop(&mut self) {
        let center = NSWorkspace::sharedWorkspace().notificationCenter();
        remove_observer(&center, self.launch_token.take());
        remove_observer(&center, self.terminate_token.take());
        if let Ok(mut map) = self.observers.lock() {
            map.clear();
        }
    }
}

fn remove_observer(
    center: &objc2_foundation::NSNotificationCenter,
    token: Option<Retained<ObserverToken>>,
) {
    let Some(token) = token else {
        return;
    };
    let any: &objc2::runtime::AnyObject = (*token).as_ref();
    unsafe { center.removeObserver(any) };
}

fn register_initial_apps(observers: &AppObserverMap, tx: &SyncSender<AxEvent>) {
    objc2::rc::autoreleasepool(|_pool| {
        let apps = NSWorkspace::sharedWorkspace().runningApplications();
        for app in apps.iter() {
            let pid = app.processIdentifier();
            maybe_insert_observer(observers, tx, pid);
        }
    });
}

fn subscribe_launch(
    observers: &AppObserverMap,
    tx: SyncSender<AxEvent>,
) -> Option<Retained<ObserverToken>> {
    let observers = observers.clone();
    let handler = move |pid: i32| {
        maybe_insert_observer(&observers, &tx, pid);
    };
    add_workspace_observer(
        unsafe { NSWorkspaceDidLaunchApplicationNotification },
        handler,
    )
}

fn subscribe_terminate(observers: &AppObserverMap) -> Option<Retained<ObserverToken>> {
    let observers = observers.clone();
    let handler = move |pid: i32| {
        if let Ok(mut map) = observers.lock() {
            map.remove(&pid);
        }
    };
    add_workspace_observer(
        unsafe { NSWorkspaceDidTerminateApplicationNotification },
        handler,
    )
}

fn add_workspace_observer(
    name: &'static NSNotificationName,
    handler: impl Fn(i32) + Send + Sync + 'static,
) -> Option<Retained<ObserverToken>> {
    let block = block2::RcBlock::new(move |note: NonNull<NSNotification>| {
        let Some(pid) = pid_from_notification(unsafe { note.as_ref() }) else {
            return;
        };
        handler(pid);
    });
    unsafe {
        let center = NSWorkspace::sharedWorkspace().notificationCenter();
        let token =
            center.addObserverForName_object_queue_usingBlock(Some(name), None, None, &block);
        Some(token)
    }
}

fn pid_from_notification(notification: &NSNotification) -> Option<i32> {
    let user_info = notification.userInfo()?;
    let key = unsafe { objc2_app_kit::NSWorkspaceApplicationKey };
    let value = user_info.objectForKey(key.as_ref())?;
    let app = value.downcast::<NSRunningApplication>().ok()?;
    Some(app.processIdentifier())
}

fn maybe_insert_observer(observers: &AppObserverMap, tx: &SyncSender<AxEvent>, pid: i32) {
    if pid <= 0 {
        return;
    }
    if !is_regular_app(pid) {
        return;
    }
    let Ok(mut map) = observers.lock() else {
        return;
    };
    if map.contains_key(&pid) {
        return;
    }
    let Some(observer) = register_app(pid, tx.clone()) else {
        return;
    };
    map.insert(observer.pid(), observer);
}
