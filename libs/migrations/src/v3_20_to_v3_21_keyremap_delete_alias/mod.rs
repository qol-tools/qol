use anyhow::{Context, Result};
use serde_json::Value;
use std::path::{Path, PathBuf};

use crate::fs_util::{list_profile_dirs, plugin_config_dirs, write_json_atomic};
use crate::{FileMigration, MigrationReport};

pub struct V3_20ToV3_21KeyremapDeleteAlias;

impl FileMigration for V3_20ToV3_21KeyremapDeleteAlias {
    fn name(&self) -> &'static str {
        NAME
    }

    fn applies(&self, config_dir: &Path) -> Result<bool> {
        for path in candidate_config_paths(config_dir)? {
            if config_path_needs_migration(&path) {
                return Ok(true);
            }
        }
        Ok(false)
    }

    fn migrate(&self, config_dir: &Path, archive_dir: &Path) -> Result<MigrationReport> {
        let mut archived = Vec::new();

        for path in candidate_config_paths(config_dir)? {
            let Some(mut value) = read_json(&path) else {
                continue;
            };
            if !migrate_config_value(&mut value) {
                continue;
            }
            let archived_path = archive_original(config_dir, archive_dir, &path)?;
            write_json_atomic(&path, &value)?;
            archived.push(archived_path);
        }

        Ok(MigrationReport {
            name: NAME.to_string(),
            archived,
        })
    }
}

const NAME: &str = "v3.20-to-v3.21-keyremap-delete-alias";
const KEYREMAP_ID: &str = "plugin-keyremap";
const KEYREMAP_UID: &str = "e1bc6f9b-95e0-46c5-951b-6cc5de5c6d87";

fn candidate_config_paths(config_dir: &Path) -> Result<Vec<PathBuf>> {
    let mut paths = Vec::new();
    push_if_file(
        &mut paths,
        config_dir
            .join("plugins")
            .join(KEYREMAP_ID)
            .join("config.json"),
    );
    push_if_file(
        &mut paths,
        config_dir
            .join("plugins")
            .join(KEYREMAP_UID)
            .join("config.json"),
    );

    for profile_dir in list_profile_dirs(&config_dir.join("profile"))? {
        for dir in plugin_config_dirs(&profile_dir) {
            push_if_file(&mut paths, dir.join(format!("{KEYREMAP_ID}.json")));
            push_if_file(&mut paths, dir.join(format!("{KEYREMAP_UID}.json")));
        }
    }

    paths.sort();
    paths.dedup();
    Ok(paths)
}

fn push_if_file(paths: &mut Vec<PathBuf>, path: PathBuf) {
    if path.is_file() {
        paths.push(path);
    }
}

fn config_path_needs_migration(path: &Path) -> bool {
    let Some(mut value) = read_json(path) else {
        return false;
    };
    migrate_config_value(&mut value)
}

fn read_json(path: &Path) -> Option<Value> {
    let raw = std::fs::read_to_string(path).ok()?;
    match serde_json::from_str(&raw) {
        Ok(value) => Some(value),
        Err(error) => {
            log::warn!("[{NAME}] skipping unparseable {}: {error}", path.display());
            None
        }
    }
}

fn migrate_config_value(value: &mut Value) -> bool {
    let Some(object) = value.as_object_mut() else {
        return false;
    };

    let mut changed = false;
    if let Some(Value::Array(char_rules)) = object.get_mut("char_rules") {
        for rule in char_rules {
            changed |= rewrite_object_string_field(rule, "from_key");
        }
    }
    if let Some(Value::Array(key_rules)) = object.get_mut("key_rules") {
        for rule in key_rules {
            changed |= rewrite_object_string_field(rule, "from_key");
            changed |= rewrite_object_string_field(rule, "to_key");
            changed |= rewrite_key_array(rule);
        }
    }
    changed
}

fn rewrite_object_string_field(value: &mut Value, field: &str) -> bool {
    let Some(object) = value.as_object_mut() else {
        return false;
    };
    let Some(field_value) = object.get_mut(field) else {
        return false;
    };
    rewrite_key_string(field_value)
}

fn rewrite_key_array(value: &mut Value) -> bool {
    let Some(object) = value.as_object_mut() else {
        return false;
    };
    let Some(Value::Array(keys)) = object.get_mut("keys") else {
        return false;
    };
    let mut changed = false;
    for key in keys {
        changed |= rewrite_key_string(key);
    }
    changed
}

fn rewrite_key_string(value: &mut Value) -> bool {
    let Some(key) = value.as_str() else {
        return false;
    };
    if !key.eq_ignore_ascii_case("delete") {
        return false;
    }
    *value = Value::String("backspace".to_string());
    true
}

