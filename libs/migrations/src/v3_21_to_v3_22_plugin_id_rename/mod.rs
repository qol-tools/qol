use anyhow::{Context, Result};
use serde_json::Value;
use std::collections::HashMap;
use std::path::{Path, PathBuf};

use crate::fs_util::{list_profile_dirs, shortcut_files, write_json_atomic};
use crate::{FileMigration, MigrationReport};

const NAME: &str = "v3.21-to-v3.22-plugin-id-rename";

const RENAMED_IDS: &[(&str, &str)] = &[
    ("plugin-alt-tab", "qol-alt-tab"),
    ("plugin-bluetooth", "qol-bluetooth"),
    ("plugin-cli-sessions", "qol-cli-sessions"),
    ("plugin-controllers", "qol-controllers"),
    ("plugin-ide-checkout", "qol-ide-checkout"),
    ("plugin-keyremap", "qol-keyremap"),
    ("plugin-launcher", "qol-launcher"),
    ("plugin-lights", "qol-lights"),
    ("plugin-monitor", "qol-monitor"),
    ("plugin-os-themes", "qol-os-themes"),
    ("plugin-pointz", "qol-pointz"),
    ("plugin-removeapp", "qol-removeapp"),
    ("plugin-sound", "qol-sound"),
    ("plugin-template", "qol-template"),
    ("plugin-window-actions", "qol-window-actions"),
];

pub struct V3_21ToV3_22PluginIdRename {
    data_root: Option<PathBuf>,
}

impl V3_21ToV3_22PluginIdRename {
    pub fn new() -> Self {
        Self { data_root: None }
    }

    pub fn with_data_root(data_root: PathBuf) -> Self {
        Self {
            data_root: Some(data_root),
        }
    }

    pub fn default_for_production() -> Self {
        Self {
            data_root: qol_config::data_dir(),
        }
    }

    fn data_dir_renames_pending(&self) -> bool {
        self.data_root.as_ref().is_some_and(|data_root| {
            dir_rename_pending(&data_root.join("plugins"))
                || dir_rename_pending(&data_root.join("host-takeover"))
        })
    }

    fn rename_data_dirs(&self, touched: &mut Vec<PathBuf>) -> Result<()> {
        let Some(data_root) = &self.data_root else {
            return Ok(());
        };
        rename_dirs(&data_root.join("plugins"), touched)?;
        rename_dirs(&data_root.join("host-takeover"), touched)
    }
}

impl Default for V3_21ToV3_22PluginIdRename {
    fn default() -> Self {
        Self::new()
    }
}

impl FileMigration for V3_21ToV3_22PluginIdRename {
    fn name(&self) -> &'static str {
        NAME
    }

    fn applies(&self, config_dir: &Path) -> Result<bool> {
        let plugins_dir = config_dir.join("plugins");
        if dir_rename_pending(&plugins_dir) {
            return Ok(true);
        }
        if self.data_dir_renames_pending() {
            return Ok(true);
        }
        if registry_has_work(&config_dir.join("plugin-registry.json"), &plugins_dir) {
            return Ok(true);
        }
        for profile_dir in list_profile_dirs(&config_dir.join("profile"))? {
            if lock_has_work(&profile_dir.join("core").join("plugins.lock.json")) {
                return Ok(true);
            }
            if shortcut_files(&profile_dir)
                .iter()
                .any(|path| shortcuts_have_work(path))
            {
                return Ok(true);
            }
        }
        Ok(false)
    }

    fn migrate(&self, config_dir: &Path, _archive_dir: &Path) -> Result<MigrationReport> {
        let plugins_dir = config_dir.join("plugins");
        let mut touched = Vec::new();

        rename_dirs(&plugins_dir, &mut touched)?;
        self.rename_data_dirs(&mut touched)?;
        rewrite_registry(
            &config_dir.join("plugin-registry.json"),
            &plugins_dir,
            &mut touched,
        )?;
        for profile_dir in list_profile_dirs(&config_dir.join("profile"))? {
            rewrite_lock(
                &profile_dir.join("core").join("plugins.lock.json"),
                &mut touched,
            )?;
            for path in shortcut_files(&profile_dir) {
                rewrite_shortcuts(&path, &mut touched)?;
            }
        }

        Ok(MigrationReport {
            name: NAME.to_string(),
            archived: touched,
        })
    }
}

