use super::*;

fn area(x: f32, y: f32, width: f32, height: f32) -> MonitorBounds {
    MonitorBounds {
        x,
        y,
        width,
        height,
    }
}

fn rect(x: f64, y: f64, width: f64, height: f64) -> WindowRect {
    WindowRect {
        x,
        y,
        width,
        height,
    }
}

fn monitor(id: &str, connector: &str, bounds: MonitorBounds, primary: bool) -> Monitor {
    Monitor {
        id: id.into(),
        connector: connector.into(),
        bounds,
        work_area: bounds,
        primary,
    }
}

fn layout() -> Vec<Monitor> {
    vec![
        monitor("left", "DP-1", area(0.0, 0.0, 1920.0, 1080.0), true),
        monitor("right", "HDMI-1", area(1920.0, 0.0, 2560.0, 1440.0), false),
    ]
}

fn saved(monitor: MonitorRef, bounds_on_monitor: WindowRect) -> WindowState {
    WindowState {
        version: SCHEMA_VERSION,
        key: "settings".into(),
        monitor,
        bounds_on_monitor,
        scale: 1.0,
        open: true,
        owner_pid: 42,
        default_placement: "center".into(),
        reopen: Some(Reopen::Settings {
            page: Some("__core-updates".into()),
        }),
    }
}

fn reference(id: &str, connector: &str, bounds: MonitorBounds) -> MonitorRef {
    MonitorRef {
        id: id.into(),
        connector: connector.into(),
        bounds,
    }
}

#[test]
fn clamp_table() {
    let screen = area(0.0, 0.0, 1000.0, 800.0);
    let cases = [
        (
            rect(100.0, 100.0, 300.0, 200.0),
            rect(100.0, 100.0, 300.0, 200.0),
        ),
        (
            rect(-50.0, 100.0, 300.0, 200.0),
            rect(0.0, 100.0, 300.0, 200.0),
        ),
        (
            rect(900.0, 700.0, 300.0, 200.0),
            rect(700.0, 600.0, 300.0, 200.0),
        ),
        (
            rect(10.0, 10.0, 1200.0, 900.0),
            rect(0.0, 0.0, 1000.0, 800.0),
        ),
    ];
    for (window, expected) in cases {
        assert_eq!(clamp_into(window, screen), expected, "{window:?}");
    }
}

#[test]
fn clamp_respects_an_offset_work_area() {
    let panel_on_top = area(1920.0, 32.0, 2560.0, 1408.0);
    assert_eq!(
        clamp_into(rect(2000.0, 0.0, 400.0, 300.0), panel_on_top),
        rect(2000.0, 32.0, 400.0, 300.0)
    );
}

#[test]
fn locate_picks_the_monitor_holding_the_centre_and_stores_relative_bounds() {
    let (on, relative) = locate(rect(2000.0, 100.0, 400.0, 300.0), &layout()).unwrap();
    assert_eq!(on.id, "right");
    assert_eq!(relative, rect(80.0, 100.0, 400.0, 300.0));
    assert!(locate(rect(0.0, 0.0, 1.0, 1.0), &[]).is_none());
}

#[test]
fn resolve_follows_the_monitor_by_identity_after_it_moved() {
    let state = saved(
        reference("right", "HDMI-1", area(1920.0, 0.0, 2560.0, 1440.0)),
        rect(80.0, 100.0, 400.0, 300.0),
    );
    let swapped = vec![
        monitor("right", "HDMI-1", area(0.0, 0.0, 2560.0, 1440.0), false),
        monitor("left", "DP-1", area(2560.0, 0.0, 1920.0, 1080.0), true),
    ];
    assert_eq!(
        resolve(&state, &swapped),
        Some(rect(80.0, 100.0, 400.0, 300.0))
    );
}

#[test]
fn resolve_falls_back_to_the_connector_when_the_identity_changed() {
    let state = saved(
        reference("old-edid", "HDMI-1", area(1920.0, 0.0, 2560.0, 1440.0)),
        rect(80.0, 100.0, 400.0, 300.0),
    );
    assert_eq!(
        resolve(&state, &layout()),
        Some(rect(2000.0, 100.0, 400.0, 300.0))
    );
}