fn archive_original(config_dir: &Path, archive_dir: &Path, path: &Path) -> Result<PathBuf> {
    let relative = path
        .strip_prefix(config_dir)
        .with_context(|| format!("{} is outside {}", path.display(), config_dir.display()))?;
    let archived_path = archive_dir.join(relative);
    if let Some(parent) = archived_path.parent() {
        std::fs::create_dir_all(parent)
            .with_context(|| format!("creating {}", parent.display()))?;
    }
    std::fs::copy(path, &archived_path).with_context(|| {
        format!(
            "archiving original {} -> {}",
            path.display(),
            archived_path.display()
        )
    })?;
    Ok(archived_path)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn migration() -> V3_20ToV3_21KeyremapDeleteAlias {
        V3_20ToV3_21KeyremapDeleteAlias
    }

    fn archive_dir(config_dir: &Path) -> PathBuf {
        let path = config_dir.join("archive");
        std::fs::create_dir_all(&path).unwrap();
        path
    }

    fn setup_profile(config_dir: &Path, name: &str) -> PathBuf {
        let root = config_dir.join("profile").join(name);
        write(&root.join("manifest.json"), b"{\"version\":1}");
        root
    }

    fn write(path: &Path, bytes: &[u8]) {
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, bytes).unwrap();
    }

    fn write_json(path: &Path, value: &Value) {
        write(path, value.to_string().as_bytes());
    }

    fn read_json_file(path: &Path) -> Value {
        serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap()
    }

    #[test]
    fn applies_returns_false_without_legacy_delete_alias_in_keyremap_config() {
        let dir = tempfile::tempdir().unwrap();
        let profile = setup_profile(dir.path(), "default");
        write_json(
            &profile.join("os/macos/plugin-configs/plugin-keyremap.json"),
            &json!({
                "char_rules": [{"from_key": "backspace", "to_char": "x"}],
                "key_rules": [
                    {"from_key": "forwarddelete", "to_key": "backspace"},
                    {"keys": ["left", "right", "backspace", "forwarddelete"]}
                ]
            }),
        );
        write_json(
            &profile.join("core/plugin-configs/plugin-other.json"),
            &json!({"key_rules": [{"from_key": "delete", "to_key": "delete"}]}),
        );

        assert!(!migration().applies(dir.path()).unwrap());
    }

    #[test]
    fn migrate_rewrites_legacy_delete_alias_in_every_key_field() {
        let dir = tempfile::tempdir().unwrap();
        let profile = setup_profile(dir.path(), "default");
        let keyremap_path = profile.join("os/macos/plugin-configs/plugin-keyremap.json");
        let original = json!({
            "char_rules": [
                {"from_mods": ["ctrl"], "from_key": "Delete", "to_char": "x"},
                {"from_char": "d", "to_char": "e"}
            ],
            "key_rules": [
                    {"from_mods": ["ctrl"], "to_mods": ["cmd"], "keys": ["left", "delete", "forwarddelete", "backspace", "DELETE", "Del"]},
                {"from_key": "delete", "to_key": "DELETE"},
                {"from_key": "forwarddelete", "to_key": "backspace"}
            ],
            "mouse_rules": [{"button": "delete"}]
        });
        write_json(&keyremap_path, &original);

        assert!(migration().applies(dir.path()).unwrap());
        let report = migration()
            .migrate(dir.path(), &archive_dir(dir.path()))
            .unwrap();

        assert_eq!(report.archived.len(), 1);
        assert_eq!(read_json_file(&report.archived[0]), original);
        assert_eq!(
            read_json_file(&keyremap_path),
            json!({
                "char_rules": [
                    {"from_mods": ["ctrl"], "from_key": "backspace", "to_char": "x"},
                    {"from_char": "d", "to_char": "e"}
                ],
                "key_rules": [
                    {"from_mods": ["ctrl"], "to_mods": ["cmd"], "keys": ["left", "backspace", "forwarddelete", "backspace", "backspace", "Del"]},
                    {"from_key": "backspace", "to_key": "backspace"},
                    {"from_key": "forwarddelete", "to_key": "backspace"}
                ],
                "mouse_rules": [{"button": "delete"}]
            })
        );
    }

    #[test]
    fn migrate_covers_uid_named_profile_config_and_runtime_cache() {
        let dir = tempfile::tempdir().unwrap();
        let profile = setup_profile(dir.path(), "default");
        let uid_path = profile.join(format!("device/plugin-configs/{KEYREMAP_UID}.json"));
        let runtime_path = dir
            .path()
            .join("plugins")
            .join(KEYREMAP_ID)
            .join("config.json");
        write_json(&uid_path, &json!({"key_rules": [{"keys": ["delete"]}]}));
        write_json(
            &runtime_path,
            &json!({"char_rules": [{"from_key": "delete", "to_char": "x"}]}),
        );

        let report = migration()
            .migrate(dir.path(), &archive_dir(dir.path()))
            .unwrap();

        assert_eq!(report.archived.len(), 2);
        assert_eq!(
            read_json_file(&uid_path),
            json!({"key_rules": [{"keys": ["backspace"]}]})
        );
        assert_eq!(
            read_json_file(&runtime_path),
            json!({"char_rules": [{"from_key": "backspace", "to_char": "x"}]})
        );
    }

    #[test]
    fn migrate_is_idempotent_after_first_pass() {
        let dir = tempfile::tempdir().unwrap();
        let profile = setup_profile(dir.path(), "default");
        let keyremap_path = profile.join("os/macos/plugin-configs/plugin-keyremap.json");
        write_json(
            &keyremap_path,
            &json!({"key_rules": [{"keys": ["delete"]}]}),
        );

        let first = migration()
            .migrate(dir.path(), &archive_dir(dir.path()))
            .unwrap();
        let second = migration()
            .migrate(dir.path(), &archive_dir(dir.path()))
            .unwrap();

        assert_eq!(first.archived.len(), 1);
        assert!(second.archived.is_empty());
        assert!(!migration().applies(dir.path()).unwrap());
    }
}