pub fn renamed_plugin_id(id: &str) -> Option<&'static str> {
    RENAMED_IDS
        .iter()
        .find(|(old, _)| *old == id)
        .map(|(_, new)| *new)
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

fn dir_rename_pending(root: &Path) -> bool {
    RENAMED_IDS.iter().copied().any(|(old, new)| {
        let src = root.join(old);
        let dst = root.join(new);
        src.symlink_metadata()
            .is_ok_and(|metadata| metadata.is_dir())
            && dst.symlink_metadata().is_err()
    })
}

fn rename_dirs(root: &Path, touched: &mut Vec<PathBuf>) -> Result<()> {
    for &(old, new) in RENAMED_IDS {
        let src = root.join(old);
        let dst = root.join(new);
        let Ok(metadata) = std::fs::symlink_metadata(&src) else {
            continue;
        };
        if metadata.file_type().is_symlink() {
            log::warn!("[{NAME}] {} is a symlink; left in place", src.display());
            continue;
        }
        if !metadata.is_dir() {
            continue;
        }
        if dst.symlink_metadata().is_ok() {
            log::warn!(
                "[{NAME}] {} already exists; not renaming {} to avoid merging two plugins",
                dst.display(),
                src.display()
            );
            continue;
        }
        std::fs::rename(&src, &dst)
            .with_context(|| format!("renaming {} to {}", src.display(), dst.display()))?;
        touched.push(dst);
    }
    Ok(())
}

fn slot_path_rewrite(slot: &Value, plugins_dir: &Path) -> Option<String> {
    let path = Path::new(slot.get("path")?.as_str()?);
    if path.parent() != Some(plugins_dir)
        || path
            .symlink_metadata()
            .is_ok_and(|metadata| metadata.file_type().is_symlink())
    {
        return None;
    }
    let new_name = path
        .file_name()
        .and_then(|name| name.to_str())
        .and_then(renamed_plugin_id)?;
    Some(plugins_dir.join(new_name).to_string_lossy().into_owned())
}

fn slot_path_needs_rewrite(entry: &Value, slot_key: &str, plugins_dir: &Path) -> bool {
    entry
        .get(slot_key)
        .is_some_and(|slot| slot_path_rewrite(slot, plugins_dir).is_some())
}

fn rewrite_slot_path(slot: &mut Value, plugins_dir: &Path) -> bool {
    let Some(new_path) = slot_path_rewrite(slot, plugins_dir) else {
        return false;
    };
    let Some(path_value) = slot.get_mut("path") else {
        return false;
    };
    *path_value = Value::String(new_path);
    true
}

fn rewrite_slot_path_on_entry(entry: &mut Value, slot_key: &str, plugins_dir: &Path) -> bool {
    let Some(slot) = entry.get_mut(slot_key) else {
        return false;
    };
    rewrite_slot_path(slot, plugins_dir)
}

fn registry_has_work(path: &Path, plugins_dir: &Path) -> bool {
    let Some(value) = read_json(path) else {
        return false;
    };
    let Some(entries) = value.get("entries").and_then(|v| v.as_array()) else {
        return false;
    };
    entries.iter().any(|entry| {
        entry
            .get("id")
            .and_then(|v| v.as_str())
            .is_some_and(|id| renamed_plugin_id(id).is_some())
            || ["active", "fallback"]
                .iter()
                .copied()
                .any(|slot_key| slot_path_needs_rewrite(entry, slot_key, plugins_dir))
    })
}

fn rewrite_registry(path: &Path, plugins_dir: &Path, touched: &mut Vec<PathBuf>) -> Result<()> {
    rewrite_renamed_array(
        path,
        "entries",
        |entry| {
            rewrite_slot_path_on_entry(entry, "active", plugins_dir)
                | rewrite_slot_path_on_entry(entry, "fallback", plugins_dir)
        },
        |_entry, final_id| final_id.map(str::to_string),
        "duplicate registry entry",
        touched,
    )
}

