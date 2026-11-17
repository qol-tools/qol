use super::*;
use crate::device_fields::slice_path;
use serde_json::json;

const UID: &str = "uid-bluetooth";

fn profile_dir(config: &Path) -> PathBuf {
    let profile = config.join("profile").join("default");
    std::fs::create_dir_all(profile.join("core")).unwrap();
    std::fs::write(profile.join("manifest.json"), "{}").unwrap();
    std::fs::write(
        profile.join("core/plugins.lock.json"),
        json!({ "plugins": [
            { "uid": UID, "id": "qol-bluetooth", "version": "1.0.0" },
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

#[test]
fn a_switch_stored_off_clears_the_chosen_devices_and_drops_the_key() {
    let config = tempfile::tempdir().unwrap();
    let archive = tempfile::tempdir().unwrap();
    let profile = profile_dir(config.path());
    let core = slice_path(profile.join("core"), UID);
    let device = slice_path(profile.join("device"), UID);
    write(
        &core,
        json!({ "auto_reconnect": false, "power_on_adapter": true }),
    );
    write(
        &device,
        json!({ "adapter": "10:91", "managed_devices": ["74:68"] }),
    );
    let migration = V3_140ToV3_141BluetoothReconnectPerDevice;

    assert!(migration.applies(config.path()).unwrap());
    let report = migration.migrate(config.path(), archive.path()).unwrap();

    assert_eq!(read(&core), json!({ "power_on_adapter": true }));
    assert_eq!(read(&device), json!({ "adapter": "10:91" }));
    assert_eq!(report.archived.len(), 2);
    assert!(!migration.applies(config.path()).unwrap());
}

#[test]
fn a_switch_stored_on_keeps_the_chosen_devices_and_drops_the_key() {
    let config = tempfile::tempdir().unwrap();
    let archive = tempfile::tempdir().unwrap();
    let profile = profile_dir(config.path());
    let core = slice_path(profile.join("core"), UID);
    let device = slice_path(profile.join("device"), UID);
    write(&core, json!({ "auto_reconnect": true }));
    write(&device, json!({ "managed_devices": ["74:68"] }));
    let migration = V3_140ToV3_141BluetoothReconnectPerDevice;

    assert!(migration.applies(config.path()).unwrap());
    let report = migration.migrate(config.path(), archive.path()).unwrap();

    assert_eq!(read(&core), json!({}));
    assert_eq!(read(&device), json!({ "managed_devices": ["74:68"] }));
    assert_eq!(report.archived.len(), 1);
    assert!(!migration.applies(config.path()).unwrap());
}

#[test]
fn a_config_without_the_switch_is_left_alone() {
    let config = tempfile::tempdir().unwrap();
    let profile = profile_dir(config.path());
    write(
        &slice_path(profile.join("device"), UID),
        json!({ "managed_devices": ["74:68"] }),
    );

    assert!(!V3_140ToV3_141BluetoothReconnectPerDevice
        .applies(config.path())
        .unwrap());
}
