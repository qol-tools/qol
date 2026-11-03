use super::*;
use crate::device_fields::slice_path;
use crate::fs_util::current_os_subdir;
use serde_json::{json, Value};
use std::path::PathBuf;

const KEYREMAP_UID: &str = "uid-keyremap";

fn profile_dir(config: &Path) -> PathBuf {
    let profile = config.join("profile").join("default");
    std::fs::create_dir_all(profile.join("core")).unwrap();
    std::fs::write(profile.join("manifest.json"), "{}").unwrap();
    std::fs::write(
        profile.join("core/plugins.lock.json"),
        json!({ "plugins": [
            { "uid": KEYREMAP_UID, "id": "qol-keyremap", "version": "1.0.0" },
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
fn a_synced_key_backend_moves_into_the_device_slice_and_the_rules_stay() {
    let config = tempfile::tempdir().unwrap();
    let archive = tempfile::tempdir().unwrap();
    let profile = profile_dir(config.path());
    let core = slice_path(profile.join("core"), KEYREMAP_UID);
    write(
        &core,
        json!({ "key_backend": "karabiner_elements", "key_rules": [{ "from": "caps_lock" }] }),
    );
    let migration = V3_137ToV3_138KeyremapBackendDevice;

    assert!(migration.applies(config.path()).unwrap());
    migration.migrate(config.path(), archive.path()).unwrap();

    assert_eq!(
        read(&core),
        json!({ "key_rules": [{ "from": "caps_lock" }] })
    );
    assert_eq!(
        read(&slice_path(profile.join("device"), KEYREMAP_UID)),
        json!({ "key_backend": "karabiner_elements" })
    );
    assert!(!migration.applies(config.path()).unwrap());
}

#[test]
fn the_os_slice_choice_wins_over_the_core_one() {
    let config = tempfile::tempdir().unwrap();
    let archive = tempfile::tempdir().unwrap();
    let profile = profile_dir(config.path());
    write(
        &slice_path(profile.join("core"), KEYREMAP_UID),
        json!({ "key_backend": "event_tap" }),
    );
    write(
        &slice_path(profile.join("os").join(current_os_subdir()), KEYREMAP_UID),
        json!({ "key_backend": "karabiner_elements" }),
    );

    V3_137ToV3_138KeyremapBackendDevice
        .migrate(config.path(), archive.path())
        .unwrap();

    assert_eq!(
        read(&slice_path(profile.join("device"), KEYREMAP_UID)),
        json!({ "key_backend": "karabiner_elements" })
    );
}