fn lock_has_work(path: &Path) -> bool {
    let Some(value) = read_json(path) else {
        return false;
    };
    value
        .get("plugins")
        .and_then(|v| v.as_array())
        .into_iter()
        .flatten()
        .any(|entry| {
            entry
                .get("id")
                .and_then(|v| v.as_str())
                .is_some_and(|id| renamed_plugin_id(id).is_some())
        })
}

fn rewrite_lock(path: &Path, touched: &mut Vec<PathBuf>) -> Result<()> {
    rewrite_renamed_array(
        path,
        "plugins",
        |_entry| false,
        |entry, final_id| {
            entry
                .get("uid")
                .and_then(|v| v.as_str())
                .map(str::to_string)
                .or_else(|| final_id.map(str::to_string))
        },
        "lock entries sharing uid",
        touched,
    )
}

fn rewrite_renamed_array(
    path: &Path,
    field: &str,
    prepare: impl Fn(&mut Value) -> bool,
    key_of: impl Fn(&Value, Option<&str>) -> Option<String>,
    warn: &str,
    touched: &mut Vec<PathBuf>,
) -> Result<()> {
    let Some(mut value) = read_json(path) else {
        return Ok(());
    };
    let Some(entries) = value.get(field).and_then(|v| v.as_array()).cloned() else {
        return Ok(());
    };

    let mut out: Vec<Value> = Vec::with_capacity(entries.len());
    let mut preferred: Vec<bool> = Vec::with_capacity(entries.len());
    let mut seen: HashMap<String, usize> = HashMap::new();
    let mut changed = false;

    for mut entry in entries {
        let old_id = entry.get("id").and_then(|v| v.as_str()).map(str::to_string);
        let new_id = old_id
            .as_deref()
            .and_then(renamed_plugin_id)
            .map(str::to_string);
        let renamed = new_id.is_some();
        if let Some(new_id) = &new_id {
            entry["id"] = Value::String(new_id.clone());
            changed = true;
        }
        changed |= prepare(&mut entry);

        let final_id = new_id.or(old_id);
        let Some(key) = key_of(&entry, final_id.as_deref()) else {
            out.push(entry);
            preferred.push(!renamed);
            continue;
        };
        let entry_preferred = !renamed;
        match seen.get(&key).copied() {
            None => {
                seen.insert(key, out.len());
                out.push(entry);
                preferred.push(entry_preferred);
            }
            Some(index) => {
                changed = true;
                if entry_preferred && !preferred[index] {
                    out[index] = entry;
                    preferred[index] = true;
                }
                log::warn!("[{NAME}] {}: {warn} {key:?} coalesced", path.display());
            }
        }
    }

    if changed {
        value[field] = Value::Array(out);
        write_json_atomic(path, &value)?;
        touched.push(path.to_path_buf());
    }
    Ok(())
}

fn rewrite_prefixed_shortcut_id(id: &str) -> Option<String> {
    RENAMED_IDS.iter().find_map(|&(old, new)| {
        let rest = id.strip_prefix(old)?.strip_prefix('-')?;
        Some(format!("{new}-{rest}"))
    })
}

fn rewrite_shortcut_plugin_id(shortcut: &mut Value, key: &str) -> bool {
    let plugin_id = shortcut
        .get(key)
        .and_then(|nested| nested.get("plugin_id"))
        .and_then(|id| id.as_str());
    let Some(new_id) = plugin_id.and_then(renamed_plugin_id) else {
        return false;
    };
    let Some(nested) = shortcut.get_mut(key) else {
        return false;
    };
    nested["plugin_id"] = Value::String(new_id.to_string());
    true
}

fn shortcut_has_work(shortcut: &Value) -> bool {
    let plugin_id_work = ["source", "action"].iter().copied().any(|key| {
        shortcut
            .get(key)
            .and_then(|nested| nested.get("plugin_id"))
            .and_then(|id| id.as_str())
            .is_some_and(|id| renamed_plugin_id(id).is_some())
    });
    let shortcut_id_work = shortcut
        .get("id")
        .and_then(|id| id.as_str())
        .is_some_and(|id| rewrite_prefixed_shortcut_id(id).is_some());
    plugin_id_work || shortcut_id_work
}

