use super::*;
use qol_window_state::{MonitorBounds, WindowRect};
use qol_window_state::{MonitorRef, WindowState, SCHEMA_VERSION};

fn state(key: &str, open: bool, owner_pid: u32, reopen: Option<Reopen>) -> WindowState {
    WindowState {
        version: SCHEMA_VERSION,
        key: key.into(),
        monitor: MonitorRef {
            id: "id".into(),
            connector: "DP-1".into(),
            bounds: MonitorBounds {
                x: 0.0,
                y: 0.0,
                width: 1920.0,
                height: 1080.0,
            },
        },
        bounds_on_monitor: WindowRect::default(),
        scale: 1.0,
        open,
        owner_pid,
        default_placement: "Center:24".into(),
        reopen,
        user_sized: false,
    }
}

fn cli_sessions() -> Reopen {
    Reopen::PluginAction {
        plugin: "qol-cli-sessions".into(),
        action: "open".into(),
    }
}

fn settings(page: &str) -> Reopen {
    Reopen::Settings {
        page: Some(page.into()),
    }
}

#[test]
fn only_open_windows_whose_owner_still_runs_are_reopened() {
    let dir = tempfile::tempdir().unwrap();
    let store = WindowStateStore::new(dir.path());
    for state in [
        state("qol-cli-sessions", true, 10, Some(cli_sessions())),
        state("settings", true, 20, Some(settings("__core-updates"))),
        state("closed", false, 10, Some(settings("closed"))),
        state("stale", true, 99, Some(settings("stale"))),
        state("no-reopen", true, 10, None),
    ] {
        store.save(&state).unwrap();
    }

    let windows = open_windows(&store, |pid| pid == 10 || pid == 20);

    assert_eq!(windows, vec![cli_sessions(), settings("__core-updates")]);
}

#[test]
fn the_reopen_list_is_taken_once() {
    let dir = tempfile::tempdir().unwrap();
    let windows = vec![cli_sessions(), settings("qol-launcher")];
    write_list(dir.path(), &windows, 1_000).unwrap();

    assert_eq!(take_list(dir.path(), 2_000), windows);
    assert_eq!(take_list(dir.path(), 2_000), Vec::new());
}

#[test]
fn a_stale_or_broken_reopen_list_is_dropped() {
    let dir = tempfile::tempdir().unwrap();
    write_list(dir.path(), &[cli_sessions()], 1_000).unwrap();
    let late = 1_000 + REOPEN_FRESH_FOR.as_millis() as u64 + 1;
    assert_eq!(take_list(dir.path(), late), Vec::new());
    assert!(!list_path(dir.path()).exists());

    std::fs::write(list_path(dir.path()), "{not json").unwrap();
    assert_eq!(take_list(dir.path(), 0), Vec::new());
    assert!(!list_path(dir.path()).exists());
}

fn explorer() -> Reopen {
    Reopen::PluginAction {
        plugin: "qol-explorer".into(),
        action: "open".into(),
    }
}

#[test]
fn a_window_whose_owner_outlived_the_restart_is_still_open() {
    let dir = tempfile::tempdir().unwrap();
    let store = WindowStateStore::new(dir.path());
    for state in [
        state("qol-explorer", true, 30, Some(explorer())),
        state("qol-cli-sessions", true, 10, Some(cli_sessions())),
        state("settings", false, 30, Some(settings("__core-updates"))),
    ] {
        store.save(&state).unwrap();
    }
    let survivor = |pid| pid == 30;

    assert!(open_by_running_owner(&store, &explorer(), survivor));
    assert!(!open_by_running_owner(&store, &cli_sessions(), survivor));
    assert!(!open_by_running_owner(
        &store,
        &settings("__core-updates"),
        survivor
    ));
}

#[test]
fn only_windows_that_closed_with_the_restart_are_reopened() {
    let windows = vec![explorer(), cli_sessions(), settings("__core-updates")];
    let settings_checks = std::cell::Cell::new(0);
    let still_open = |window: &Reopen| match window {
        Reopen::Settings { .. } => {
            settings_checks.set(settings_checks.get() + 1);
            settings_checks.get() == 1
        }
        window => *window == explorer(),
    };
    let mut reopened = Vec::new();

    reopen_once_closed(windows, Duration::from_millis(600), still_open, |window| {
        reopened.push(window)
    });

    assert_eq!(reopened, vec![cli_sessions(), settings("__core-updates")]);
}

#[test]
fn a_survivor_is_dropped_once_the_grace_runs_out() {
    let mut reopened = Vec::new();

    reopen_once_closed(
        vec![explorer()],
        Duration::ZERO,
        |_| true,
        |window| reopened.push(window),
    );

    assert!(reopened.is_empty());
}
