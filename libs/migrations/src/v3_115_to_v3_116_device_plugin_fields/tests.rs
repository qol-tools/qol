use super::*;
use qol_platform::Os;
use serde_json::json;

const BLUETOOTH_UID: &str = "uid-bluetooth";
const SHOT_UID: &str = "uid-shot";

fn profile_dir(config: &Path) -> PathBuf {
    let profile = config.join("profile").join("default");
    std::fs::create_dir_all(profile.join("core")).unwrap();
    std::fs::write(profile.join("manifest.json"), "{}").unwrap();
    std::fs::write(
        profile.join("core/plugins.lock.json"),
        json!({ "plugins": [
            { "uid": BLUETOOTH_UID, "id": "qol-bluetooth", "version": "1.0.0" },
            { "uid": SHOT_UID, "id": "qol-shot", "version": "1.0.0" },
            { "uid": "uid-other", "id": "qol-clipboard", "version": "1.0.0" },
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

fn core(profile: &Path, uid: &str) -> PathBuf {
    slice_path(profile.join("core"), uid)
}

fn own_os(profile: &Path, uid: &str) -> PathBuf {
    slice_path(profile.join("os").join(current_os_subdir()), uid)
}

fn device(profile: &Path, uid: &str) -> PathBuf {
    slice_path(profile.join("device"), uid)
}

fn other_os() -> &'static str {
    if current_os_subdir() == Os::Linux.name() {
        Os::Macos.name()
    } else {
        Os::Linux.name()
    }
}

#[test]
fn synced_device_fields_move_into_the_device_slice() {
    let config = tempfile::tempdir().unwrap();
    let archive = tempfile::tempdir().unwrap();
    let profile = profile_dir(config.path());
    write(
        &core(&profile, BLUETOOTH_UID),
        json!({ "adapter": "10:91:D1:28:D5:A6", "managed_devices": ["74:68:59:7F:5F:E9"], "auto_reconnect": true }),
    );
    write(
        &own_os(&profile, SHOT_UID),
        json!({ "audio": { "enabled": true, "mic_device": "usb-mic", "system_device": "default" }, "video": { "crf": 18 } }),
    );
    let migration = V3_115ToV3_116DevicePluginFields;

    assert!(migration.applies(config.path()).unwrap());
    let report = migration.migrate(config.path(), archive.path()).unwrap();

    assert_eq!(
        read(&core(&profile, BLUETOOTH_UID)),
        json!({ "auto_reconnect": true })
    );
    assert_eq!(
        read(&device(&profile, BLUETOOTH_UID)),
        json!({ "adapter": "10:91:D1:28:D5:A6", "managed_devices": ["74:68:59:7F:5F:E9"] })
    );
    assert_eq!(
        read(&own_os(&profile, SHOT_UID)),
        json!({ "audio": { "enabled": true }, "video": { "crf": 18 } })
    );
    assert_eq!(
        read(&device(&profile, SHOT_UID)),
        json!({ "audio": { "mic_device": "usb-mic", "system_device": "default" } })
    );
    assert_eq!(report.archived.len(), 2);
    assert!(!migration.applies(config.path()).unwrap());
}

#[test]
fn an_existing_device_value_wins_and_the_synced_copy_is_dropped() {
    let config = tempfile::tempdir().unwrap();
    let archive = tempfile::tempdir().unwrap();
    let profile = profile_dir(config.path());
    write(
        &core(&profile, BLUETOOTH_UID),
        json!({ "adapter": "84:2F:57:43:D1:14", "retry_max_seconds": 60 }),
    );
    write(
        &device(&profile, BLUETOOTH_UID),
        json!({ "adapter": "10:91:D1:28:D5:A6" }),
    );

    V3_115ToV3_116DevicePluginFields
        .migrate(config.path(), archive.path())
        .unwrap();

    assert_eq!(
        read(&core(&profile, BLUETOOTH_UID)),
        json!({ "retry_max_seconds": 60 })
    );
    assert_eq!(
        read(&device(&profile, BLUETOOTH_UID)),
        json!({ "adapter": "10:91:D1:28:D5:A6" })
    );
}

#[test]
fn the_os_slice_value_beats_the_core_value_like_the_merge_does() {
    let config = tempfile::tempdir().unwrap();
    let archive = tempfile::tempdir().unwrap();
    let profile = profile_dir(config.path());
    write(
        &core(&profile, SHOT_UID),
        json!({ "audio": { "mic_device": "core-mic" } }),
    );
    write(
        &own_os(&profile, SHOT_UID),
        json!({ "audio": { "mic_device": "os-mic" } }),
    );

    V3_115ToV3_116DevicePluginFields
        .migrate(config.path(), archive.path())
        .unwrap();

    assert_eq!(read(&core(&profile, SHOT_UID)), json!({}));
    assert_eq!(read(&own_os(&profile, SHOT_UID)), json!({}));
    assert_eq!(
        read(&device(&profile, SHOT_UID)),
        json!({ "audio": { "mic_device": "os-mic" } })
    );
}

#[test]
fn other_os_buckets_and_unlisted_plugins_are_left_alone() {
    let config = tempfile::tempdir().unwrap();
    let archive = tempfile::tempdir().unwrap();
    let profile = profile_dir(config.path());
    let other_bucket = slice_path(profile.join("os").join(other_os()), SHOT_UID);
    let shot = json!({ "audio": { "mic_device": "mac-mic" } });
    let clipboard = json!({ "adapter": "not-a-device-field-here" });
    write(&other_bucket, shot.clone());
    write(&core(&profile, "uid-other"), clipboard.clone());
    let migration = V3_115ToV3_116DevicePluginFields;

    assert!(!migration.applies(config.path()).unwrap());
    let report = migration.migrate(config.path(), archive.path()).unwrap();

    assert!(report.archived.is_empty());
    assert_eq!(read(&other_bucket), shot);
    assert_eq!(read(&core(&profile, "uid-other")), clipboard);
    assert!(!device(&profile, SHOT_UID).exists());
}

#[test]
fn applies_is_false_without_profiles_or_lock() {
    let config = tempfile::tempdir().unwrap();
    assert!(!V3_115ToV3_116DevicePluginFields
        .applies(config.path())
        .unwrap());
    let profile = config.path().join("profile").join("default");
    std::fs::create_dir_all(&profile).unwrap();
    std::fs::write(profile.join("manifest.json"), "{}").unwrap();
    assert!(!V3_115ToV3_116DevicePluginFields
        .applies(config.path())
        .unwrap());
}

#[test]
fn a_rerun_after_the_device_write_finishes_the_strip_without_losing_the_value() {
    let config = tempfile::tempdir().unwrap();
    let archive = tempfile::tempdir().unwrap();
    let profile = profile_dir(config.path());
    write(
        &core(&profile, BLUETOOTH_UID),
        json!({ "adapter": "10:91:D1:28:D5:A6", "auto_reconnect": true }),
    );
    write(
        &device(&profile, BLUETOOTH_UID),
        json!({ "adapter": "10:91:D1:28:D5:A6" }),
    );
    let migration = V3_115ToV3_116DevicePluginFields;

    assert!(migration.applies(config.path()).unwrap());
    migration.migrate(config.path(), archive.path()).unwrap();

    assert_eq!(
        read(&core(&profile, BLUETOOTH_UID)),
        json!({ "auto_reconnect": true })
    );
    assert_eq!(
        read(&device(&profile, BLUETOOTH_UID)),
        json!({ "adapter": "10:91:D1:28:D5:A6" })
    );
    assert!(!migration.applies(config.path()).unwrap());
}