#[test]
fn resolve_moves_to_the_nearest_monitor_when_the_saved_one_is_gone() {
    let state = saved(
        reference("gone", "DP-3", area(4480.0, 0.0, 1920.0, 1080.0)),
        rect(1700.0, 900.0, 400.0, 300.0),
    );
    assert_eq!(
        resolve(&state, &layout()),
        Some(rect(3620.0, 900.0, 400.0, 300.0))
    );
}

#[test]
fn resolve_prefers_the_primary_when_monitors_are_equally_near() {
    let state = saved(
        reference("gone", "DP-3", area(-200.0, 2000.0, 1920.0, 1080.0)),
        rect(0.0, 10.0, 400.0, 300.0),
    );
    let stacked = vec![
        monitor("a", "DP-1", area(-1000.0, 0.0, 1000.0, 1000.0), false),
        monitor("b", "DP-2", area(0.0, 0.0, 1000.0, 1000.0), true),
    ];
    let resolved = resolve(&state, &stacked).unwrap();
    assert_eq!(resolved, rect(0.0, 10.0, 400.0, 300.0));
    assert!(resolve(&state, &[]).is_none());
}

#[test]
fn state_round_trips_through_the_store() {
    let dir = tempfile::tempdir().unwrap();
    let store = WindowStateStore::new(dir.path().join("window-state"));
    let state = saved(
        reference("left", "DP-1", area(0.0, 0.0, 1920.0, 1080.0)),
        rect(80.0, 100.0, 400.0, 300.0),
    );
    store.save(&state).unwrap();
    assert_eq!(store.load("settings"), Some(state.clone()));

    let cli = WindowState {
        key: "qol-cli-sessions".into(),
        reopen: Some(Reopen::PluginAction {
            plugin: "qol-cli-sessions".into(),
            action: "open".into(),
        }),
        ..state.clone()
    };
    store.save(&cli).unwrap();
    assert_eq!(store.list(), vec![cli, state]);
    let names: Vec<_> = std::fs::read_dir(store.dir())
        .unwrap()
        .map(|entry| entry.unwrap().file_name().into_string().unwrap())
        .collect();
    assert_eq!(names.len(), 2, "{names:?}");
}

#[test]
fn store_rejects_unsafe_keys_and_foreign_files() {
    let dir = tempfile::tempdir().unwrap();
    let store = WindowStateStore::new(dir.path());
    let mut state = saved(
        reference("left", "DP-1", area(0.0, 0.0, 1920.0, 1080.0)),
        rect(0.0, 0.0, 10.0, 10.0),
    );
    state.key = "../escape".into();
    assert!(store.save(&state).is_err());
    assert_eq!(store.load("../escape"), None);

    std::fs::write(dir.path().join("broken.json"), "{not json").unwrap();
    std::fs::write(
        dir.path().join("renamed.json"),
        serde_json::to_vec(&WindowState {
            key: "other".into(),
            ..state.clone()
        })
        .unwrap(),
    )
    .unwrap();
    assert_eq!(store.list(), Vec::new());
}

#[test]
fn key_validation_table() {
    for (key, valid) in [
        ("settings", true),
        ("qol-cli-sessions", true),
        ("a_b9", true),
        ("", false),
        ("Settings", false),
        ("a/b", false),
        ("a.json", false),
    ] {
        assert_eq!(is_valid_key(key), valid, "{key}");
    }
}

#[test]
fn reopen_serialises_with_a_kind_tag() {
    let json = serde_json::to_value(Reopen::PluginAction {
        plugin: "qol-cli-sessions".into(),
        action: "open".into(),
    })
    .unwrap();
    assert_eq!(
        json,
        serde_json::json!({"kind": "plugin_action", "plugin": "qol-cli-sessions", "action": "open"})
    );
}
