use super::*;
use std::fs;

#[cfg(any(target_os = "linux", target_os = "macos"))]
mod swaps;

#[test]
fn keep_tray_refuses_while_a_tray_build_runs_whichever_pull_request_it_came_from() {
    let running = State {
        pull_request: Some(412),
        tray: Some("/t/qol-tray".into()),
        ..State::default()
    };
    let plugins_only = State {
        pull_request: Some(412),
        ..State::default()
    };
    let refused = |tried| {
        Some(RestartsTray::Running {
            tried,
            running: 412,
        })
    };
    assert_eq!(running_tray_refusal(413, true, &running), refused(413));
    assert_eq!(running_tray_refusal(412, true, &running), refused(412));
    assert_eq!(running_tray_refusal(413, false, &running), None);
    assert_eq!(running_tray_refusal(413, true, &plugins_only), None);
    assert_eq!(running_tray_refusal(413, true, &State::default()), None);
}

#[test]
fn the_refusals_name_the_pull_requests() {
    assert!(RestartsTray::Carries(412)
        .to_string()
        .contains("#412 also replaces qol-tray"));
    assert!(RestartsTray::Running {
        tried: 413,
        running: 412
    }
    .to_string()
    .contains("runs pull request #412's build"));
}

#[test]
fn the_installed_tray_is_the_first_one_on_disk() {
    let root = tempfile::tempdir().unwrap();
    let user = root
        .path()
        .join("home/Applications/QoL Tray.app/Contents/MacOS/qol-tray");
    let system = root
        .path()
        .join("Applications/QoL Tray.app/Contents/MacOS/qol-tray");
    let binaries = [user.clone(), system.clone()];
    assert_eq!(first_installed(&binaries), Some(user.clone()));
    fs::create_dir_all(system.parent().unwrap()).unwrap();
    fs::write(&system, b"tray").unwrap();
    assert_eq!(first_installed(&binaries), Some(system.clone()));
    fs::create_dir_all(user.parent().unwrap()).unwrap();
    fs::write(&user, b"tray").unwrap();
    assert_eq!(first_installed(&binaries), Some(user));
    assert_eq!(first_installed(&[]), None);
}

#[test]
fn a_recorded_install_wins_over_the_default_places() {
    let recorded = PathBuf::from("/Applications/QoL Tray.app/Contents/MacOS/qol-tray");
    let state = State {
        installed_tray: Some(recorded.clone()),
        ..State::default()
    };
    assert_eq!(installed_tray(&state).unwrap(), recorded);
}

#[test]
fn plain_name_refuses_paths() {
    for good in ["qol-cli-sessions", "qol-cli-sessions-v0.127.1"] {
        assert!(plain_name(good), "{good}");
    }
    for bad in ["", ".", "..", "a/b", "../a", "a\\b", "/abs"] {
        assert!(!plain_name(bad), "{bad}");
    }
}
