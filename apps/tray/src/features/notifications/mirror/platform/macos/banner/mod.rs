mod ax;
mod claims;
mod tree;

use std::collections::VecDeque;
use std::ffi::c_void;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Mutex;
use std::time::Duration;

use core_foundation::base::CFTypeRef;
use core_foundation::runloop::{kCFRunLoopDefaultMode, CFRunLoop};
use core_foundation::string::CFStringRef;

use super::toast::{self, Mirrored};

pub(super) use claims::claimed;

const NOTIFICATION_CENTER: &str = "com.apple.notificationcenterui";
const RECHECK: Duration = Duration::from_secs(5);
const OFFSCREEN: f64 = -30_000.0;
const SEEN_CAPACITY: usize = 32;

static NEXT_GENERATION: AtomicU64 = AtomicU64::new(1);
static WATCH: Mutex<Option<Watch>> = Mutex::new(None);

static SEEN: Mutex<VecDeque<String>> = Mutex::new(VecDeque::new());

struct Watch {
    generation: u64,
    run_loop: Option<CFRunLoop>,
}

pub(super) fn sync(enabled: bool) {
    let mut watch = lock();
    if !enabled {
        if let Some(stopped) = watch.take() {
            if let Some(run_loop) = stopped.run_loop {
                run_loop.stop();
            }
            log::info!("notification banners: stopped hiding macOS banners");
        }
        return;
    }
    if watch.is_some() {
        return;
    }
    let generation = NEXT_GENERATION.fetch_add(1, Ordering::Relaxed);
    let spawned = std::thread::Builder::new()
        .name("qol-notification-banners".to_string())
        .spawn(move || run(generation));
    match spawned {
        Ok(_) => {
            *watch = Some(Watch {
                generation,
                run_loop: None,
            })
        }
        Err(error) => log::warn!("notification banners: not started: {error}"),
    }
}

fn lock() -> std::sync::MutexGuard<'static, Option<Watch>> {
    WATCH
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

fn is_current(generation: u64) -> bool {
    lock()
        .as_ref()
        .is_some_and(|watch| watch.generation == generation)
}

fn register(generation: u64) -> bool {
    let mut watch = lock();
    match watch.as_mut() {
        Some(watch) if watch.generation == generation => {
            watch.run_loop = Some(CFRunLoop::get_current());
            true
        }
        _ => false,
    }
}

fn run(generation: u64) {
    if !register(generation) {
        return;
    }
    let mut observer: Option<ax::Observer> = None;
    let mut failed_pid = None;
    while is_current(generation) {
        let pid = notification_center_pid();
        if observer.as_ref().map(|observer| observer.pid) != pid {
            observer = None;
            if let Some(pid) = pid {
                match ax::Observer::attach(pid, on_created) {
                    Ok(attached) => {
                        log::info!(
                            "notification banners: watching Notification Center (pid {pid})"
                        );
                        failed_pid = None;
                        observer = Some(attached);
                    }
                    Err(error) => {
                        if failed_pid != Some(pid) {
                            log::warn!(
                                "notification banners: cannot watch Notification Center: {error:#}"
                            );
                        }
                        failed_pid = Some(pid);
                    }
                }
            }
        }
        if observer.is_some() {
            CFRunLoop::run_in_mode(unsafe { kCFRunLoopDefaultMode }, RECHECK, false);
        } else {
            std::thread::sleep(RECHECK);
        }
    }
}

fn notification_center_pid() -> Option<i32> {
    use objc2_app_kit::NSRunningApplication;
    use objc2_foundation::NSString;

    objc2::rc::autoreleasepool(|_pool| {
        NSRunningApplication::runningApplicationsWithBundleIdentifier(&NSString::from_str(
            NOTIFICATION_CENTER,
        ))
        .iter()
        .map(|app| app.processIdentifier())
        .find(|pid| *pid > 0)
    })
}

extern "C" fn on_created(
    _observer: CFTypeRef,
    element: CFTypeRef,
    _notification: CFStringRef,
    _refcon: *mut c_void,
) {
    let Some(element) = ax::callback_element(element) else {
        return;
    };
    let banners = tree::banners(&tree::snapshot(&element));
    if banners.is_empty() {
        return;
    }
    match element.window() {
        Some(window) => {
            if let Err(error) = window.move_to(OFFSCREEN, OFFSCREEN) {
                log::warn!("notification banners: cannot hide a macOS banner: {error:#}");
            }
        }
        None => log::warn!("notification banners: banner has no window to hide"),
    }
    for banner in banners {
        if first_sighting(&banner.id) {
            claims::record(&banner.title, &banner.body);
            toast::show(
                &banner.app,
                &Mirrored {
                    app_id: banner.app.clone(),
                    title: banner.title,
                    subtitle: banner.subtitle,
                    body: banner.body,
                },
            );
        }
    }
}

fn first_sighting(id: &str) -> bool {
    let mut seen = SEEN.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
    if seen.iter().any(|known| known == id) {
        return false;
    }
    if seen.len() == SEEN_CAPACITY {
        seen.pop_front();
    }
    seen.push_back(id.to_string());
    true
}
