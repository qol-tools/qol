use super::*;
use std::fs;
use tempfile::TempDir;

const VALID_MANIFEST: &str = r#"
[plugin]
name = "Test Plugin"
description = "A test plugin"
version = "1.0.0"

[menu]
label = "Test"
items = []
"#;

const RUNTIME_MANIFEST: &str = r#"
[plugin]
name = "Runtime Plugin"
description = "A runtime plugin"
version = "1.0.0"

[menu]
label = "Runtime"
items = []

[runtime]
command = "runtime-plugin"
"#;

const DAEMON_MANIFEST: &str = r#"
[plugin]
name = "Daemon Plugin"
description = "A daemon plugin"
version = "1.0.0"

[menu]
label = "Daemon"
items = []

[daemon]
enabled = true
command = "daemon-plugin"
"#;

const DISABLED_DAEMON_MANIFEST: &str = r#"
[plugin]
name = "Disabled Daemon Plugin"
description = "A daemon plugin"
version = "1.0.0"

[menu]
label = "Daemon"
items = []

[daemon]
enabled = false
command = "daemon-plugin"
"#;

#[test]
fn default_plugin_dir_ends_with_qol_tray_plugins() {
    let dir = PluginLoader::default_plugin_dir().unwrap();
    assert!(dir.ends_with("plugins"));
    assert!(dir.to_string_lossy().contains("qol-tray"));
}

#[test]
fn load_from_dir_returns_empty_when_no_valid_plugins() {
    let temp_dir = TempDir::new().unwrap();

    assert!(PluginLoader::load_from_dir(temp_dir.path())
        .unwrap()
        .is_empty());

    fs::write(temp_dir.path().join("file.txt"), "content").unwrap();
    assert!(PluginLoader::load_from_dir(temp_dir.path())
        .unwrap()
        .is_empty());

    fs::create_dir(temp_dir.path().join("no-manifest")).unwrap();
    assert!(PluginLoader::load_from_dir(temp_dir.path())
        .unwrap()
        .is_empty());

    let nonexistent = std::path::PathBuf::from("/nonexistent/path");
    assert!(PluginLoader::load_from_dir(&nonexistent)
        .unwrap()
        .is_empty());
}

#[test]
fn load_from_dir_loads_valid_plugin() {
    let temp_dir = TempDir::new().unwrap();
    let plugin_dir = temp_dir.path().join("test-plugin");
    fs::create_dir(&plugin_dir).unwrap();
    fs::write(plugin_dir.join("plugin.toml"), VALID_MANIFEST).unwrap();

    let result = PluginLoader::load_from_dir(temp_dir.path()).unwrap();

    assert_eq!(result.len(), 1);
    assert_eq!(result[0].id.as_str(), "test-plugin");
    assert_eq!(result[0].manifest.plugin.name, "Test Plugin");
}

#[test]
fn load_plugin_fails_for_invalid_dirs() {
    let temp_dir = TempDir::new().unwrap();
    let cases = [("no-toml", None), ("bad-toml", Some("invalid {{{"))];
    for (name, content) in cases {
        let dir = temp_dir.path().join(name);
        fs::create_dir(&dir).unwrap();
        if let Some(c) = content {
            fs::write(dir.join("plugin.toml"), c).unwrap();
        }
        assert!(PluginLoader::load_plugin(&dir).is_err(), "case: {name}");
    }
}

#[test]
fn load_plugin_extracts_id_from_directory_name() {
    let temp_dir = TempDir::new().unwrap();
    let plugin_dir = temp_dir.path().join("my-custom-plugin");
    fs::create_dir(&plugin_dir).unwrap();
    fs::write(plugin_dir.join("plugin.toml"), VALID_MANIFEST).unwrap();

    let plugin = PluginLoader::load_plugin(&plugin_dir).unwrap();

    assert_eq!(plugin.id.as_str(), "my-custom-plugin");
}

#[test]
fn load_plugin_parses_manifest_fields() {
    let temp_dir = TempDir::new().unwrap();
    fs::write(temp_dir.path().join("plugin.toml"), VALID_MANIFEST).unwrap();

    let plugin = PluginLoader::load_plugin(temp_dir.path()).unwrap();

    assert_eq!(plugin.manifest.plugin.name, "Test Plugin");
    assert_eq!(plugin.manifest.plugin.description, "A test plugin");
    assert_eq!(plugin.manifest.plugin.version, "1.0.0");
}

