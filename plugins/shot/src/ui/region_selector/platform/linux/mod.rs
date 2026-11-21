use anyhow::Result;
use gpui::{Bounds, Pixels, Point, WindowDecorations, WindowKind};
use qol_gpui::monitor::{ActiveMonitor, MonitorTracker};
use std::rc::Rc;
use std::sync::mpsc;
use std::time::{Duration, Instant};

use crate::capture::frozen_frame::FrozenFrame;
use crate::capture::space::CaptureKind;
use crate::platform::{
    configure_selector_window, default_window_target, detectable_window_rects, full_screen_bounds,
    prepare_selector_window, reassert_parked, rect_from_bounds, runtime_monitor_rects,
};
use crate::ui::region_selector::{DetectedTarget, SnapshotTargets};
use crate::{Monitor, Rect};

mod selector_cache;

const SLOW_SELECTOR_OPEN: Duration = Duration::from_millis(50);

thread_local! {
    static SELECTOR_CACHE: selector_cache::SelectorCache = selector_cache::SelectorCache::default();
}

struct SelectorTopology {
    cursor: Option<(ActiveMonitor, Option<Point<Pixels>>)>,
    monitors: Vec<ActiveMonitor>,
}

pub fn pre_create_selector(cx: &mut gpui::App) {
    let bounds = selector_bounds(None);
    let monitors = MonitorTracker::start(cx).all_monitors_or_snapshot();
    let monitor_bounds = selector_monitor_bounds(monitors, bounds);
    let selector = selector_window(bounds, monitor_bounds, None, None, None, None, None);
    SELECTOR_CACHE.with(|cache| {
        let Some(title) =
            selector_cache::pre_create_cached(cache, selector, CaptureKind::Screenshot, cx)
        else {
            return;
        };
        prepare_selector_window(&title, rect_from_bounds(bounds));
        reassert_parked(&title, cx);
    });
}

pub fn select_region(kind: CaptureKind, frozen_frame: Option<FrozenFrame>) -> Result<Option<Rect>> {
    crate::ui::region_selector::select_region_blocking_with(move |tx, cx| {
        let tracker = MonitorTracker::start(cx);
        let cursor = tracker.snapshot_cursor();
        let monitors = tracker.all_monitors_or_snapshot();
        let active_bounds = Some(tracker_active_bounds_source(tracker));
        open_region_selector_with_sender(
            tx,
            true,
            kind,
            SelectorTopology { cursor, monitors },
            active_bounds,
            frozen_frame,
            cx,
        );
    })
}

pub fn select_region_in_app(
    cx: &mut gpui::App,
    kind: CaptureKind,
    cursor: Option<(ActiveMonitor, Option<Point<Pixels>>)>,
    monitors: Vec<ActiveMonitor>,
    frozen_frame: Option<FrozenFrame>,
) -> Option<mpsc::Receiver<Option<Rect>>> {
    let active_bounds = Some(tracker_active_bounds_source(MonitorTracker::start(cx)));
    Some(open_region_selector(
        cx,
        kind,
        SelectorTopology { cursor, monitors },
        active_bounds,
        frozen_frame,
    ))
}

fn open_region_selector(
    cx: &mut gpui::App,
    kind: CaptureKind,
    topology: SelectorTopology,
    active_bounds: Option<crate::ui::region_selector::ActiveBoundsSource>,
    frozen_frame: Option<FrozenFrame>,
) -> mpsc::Receiver<Option<Rect>> {
    let (tx, rx) = mpsc::channel();
    open_region_selector_with_sender(tx, false, kind, topology, active_bounds, frozen_frame, cx);
    rx
}

