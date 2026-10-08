use super::*;

fn bounds(x: f32, y: f32, width: f32, height: f32) -> MonitorBounds {
    MonitorBounds {
        x,
        y,
        width,
        height,
    }
}

fn state(key: &str, open: bool) -> WindowState {
    WindowState {
        version: SCHEMA_VERSION,
        key: key.into(),
        monitor: qol_window_state::MonitorRef {
            id: "id".into(),
            connector: "DP-1".into(),
            bounds: bounds(0.0, 0.0, 1920.0, 1080.0),
        },
        bounds_on_monitor: WindowRect {
            x: 10.0,
            y: 20.0,
            width: 300.0,
            height: 200.0,
        },
        scale: 1.0,
        open,
        owner_pid: std::process::id(),
        default_placement: MonitorPlacement::center().memory_label(),
        reopen: None,
    }
}

#[test]
fn a_flush_writes_the_latest_queued_state_once() {
    let dir = tempfile::tempdir().unwrap();
    let store = WindowStateStore::new(dir.path());
    let key = "memory-test-flush-key";
    for open in [true, false] {
        PENDING
            .lock()
            .unwrap()
            .insert(key.into(), (store.clone(), state(key, open)));
    }
    flush_key(key);
    assert_eq!(store.load(key), Some(state(key, false)));
    std::fs::remove_file(dir.path().join(format!("{key}.json"))).unwrap();
    flush_key(key);
    assert_eq!(store.load(key), None);
}

#[test]
fn flush_all_drains_every_pending_window() {
    let dir = tempfile::tempdir().unwrap();
    let store = WindowStateStore::new(dir.path());
    let keys = ["memory-test-all-a", "memory-test-all-b"];
    for key in keys {
        PENDING
            .lock()
            .unwrap()
            .insert(key.into(), (store.clone(), state(key, true)));
    }
    flush_all();
    for key in keys {
        assert_eq!(store.load(key), Some(state(key, true)));
    }
}

#[test]
fn a_window_belongs_to_the_monitor_holding_its_centre() {
    let monitor = bounds(1920.0, 0.0, 2560.0, 1440.0);
    let window = |x| WindowRect {
        x,
        y: 100.0,
        width: 400.0,
        height: 300.0,
    };
    assert!(contains(monitor, window(1800.0)));
    assert!(!contains(monitor, window(1700.0)));
}

#[test]
fn placement_labels_tell_defaults_apart() {
    use crate::placement::{Corner, CORNER_MARGIN};
    let labels = [
        MonitorPlacement::center().memory_label(),
        MonitorPlacement::corner(Corner::BottomRight, CORNER_MARGIN).memory_label(),
        MonitorPlacement::corner(Corner::TopLeft, CORNER_MARGIN).memory_label(),
        MonitorPlacement::corner(Corner::TopLeft, 8.0).memory_label(),
    ];
    let unique: std::collections::HashSet<_> = labels.iter().collect();
    assert_eq!(unique.len(), labels.len(), "{labels:?}");
}

#[test]
fn geometry_ids_stand_in_for_monitors_without_identity() {
    assert_eq!(
        geometry_id(bounds(-1920.0, 0.0, 1920.0, 1080.0)),
        "geometry--1920-0-1920x1080"
    );
}
