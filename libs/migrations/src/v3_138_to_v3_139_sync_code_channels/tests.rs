use super::*;
use crate::device_fields::slice_path;
use serde_json::{json, Value};

const VOICE_UID: &str = "uid-voice";

fn profile_dir(config: &Path) -> PathBuf {
    let profile = config.join("profile").join("default");
    std::fs::create_dir_all(profile.join("core")).unwrap();
    std::fs::write(profile.join("manifest.json"), "{}").unwrap();
    std::fs::write(
        profile.join("core/plugins.lock.json"),
        json!({ "plugins": [
            { "uid": VOICE_UID, "id": "qol-voice", "version": "1.0.0" },
        ]})
        .to_string(),
    )
    .unwrap();
    profile
}

fn write(path: &Path, value: Value) {
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(path, value.to_string()).unwrap();
}

fn read(path: &Path) -> Value {
    serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap()
}

fn os_dir(profile: &Path) -> PathBuf {
    profile.join("os").join(current_os_subdir())
}

#[test]
fn synced_recognition_routing_moves_into_the_device_slice_and_the_rest_stays() {
    let config = tempfile::tempdir().unwrap();
    let archive = tempfile::tempdir().unwrap();
    let profile = profile_dir(config.path());
    let os_slice = slice_path(os_dir(&profile), VOICE_UID);
    write(
        &os_slice,
        json!({ "recognition": {
            "provider": "websocket",
            "websocket_endpoint": "ws://attacker.example:5001",
            "language": "en",
        }}),
    );
    let migration = V3_138ToV3_139SyncCodeChannels;

    assert!(migration.applies(config.path()).unwrap());
    migration.migrate(config.path(), archive.path()).unwrap();

    assert_eq!(
        read(&os_slice),
        json!({ "recognition": { "language": "en" } })
    );
    assert_eq!(
        read(&slice_path(profile.join("device"), VOICE_UID)),
        json!({ "recognition": {
            "provider": "websocket",
            "websocket_endpoint": "ws://attacker.example:5001",
        }})
    );
    assert!(!migration.applies(config.path()).unwrap());
}

#[test]
fn a_pending_task_runner_copy_alone_makes_the_migration_apply() {
    let config = tempfile::tempdir().unwrap();
    let archive = tempfile::tempdir().unwrap();
    let profile = profile_dir(config.path());
    write(
        &os_dir(&profile).join(TASK_RUNNER_FILE),
        json!({ "tasks": [{ "name": "build" }] }),
    );
    let migration = V3_138ToV3_139SyncCodeChannels;

    assert!(migration.applies(config.path()).unwrap());
    migration.migrate(config.path(), archive.path()).unwrap();

    assert_eq!(
        read(&profile.join("device").join(TASK_RUNNER_FILE)),
        json!({ "tasks": [{ "name": "build" }] })
    );
    assert_eq!(
        read(&os_dir(&profile).join(TASK_RUNNER_FILE)),
        json!({ "tasks": [{ "name": "build" }] })
    );
    assert!(!migration.applies(config.path()).unwrap());
}

#[test]
fn an_existing_device_task_runner_file_is_never_overwritten() {
    let config = tempfile::tempdir().unwrap();
    let archive = tempfile::tempdir().unwrap();
    let profile = profile_dir(config.path());
    write(
        &os_dir(&profile).join(TASK_RUNNER_FILE),
        json!({ "tasks": ["synced"] }),
    );
    write(
        &profile.join("device").join(TASK_RUNNER_FILE),
        json!({ "tasks": ["local"] }),
    );
    let migration = V3_138ToV3_139SyncCodeChannels;

    assert!(!migration.applies(config.path()).unwrap());
    migration.migrate(config.path(), archive.path()).unwrap();

    assert_eq!(
        read(&profile.join("device").join(TASK_RUNNER_FILE)),
        json!({ "tasks": ["local"] })
    );
}

#[test]
fn a_second_run_changes_nothing() {
    let config = tempfile::tempdir().unwrap();
    let archive = tempfile::tempdir().unwrap();
    let profile = profile_dir(config.path());
    write(
        &slice_path(os_dir(&profile), VOICE_UID),
        json!({ "recognition": { "provider": "websocket" } }),
    );
    write(
        &os_dir(&profile).join(TASK_RUNNER_FILE),
        json!({ "tasks": [] }),
    );
    let migration = V3_138ToV3_139SyncCodeChannels;
    migration.migrate(config.path(), archive.path()).unwrap();
    let device_slice = read(&slice_path(profile.join("device"), VOICE_UID));
    let device_runner = read(&profile.join("device").join(TASK_RUNNER_FILE));

    let report = migration.migrate(config.path(), archive.path()).unwrap();

    assert!(report.archived.is_empty());
    assert_eq!(
        read(&slice_path(profile.join("device"), VOICE_UID)),
        device_slice
    );
    assert_eq!(
        read(&profile.join("device").join(TASK_RUNNER_FILE)),
        device_runner
    );
}

#[test]
fn nothing_applies_without_profiles() {
    let config = tempfile::tempdir().unwrap();

    assert!(!V3_138ToV3_139SyncCodeChannels
        .applies(config.path())
        .unwrap());
}
