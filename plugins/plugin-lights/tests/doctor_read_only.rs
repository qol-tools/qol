use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

use qol_headless::DoctorReport;

#[derive(Debug, Clone, PartialEq, Eq)]
enum SnapshotEntry {
    Directory,
    File(Vec<u8>),
    Other,
}

#[test]
fn doctor_never_changes_config_key_state_or_runtime_paths() {
    let root = tempfile::tempdir().unwrap();
    let data_home = root.path().join("data");
    let config_home = root.path().join("config");
    let plugin_config = data_home
        .join("qol-tray")
        .join("plugins")
        .join("plugin-lights")
        .join("config.json");
    let daemon_socket = root.path().join("daemon.sock");
    let state_socket = root.path().join("state.sock");
    let state_sentinel = root.path().join("state-sentinel");
    fs::create_dir_all(plugin_config.parent().unwrap()).unwrap();
    fs::create_dir_all(&config_home).unwrap();
    fs::write(
        &plugin_config,
        br#"{"backend":{"serial_port":"auto","network_key":"auto"}}"#,
    )
    .unwrap();
    fs::write(&state_sentinel, b"unchanged-state").unwrap();
    let before = snapshot(root.path());

    for args in [["--json", "doctor"], ["doctor", "--json"]] {
        let output = run_doctor(
            root.path(),
            &data_home,
            &config_home,
            &daemon_socket,
            &state_socket,
            args,
        );
        let report: DoctorReport =
            serde_json::from_slice(&output.stdout).expect("doctor must return JSON");

        assert!(
            output.status.success(),
            "stderr={}",
            String::from_utf8_lossy(&output.stderr)
        );
        assert!(output.stderr.is_empty());
        assert_eq!(report.plugin_id, "plugin-lights");
        assert_eq!(
            report
                .checks
                .iter()
                .map(|check| check.id.as_str())
                .collect::<Vec<_>>(),
            [
                "platform_supported",
                "config_readable",
                "coordinator_candidates",
            ]
        );
    }

    assert_eq!(snapshot(root.path()), before);
    assert_eq!(
        fs::read_to_string(&plugin_config).unwrap(),
        r#"{"backend":{"serial_port":"auto","network_key":"auto"}}"#
    );
    assert_eq!(
        fs::read_to_string(&state_sentinel).unwrap(),
        "unchanged-state"
    );
    assert!(!daemon_socket.exists());
    assert!(!state_socket.exists());
}

fn run_doctor(
    root: &Path,
    data_home: &Path,
    config_home: &Path,
    daemon_socket: &Path,
    state_socket: &Path,
    args: [&str; 2],
) -> Output {
    Command::new(env!("CARGO_BIN_EXE_plugin-lights"))
        .args(args)
        .current_dir(root)
        .env("XDG_DATA_HOME", data_home)
        .env("XDG_CONFIG_HOME", config_home)
        .env(qol_conventions::ENV_PLUGIN_ID, "plugin-lights")
        .env(qol_conventions::ENV_DAEMON_SOCKET, daemon_socket)
        .env(qol_conventions::ENV_STATE_SOCKET, state_socket)
        .env_remove(qol_conventions::ENV_INSTALL_ID)
        .output()
        .unwrap()
}

fn snapshot(root: &Path) -> BTreeMap<PathBuf, SnapshotEntry> {
    let mut entries = BTreeMap::new();
    collect_snapshot(root, root, &mut entries);
    entries
}

fn collect_snapshot(root: &Path, directory: &Path, entries: &mut BTreeMap<PathBuf, SnapshotEntry>) {
    let mut children = fs::read_dir(directory)
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .collect::<Vec<_>>();
    children.sort();

    for path in children {
        let relative = path.strip_prefix(root).unwrap().to_path_buf();
        let metadata = fs::symlink_metadata(&path).unwrap();
        if metadata.is_dir() {
            entries.insert(relative, SnapshotEntry::Directory);
            collect_snapshot(root, &path, entries);
            continue;
        }
        if metadata.is_file() {
            entries.insert(relative, SnapshotEntry::File(fs::read(&path).unwrap()));
            continue;
        }
        entries.insert(relative, SnapshotEntry::Other);
    }
}
