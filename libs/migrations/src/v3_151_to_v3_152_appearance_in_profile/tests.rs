use super::*;
use serde_json::json;
use std::path::PathBuf;

fn profile_dir(config: &Path, name: &str) -> PathBuf {
    let profile = config.join("profile").join(name);
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

fn legacy_theme() -> Value {
    json!({ "accent": "blue", "theme": null, "native_theme": "slate", "animations": "off" })
}

fn legacy_notifications() -> Value {
    json!({
        "enabled": false,
        "use_system_notifications": true,
        "handler": "both",
        "other_apps": "os",
        "placement": "top-left",
        "message_stay_seconds": 4.0,
    })
}

fn run(config: &Path) -> MigrationReport {
    let archive = tempfile::tempdir().unwrap();
    V3_151ToV3_152AppearanceInProfile
        .migrate(config, archive.path())
        .unwrap()
}

#[test]
fn every_profile_gets_the_look_and_the_notification_choices_in_their_scopes() {
    let config = tempfile::tempdir().unwrap();
    let profiles = [
        profile_dir(config.path(), "default"),
        profile_dir(config.path(), "work"),
    ];
    write(&config.path().join(THEME_SETTINGS_FILE), legacy_theme());
    write(
        &config.path().join(NOTIFICATIONS_SETTINGS_FILE),
        legacy_notifications(),
    );

    assert!(V3_151ToV3_152AppearanceInProfile
        .applies(config.path())
        .unwrap());
    let report = run(config.path());

    assert_eq!(report.archived.len(), 8);
    for profile in &profiles {
        assert_eq!(
            read(&THEME.path_in(profile, Scope::Core)),
            json!({ "accent": "blue", "native_theme": "slate", "animations": "off" })
        );
        assert_eq!(
            read(&NOTIFICATIONS.path_in(profile, Scope::Core)),
            json!({ "handler": "both", "placement": "top-left", "message_stay_seconds": 4.0 })
        );
        assert_eq!(
            read(&NOTIFICATIONS.path_in(profile, Scope::Os)),
            json!({ "other_apps": "os" })
        );
        assert_eq!(
            read(&NOTIFICATIONS.path_in(profile, Scope::Device)),
            json!({ "enabled": false })
        );
    }
    assert_eq!(
        read(&config.path().join(THEME_SETTINGS_FILE)),
        legacy_theme()
    );
    assert_eq!(
        read(&config.path().join(NOTIFICATIONS_SETTINGS_FILE)),
        legacy_notifications()
    );
}

#[test]
fn a_choice_the_profile_already_holds_wins_over_the_legacy_file() {
    let config = tempfile::tempdir().unwrap();
    let profile = profile_dir(config.path(), "default");
    write(&config.path().join(THEME_SETTINGS_FILE), legacy_theme());
    write(
        &config.path().join(NOTIFICATIONS_SETTINGS_FILE),
        legacy_notifications(),
    );
    let synced = THEME.path_in(&profile, Scope::Core);
    write(&synced, json!({ "accent": "green" }));

    run(config.path());

    assert_eq!(read(&synced), json!({ "accent": "green" }));
    assert_eq!(
        read(&NOTIFICATIONS.path_in(&profile, Scope::Device)),
        json!({ "enabled": false })
    );

    write(&synced, json!({ "accent": "violet" }));
    assert!(run(config.path()).archived.is_empty());
    assert_eq!(read(&synced), json!({ "accent": "violet" }));
}

#[test]
fn settings_never_chosen_are_left_out_so_another_computers_choice_can_sync_in() {
    let config = tempfile::tempdir().unwrap();
    let profile = profile_dir(config.path(), "default");
    write(
        &config.path().join(THEME_SETTINGS_FILE),
        json!({ "accent": null, "theme": null, "native_theme": null }),
    );
    write(
        &config.path().join(NOTIFICATIONS_SETTINGS_FILE),
        json!({
            "enabled": true,
            "use_system_notifications": false,
            "handler": "qol",
            "other_apps": "qol",
            "placement": null,
        }),
    );

    assert!(V3_151ToV3_152AppearanceInProfile
        .applies(config.path())
        .unwrap());
    assert!(run(config.path()).archived.is_empty());

    for file in [&THEME, &NOTIFICATIONS] {
        for scope in Scope::ALL {
            assert!(!file.path_in(&profile, scope).exists(), "{scope:?}");
        }
    }
}

#[test]
fn a_legacy_system_notifications_switch_becomes_the_handler() {
    let config = tempfile::tempdir().unwrap();
    let profile = profile_dir(config.path(), "default");
    write(
        &config.path().join(NOTIFICATIONS_SETTINGS_FILE),
        json!({ "use_system_notifications": true }),
    );

    run(config.path());

    assert_eq!(
        read(&NOTIFICATIONS.path_in(&profile, Scope::Core)),
        json!({ "handler": "both" })
    );
}

#[test]
fn it_waits_for_a_legacy_file_and_a_profile() {
    let config = tempfile::tempdir().unwrap();
    let migration = V3_151ToV3_152AppearanceInProfile;

    assert!(!migration.applies(config.path()).unwrap());
    write(&config.path().join(THEME_SETTINGS_FILE), legacy_theme());
    assert!(!migration.applies(config.path()).unwrap());
    profile_dir(config.path(), "default");
    assert!(migration.applies(config.path()).unwrap());

    std::fs::remove_file(config.path().join(THEME_SETTINGS_FILE)).unwrap();
    assert!(!migration.applies(config.path()).unwrap());
    write(&config.path().join(NOTIFICATIONS_SETTINGS_FILE), json!({}));
    assert!(migration.applies(config.path()).unwrap());
}

#[test]
fn an_unreadable_legacy_file_seeds_nothing() {
    let config = tempfile::tempdir().unwrap();
    let profile = profile_dir(config.path(), "default");
    std::fs::write(config.path().join(THEME_SETTINGS_FILE), "{ not json").unwrap();
    std::fs::write(config.path().join(NOTIFICATIONS_SETTINGS_FILE), "[1, 2]").unwrap();

    assert!(run(config.path()).archived.is_empty());
    assert!(!THEME.path_in(&profile, Scope::Core).exists());
}
