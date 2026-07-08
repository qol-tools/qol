use anyhow::Result;
use gpui::{Bounds, Pixels};
use qol_gpui::monitor::{ActiveMonitor, MonitorTracker};
use qol_gpui::platform::{ghost_window_decorations, ghost_window_kind};
use qol_gpui::window::centered_window_placement;
use std::ffi::c_void;
use std::sync::mpsc;
use std::time::Instant;

use crate::space::CaptureKind;
use crate::{Monitor, Rect};

use super::display::{active_display_bounds, rect_intersection};

type CGEventRef = *const c_void;
const CG_EVENT_SOURCE_STATE_HID_SYSTEM: i32 = 1;
const CG_MOUSE_BUTTON_LEFT: i32 = 0;

#[repr(C)]
#[derive(Clone, Copy)]
struct CGPoint {
    x: f64,
    y: f64,
}

#[link(name = "CoreGraphics", kind = "framework")]
extern "C" {
    fn CGEventCreate(source: *const c_void) -> CGEventRef;
    fn CGEventGetLocation(event: CGEventRef) -> CGPoint;
    fn CGEventSourceButtonState(state_id: i32, button: i32) -> bool;
}

#[link(name = "CoreFoundation", kind = "framework")]
extern "C" {
    fn CFRelease(cf: *const c_void);
}

pub fn select_region(kind: CaptureKind) -> Result<Option<Rect>> {
    crate::region_selector::select_region_blocking_with(move |tx, cx| {
        let tracker = MonitorTracker::start(cx);
        let monitor = tracker.snapshot_monitor();
        let monitors = selector_monitors(&tracker);
        qol_runtime::probe!(
            "SHOT_SELECT_PLATFORM",
            "mode=blocking monitor={} monitors={}",
            monitor.is_some(),
            monitors.len()
        );
        open_region_selector_with_sender(tx, true, kind, monitor, monitors, cx);
    })
}

pub fn select_region_in_app(
    cx: &mut gpui::App,
    kind: CaptureKind,
    monitor: Option<ActiveMonitor>,
    monitors: Vec<ActiveMonitor>,
) -> Option<mpsc::Receiver<Option<Rect>>> {
    qol_runtime::probe!(
        "SHOT_SELECT_PLATFORM",
        "mode=in-app monitor={} monitors={}",
        monitor.is_some(),
        monitors.len()
    );
    Some(open_region_selector(cx, kind, monitor, monitors))
}

fn open_region_selector(
    cx: &mut gpui::App,
    kind: CaptureKind,
    monitor: Option<ActiveMonitor>,
    monitors: Vec<ActiveMonitor>,
) -> mpsc::Receiver<Option<Rect>> {
    let (tx, rx) = mpsc::channel();
    open_region_selector_with_sender(tx, false, kind, monitor, monitors, cx);
    rx
}

fn open_region_selector_with_sender(
    tx: mpsc::Sender<Option<Rect>>,
    quit_on_finish: bool,
    kind: CaptureKind,
    monitor: Option<ActiveMonitor>,
    monitors: Vec<ActiveMonitor>,
    cx: &mut gpui::App,
) {
    let selectors = selector_windows(monitor.as_ref(), monitors, cx);
    let titles = selectors
        .iter()
        .map(|selector| selector.title().to_string())
        .collect::<Vec<_>>();
    qol_runtime::probe!(
        "SHOT_SELECT_PLATFORM",
        "mode=open selectors={} quit_on_finish={quit_on_finish}",
        titles.len()
    );
    if crate::region_selector::open_all(tx, quit_on_finish, selectors, kind, cx) {
        for title in titles {
            configure_selector_window(title, cx);
        }
        cx.activate(true);
    }
}

fn selector_windows(
    monitor: Option<&ActiveMonitor>,
    monitors: Vec<ActiveMonitor>,
    cx: &gpui::App,
) -> Vec<crate::region_selector::SelectorWindow> {
    let active_bounds = monitor.map(|monitor| monitor.bounds());
    let map_rect = selector_rect_mapper();
    let global_pointer: Option<crate::region_selector::GlobalPointerSource> =
        Some(std::rc::Rc::new(MacPointerSource));
    let monitors = if monitors.is_empty() {
        monitor.cloned().into_iter().collect()
    } else {
        monitors
    };
    if monitors.is_empty() {
        return vec![selector_window(
            selector_fallback_bounds(),
            active_bounds,
            None,
            true,
            map_rect,
            global_pointer,
        )];
    }
    monitors
        .into_iter()
        .map(|monitor| {
            let bounds = monitor.bounds();
            let placement = centered_window_placement(Some(&monitor), bounds.size, cx);
            selector_window(
                bounds,
                active_bounds,
                placement.display_id,
                selector_focus(bounds, active_bounds),
                map_rect.clone(),
                global_pointer.clone(),
            )
        })
        .collect()
}

