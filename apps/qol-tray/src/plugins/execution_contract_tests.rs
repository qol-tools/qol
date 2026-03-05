use super::execution_contract::resolve_plugin_command_path;
#[cfg(feature = "dev")]
use super::execution_contract::resolve_plugin_command_path_for_source;
#[cfg(feature = "dev")]
use crate::plugins::PluginSource;
use std::fs;
use tempfile::TempDir;

#[test]
fn resolve_plugin_command_path_rejects_nested_command() {
    let temp_dir = TempDir::new().unwrap();
    fs::write(temp_dir.path().join("binary"), "").unwrap();

    let resolved = resolve_plugin_command_path(temp_dir.path(), "nested/binary");
    assert!(resolved.is_none());
}

#[test]
fn resolve_plugin_command_path_resolves_regular_file() {
    let temp_dir = TempDir::new().unwrap();
    let binary = temp_dir.path().join("binary");
    fs::write(&binary, "").unwrap();

    let resolved = resolve_plugin_command_path(temp_dir.path(), "binary");
    assert_eq!(resolved, Some(binary));
}

#[cfg(unix)]
#[test]
fn resolve_plugin_command_path_rejects_symlink_escape() {
    let temp_dir = TempDir::new().unwrap();
    let outside_dir = TempDir::new().unwrap();
    let outside_binary = outside_dir.path().join("outside-binary");
    fs::write(&outside_binary, "").unwrap();

    let escaped = temp_dir.path().join("binary");
    std::os::unix::fs::symlink(&outside_binary, &escaped).unwrap();

    let resolved = resolve_plugin_command_path(temp_dir.path(), "binary");
    assert!(resolved.is_none());
}

#[cfg(unix)]
#[test]
fn resolve_plugin_command_path_allows_internal_symlink() {
    let temp_dir = TempDir::new().unwrap();
    let real_binary = temp_dir.path().join("real-binary");
    fs::write(&real_binary, "").unwrap();

    let linked_binary = temp_dir.path().join("binary");
    std::os::unix::fs::symlink(&real_binary, &linked_binary).unwrap();

    let resolved = resolve_plugin_command_path(temp_dir.path(), "binary");
    assert_eq!(resolved, Some(linked_binary));
}

#[cfg(feature = "dev")]
#[test]
fn resolve_plugin_command_path_prefers_debug_binary_for_dev_linked_plugins() {
    let temp_dir = TempDir::new().unwrap();
    let root_binary = temp_dir.path().join("binary");
    let debug_binary = temp_dir.path().join("target").join("debug").join("binary");
    fs::create_dir_all(debug_binary.parent().unwrap()).unwrap();
    fs::write(&root_binary, "root").unwrap();
    fs::write(&debug_binary, "debug").unwrap();

    let resolved = resolve_plugin_command_path_for_source(
        temp_dir.path(),
        "binary",
        Some(&PluginSource::DevLinked),
    );

    assert_eq!(resolved, Some(debug_binary));
}