fn open_region_selector_with_sender(
    tx: mpsc::Sender<Option<Rect>>,
    quit_on_finish: bool,
    kind: CaptureKind,
    topology: SelectorTopology,
    active_bounds: Option<crate::ui::region_selector::ActiveBoundsSource>,
    frozen_frame: Option<FrozenFrame>,
    cx: &mut gpui::App,
) {
    let started = Instant::now();
    let (monitor, pointer) = match topology.cursor {
        Some((monitor, pointer)) => (Some(monitor), pointer),
        None => (None, None),
    };
    let bounds = selector_bounds(frozen_frame.as_ref());
    let monitor_bounds = selector_monitor_bounds(topology.monitors, bounds);
    let hover_target = snapshot_hover_target(bounds);
    let default_target = initial_default_target(
        pointer,
        hover_target.as_deref(),
        default_window_target(bounds),
        monitor
            .as_ref()
            .map(|monitor| rect_from_bounds(monitor.bounds())),
    );
    trace_default_target(default_target, bounds);
    let selector = selector_window(
        bounds,
        monitor_bounds,
        monitor,
        active_bounds,
        hover_target,
        default_target,
        frozen_frame,
    );
    let discovered = started.elapsed();
    let path = present_selector(tx, quit_on_finish, kind, selector, bounds, cx);
    report_selector_open(path, discovered, started.elapsed());
}

fn present_selector(
    tx: mpsc::Sender<Option<Rect>>,
    quit_on_finish: bool,
    kind: CaptureKind,
    selector: crate::ui::region_selector::SelectorWindow,
    bounds: Bounds<Pixels>,
    cx: &mut gpui::App,
) -> SelectorOpenPath {
    if !quit_on_finish {
        let mut tx = Some(tx);
        let mut selector = Some(selector);
        let reveal_bounds = rect_from_bounds(bounds);
        let reveal: crate::ui::region_selector::SelectorReveal = Rc::new(move |title| {
            configure_selector_window(title, reveal_bounds);
        });
        let title = SELECTOR_CACHE.with(|cache| {
            selector_cache::open_cached(cache, &mut tx, &mut selector, kind, reveal, cx)
        });
        if title.is_some() {
            cx.activate(true);
            return SelectorOpenPath::Reuse;
        }
        let adopted = SELECTOR_CACHE
            .with(|cache| selector_cache::open_adopted(cache, &mut tx, &mut selector, kind, cx));
        if let Some(title) = adopted {
            configure_selector_window(title, rect_from_bounds(bounds));
            cx.activate(true);
            return SelectorOpenPath::Adopt;
        }
        let (Some(tx), Some(selector)) = (tx, selector) else {
            return SelectorOpenPath::Failed;
        };
        return open_cold(tx, false, kind, selector, bounds, cx);
    }
    open_cold(tx, true, kind, selector, bounds, cx)
}

fn open_cold(
    tx: mpsc::Sender<Option<Rect>>,
    quit_on_finish: bool,
    kind: CaptureKind,
    selector: crate::ui::region_selector::SelectorWindow,
    bounds: Bounds<Pixels>,
    cx: &mut gpui::App,
) -> SelectorOpenPath {
    let title = selector.title().to_string();
    if !crate::ui::region_selector::open_all(tx, quit_on_finish, vec![selector], kind, cx) {
        return SelectorOpenPath::Failed;
    }
    configure_selector_window(title, rect_from_bounds(bounds));
    cx.activate(true);
    SelectorOpenPath::Cold
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum SelectorOpenPath {
    Reuse,
    Adopt,
    Cold,
    Failed,
}

impl SelectorOpenPath {
    fn label(self) -> &'static str {
        match self {
            Self::Reuse => "reuse",
            Self::Adopt => "adopt",
            Self::Cold => "cold",
            Self::Failed => "failed",
        }
    }
}

fn report_selector_open(path: SelectorOpenPath, discovered: Duration, total: Duration) {
    qol_runtime::probe!(
        "SHOT_SELECT_LATENCY",
        "path={} discover_ms={} open_ms={}",
        path.label(),
        discovered.as_millis(),
        total.as_millis()
    );
    if let Some(message) = slow_selector_open_message(path, discovered, total) {
        log::warn!("{message}");
    }
}

fn slow_selector_open_message(
    path: SelectorOpenPath,
    discovered: Duration,
    total: Duration,
) -> Option<String> {
    if total < SLOW_SELECTOR_OPEN {
        return None;
    }
    Some(format!(
        "capture selector took {} ms to open (path={}, target discovery {} ms)",
        total.as_millis(),
        path.label(),
        discovered.as_millis()
    ))
}

