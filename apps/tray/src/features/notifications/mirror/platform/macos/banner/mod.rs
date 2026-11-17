mod ax;
mod claims;
mod tree;

use std::collections::VecDeque;
use std::ffi::c_void;
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicU8, Ordering};
use std::sync::Mutex;
use std::time::Duration;

use anyhow::{bail, Result};
use core_foundation::base::CFTypeRef;
use core_foundation::runloop::{kCFRunLoopDefaultMode, CFRunLoop};
use core_foundation::string::CFStringRef;

use super::retained::{self, AppRef, Entry};
use super::toast::{self, Mirrored};
use crate::features::notifications::takeover::{Health, Permission};

pub(super) use claims::claimed;

#[derive(Clone, Debug, PartialEq, Eq)]
pub(in super::super) struct BannerMatch {
    pub(in super::super) pid: i32,
    pub(in super::super) banner_id: String,
}

pub(in super::super) fn press(matcher: &BannerMatch) -> Result<()> {
    let app = ax::Element::application(matcher.pid)
        .ok_or_else(|| anyhow::anyhow!("Notification Center AX element unavailable"))?;
    for window in app.windows() {
        for live in tree::live_banners(&window) {
            if live.banner.id == matcher.banner_id {
                return live.element.press_descendant();
            }
        }
    }
    bail!("banner no longer available")
}

const NOTIFICATION_CENTER: &str = "com.apple.notificationcenterui";
const RECHECK: Duration = Duration::from_secs(5);
const OFFSCREEN: f64 = -30_000.0;
const SEEN_CAPACITY: usize = 32;

const NOT_LOOKED: u8 = 0;
const WATCHING: u8 = 1;
const CANNOT_WATCH: u8 = 2;
const NO_NOTIFICATION_CENTER: u8 = 3;
const CANNOT_WATCH_REASON: &str = "Notification Center cannot be watched";
const NO_NOTIFICATION_CENTER_REASON: &str = "Notification Center is not running";
const CANNOT_HIDE_REASON: &str = "a macOS banner could not be moved off screen";

static WATCH_STATE: AtomicU8 = AtomicU8::new(NOT_LOOKED);
static HIDE_FAILED: AtomicBool = AtomicBool::new(false);
static NEXT_GENERATION: AtomicU64 = AtomicU64::new(1);
static WATCH: Mutex<Option<Watch>> = Mutex::new(None);

static SEEN: Mutex<VecDeque<String>> = Mutex::new(VecDeque::new());

struct Watch {
    generation: u64,
    run_loop: Option<CFRunLoop>,
}

pub(super) fn health() -> Health {
    let trusted =
        qol_platform::permission_status(qol_platform::Permission::InputCapture).is_allowed();
    watch_health(
        WATCH_STATE.load(Ordering::Relaxed),
        HIDE_FAILED.load(Ordering::Relaxed),
        trusted,
    )
}

fn watch_health(state: u8, hide_failed: bool, trusted: bool) -> Health {
    match state {
        _ if !trusted => Health::NeedsPermission(Permission::Accessibility),
        WATCHING if hide_failed => Health::Failed(CANNOT_HIDE_REASON.to_string()),
        WATCHING => Health::Active,
        CANNOT_WATCH => Health::Failed(CANNOT_WATCH_REASON.to_string()),
        NO_NOTIFICATION_CENTER => Health::Failed(NO_NOTIFICATION_CENTER_REASON.to_string()),
        _ => Health::Starting,
    }
}

fn note_watch(generation: u64, state: u8) {
    if is_current(generation) {
        WATCH_STATE.store(state, Ordering::Relaxed);
    }
}

pub(super) fn sync(enabled: bool) {
    let mut watch = lock();
    if !enabled {
        WATCH_STATE.store(NOT_LOOKED, Ordering::Relaxed);
        HIDE_FAILED.store(false, Ordering::Relaxed);
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
        if pid.is_none() {
            note_watch(generation, NO_NOTIFICATION_CENTER);
        }
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
                        note_watch(generation, WATCHING);
                    }
                    Err(error) => {
                        if failed_pid != Some(pid) {
                            log::warn!(
                                "notification banners: cannot watch Notification Center: {error:#}"
                            );
                        }
                        failed_pid = Some(pid);
                        note_watch(generation, CANNOT_WATCH);
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
            let moved = window.move_to(OFFSCREEN, OFFSCREEN);
            HIDE_FAILED.store(moved.is_err(), Ordering::Relaxed);
            if let Err(error) = moved {
                log::warn!("notification banners: cannot hide a macOS banner: {error:#}");
            }
        }
        None => log::warn!("notification banners: banner has no window to hide"),
    }
    let pid = notification_center_pid();
    for banner in banners {
        if first_sighting(&banner.id) {
            claims::record(&banner.title, &banner.body);
            let retained_id = retained::next_id();
            retained::insert(
                &retained_id,
                Entry {
                    app: AppRef::DisplayName(banner.app.clone()),
                    matcher: pid.map(|pid| BannerMatch {
                        pid,
                        banner_id: banner.id.clone(),
                    }),
                },
            );
            toast::show(
                &banner.app,
                &Mirrored {
                    app_id: banner.app.clone(),
                    title: banner.title,
                    subtitle: banner.subtitle,
                    body: banner.body,
                },
                Some(&retained_id),
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_watch_says_what_stops_it_from_hiding_banners() {
        let cases = [
            (WATCHING, false, true, Health::Active),
            (
                WATCHING,
                false,
                false,
                Health::NeedsPermission(Permission::Accessibility),
            ),
            (
                WATCHING,
                true,
                true,
                Health::Failed(CANNOT_HIDE_REASON.to_string()),
            ),
            (NOT_LOOKED, false, true, Health::Starting),
            (
                NOT_LOOKED,
                false,
                false,
                Health::NeedsPermission(Permission::Accessibility),
            ),
            (
                CANNOT_WATCH,
                false,
                false,
                Health::NeedsPermission(Permission::Accessibility),
            ),
            (
                CANNOT_WATCH,
                false,
                true,
                Health::Failed(CANNOT_WATCH_REASON.to_string()),
            ),
            (
                NO_NOTIFICATION_CENTER,
                false,
                true,
                Health::Failed(NO_NOTIFICATION_CENTER_REASON.to_string()),
            ),
        ];
        for (state, hide_failed, trusted, health) in cases {
            assert_eq!(watch_health(state, hide_failed, trusted), health, "{state}");
        }
    }
}