fn shortcuts_have_work(path: &Path) -> bool {
    let Some(value) = read_json(path) else {
        return false;
    };
    value
        .get("shortcuts")
        .and_then(|v| v.as_array())
        .into_iter()
        .flatten()
        .any(shortcut_has_work)
}

fn rewrite_shortcuts(path: &Path, touched: &mut Vec<PathBuf>) -> Result<()> {
    let Some(mut value) = read_json(path) else {
        return Ok(());
    };
    let Some(shortcuts) = value.get_mut("shortcuts").and_then(|v| v.as_array_mut()) else {
        return Ok(());
    };

    let mut changed = false;
    for shortcut in shortcuts.iter_mut() {
        changed |= rewrite_shortcut_plugin_id(shortcut, "source");
        changed |= rewrite_shortcut_plugin_id(shortcut, "action");
        let new_shortcut_id = shortcut
            .get("id")
            .and_then(|id| id.as_str())
            .and_then(rewrite_prefixed_shortcut_id);
        if let Some(new_shortcut_id) = new_shortcut_id {
            shortcut["id"] = Value::String(new_shortcut_id);
            changed = true;
        }
    }

    if changed {
        write_json_atomic(path, &value)?;
        touched.push(path.to_path_buf());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    const LIGHTS_UID: &str = "368871df-de60-4a7a-ab7c-8d33fcd22511";

    fn migration() -> V3_21ToV3_22PluginIdRename {
        V3_21ToV3_22PluginIdRename::new()
    }

    fn migration_with_data(config_dir: &Path) -> V3_21ToV3_22PluginIdRename {
        V3_21ToV3_22PluginIdRename::with_data_root(config_dir.join("data"))
    }

    fn archive(config_dir: &Path) -> PathBuf {
        let path = config_dir.join("archive");
        std::fs::create_dir_all(&path).unwrap();
        path
    }

    fn run(config_dir: &Path) -> MigrationReport {
        migration_with_data(config_dir)
            .migrate(config_dir, &archive(config_dir))
            .unwrap()
    }

    fn write(path: &Path, bytes: &[u8]) {
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, bytes).unwrap();
    }

    fn write_json(path: &Path, value: &serde_json::Value) {
        write(path, value.to_string().as_bytes());
    }

    fn read_back(path: &Path) -> serde_json::Value {
        serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap()
    }

    fn setup_profile(config_dir: &Path, name: &str) -> PathBuf {
        let root = config_dir.join("profile").join(name);
        write(&root.join("manifest.json"), b"{\"version\":1}");
        root
    }

    fn path_string(path: &Path) -> String {
        path.to_string_lossy().into_owned()
    }

    #[test]
    fn applies_is_false_on_a_config_dir_with_no_plugin_state() {
        let dir = tempfile::tempdir().unwrap();
        assert!(!migration().applies(dir.path()).unwrap());
    }

    #[test]
    fn plugin_directory_is_renamed_with_its_payload() {
        let dir = tempfile::tempdir().unwrap();
        let plugins = dir.path().join("plugins");
        write(
            &plugins.join("plugin-lights/config.json"),
            b"{\"preview_size\":320}",
        );

        assert!(migration().applies(dir.path()).unwrap());
        let report = run(dir.path());

        assert!(plugins.join("qol-lights/config.json").is_file());
        assert!(!plugins.join("plugin-lights").exists());
        assert_eq!(report.archived, vec![plugins.join("qol-lights")]);
        assert!(!migration().applies(dir.path()).unwrap());
    }

    #[test]
    fn existing_new_directory_is_never_merged_over() {
        let dir = tempfile::tempdir().unwrap();
        let plugins = dir.path().join("plugins");
        write(&plugins.join("plugin-sound/config.json"), b"{\"old\":true}");
        write(&plugins.join("qol-sound/config.json"), b"{\"new\":true}");

        run(dir.path());

        assert_eq!(
            read_back(&plugins.join("plugin-sound/config.json")),
            json!({"old": true})
        );
        assert_eq!(
            read_back(&plugins.join("qol-sound/config.json")),
            json!({"new": true})
        );
        assert!(!migration().applies(dir.path()).unwrap());
    }

    #[test]
    fn data_plugin_directories_move_with_their_payload() {
        let dir = tempfile::tempdir().unwrap();
        let data = dir.path().join("data");
        write(
            &data.join("plugins/plugin-pointz/devices.json"),
            b"{\"devices\":[]}",
        );
        write(
            &data.join("host-takeover/plugin-bluetooth/takeover-blueman"),
            b"restore hint",
        );

        assert!(migration_with_data(dir.path()).applies(dir.path()).unwrap());
        let report = run(dir.path());

        assert!(data.join("plugins/qol-pointz/devices.json").is_file());
        assert!(!data.join("plugins/plugin-pointz").exists());
        assert!(data
            .join("host-takeover/qol-bluetooth/takeover-blueman")
            .is_file());
        assert!(!data.join("host-takeover/plugin-bluetooth").exists());
        assert!(report.archived.contains(&data.join("plugins/qol-pointz")));
        assert!(report
            .archived
            .contains(&data.join("host-takeover/qol-bluetooth")));
    }

    #[test]
    fn existing_data_directory_is_never_merged_over() {
        let dir = tempfile::tempdir().unwrap();
        let data = dir.path().join("data");
        write(
            &data.join("plugins/plugin-pointz/devices.json"),
            b"{\"old\":true}",
        );
        write(
            &data.join("plugins/qol-pointz/devices.json"),
            b"{\"new\":true}",
        );

        run(dir.path());

        assert_eq!(
            read_back(&data.join("plugins/plugin-pointz/devices.json")),
            json!({"old": true})
        );
        assert_eq!(
            read_back(&data.join("plugins/qol-pointz/devices.json")),
            json!({"new": true})
        );
        assert!(!migration_with_data(dir.path()).applies(dir.path()).unwrap());
    }

    #[test]
    fn data_directory_moves_are_idempotent() {
        let dir = tempfile::tempdir().unwrap();
        let data = dir.path().join("data");
        write(
            &data.join("host-takeover/plugin-bluetooth/takeover-blueman"),
            b"hint",
        );

        assert!(migration_with_data(dir.path()).applies(dir.path()).unwrap());
        let first = run(dir.path());
        assert!(!first.archived.is_empty());
        assert!(!migration_with_data(dir.path()).applies(dir.path()).unwrap());

        let second = run(dir.path());
        assert!(second.archived.is_empty());
        assert!(data
            .join("host-takeover/qol-bluetooth/takeover-blueman")
            .is_file());
    }

    #[cfg(unix)]
    #[test]
    fn symlinked_plugin_directory_is_left_for_the_registry() {
        let dir = tempfile::tempdir().unwrap();
        let plugins = dir.path().join("plugins");
        std::fs::create_dir_all(&plugins).unwrap();
        let dev = dir.path().join("dev-lights");
        write(&dev.join("config.json"), b"{\"dev\":true}");
        std::os::unix::fs::symlink(&dev, plugins.join("plugin-lights")).unwrap();
        let registry = dir.path().join("plugin-registry.json");
        write_json(
            &registry,
            &json!({"version":1,"entries":[
                {"id":"plugin-lights","active":{"path":path_string(&plugins.join("plugin-lights")),"source":{"type":"release-asset"}}}
            ]}),
        );

        assert!(migration().applies(dir.path()).unwrap());
        run(dir.path());

        assert!(plugins
            .join("plugin-lights")
            .symlink_metadata()
            .unwrap()
            .file_type()
            .is_symlink());
        assert!(!plugins.join("qol-lights").exists());
        assert!(plugins.join("plugin-lights/config.json").is_file());
        let value = read_back(&registry);
        assert_eq!(value["entries"][0]["id"], "qol-lights");
        assert_eq!(
            value["entries"][0]["active"]["path"],
            path_string(&plugins.join("plugin-lights"))
        );
        assert!(!migration().applies(dir.path()).unwrap());
    }

    #[test]
    fn registry_ids_and_slot_paths_are_rewritten_and_duplicates_coalesce() {
        let dir = tempfile::tempdir().unwrap();
        let plugins = dir.path().join("plugins");
        let registry = dir.path().join("plugin-registry.json");
        let old_path = plugins.join("plugin-lights");
        let new_path = plugins.join("qol-lights");
        write_json(
            &registry,
            &json!({"version":1,"entries":[
                {"id":"plugin-lights","active":{"path":path_string(&old_path),"source":{"type":"release-asset"}}},
                {"id":"qol-lights","active":{"path":path_string(&new_path),"source":{"type":"release-asset"}},"fallback":{"path":path_string(&old_path),"source":{"type":"release-asset"}}},
                {"id":"plugin-unknown","active":{"path":"/opt/plugin-unknown","source":{"type":"release-asset"}}}
            ]}),
        );

        assert!(migration().applies(dir.path()).unwrap());
        run(dir.path());

        let value = read_back(&registry);
        let entries = value["entries"].as_array().unwrap();
        assert_eq!(entries.len(), 2);
        assert_eq!(entries[0]["id"], "qol-lights");
        assert_eq!(entries[0]["active"]["path"], path_string(&new_path));
        assert_eq!(entries[0]["fallback"]["path"], path_string(&new_path));
        assert_eq!(entries[1]["id"], "plugin-unknown");
        assert_eq!(entries[1]["active"]["path"], "/opt/plugin-unknown");
        assert!(!migration().applies(dir.path()).unwrap());
    }

    #[test]
    fn registry_paths_outside_the_plugins_dir_are_left_alone() {
        let dir = tempfile::tempdir().unwrap();
        let registry = dir.path().join("plugin-registry.json");
        write_json(
            &registry,
            &json!({"version":1,"entries":[
                {"id":"plugin-alt-tab","active":{"path":"/other/plugins/plugin-alt-tab","source":{"type":"release-asset"}}}
            ]}),
        );

        run(dir.path());

        let value = read_back(&registry);
        assert_eq!(value["entries"][0]["id"], "qol-alt-tab");
        assert_eq!(
            value["entries"][0]["active"]["path"],
            "/other/plugins/plugin-alt-tab"
        );
    }

    #[test]
    fn lock_ids_are_rewritten_uid_untouched_and_entries_coalesce() {
        let dir = tempfile::tempdir().unwrap();
        let profile = setup_profile(dir.path(), "default");
        let lock = profile.join("core/plugins.lock.json");
        write_json(
            &lock,
            &json!({"version":1,"plugins":[
                {"uid":LIGHTS_UID,"id":"plugin-lights","repo_url":"old","version":"1.0.0"},
                {"uid":LIGHTS_UID,"id":"qol-lights","repo_url":"new","version":"2.0.0"},
                {"uid":"uid-other","id":"plugin-unknown","repo_url":"x","version":"1.0.0"}
            ]}),
        );

        assert!(migration().applies(dir.path()).unwrap());
        run(dir.path());

        let value = read_back(&lock);
        let entries = value["plugins"].as_array().unwrap();
        assert_eq!(entries.len(), 2);
        assert_eq!(entries[0]["uid"], LIGHTS_UID);
        assert_eq!(entries[0]["id"], "qol-lights");
        assert_eq!(entries[0]["repo_url"], "new");
        assert_eq!(entries[1]["id"], "plugin-unknown");
        assert!(!migration().applies(dir.path()).unwrap());
    }

    #[test]
    fn lock_entries_without_uid_coalesce_on_the_rewritten_id() {
        let dir = tempfile::tempdir().unwrap();
        let profile = setup_profile(dir.path(), "default");
        let lock = profile.join("core/plugins.lock.json");
        write_json(
            &lock,
            &json!({"version":1,"plugins":[
                {"id":"plugin-lights","repo_url":"old","version":"1.0.0"},
                {"id":"qol-lights","repo_url":"new","version":"2.0.0"}
            ]}),
        );

        run(dir.path());

        let value = read_back(&lock);
        let entries = value["plugins"].as_array().unwrap();
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0]["id"], "qol-lights");
        assert_eq!(entries[0]["repo_url"], "new");
    }

    #[test]
    fn shortcuts_rewrite_source_action_and_managed_id_keeping_preferences() {
        let dir = tempfile::tempdir().unwrap();
        let profile = setup_profile(dir.path(), "default");
        let shortcuts = profile.join("os/linux/shortcuts.json");
        write_json(
            &shortcuts,
            &json!({"shortcuts":[
                {"id":"plugin-lights-toggle","name":"Toggle Lights","enabled":false,"export_to_launcher":true,
                 "source":{"type":"plugin_manifest","plugin_id":"plugin-lights","shortcut_id":"toggle"},
                 "action":{"type":"plugin_action","plugin_id":"plugin-lights","action":"toggle"}},
                {"id":"docs","name":"Docs","enabled":true,"export_to_launcher":false,
                 "action":{"type":"open_url","url":"https://example.test/docs"}}
            ]}),
        );

        assert!(migration().applies(dir.path()).unwrap());
        run(dir.path());

        let value = read_back(&shortcuts);
        let entries = value["shortcuts"].as_array().unwrap();
        assert_eq!(entries.len(), 2);
        assert_eq!(entries[0]["id"], "qol-lights-toggle");
        assert_eq!(entries[0]["source"]["plugin_id"], "qol-lights");
        assert_eq!(entries[0]["action"]["plugin_id"], "qol-lights");
        assert_eq!(entries[0]["enabled"], false);
        assert_eq!(entries[0]["export_to_launcher"], true);
        assert_eq!(entries[1]["action"]["url"], "https://example.test/docs");
        assert!(!migration().applies(dir.path()).unwrap());
    }

    #[test]
    fn migration_is_idempotent_on_a_second_pass() {
        let dir = tempfile::tempdir().unwrap();
        let plugins = dir.path().join("plugins");
        let registry = dir.path().join("plugin-registry.json");
        let profile = setup_profile(dir.path(), "default");
        let lock = profile.join("core/plugins.lock.json");
        let shortcuts = profile.join("os/linux/shortcuts.json");

        write(&plugins.join("plugin-lights/config.json"), b"{}");
        write_json(
            &registry,
            &json!({"version":1,"entries":[
                {"id":"plugin-lights","active":{"path":path_string(&plugins.join("plugin-lights")),"source":{"type":"release-asset"}}}
            ]}),
        );
        write_json(
            &lock,
            &json!({"version":1,"plugins":[
                {"uid":LIGHTS_UID,"id":"plugin-lights","repo_url":"x","version":"1.0.0"}
            ]}),
        );
        write_json(
            &shortcuts,
            &json!({"shortcuts":[
                {"id":"plugin-lights-toggle","name":"T","enabled":true,"export_to_launcher":false,
                 "source":{"type":"plugin_manifest","plugin_id":"plugin-lights","shortcut_id":"toggle"},
                 "action":{"type":"plugin_action","plugin_id":"plugin-lights","action":"toggle"}}
            ]}),
        );

        assert!(migration().applies(dir.path()).unwrap());
        let first = run(dir.path());
        assert!(!first.archived.is_empty());
        assert!(!migration().applies(dir.path()).unwrap());

        let second = run(dir.path());
        assert!(second.archived.is_empty());
    }

    #[test]
    fn each_profile_os_bucket_is_migrated_independently() {
        let dir = tempfile::tempdir().unwrap();
        let default = setup_profile(dir.path(), "default");
        let work = setup_profile(dir.path(), "work");
        let default_shortcuts = default.join("os/linux/shortcuts.json");
        let work_shortcuts = work.join("os/macos/shortcuts.json");
        for path in [&default_shortcuts, &work_shortcuts] {
            write_json(
                path,
                &json!({"shortcuts":[
                    {"id":"plugin-lights-toggle","name":"T","enabled":true,"export_to_launcher":false,
                     "action":{"type":"plugin_action","plugin_id":"plugin-lights","action":"toggle"}}
                ]}),
            );
        }

        run(dir.path());

        assert_eq!(
            read_back(&default_shortcuts)["shortcuts"][0]["id"],
            "qol-lights-toggle"
        );
        assert_eq!(
            read_back(&work_shortcuts)["shortcuts"][0]["action"]["plugin_id"],
            "qol-lights"
        );
    }

    fn fixture_root() -> PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR")).join("fixtures/v3_21_to_v3_22_plugin_id_rename")
    }

    const CONFIG_TOKEN: &str = "{{config}}";

    fn copy_tree(src: &Path, dst: &Path, config_dir: &Path) {
        std::fs::create_dir_all(dst).unwrap();
        for entry in std::fs::read_dir(src).unwrap() {
            let entry = entry.unwrap();
            let from = entry.path();
            let to = dst.join(entry.file_name());
            if entry.file_type().unwrap().is_dir() {
                copy_tree(&from, &to, config_dir);
                continue;
            }
            let text = std::fs::read_to_string(&from)
                .unwrap()
                .replace(CONFIG_TOKEN, &path_string(config_dir));
            std::fs::write(&to, text).unwrap();
        }
    }

    fn expand_config_token(value: &mut serde_json::Value, config_dir: &Path) {
        match value {
            serde_json::Value::String(text) => {
                *text = text.replace(CONFIG_TOKEN, &path_string(config_dir));
            }
            serde_json::Value::Array(items) => {
                for item in items {
                    expand_config_token(item, config_dir);
                }
            }
            serde_json::Value::Object(map) => {
                for item in map.values_mut() {
                    expand_config_token(item, config_dir);
                }
            }
            _ => {}
        }
    }

    fn relative_files(root: &Path, base: &Path, out: &mut Vec<(PathBuf, PathBuf)>) {
        for entry in std::fs::read_dir(base).unwrap() {
            let entry = entry.unwrap();
            let path = entry.path();
            if entry.file_type().unwrap().is_dir() {
                relative_files(root, &path, out);
                continue;
            }
            let rel = path.strip_prefix(root).unwrap().to_path_buf();
            out.push((rel, path));
        }
    }

    #[test]
    fn before_fixture_migrates_into_the_after_fixture_shape() {
        let dir = tempfile::tempdir().unwrap();
        let work = dir.path().join("config");
        copy_tree(&fixture_root().join("before"), &work, &work);

        assert!(
            migration().applies(&work).unwrap(),
            "the before fixture is old-id-keyed and must report work to do"
        );
        run(&work);
        assert!(
            !migration().applies(&work).unwrap(),
            "a second applies() over the migrated tree must be a no-op"
        );

        let expected_root = fixture_root().join("after");
        let mut expected = Vec::new();
        relative_files(&expected_root, &expected_root, &mut expected);
        expected.sort();

        let mut produced = Vec::new();
        relative_files(&work, &work, &mut produced);
        produced.retain(|(_, abs)| !abs.starts_with(work.join("archive")));
        produced.sort();

        let produced_names: Vec<&PathBuf> = produced.iter().map(|(rel, _)| rel).collect();
        let expected_names: Vec<&PathBuf> = expected.iter().map(|(rel, _)| rel).collect();
        assert_eq!(
            produced_names, expected_names,
            "migrated file set must match the after fixture exactly"
        );

        for ((rel, produced_abs), (_, expected_abs)) in produced.iter().zip(expected.iter()) {
            let produced_value: serde_json::Value =
                serde_json::from_str(&std::fs::read_to_string(produced_abs).unwrap()).unwrap();
            let mut expected_value: serde_json::Value =
                serde_json::from_str(&std::fs::read_to_string(expected_abs).unwrap()).unwrap();
            expand_config_token(&mut expected_value, &work);
            assert_eq!(
                produced_value,
                expected_value,
                "content mismatch for {}",
                rel.display()
            );
        }
    }
}
