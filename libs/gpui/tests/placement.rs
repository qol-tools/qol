use gpui::{px, size, Bounds, Pixels};
use qol_gpui::monitor::ActiveMonitor;
use qol_gpui::protocol::RuntimeEvent;
use qol_runtime::MonitorBounds;

fn mon(x: f32, y: f32, w: f32, h: f32) -> MonitorBounds {
    MonitorBounds {
        x,
        y,
        width: w,
        height: h,
    }
}

fn native(value: Pixels, monitor: &ActiveMonitor) -> f64 {
    (value.to_f64() * f64::from(monitor.scale())).round()
}

fn native_center_x(bounds: Bounds<Pixels>, monitor: &ActiveMonitor) -> f64 {
    native(bounds.origin.x + bounds.size.width / 2.0, monitor)
}

#[test]
fn from_event_resolves_monitor_only_for_carrying_events() {
    let m = mon(1920.0, 0.0, 2560.0, 1440.0);
    let cases = [
        (
            RuntimeEvent::ActiveMonitorChanged {
                monitor_idx: Some(1),
                monitor: Some(m),
            },
            true,
        ),
        (
            RuntimeEvent::FocusChanged {
                window_id: None,
                monitor_idx: Some(1),
                monitor: Some(m),
            },
            true,
        ),
        (
            RuntimeEvent::ActiveMonitorChanged {
                monitor_idx: None,
                monitor: None,
            },
            false,
        ),
        (RuntimeEvent::CursorMoved { x: 10.0, y: 20.0 }, false),
        (RuntimeEvent::WindowListChanged, false),
        (
            RuntimeEvent::MonitorsChanged {
                monitors: Vec::new(),
            },
            false,
        ),
    ];
    for (event, expects_monitor) in cases {
        let resolved = ActiveMonitor::from_event(&event);
        assert_eq!(resolved.is_some(), expects_monitor, "event: {event:?}");
        if let Some(monitor) = resolved {
            let scale = monitor.scale();
            assert_eq!(
                monitor.size(),
                (2560.0 / scale, 1440.0 / scale),
                "event: {event:?}"
            );
        }
    }
}

#[test]
fn centered_bounds_matches_known_placements() {
    let middle = ActiveMonitor::from_bounds(mon(1920.0, 0.0, 2560.0, 1440.0));
    let scale = middle.scale();
    let cases = [
        ((500.0, 42.0), 2950.0, 466.0),
        ((1000.0, 144.0), 2700.0, 432.0),
    ];
    for ((width, height), expected_x, expected_y) in cases {
        let win = size(px(width / scale), px(height / scale));
        let bounds = middle.centered_bounds(win);
        assert_eq!(native(bounds.origin.x, &middle), expected_x, "win: {win:?}");
        assert_eq!(native(bounds.origin.y, &middle), expected_y, "win: {win:?}");
    }
}

#[test]
fn different_window_sizes_share_one_monitor_center() {
    let monitors = [
        mon(0.0, 360.0, 1920.0, 1080.0),
        mon(1920.0, 0.0, 2560.0, 1440.0),
        mon(4480.0, 0.0, 2560.0, 1440.0),
    ];
    let picker = size(px(1918.0), px(1078.0));
    let launcher = size(px(500.0), px(42.0));
    for m in monitors {
        let monitor = ActiveMonitor::from_bounds(m);
        let picker_center = native_center_x(monitor.centered_bounds(picker), &monitor);
        let launcher_center = native_center_x(monitor.centered_bounds(launcher), &monitor);
        assert_eq!(
            picker_center, launcher_center,
            "two ghost sizes must center on the same monitor: {m:?}"
        );
        assert_eq!(
            picker_center,
            (m.x + m.width / 2.0) as f64,
            "center must be the monitor midline: {m:?}"
        );
    }
}
