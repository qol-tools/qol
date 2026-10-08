use super::*;
use qol_windowing::window_state::{MonitorRef, WindowState, SCHEMA_VERSION};
use qol_windowing::{MonitorBounds, WindowRect};

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
