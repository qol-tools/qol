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
        user_sized: false,
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

#[gpui::test]
fn a_restore_lands_on_the_active_monitor_at_the_saved_offset(cx: &mut gpui::TestAppContext) {
    let dir = tempfile::tempdir().unwrap();
    let saved = state("memory-test-active-key", true);
    let remembered = Remembered {
        store: WindowStateStore::new(dir.path()),
        key: saved.key.clone(),
        reopen: RefCell::new(None),
        default_placement: saved.default_placement.clone(),
        last: RefCell::new(Some(saved)),
        save_generation: Cell::new(0),
        user_sized: Cell::new(false),
        unsaved_bounds: Cell::new(None),
    };
    let active = ActiveMonitor::from_bounds(bounds(100_000.0, 0.0, 2560.0, 1440.0));
    let restored = cx
        .update(|cx| {
            remembered.restore(
                RestoreSize::AtLeast(size(px(300.0), px(200.0))),
                Some(&active),
                cx,
            )
        })
        .unwrap();
    assert_eq!(
        restored.bounds,
        to_bounds(WindowRect {
            x: 100_010.0,
            y: 20.0,
            width: 300.0,
            height: 200.0,
        })
    );
    assert_eq!(restored.monitor.bounds(), active.bounds());
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

#[test]
fn a_fixed_size_window_keeps_its_size_on_a_smaller_monitor() {
    let mut saved = state("memory-test-small-monitor", true);
    saved.bounds_on_monitor = WindowRect {
        x: 160.0,
        y: 60.0,
        width: 1040.0,
        height: 640.0,
    };
    let screen = bounds(0.0, 0.0, 900.0, 600.0);
    let placed = qol_window_state::place_on(&saved, screen, screen);
    assert_eq!(
        with_size_of(placed, saved.bounds_on_monitor),
        WindowRect {
            x: 0.0,
            y: 0.0,
            width: 1040.0,
            height: 640.0,
        }
    );
}

#[test]
fn a_restored_size_grows_to_the_minimum_and_stays_inside_the_work_area() {
    let screen = bounds(0.0, 0.0, 1920.0, 1040.0);
    let at = |x: f64, y: f64, width: f64, height: f64| WindowRect {
        x,
        y,
        width,
        height,
    };
    let cases = [
        (
            at(100.0, 100.0, 300.0, 200.0),
            size(px(400.0), px(250.0)),
            at(100.0, 100.0, 400.0, 250.0),
            "a saved size under the minimum grows to it",
        ),
        (
            at(100.0, 100.0, 800.0, 600.0),
            size(px(400.0), px(250.0)),
            at(100.0, 100.0, 800.0, 600.0),
            "a saved size over the minimum is kept",
        ),
        (
            at(1800.0, 900.0, 300.0, 200.0),
            size(px(400.0), px(250.0)),
            at(1520.0, 790.0, 400.0, 250.0),
            "growing near the edge moves the window back on screen",
        ),
        (
            at(0.0, 0.0, 2400.0, 1400.0),
            size(px(400.0), px(250.0)),
            at(0.0, 0.0, 1920.0, 1040.0),
            "a saved size larger than the work area is clamped to it",
        ),
        (
            at(0.0, 0.0, 300.0, 200.0),
            size(px(2400.0), px(1400.0)),
            at(0.0, 0.0, 1920.0, 1040.0),
            "a minimum larger than the work area yields to the screen",
        ),
    ];
    for (saved, min, expected, note) in cases {
        assert_eq!(at_least(saved, min, screen), expected, "{note}");
    }
}

#[test]
fn a_user_size_reads_back_only_when_the_user_sized_this_placement() {
    let centre = MonitorPlacement::center().memory_label();
    let min = size(px(100.0), px(100.0));
    let sized = WindowState {
        user_sized: true,
        ..state("memory-test-user-size", false)
    };
    let cases = [
        (
            Some(sized.clone()),
            centre.as_str(),
            Some(size(px(300.0), px(200.0))),
            "a dragged size",
        ),
        (
            Some(state("memory-test-user-size", false)),
            centre.as_str(),
            None,
            "a size nobody dragged",
        ),
        (
            Some(sized),
            "TopCenter:48",
            None,
            "a size from another placement",
        ),
        (None, centre.as_str(), None, "nothing saved"),
    ];
    for (saved, placement, expected, note) in cases {
        assert_eq!(
            fitted_user_size(saved, placement, min, None),
            expected,
            "{note}"
        );
    }
}

#[test]
fn a_user_size_grows_to_the_minimum_and_fits_the_work_area() {
    let centre = MonitorPlacement::center().memory_label();
    let saved = |width: f64, height: f64| {
        let mut state = state("memory-test-user-fit", false);
        state.user_sized = true;
        state.bounds_on_monitor.width = width;
        state.bounds_on_monitor.height = height;
        Some(state)
    };
    let min = size(px(400.0), px(250.0));
    let small = Bounds::new(point(px(1920.0), px(0.0)), size(px(1280.0), px(700.0)));
    let cases = [
        (saved(100.0, 50.0), None, min, "grows to the minimum"),
        (
            saved(1600.0, 1000.0),
            Some(small),
            size(px(1280.0), px(700.0)),
            "shrinks into a smaller work area",
        ),
        (
            saved(900.0, 600.0),
            Some(small),
            size(px(900.0), px(600.0)),
            "a size that fits is kept",
        ),
    ];
    for (saved, area, expected, note) in cases {
        assert_eq!(
            fitted_user_size(saved, &centre, min, area),
            Some(expected),
            "{note}"
        );
    }
}

#[test]
fn a_user_size_reads_a_queued_save_before_the_file() {
    let dir = tempfile::tempdir().unwrap();
    let store = WindowStateStore::new(dir.path());
    let key = "memory-test-latest";
    store.save(&state(key, true)).unwrap();
    let queued = state(key, false);
    PENDING
        .lock()
        .unwrap()
        .insert(key.into(), (store.clone(), queued.clone()));
    assert_eq!(latest(&store, key), Some(queued));
    flush_key(key);
    assert_eq!(latest(&store, key), Some(state(key, false)));
}
