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

fn center_x(bounds: Bounds<Pixels>) -> f64 {
    bounds.origin.x.to_f64() + bounds.size.width.to_f64() / 2.0
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
            assert_eq!(monitor.size(), (2560.0, 1440.0), "event: {event:?}");
        }
    }
}

#[test]
fn centered_bounds_matches_known_placements() {
    let middle = ActiveMonitor::from_bounds(mon(1920.0, 0.0, 2560.0, 1440.0));
    let cases = [
        (size(px(500.0), px(42.0)), 2950.0, 466.0),
        (size(px(1000.0), px(144.0)), 2700.0, 432.0),
    ];
    for (win, expected_x, expected_y) in cases {
        let bounds = middle.centered_bounds(win);
        assert_eq!(bounds.origin.x.to_f64(), expected_x, "win: {win:?}");
        assert_eq!(bounds.origin.y.to_f64(), expected_y, "win: {win:?}");
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
        let picker_center = center_x(monitor.centered_bounds(picker));
        let launcher_center = center_x(monitor.centered_bounds(launcher));
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