fn selector_window(
    bounds: Bounds<Pixels>,
    active_bounds: Option<Bounds<Pixels>>,
    display_id: Option<gpui::DisplayId>,
    focus: bool,
    map_rect: crate::region_selector::RectMapper,
    global_pointer: Option<crate::region_selector::GlobalPointerSource>,
) -> crate::region_selector::SelectorWindow {
    crate::region_selector::SelectorWindow::new(
        bounds,
        active_bounds,
        None,
        crate::region_selector::SelectorWindowOptions {
            display_id,
            kind: ghost_window_kind(),
            decorations: ghost_window_decorations(false),
            focus,
        },
        crate::region_selector::SelectorWindowSources {
            map_rect,
            global_pointer,
            active_bounds: None,
            hover_target: None,
        },
    )
}

fn selector_focus(bounds: Bounds<Pixels>, active_bounds: Option<Bounds<Pixels>>) -> bool {
    match active_bounds {
        Some(active_bounds) => bounds == active_bounds,
        None => true,
    }
}

struct MacPointerSource;

impl crate::region_selector::GlobalPointer for MacPointerSource {
    fn position(&self) -> Option<gpui::Point<Pixels>> {
        let event = CfGuard::new(unsafe { CGEventCreate(std::ptr::null()) })?;
        let location = unsafe { CGEventGetLocation(event.as_ptr()) };
        Some(gpui::point(
            gpui::px(location.x as f32),
            gpui::px(location.y as f32),
        ))
    }

    fn primary_button_down(&self) -> bool {
        unsafe { CGEventSourceButtonState(CG_EVENT_SOURCE_STATE_HID_SYSTEM, CG_MOUSE_BUTTON_LEFT) }
    }
}

struct CfGuard(*const c_void);

impl CfGuard {
    fn new(ptr: *const c_void) -> Option<Self> {
        if ptr.is_null() {
            return None;
        }
        Some(Self(ptr))
    }

    fn as_ptr(&self) -> *const c_void {
        self.0
    }
}

impl Drop for CfGuard {
    fn drop(&mut self) {
        unsafe { CFRelease(self.0) }
    }
}

fn selector_monitors(tracker: &MonitorTracker) -> Vec<ActiveMonitor> {
    let monitors = tracker.all_monitors();
    if !monitors.is_empty() {
        return monitors;
    }
    tracker.snapshot_monitor().into_iter().collect()
}

fn selector_fallback_bounds() -> Bounds<Pixels> {
    let displays = active_display_bounds()
        .ok()
        .filter(|displays| !displays.is_empty())
        .and_then(|displays| displays.into_iter().next());
    displays
        .map(crate::region_selector::bounds_from_monitor)
        .unwrap_or_else(crate::region_selector::fallback_bounds)
}

fn selector_rect_mapper() -> crate::region_selector::RectMapper {
    let displays = active_display_bounds().unwrap_or_default();
    std::rc::Rc::new(move |rect| map_selector_rect_to_capture(rect, &displays))
}

pub(super) fn map_selector_rect_to_capture(rect: Rect, displays: &[Monitor]) -> Option<Rect> {
    if displays.is_empty() {
        return Some(rect);
    }

    if displays
        .iter()
        .any(|display| rect_intersection(rect, *display).is_some())
    {
        return Some(rect);
    }

    None
}

fn configure_selector_window(title: String, cx: &mut gpui::App) {
    cx.defer(move |_| configure_selector_window_now(&title));
}

fn configure_selector_window_now(title: &str) {
    let started = Instant::now();
    if qol_gpui::popup_window::configure_overlay_window(title) {
        qol_runtime::probe!(
            "SHOT_SELECT_OVERLAY",
            "ms={} result=mapped",
            started.elapsed().as_millis()
        );
        return;
    }
    qol_runtime::probe!(
        "SHOT_SELECT_OVERLAY",
        "ms={} result=missing",
        started.elapsed().as_millis()
    );
}