#[test]
fn load_from_dir_skips_backup_directories() {
    let temp_dir = TempDir::new().unwrap();

    let backup_dir = temp_dir.path().join("plugin-foo.backup");
    fs::create_dir(&backup_dir).unwrap();
    fs::write(backup_dir.join("plugin.toml"), VALID_MANIFEST).unwrap();

    let result = PluginLoader::load_from_dir(temp_dir.path()).unwrap();

    assert!(result.is_empty());
}

#[test]
fn load_from_dir_handles_mixed_valid_and_invalid() {
    let temp_dir = TempDir::new().unwrap();

    let valid = temp_dir.path().join("valid-plugin");
    fs::create_dir(&valid).unwrap();
    fs::write(valid.join("plugin.toml"), VALID_MANIFEST).unwrap();

    let no_manifest = temp_dir.path().join("no-manifest");
    fs::create_dir(&no_manifest).unwrap();

    let invalid_toml = temp_dir.path().join("invalid-toml");
    fs::create_dir(&invalid_toml).unwrap();
    fs::write(invalid_toml.join("plugin.toml"), "not valid toml {{{").unwrap();

    fs::write(temp_dir.path().join("just-a-file.txt"), "content").unwrap();

    let result = PluginLoader::load_from_dir(temp_dir.path()).unwrap();

    assert_eq!(result.len(), 1);
    assert_eq!(result[0].id.as_str(), "valid-plugin");
}

#[test]
fn load_plugin_handles_special_characters_in_id() {
    let temp_dir = TempDir::new().unwrap();
    let cases = ["plugin-with-dashes", "plugin_with_underscores", "plugin123"];

    for name in cases {
        let plugin_dir = temp_dir.path().join(name);
        fs::create_dir(&plugin_dir).unwrap();
        fs::write(plugin_dir.join("plugin.toml"), VALID_MANIFEST).unwrap();

        let plugin = PluginLoader::load_plugin(&plugin_dir).unwrap();
        assert_eq!(plugin.id.as_str(), name, "plugin name: {}", name);
    }
}

#[test]
fn load_plugin_rejects_missing_runtime_binary() {
    let temp_dir = TempDir::new().unwrap();
    fs::write(temp_dir.path().join("plugin.toml"), RUNTIME_MANIFEST).unwrap();

    let error = PluginLoader::load_plugin(temp_dir.path()).unwrap_err();
    assert!(error.to_string().contains("runtime.command"));
}

#[test]
fn load_plugin_accepts_runtime_binary_when_present() {
    let temp_dir = TempDir::new().unwrap();
    fs::write(temp_dir.path().join("plugin.toml"), RUNTIME_MANIFEST).unwrap();
    fs::write(temp_dir.path().join("runtime-plugin"), b"binary").unwrap();

    let plugin = PluginLoader::load_plugin(temp_dir.path()).unwrap();
    assert_eq!(
        plugin.id,
        temp_dir
            .path()
            .file_name()
            .unwrap()
            .to_str()
            .unwrap()
            .into()
    );
}

#[test]
fn load_plugin_rejects_missing_daemon_binary_when_enabled() {
    let temp_dir = TempDir::new().unwrap();
    fs::write(temp_dir.path().join("plugin.toml"), DAEMON_MANIFEST).unwrap();

    let error = PluginLoader::load_plugin(temp_dir.path()).unwrap_err();
    assert!(error.to_string().contains("daemon.command"));
}

#[test]
fn load_plugin_allows_missing_daemon_binary_when_disabled() {
    let temp_dir = TempDir::new().unwrap();
    fs::write(
        temp_dir.path().join("plugin.toml"),
        DISABLED_DAEMON_MANIFEST,
    )
    .unwrap();

    let plugin = PluginLoader::load_plugin(temp_dir.path()).unwrap();
    assert_eq!(
        plugin.id,
        temp_dir
            .path()
            .file_name()
            .unwrap()
            .to_str()
            .unwrap()
            .into()
    );
}

#[test]
fn load_plugin_skips_binary_check_for_unsupported_platform() {
    let temp_dir = TempDir::new().unwrap();
    fs::write(
        temp_dir.path().join("plugin.toml"),
        unsupported_platform_manifest(),
    )
    .unwrap();
    let plugin = PluginLoader::load_plugin(temp_dir.path()).unwrap();
    assert!(!plugin.manifest.plugin.supports_current_platform());
}

fn unsupported_platform_manifest() -> String {
    let platform = if cfg!(target_os = "linux") {
        "windows"
    } else {
        "linux"
    };
    format!(
        r#"[plugin]
name = "Unsupported"
description = ""
version = "1.0.0"
platforms = ["{platform}"]

[menu]
label = "Unsupported"
items = []

[runtime]
command = "runtime-plugin"
"#
    )
}
