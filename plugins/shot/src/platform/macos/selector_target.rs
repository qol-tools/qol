use gpui::{Pixels, Point};
use qol_gpui::monitor::ActiveMonitor;
use std::ffi::c_void;
use std::rc::Rc;

use crate::ui::region_selector::{
    DetectedTarget, DetectedTargetRole, HoverTarget, HoverTargetSource,
};
use crate::Rect;

const MIN_TARGET_SIZE: i32 = 24;
const WINDOW_LIST_ON_SCREEN_ONLY: u32 = 1;
const WINDOW_LIST_EXCLUDE_DESKTOP: u32 = 1 << 4;
const WINDOW_LIST_OPTIONS: u32 = WINDOW_LIST_ON_SCREEN_ONLY | WINDOW_LIST_EXCLUDE_DESKTOP;
const NULL_WINDOW_ID: u32 = 0;
const NORMAL_WINDOW_LAYER: i32 = 0;
const CF_NUMBER_I32: isize = 3;
const UTF8: u32 = 0x0800_0100;

#[repr(C)]
#[derive(Clone, Copy)]
struct CGPoint {
    x: f64,
    y: f64,
}

#[repr(C)]
#[derive(Clone, Copy)]
struct CGSize {
    width: f64,
    height: f64,
}

#[repr(C)]
#[derive(Clone, Copy)]
struct CGRect {
    origin: CGPoint,
    size: CGSize,
}

#[link(name = "CoreGraphics", kind = "framework")]
extern "C" {
    fn CGWindowListCopyWindowInfo(options: u32, relative_to_window: u32) -> *const c_void;
    fn CGRectMakeWithDictionaryRepresentation(dictionary: *const c_void, rect: *mut CGRect)
        -> bool;
}

#[link(name = "CoreFoundation", kind = "framework")]
extern "C" {
    fn CFRelease(value: *const c_void);
    fn CFArrayGetCount(array: *const c_void) -> isize;
    fn CFArrayGetValueAtIndex(array: *const c_void, index: isize) -> *const c_void;
    fn CFDictionaryGetValue(dictionary: *const c_void, key: *const c_void) -> *const c_void;
    fn CFNumberGetValue(number: *const c_void, kind: isize, value: *mut c_void) -> bool;
    fn CFStringCreateWithBytes(
        allocator: *const c_void,
        bytes: *const u8,
        byte_count: isize,
        encoding: u32,
        external_representation: bool,
    ) -> *const c_void;
}

pub(super) fn snapshot(monitors: &[ActiveMonitor]) -> Option<HoverTargetSource> {
    let discovery = discover_visible_windows();
    let monitor_rects = monitors
        .iter()
        .map(|monitor| rect_from_bounds(monitor.bounds()))
        .collect::<Vec<_>>();
    let mut too_small = 0;
    let mut off_screen = 0;
    let windows = discovery
        .windows
        .into_iter()
        .filter(|rect| {
            if rect.w < MIN_TARGET_SIZE || rect.h < MIN_TARGET_SIZE {
                too_small += 1;
                return false;
            }
            if !monitor_rects.is_empty()
                && !monitor_rects
                    .iter()
                    .any(|monitor| rects_intersect(*rect, *monitor))
            {
                off_screen += 1;
                return false;
            }
            true
        })
        .collect::<Vec<_>>();
    qol_runtime::probe!(
        "SHOT_SELECT_DISCOVERY",
        "platform=macos available={} total={} accepted={} own={} non_normal={} invalid={} too_small={too_small} off_screen={off_screen}",
        discovery.available,
        discovery.total,
        windows.len(),
        discovery.own,
        discovery.non_normal,
        discovery.invalid
    );
    if windows.is_empty() && monitor_rects.is_empty() {
        return None;
    }
    Some(Rc::new(SnapshotTargets {
        windows,
        monitors: monitor_rects,
    }))
}

struct SnapshotTargets {
    windows: Vec<Rect>,
    monitors: Vec<Rect>,
}

impl HoverTarget for SnapshotTargets {
    fn target_at(&self, point: Point<Pixels>) -> Option<DetectedTarget> {
        let x = f32::from(point.x).round() as i32;
        let y = f32::from(point.y).round() as i32;
        let hit =
            |rect: &Rect| x >= rect.x && x < rect.x + rect.w && y >= rect.y && y < rect.y + rect.h;
        if let Some(rect) = self.windows.iter().copied().find(hit) {
            return Some(detected_target(rect, true));
        }
        self.monitors
            .iter()
            .copied()
            .find(hit)
            .map(|rect| detected_target(rect, false))
    }
}

fn detected_target(rect: Rect, is_window: bool) -> DetectedTarget {
    DetectedTarget {
        rect,
        role: DetectedTargetRole { is_window },
    }
}

#[derive(Default)]
struct WindowDiscovery {
    windows: Vec<Rect>,
    available: bool,
    total: usize,
    own: usize,
    non_normal: usize,
    invalid: usize,
}

