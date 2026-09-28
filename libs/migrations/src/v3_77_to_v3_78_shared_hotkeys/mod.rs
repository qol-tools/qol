use anyhow::Result;
use serde_json::{json, Value};
use std::path::{Path, PathBuf};

use crate::fs_util::{
    archive_original, current_os_subdir, hotkey_files, list_profile_dirs, write_json_atomic,
};
use crate::hotkey_layers::absorb;
use crate::{FileMigration, MigrationReport};

pub(crate) struct V3_77ToV3_78SharedHotkeys;

const NAME: &str = "v3.77-to-v3.78-shared-hotkeys";

impl FileMigration for V3_77ToV3_78SharedHotkeys {
    fn name(&self) -> &'static str {
        NAME
    }

    fn applies(&self, config_dir: &Path) -> Result<bool> {
        for profile_dir in list_profile_dirs(&config_dir.join("profile"))? {
            if hotkey_files(&profile_dir)
                .iter()
                .any(|path| read_bindings(path).is_some_and(|bindings| !bindings.is_empty()))
            {
                return Ok(true);
            }
        }
        Ok(false)
    }

    fn migrate(&self, config_dir: &Path, archive_dir: &Path) -> Result<MigrationReport> {
        let mut archived = Vec::new();
        for profile_dir in list_profile_dirs(&config_dir.join("profile"))? {
            share_profile_hotkeys(config_dir, archive_dir, &profile_dir, &mut archived)?;
        }
        Ok(MigrationReport {
            name: NAME.to_string(),
            archived,
        })
    }
}

fn share_profile_hotkeys(
    config_dir: &Path,
    archive_dir: &Path,
    profile_dir: &Path,
    archived: &mut Vec<PathBuf>,
) -> Result<()> {
    let core_path = profile_dir.join("core").join("hotkeys.json");
    let own_path = profile_dir
        .join("os")
        .join(current_os_subdir())
        .join("hotkeys.json");
    let own = if own_path.is_file() {
        let Some(own) = read_bindings(&own_path) else {
            return Ok(());
        };
        own
    } else {
        Vec::new()
    };
    let Some(original_core) = (if core_path.is_file() {
        read_bindings(&core_path)
    } else {
        Some(Vec::new())
    }) else {
        return Ok(());
    };

    let mut core = original_core.clone();
    let own_kept = own
        .iter()
        .filter(|binding| !absorb(&mut core, binding))
        .cloned()
        .collect::<Vec<_>>();
    let mut others = hotkey_files(profile_dir);
    others.retain(|path| path != &own_path);
    others.sort();
    for other in others {
        for binding in read_bindings(&other).unwrap_or_default() {
            absorb(&mut core, &binding);
        }
    }

    if core != original_core {
        if core_path.is_file() {
            archived.push(archive_original(config_dir, archive_dir, &core_path)?);
        }
        write_json_atomic(&core_path, &json!({ "hotkeys": core }))?;
    }
    if own_kept != own {
        archived.push(archive_original(config_dir, archive_dir, &own_path)?);
        write_json_atomic(&own_path, &json!({ "hotkeys": own_kept }))?;
    }
    Ok(())
}

fn read_bindings(path: &Path) -> Option<Vec<Value>> {
    let raw = std::fs::read_to_string(path).ok()?;
    match serde_json::from_str::<Value>(&raw) {
        Ok(value) => Some(
            value
                .get("hotkeys")
                .and_then(Value::as_array)
                .cloned()
                .unwrap_or_default(),
        ),
        Err(error) => {
            log::warn!("[{NAME}] skipping unparseable {}: {error}", path.display());
            None
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn binding(id: &str, key: &str, action: &str) -> Value {
        json!({ "id": id, "key": key, "plugin_uid": "p", "action": action, "enabled": true })
    }

    fn write(path: &Path, bindings: &[Value]) {
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, json!({ "hotkeys": bindings }).to_string()).unwrap();
    }

    fn read(path: &Path) -> Vec<Value> {
        read_bindings(path).unwrap()
    }

    fn profile_dir(config: &Path) -> PathBuf {
        let profile = config.join("profile").join("default");
        std::fs::create_dir_all(&profile).unwrap();
        std::fs::write(profile.join("manifest.json"), "{}").unwrap();
        profile
    }

    fn other_os() -> &'static str {
        if current_os_subdir() == "linux" {
            "macos"
        } else {
            "linux"
        }
    }

    #[test]
    fn every_os_file_is_shared_and_only_this_os_file_is_emptied() {
        let config = tempfile::tempdir().unwrap();
        let archive = tempfile::tempdir().unwrap();
        let profile = profile_dir(config.path());
        let own = profile
            .join("os")
            .join(current_os_subdir())
            .join("hotkeys.json");
        let other = profile.join("os").join(other_os()).join("hotkeys.json");
        let core = profile.join("core").join("hotkeys.json");
        write(&core, &[]);
        write(
            &own,
            &[
                binding("rec-here", "Shift+Super+R", "record"),
                binding("snap", "Super+Left", "snap-left"),
            ],
        );
        write(
            &other,
            &[
                binding("rec-there", "Shift+Super+R", "record"),
                binding("snap", "Super+Left", "snap-left"),
                binding("glide", "Ctrl+Super+Up", "glide-up"),
            ],
        );
        let migration = V3_77ToV3_78SharedHotkeys;

        assert!(migration.applies(config.path()).unwrap());
        let report = migration.migrate(config.path(), archive.path()).unwrap();

        assert_eq!(
            read(&core),
            vec![
                binding("rec-here", "Shift+Super+R", "record"),
                binding("snap", "Super+Left", "snap-left"),
                binding("glide", "Ctrl+Super+Up", "glide-up"),
            ]
        );
        assert!(read(&own).is_empty());
        assert_eq!(read(&other).len(), 3);
        assert_eq!(report.archived.len(), 2);
    }

    #[test]
    fn a_binding_that_clashes_with_a_shared_one_stays_on_its_own_os() {
        let config = tempfile::tempdir().unwrap();
        let archive = tempfile::tempdir().unwrap();
        let profile = profile_dir(config.path());
        let own = profile
            .join("os")
            .join(current_os_subdir())
            .join("hotkeys.json");
        let core = profile.join("core").join("hotkeys.json");
        write(&core, &[binding("night", "Super+N", "night")]);
        write(
            &own,
            &[
                binding("notes", "Super+N", "notes"),
                binding("snap", "Super+Left", "snap-left"),
            ],
        );

        V3_77ToV3_78SharedHotkeys
            .migrate(config.path(), archive.path())
            .unwrap();

        assert_eq!(
            read(&core),
            vec![
                binding("night", "Super+N", "night"),
                binding("snap", "Super+Left", "snap-left"),
            ]
        );
        assert_eq!(read(&own), vec![binding("notes", "Super+N", "notes")]);
    }

    #[test]
    fn an_unreadable_os_file_is_left_alone() {
        let config = tempfile::tempdir().unwrap();
        let archive = tempfile::tempdir().unwrap();
        let profile = profile_dir(config.path());
        let own = profile
            .join("os")
            .join(current_os_subdir())
            .join("hotkeys.json");
        std::fs::create_dir_all(own.parent().unwrap()).unwrap();
        std::fs::write(&own, "{ not json").unwrap();

        V3_77ToV3_78SharedHotkeys
            .migrate(config.path(), archive.path())
            .unwrap();

        assert_eq!(std::fs::read_to_string(&own).unwrap(), "{ not json");
        assert!(!profile.join("core").join("hotkeys.json").exists());
    }
}
