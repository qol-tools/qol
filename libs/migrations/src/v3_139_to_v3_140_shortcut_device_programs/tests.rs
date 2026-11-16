use super::*;
use serde_json::json;

fn profile_dir(config: &Path) -> PathBuf {
    let profile = config.join("profile").join("default");
    std::fs::create_dir_all(&profile).unwrap();
    std::fs::write(profile.join("manifest.json"), "{}").unwrap();
    profile
}

fn write(path: &Path, value: Value) {
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(path, value.to_string()).unwrap();
}

fn read(path: &Path) -> Value {
    serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap()
}

fn shortcuts_file(profile: &Path) -> PathBuf {
    profile
        .join("os")
        .join(current_os_subdir())
        .join(SHORTCUTS_FILE)
}

fn programs_file(profile: &Path) -> PathBuf {
    profile.join("device").join(PROGRAMS_FILE)
}

fn sample_shortcuts() -> Value {
    json!({ "shortcuts": [
        {
            "id": "editor",
            "name": "Editor",
            "action": { "type": "launch_app", "app": { "type": "name", "name": "gedit" } }
        },
        {
            "id": "docs",
            "name": "Docs",
            "action": {
                "type": "open_url",
                "url": "https://x.io",
                "browser_override": { "type": "path", "path": "/usr/bin/firefox" }
            }
        },
        {
            "id": "plain",
            "name": "Plain",
            "action": { "type": "open_url", "url": "https://x.io" }
        },
        {
            "id": "lights",
            "name": "Lights",
            "action": { "type": "plugin_action", "plugin_id": "qol-lights", "action": "toggle" }
        },
    ]})
}

#[test]
fn the_device_file_is_seeded_from_the_shortcut_programs() {
    let config = tempfile::tempdir().unwrap();
    let archive = tempfile::tempdir().unwrap();
    let profile = profile_dir(config.path());
    write(&shortcuts_file(&profile), sample_shortcuts());
    let migration = V3_139ToV3_140ShortcutDevicePrograms;

    assert!(migration.applies(config.path()).unwrap());
    migration.migrate(config.path(), archive.path()).unwrap();

    assert_eq!(
        read(&programs_file(&profile)),
        json!({
            "editor": { "app": { "type": "name", "name": "gedit" } },
            "docs": { "browser_override": { "type": "path", "path": "/usr/bin/firefox" } },
        })
    );
    assert_eq!(read(&shortcuts_file(&profile)), sample_shortcuts());
}

#[test]
fn a_second_run_changes_nothing() {
    let config = tempfile::tempdir().unwrap();
    let archive = tempfile::tempdir().unwrap();
    let profile = profile_dir(config.path());
    write(&shortcuts_file(&profile), sample_shortcuts());
    let migration = V3_139ToV3_140ShortcutDevicePrograms;
    migration.migrate(config.path(), archive.path()).unwrap();
    let seeded = read(&programs_file(&profile));

    assert!(!migration.applies(config.path()).unwrap());
    let report = migration.migrate(config.path(), archive.path()).unwrap();

    assert!(report.archived.is_empty());
    assert_eq!(read(&programs_file(&profile)), seeded);
}

#[test]
fn an_existing_device_file_is_never_overwritten() {
    let config = tempfile::tempdir().unwrap();
    let archive = tempfile::tempdir().unwrap();
    let profile = profile_dir(config.path());
    write(&shortcuts_file(&profile), sample_shortcuts());
    write(&programs_file(&profile), json!({}));
    let migration = V3_139ToV3_140ShortcutDevicePrograms;

    assert!(!migration.applies(config.path()).unwrap());
    migration.migrate(config.path(), archive.path()).unwrap();

    assert_eq!(read(&programs_file(&profile)), json!({}));
}

#[test]
fn nothing_applies_without_a_shortcuts_file_or_profiles() {
    let config = tempfile::tempdir().unwrap();
    let migration = V3_139ToV3_140ShortcutDevicePrograms;
    assert!(!migration.applies(config.path()).unwrap());

    profile_dir(config.path());

    assert!(!migration.applies(config.path()).unwrap());
}

#[test]
fn an_unreadable_shortcuts_file_seeds_an_empty_device_file() {
    let config = tempfile::tempdir().unwrap();
    let archive = tempfile::tempdir().unwrap();
    let profile = profile_dir(config.path());
    let path = shortcuts_file(&profile);
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(&path, "not json").unwrap();

    V3_139ToV3_140ShortcutDevicePrograms
        .migrate(config.path(), archive.path())
        .unwrap();

    assert_eq!(read(&programs_file(&profile)), json!({}));
}