fn initial_default_target(
    pointer: Option<Point<Pixels>>,
    hover_target: Option<&dyn crate::ui::region_selector::HoverTarget>,
    focused_window: Option<Rect>,
    monitor: Option<Rect>,
) -> Option<crate::ui::region_selector::DetectedTarget> {
    let pointer_target = pointer.and_then(|point| hover_target?.target_at(point));
    let pointer_monitor = pointer
        .zip(monitor)
        .map(|(_, rect)| DetectedTarget::monitor(rect));
    pointer_target
        .or(pointer_monitor)
        .or_else(|| focused_window.map(DetectedTarget::window))
        .or_else(|| monitor.map(DetectedTarget::monitor))
}

fn selector_window(
    bounds: Bounds<Pixels>,
    monitor_bounds: Vec<Bounds<Pixels>>,
    monitor: Option<ActiveMonitor>,
    active_bounds: Option<crate::ui::region_selector::ActiveBoundsSource>,
    hover_target: Option<crate::ui::region_selector::HoverTargetSource>,
    default_target: Option<crate::ui::region_selector::DetectedTarget>,
    frozen_frame: Option<FrozenFrame>,
) -> crate::ui::region_selector::SelectorWindow {
    crate::ui::region_selector::SelectorWindow::new(
        bounds,
        monitor_bounds,
        monitor.map(|monitor| monitor.bounds()),
        default_target,
        crate::ui::region_selector::SelectorWindowOptions {
            display_id: None,
            placement: None,
            kind: WindowKind::PopUp,
            decorations: WindowDecorations::Client,
            focus: true,
        },
        crate::ui::region_selector::SelectorWindowSources {
            map_rect: selector_cache::identity_rect_mapper(),
            global_pointer: None,
            cancel_signal: Some(Rc::new(qol_gpui::platform::is_escape_held)),
            active_bounds,
            hover_target,
            frozen_frame,
        },
    )
}

fn selector_monitor_bounds(
    monitors: Vec<ActiveMonitor>,
    fallback: Bounds<Pixels>,
) -> Vec<Bounds<Pixels>> {
    let bounds = monitors
        .into_iter()
        .map(|monitor| monitor.bounds())
        .collect::<Vec<_>>();
    if bounds.is_empty() {
        return vec![fallback];
    }
    bounds
}

fn snapshot_hover_target(
    selector_bounds: Bounds<Pixels>,
) -> Option<crate::ui::region_selector::HoverTargetSource> {
    let include_frame = crate::config::load().capture.include_window_frame;
    let windows = detectable_window_rects(selector_bounds, include_frame);
    let monitors = runtime_monitor_rects();
    qol_runtime::probe!(
        "SHOT_SELECT_TARGET",
        "snapshot_windows={} monitors={} frame={include_frame}",
        windows.len(),
        monitors.len()
    );
    if windows.is_empty() && monitors.is_empty() {
        return None;
    }
    Some(Rc::new(SnapshotTargets { windows, monitors }))
}

fn trace_default_target(
    target: Option<crate::ui::region_selector::DetectedTarget>,
    selector_bounds: Bounds<Pixels>,
) {
    #[cfg(debug_assertions)]
    {
        let target = target
            .map(|target| {
                let role = if target.role.is_window {
                    "window"
                } else {
                    "monitor"
                };
                format!(
                    "{role}:{}",
                    crate::capture::geometry::rect_label(target.rect)
                )
            })
            .unwrap_or_else(|| "none".to_string());
        let bounds = rect_from_bounds(selector_bounds);
        let bounds = crate::capture::geometry::rect_label(bounds);
        qol_runtime::probe!("SHOT_SELECT_TARGET", "default={target} selector={bounds}");
    }
    #[cfg(not(debug_assertions))]
    let _ = (target, selector_bounds);
}

const ACTIVE_BOUNDS_SAMPLE_TTL: std::time::Duration = std::time::Duration::from_millis(200);

fn tracker_active_bounds_source(
    tracker: MonitorTracker,
) -> crate::ui::region_selector::ActiveBoundsSource {
    Rc::new(TrackerActiveBounds {
        tracker,
        cache: std::cell::RefCell::new(None),
    })
}

struct TrackerActiveBounds {
    tracker: MonitorTracker,
    cache: std::cell::RefCell<Option<(std::time::Instant, Option<Bounds<Pixels>>)>>,
}