fn discover_visible_windows() -> WindowDiscovery {
    let Some(list) =
        CfGuard::new(unsafe { CGWindowListCopyWindowInfo(WINDOW_LIST_OPTIONS, NULL_WINDOW_ID) })
    else {
        return WindowDiscovery::default();
    };
    let Some(keys) = WindowKeys::new() else {
        return WindowDiscovery::default();
    };
    let own_pid = std::process::id() as i32;
    let count = unsafe { CFArrayGetCount(list.as_ptr()) }.max(0);
    let mut discovery = WindowDiscovery {
        available: true,
        total: count as usize,
        ..WindowDiscovery::default()
    };
    for index in 0..count {
        let dictionary = unsafe { CFArrayGetValueAtIndex(list.as_ptr(), index) };
        let Some(layer) = dictionary_i32(dictionary, keys.layer.as_ptr()) else {
            discovery.invalid += 1;
            continue;
        };
        if layer != NORMAL_WINDOW_LAYER {
            discovery.non_normal += 1;
            continue;
        }
        let Some(pid) = dictionary_i32(dictionary, keys.pid.as_ptr()) else {
            discovery.invalid += 1;
            continue;
        };
        if pid == own_pid {
            discovery.own += 1;
            continue;
        }
        let Some(rect) = dictionary_rect(dictionary, keys.bounds.as_ptr()) else {
            discovery.invalid += 1;
            continue;
        };
        discovery.windows.push(rect);
    }
    discovery
}

struct WindowKeys {
    layer: CfGuard,
    pid: CfGuard,
    bounds: CfGuard,
}

impl WindowKeys {
    fn new() -> Option<Self> {
        Some(Self {
            layer: cf_string(b"kCGWindowLayer")?,
            pid: cf_string(b"kCGWindowOwnerPID")?,
            bounds: cf_string(b"kCGWindowBounds")?,
        })
    }
}

struct CfGuard(*const c_void);

impl CfGuard {
    fn new(value: *const c_void) -> Option<Self> {
        (!value.is_null()).then_some(Self(value))
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

fn cf_string(value: &[u8]) -> Option<CfGuard> {
    CfGuard::new(unsafe {
        CFStringCreateWithBytes(
            std::ptr::null(),
            value.as_ptr(),
            value.len() as isize,
            UTF8,
            false,
        )
    })
}

fn dictionary_i32(dictionary: *const c_void, key: *const c_void) -> Option<i32> {
    if dictionary.is_null() {
        return None;
    }
    let value = unsafe { CFDictionaryGetValue(dictionary, key) };
    if value.is_null() {
        return None;
    }
    let mut result = 0i32;
    unsafe { CFNumberGetValue(value, CF_NUMBER_I32, &mut result as *mut i32 as *mut c_void) }
        .then_some(result)
}

fn dictionary_rect(dictionary: *const c_void, key: *const c_void) -> Option<Rect> {
    if dictionary.is_null() {
        return None;
    }
    let value = unsafe { CFDictionaryGetValue(dictionary, key) };
    if value.is_null() {
        return None;
    }
    let mut bounds = CGRect {
        origin: CGPoint { x: 0.0, y: 0.0 },
        size: CGSize {
            width: 0.0,
            height: 0.0,
        },
    };
    if !unsafe { CGRectMakeWithDictionaryRepresentation(value, &mut bounds) } {
        return None;
    }
    let rect = Rect {
        x: bounds.origin.x.round() as i32,
        y: bounds.origin.y.round() as i32,
        w: bounds.size.width.round() as i32,
        h: bounds.size.height.round() as i32,
    };
    (rect.w > 0 && rect.h > 0).then_some(rect)
}

fn rect_from_bounds(bounds: gpui::Bounds<Pixels>) -> Rect {
    Rect {
        x: f32::from(bounds.origin.x).round() as i32,
        y: f32::from(bounds.origin.y).round() as i32,
        w: f32::from(bounds.size.width).round() as i32,
        h: f32::from(bounds.size.height).round() as i32,
    }
}

fn rects_intersect(left: Rect, right: Rect) -> bool {
    left.x < right.x + right.w
        && left.x + left.w > right.x
        && left.y < right.y + right.h
        && left.y + left.h > right.y
}

#[cfg(test)]
mod tests {
    use gpui::{point, px};

    use super::{detected_target, HoverTarget, Rect, SnapshotTargets};

    #[test]
    fn hit_testing_prefers_the_frontmost_window_then_the_monitor() {
        let front = Rect {
            x: 40,
            y: 40,
            w: 120,
            h: 80,
        };
        let back = Rect {
            x: 0,
            y: 0,
            w: 240,
            h: 180,
        };
        let monitor = Rect {
            x: 0,
            y: 0,
            w: 400,
            h: 300,
        };
        let targets = SnapshotTargets {
            windows: vec![front, back],
            monitors: vec![monitor],
        };

        assert_eq!(
            targets.target_at(point(px(80.0), px(60.0))),
            Some(detected_target(front, true))
        );
        assert_eq!(
            targets.target_at(point(px(300.0), px(200.0))),
            Some(detected_target(monitor, false))
        );
    }
}