impl crate::ui::region_selector::ActiveBounds for TrackerActiveBounds {
    fn active_bounds(&self) -> Option<Bounds<Pixels>> {
        if let Some((sampled_at, bounds)) = *self.cache.borrow() {
            if sampled_at.elapsed() < ACTIVE_BOUNDS_SAMPLE_TTL {
                return bounds;
            }
        }
        let bounds = self
            .tracker
            .snapshot_cursor()
            .map(|(monitor, _)| monitor.bounds());
        *self.cache.borrow_mut() = Some((std::time::Instant::now(), bounds));
        bounds
    }
}

fn selector_bounds(frozen_frame: Option<&FrozenFrame>) -> Bounds<Pixels> {
    if let Some(frame) = frozen_frame {
        let bounds = frame.bounds();
        return crate::ui::region_selector::bounds_from_monitor(Monitor {
            x: bounds.x,
            y: bounds.y,
            w: bounds.w,
            h: bounds.h,
        });
    }
    full_screen_bounds()
        .map(crate::ui::region_selector::bounds_from_monitor)
        .unwrap_or_else(|_| crate::ui::region_selector::fallback_bounds())
}

#[cfg(test)]
mod tests {
    use super::{initial_default_target, selector_monitor_bounds};
    use crate::ui::region_selector::{DetectedTarget, HoverTarget, SnapshotTargets};
    use crate::Rect;
    use gpui::{point, px, size, Bounds};
    use qol_gpui::monitor::ActiveMonitor;
    use qol_runtime::MonitorBounds;

    #[test]
    fn initial_target_follows_cursor_before_focus_fallbacks() {
        let hovered = Rect {
            x: 2560,
            y: 0,
            w: 1920,
            h: 1080,
        };
        let focused = Rect {
            x: 0,
            y: 32,
            w: 2560,
            h: 1366,
        };
        let cursor_monitor = Rect {
            x: 2560,
            y: 0,
            w: 1920,
            h: 1080,
        };
        let source = SnapshotTargets {
            windows: vec![hovered],
            monitors: vec![cursor_monitor],
        };
        let cases = [
            (
                Some(point(px(3500.0), px(500.0))),
                Some(&source as &dyn HoverTarget),
                Some(focused),
                Some(cursor_monitor),
                Some(DetectedTarget::window(hovered)),
            ),
            (
                Some(point(px(5000.0), px(500.0))),
                Some(&source as &dyn HoverTarget),
                Some(focused),
                Some(cursor_monitor),
                Some(DetectedTarget::monitor(cursor_monitor)),
            ),
            (
                None,
                Some(&source as &dyn HoverTarget),
                Some(focused),
                Some(cursor_monitor),
                Some(DetectedTarget::window(focused)),
            ),
            (
                None,
                None,
                None,
                Some(cursor_monitor),
                Some(DetectedTarget::monitor(cursor_monitor)),
            ),
            (None, None, None, None, None),
        ];
        for (pointer, hover, focused, monitor, expected) in cases {
            assert_eq!(
                initial_default_target(pointer, hover, focused, monitor),
                expected
            );
        }
    }

    #[test]
    fn selector_keeps_physical_monitors_separate_from_the_spanning_viewport() {
        let viewport = Bounds::new(point(px(0.0), px(0.0)), size(px(4480.0), px(1440.0)));
        let primary = MonitorBounds {
            x: 0.0,
            y: 0.0,
            width: 2560.0,
            height: 1440.0,
        };
        let secondary = MonitorBounds {
            x: 2560.0,
            y: 0.0,
            width: 1920.0,
            height: 1080.0,
        };

        assert_eq!(
            selector_monitor_bounds(
                vec![
                    ActiveMonitor::from_bounds(primary),
                    ActiveMonitor::from_bounds(secondary),
                ],
                viewport,
            ),
            vec![
                Bounds::new(point(px(0.0), px(0.0)), size(px(2560.0), px(1440.0))),
                Bounds::new(point(px(2560.0), px(0.0)), size(px(1920.0), px(1080.0))),
            ],
            "the Linux selector viewport must not masquerade as one physical monitor"
        );
        assert_eq!(
            selector_monitor_bounds(Vec::new(), viewport),
            vec![viewport],
            "the viewport remains the last-resort fallback when topology is unavailable"
        );
    }
}
