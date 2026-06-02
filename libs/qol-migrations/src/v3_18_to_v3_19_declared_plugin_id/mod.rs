use anyhow::{Context, Result};
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

use crate::{FileMigration, MigrationReport};

/// Re-key all on-disk plugin state from a legacy alias to the plugin's declared
/// manifest id.
///
/// Before declared ids, a plugin's config was keyed by whichever name happened
/// to be used - the folder name or the binary command name (the two halves of
/// the old `PLUGIN_NAMES` arrays). Now identity is the manifest `id`. This
/// migration reads each installed plugin's manifest, derives `alias -> id` from
/// its `runtime.command` / `daemon.command`, and rewrites every artifact still
/// keyed by an alias: the registry, each profile's plugin lock, scoped config
/// filenames, hotkey bindings, and the runtime config dir.
///
/// It is a no-op for plugins whose alias already equals the declared id (the
/// common case), runs idempotently over partially-migrated state (only keys
/// still equal to an alias are touched), and never merges two plugins: an alias
/// that collides with a live id, a declared id claimed by two folders, or an
/// alias mapped to two ids is dropped rather than applied.
pub struct V3_18ToV3_19DeclaredPluginId;

impl V3_18ToV3_19DeclaredPluginId {
    pub fn new() -> Self {
        Self
    }

    pub fn default_for_production() -> Self {
        Self
    }
}

impl Default for V3_18ToV3_19DeclaredPluginId {
    fn default() -> Self {
        Self::new()
    }
}

const NAME: &str = "v3.18-to-v3.19-declared-plugin-id";

fn is_safe_id(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 64
        && !value.starts_with('-')
        && value
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
}

fn declared_id_of(manifest: &toml::Value, folder: &str) -> Option<String> {
    let declared = manifest
        .get("plugin")
        .and_then(|p| p.get("id"))
        .and_then(|i| i.as_str())
        .filter(|id| is_safe_id(id))
        .map(|s| s.to_string())
        .unwrap_or_else(|| folder.to_string());
    is_safe_id(&declared).then_some(declared)
}

fn command_aliases(manifest: &toml::Value, declared_id: &str) -> Vec<String> {
    let mut aliases = Vec::new();
    for section in ["runtime", "daemon"] {
        let command = manifest
            .get(section)
            .and_then(|s| s.get("command"))
            .and_then(|c| c.as_str());
        if let Some(command) = command {
            if is_safe_id(command)
                && command != declared_id
                && !aliases.contains(&command.to_string())
            {
                aliases.push(command.to_string());
            }
        }
    }
    aliases
}

/// `alias -> declared_id`, derived from installed manifests, with every
/// ambiguous mapping removed so the migration can never merge two plugins.
fn build_remap(plugins_dir: &Path) -> HashMap<String, String> {
    let Ok(read_dir) = std::fs::read_dir(plugins_dir) else {
        return HashMap::new();
    };
    let mut dirs: Vec<PathBuf> = read_dir
        .filter_map(|e| e.ok())
        .map(|e| e.path())
        .filter(|p| p.is_dir())
        .collect();
    dirs.sort();

    let mut plugins: Vec<(String, Vec<String>)> = Vec::new();
    let mut id_counts: HashMap<String, usize> = HashMap::new();
    for dir in dirs {
        let folder = dir.file_name().and_then(|n| n.to_str()).unwrap_or_default();
        let Ok(content) = std::fs::read_to_string(dir.join("plugin.toml")) else {
            continue;
        };
        let Ok(manifest) = toml::from_str::<toml::Value>(&content) else {
            continue;
        };
        let Some(declared_id) = declared_id_of(&manifest, folder) else {
            continue;
        };
        let aliases = command_aliases(&manifest, &declared_id);
        *id_counts.entry(declared_id.clone()).or_insert(0) += 1;
        plugins.push((declared_id, aliases));
    }

    let live_ids: HashSet<&String> = plugins.iter().map(|(id, _)| id).collect();
    let mut remap: HashMap<String, String> = HashMap::new();
    let mut conflicted: HashSet<String> = HashSet::new();
    for (id, aliases) in &plugins {
        if id_counts.get(id).copied().unwrap_or(0) > 1 {
            log::error!("[{NAME}] declared id {id:?} is claimed by more than one plugin; not remapping its aliases");
            continue;
        }
        for alias in aliases {
            if live_ids.contains(alias) {
                continue;
            }
            match remap.get(alias) {
                Some(existing) if existing != id => {
                    conflicted.insert(alias.clone());
                }
                _ => {
                    remap.insert(alias.clone(), id.clone());
                }
            }
        }
    }
    for alias in conflicted {
        log::error!("[{NAME}] alias {alias:?} maps to more than one plugin id; dropping it to avoid merging state");
        remap.remove(&alias);
    }
    remap
}

fn is_safe_path_component(name: &str) -> bool {
    !name.is_empty()
        && name != "."
        && name != ".."
        && !name.contains('/')
        && !name.contains('\\')
        && !name.contains('\0')
}

fn list_profile_dirs(profile_root: &Path) -> Result<Vec<PathBuf>> {
    if !profile_root.exists() {
        return Ok(Vec::new());
    }
    let mut out = Vec::new();
    for entry in std::fs::read_dir(profile_root)
        .with_context(|| format!("reading {}", profile_root.display()))?
    {
        let entry = entry?;
        if entry.file_type()?.is_dir() && entry.path().join("manifest.json").is_file() {
            out.push(entry.path());
        }
    }
    out.sort();
    Ok(out)
}

fn list_subdirs(dir: &Path) -> Vec<PathBuf> {
    let Ok(read_dir) = std::fs::read_dir(dir) else {
        return Vec::new();
    };
    let mut out: Vec<PathBuf> = read_dir
        .filter_map(|e| e.ok())
        .map(|e| e.path())
        .filter(|p| p.is_dir())
        .collect();
    out.sort();
    out
}

/// Every plugin-configs dir on disk for a profile: core, device, and each os
/// bucket. All buckets are walked so a synced foreign-OS profile is migrated
/// too, not just the running OS.
fn plugin_config_dirs(profile_dir: &Path) -> Vec<PathBuf> {
    let mut dirs = vec![
        profile_dir.join("core").join("plugin-configs"),
        profile_dir.join("device").join("plugin-configs"),
    ];
    for bucket in list_subdirs(&profile_dir.join("os")) {
        dirs.push(bucket.join("plugin-configs"));
    }
    dirs
}

fn hotkey_files(profile_dir: &Path) -> Vec<PathBuf> {
    list_subdirs(&profile_dir.join("os"))
        .into_iter()
        .map(|bucket| bucket.join("hotkeys.json"))
        .filter(|p| p.is_file())
        .collect()
}

fn write_json_atomic(path: &Path, value: &serde_json::Value) -> Result<()> {
    let parent = path
        .parent()
        .ok_or_else(|| anyhow::anyhow!("no parent for {}", path.display()))?;
    std::fs::create_dir_all(parent).with_context(|| format!("creating {}", parent.display()))?;
    let serialized = serde_json::to_string_pretty(value).context("serializing migrated json")?;
    let tmp = path.with_extension("json.tmp");
    std::fs::write(&tmp, serialized).with_context(|| format!("writing {}", tmp.display()))?;
    std::fs::rename(&tmp, path).with_context(|| format!("finalizing {}", path.display()))
}

/// Rewrite an `id_field` on each object in `value[array_key]` according to the
/// remap, but never onto an id that is already taken in the array (that would
/// merge two entries). Returns whether the file changed.
fn rewrite_array_ids(
    path: &Path,
    array_key: &str,
    id_field: &str,
    remap: &HashMap<String, String>,
) -> Result<bool> {
    if !path.is_file() {
        return Ok(false);
    }
    let raw =
        std::fs::read_to_string(path).with_context(|| format!("reading {}", path.display()))?;
    let mut value: serde_json::Value = match serde_json::from_str(&raw) {
        Ok(value) => value,
        Err(error) => {
            log::warn!("[{NAME}] skipping unparseable {}: {error}", path.display());
            return Ok(false);
        }
    };
    let Some(array) = value.get_mut(array_key).and_then(|v| v.as_array_mut()) else {
        return Ok(false);
    };

    let mut taken: HashSet<String> = array
        .iter()
        .filter_map(|o| o.get(id_field))
        .filter_map(|v| v.as_str())
        .map(|s| s.to_string())
        .collect();

    let mut changed = false;
    for object in array.iter_mut() {
        let Some(current) = object.get(id_field).and_then(|v| v.as_str()) else {
            continue;
        };
        let Some(new_id) = remap.get(current) else {
            continue;
        };
        if taken.contains(new_id) {
            log::warn!(
                "[{NAME}] {}: {id_field} {current:?} -> {new_id:?} skipped; target already present",
                path.display()
            );
            continue;
        }
        taken.insert(new_id.clone());
        object[id_field] = serde_json::Value::String(new_id.clone());
        changed = true;
    }

    if changed {
        write_json_atomic(path, &value)?;
    }
    Ok(changed)
}

fn legacy_sidecar_path(src: &Path) -> PathBuf {
    let mut name = src
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default();
    name.push_str(".legacy");
    src.with_file_name(name)
}

/// Move `src` to `dst`, preserving any differing existing `dst`: identical
/// content drops `src`, differing content parks `src` at a `.legacy` sidecar
/// and never clobbers `dst`.
fn move_preserving(src: &Path, dst: &Path) -> Result<bool> {
    if let Some(parent) = dst.parent() {
        std::fs::create_dir_all(parent)
            .with_context(|| format!("creating {}", parent.display()))?;
    }
    if !dst.exists() {
        std::fs::rename(src, dst)
            .with_context(|| format!("renaming {} to {}", src.display(), dst.display()))?;
        return Ok(true);
    }
    let src_bytes = std::fs::read(src).with_context(|| format!("reading {}", src.display()))?;
    let dst_bytes = std::fs::read(dst).with_context(|| format!("reading {}", dst.display()))?;
    if src_bytes == dst_bytes {
        std::fs::remove_file(src)
            .with_context(|| format!("removing redundant {}", src.display()))?;
    } else {
        let sidecar = legacy_sidecar_path(src);
        if sidecar.exists() {
            std::fs::remove_file(&sidecar)
                .with_context(|| format!("clearing prior sidecar {}", sidecar.display()))?;
        }
        std::fs::rename(src, &sidecar).with_context(|| {
            format!(
                "preserving legacy {} at {}",
                src.display(),
                sidecar.display()
            )
        })?;
        log::warn!(
            "[{NAME}] {} differs from existing {}; preserved legacy at {}",
            src.display(),
            dst.display(),
            sidecar.display()
        );
    }
    Ok(false)
}

/// Rename `<alias>.json` config files in `dir` to `<declared-id>.json`.
fn rename_config_files(
    dir: &Path,
    remap: &HashMap<String, String>,
    touched: &mut Vec<PathBuf>,
) -> Result<()> {
    if !dir.is_dir() {
        return Ok(());
    }
    let mut entries: Vec<PathBuf> = std::fs::read_dir(dir)
        .with_context(|| format!("reading {}", dir.display()))?
        .filter_map(|e| e.ok())
        .map(|e| e.path())
        .collect();
    entries.sort();

    for src in entries {
        if !src.is_file() || src.extension().is_none_or(|e| e != "json") {
            continue;
        }
        let Some(stem) = src.file_stem().and_then(|s| s.to_str()) else {
            continue;
        };
        let Some(new_id) = remap.get(stem) else {
            continue;
        };
        let dst = dir.join(format!("{new_id}.json"));
        if move_preserving(&src, &dst)? {
            touched.push(dst);
        }
    }
    Ok(())
}

/// Move runtime config from `plugins/<alias>/config.json` to
/// `plugins/<declared-id>/config.json`.
fn migrate_runtime_config_dirs(
    plugins_dir: &Path,
    remap: &HashMap<String, String>,
    touched: &mut Vec<PathBuf>,
) -> Result<()> {
    for (alias, new_id) in remap {
        if !is_safe_path_component(alias) || !is_safe_path_component(new_id) {
            continue;
        }
        let src = plugins_dir.join(alias).join("config.json");
        if !src.is_file() {
            continue;
        }
        let dst = plugins_dir.join(new_id).join("config.json");
        if move_preserving(&src, &dst)? {
            touched.push(dst);
        }
    }
    Ok(())
}

fn any_array_id_in_remap(
    path: &Path,
    array_key: &str,
    id_field: &str,
    remap: &HashMap<String, String>,
) -> bool {
    let Ok(raw) = std::fs::read_to_string(path) else {
        return false;
    };
    let Ok(value) = serde_json::from_str::<serde_json::Value>(&raw) else {
        return false;
    };
    value
        .get(array_key)
        .and_then(|v| v.as_array())
        .into_iter()
        .flatten()
        .filter_map(|o| o.get(id_field))
        .filter_map(|v| v.as_str())
        .any(|id| remap.contains_key(id))
}

fn any_config_file_in_remap(dir: &Path, remap: &HashMap<String, String>) -> bool {
    let Ok(read_dir) = std::fs::read_dir(dir) else {
        return false;
    };
    read_dir.filter_map(|e| e.ok()).any(|entry| {
        entry
            .path()
            .file_stem()
            .and_then(|s| s.to_str())
            .is_some_and(|stem| remap.contains_key(stem))
    })
}

fn migration_has_work(config_dir: &Path, remap: &HashMap<String, String>) -> bool {
    if any_array_id_in_remap(
        &config_dir.join("plugin-registry.json"),
        "entries",
        "id",
        remap,
    ) {
        return true;
    }
    for alias in remap.keys() {
        if config_dir
            .join("plugins")
            .join(alias)
            .join("config.json")
            .is_file()
        {
            return true;
        }
    }
    let Ok(profiles) = list_profile_dirs(&config_dir.join("profile")) else {
        return false;
    };
    for profile_dir in profiles {
        if any_array_id_in_remap(
            &profile_dir.join("core").join("plugins.lock.json"),
            "plugins",
            "id",
            remap,
        ) {
            return true;
        }
        if plugin_config_dirs(&profile_dir)
            .iter()
            .any(|dir| any_config_file_in_remap(dir, remap))
        {
            return true;
        }
        if hotkey_files(&profile_dir)
            .iter()
            .any(|file| any_array_id_in_remap(file, "hotkeys", "plugin_id", remap))
        {
            return true;
        }
    }
    false
}

impl FileMigration for V3_18ToV3_19DeclaredPluginId {
    fn name(&self) -> &'static str {
        NAME
    }

    fn applies(&self, config_dir: &Path) -> Result<bool> {
        let remap = build_remap(&config_dir.join("plugins"));
        if remap.is_empty() {
            return Ok(false);
        }
        Ok(migration_has_work(config_dir, &remap))
    }

    fn migrate(&self, config_dir: &Path, _archive_dir: &Path) -> Result<MigrationReport> {
        let remap = build_remap(&config_dir.join("plugins"));
        let mut touched = Vec::new();

        rewrite_array_ids(
            &config_dir.join("plugin-registry.json"),
            "entries",
            "id",
            &remap,
        )?;
        migrate_runtime_config_dirs(&config_dir.join("plugins"), &remap, &mut touched)?;

        for profile_dir in list_profile_dirs(&config_dir.join("profile"))? {
            rewrite_array_ids(
                &profile_dir.join("core").join("plugins.lock.json"),
                "plugins",
                "id",
                &remap,
            )?;
            for config_dir in plugin_config_dirs(&profile_dir) {
                rename_config_files(&config_dir, &remap, &mut touched)?;
            }
            for hotkeys in hotkey_files(&profile_dir) {
                rewrite_array_ids(&hotkeys, "hotkeys", "plugin_id", &remap)?;
            }
        }

        Ok(MigrationReport {
            name: NAME.to_string(),
            archived: touched,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn migration() -> V3_18ToV3_19DeclaredPluginId {
        V3_18ToV3_19DeclaredPluginId::new()
    }

    fn archive(dir: &Path) -> PathBuf {
        let path = dir.join("archive");
        std::fs::create_dir_all(&path).unwrap();
        path
    }

    fn run(config_dir: &Path) -> MigrationReport {
        migration()
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

    fn read_json(path: &Path) -> serde_json::Value {
        serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap()
    }

    fn write_manifest(config_dir: &Path, folder: &str, id: &str, command: Option<&str>) {
        let runtime = command
            .map(|c| format!("\n[runtime]\ncommand = \"{c}\"\n"))
            .unwrap_or_default();
        let toml = format!(
            "[plugin]\nid = \"{id}\"\nname = \"x\"\ndescription = \"\"\nversion = \"1.0.0\"\n\n[menu]\nlabel = \"x\"\nitems = []\n{runtime}"
        );
        write(
            &config_dir.join("plugins").join(folder).join("plugin.toml"),
            toml.as_bytes(),
        );
    }

    fn setup_profile(config_dir: &Path, name: &str) -> PathBuf {
        let root = config_dir.join("profile").join(name);
        write(&root.join("manifest.json"), b"{\"version\":1}");
        root
    }

    #[test]
    fn remap_derives_alias_from_the_binary_command() {
        let dir = tempfile::tempdir().unwrap();
        write_manifest(
            dir.path(),
            "plugin-alt-tab",
            "plugin-alt-tab",
            Some("alt-tab"),
        );
        let remap = build_remap(&dir.path().join("plugins"));
        assert_eq!(remap.get("alt-tab"), Some(&"plugin-alt-tab".to_string()));
        assert_eq!(remap.len(), 1);
    }

    #[test]
    fn plugin_whose_command_equals_its_id_yields_no_remap() {
        let dir = tempfile::tempdir().unwrap();
        write_manifest(
            dir.path(),
            "plugin-lights",
            "plugin-lights",
            Some("plugin-lights"),
        );
        write_manifest(dir.path(), "plugin-os-themes", "plugin-os-themes", None);
        assert!(build_remap(&dir.path().join("plugins")).is_empty());
        assert!(!migration().applies(dir.path()).unwrap());
    }

    #[test]
    fn registry_entry_id_is_rekeyed_from_alias_to_declared_id() {
        let dir = tempfile::tempdir().unwrap();
        write_manifest(
            dir.path(),
            "plugin-alt-tab",
            "plugin-alt-tab",
            Some("alt-tab"),
        );
        write_json(
            &dir.path().join("plugin-registry.json"),
            &json!({"version":1,"entries":[{"id":"alt-tab","active":{"path":"/p","source":{"type":"release-asset"}}}]}),
        );

        assert!(migration().applies(dir.path()).unwrap());
        run(dir.path());

        let registry = read_json(&dir.path().join("plugin-registry.json"));
        assert_eq!(registry["entries"][0]["id"], "plugin-alt-tab");
    }

    #[test]
    fn lock_and_hotkeys_are_rekeyed_but_binding_id_is_left_alone() {
        let dir = tempfile::tempdir().unwrap();
        write_manifest(
            dir.path(),
            "plugin-alt-tab",
            "plugin-alt-tab",
            Some("alt-tab"),
        );
        let profile = setup_profile(dir.path(), "default");
        write_json(
            &profile.join("core").join("plugins.lock.json"),
            &json!({"version":1,"plugins":[{"id":"alt-tab","repo_url":"x","version":"1.0.0"}]}),
        );
        write_json(
            &profile.join("os").join("linux").join("hotkeys.json"),
            &json!({"hotkeys":[{"id":"hk-1","key":"Super+Tab","plugin_id":"alt-tab","action":"open","enabled":true}]}),
        );

        run(dir.path());

        let lock = read_json(&profile.join("core").join("plugins.lock.json"));
        assert_eq!(lock["plugins"][0]["id"], "plugin-alt-tab");
        let hotkeys = read_json(&profile.join("os").join("linux").join("hotkeys.json"));
        assert_eq!(hotkeys["hotkeys"][0]["plugin_id"], "plugin-alt-tab");
        assert_eq!(
            hotkeys["hotkeys"][0]["id"], "hk-1",
            "the binding's own id must never be rewritten"
        );
    }

    #[test]
    fn scoped_config_filenames_are_renamed_across_core_device_and_every_os_bucket() {
        let dir = tempfile::tempdir().unwrap();
        write_manifest(
            dir.path(),
            "plugin-alt-tab",
            "plugin-alt-tab",
            Some("alt-tab"),
        );
        let profile = setup_profile(dir.path(), "default");
        write(&profile.join("core/plugin-configs/alt-tab.json"), b"\"c\"");
        write(
            &profile.join("device/plugin-configs/alt-tab.json"),
            b"\"d\"",
        );
        write(
            &profile.join("os/linux/plugin-configs/alt-tab.json"),
            b"\"l\"",
        );
        write(
            &profile.join("os/macos/plugin-configs/alt-tab.json"),
            b"\"m\"",
        );

        run(dir.path());

        for (scope, expected) in [
            ("core/plugin-configs", "\"c\""),
            ("device/plugin-configs", "\"d\""),
            ("os/linux/plugin-configs", "\"l\""),
            ("os/macos/plugin-configs", "\"m\""),
        ] {
            let renamed = profile.join(scope).join("plugin-alt-tab.json");
            assert!(renamed.is_file(), "{scope} should be renamed");
            assert_eq!(std::fs::read_to_string(&renamed).unwrap(), expected);
            assert!(
                !profile.join(scope).join("alt-tab.json").exists(),
                "{scope} alias file removed"
            );
        }
    }

    #[test]
    fn runtime_config_dir_is_moved_to_the_declared_id() {
        let dir = tempfile::tempdir().unwrap();
        write_manifest(
            dir.path(),
            "plugin-alt-tab",
            "plugin-alt-tab",
            Some("alt-tab"),
        );
        write(
            &dir.path().join("plugins/alt-tab/config.json"),
            b"\"runtime\"",
        );

        run(dir.path());

        let moved = dir.path().join("plugins/plugin-alt-tab/config.json");
        assert!(moved.is_file());
        assert_eq!(std::fs::read_to_string(&moved).unwrap(), "\"runtime\"");
        assert!(!dir.path().join("plugins/alt-tab/config.json").exists());
    }

    #[test]
    fn migration_is_idempotent_on_a_second_pass() {
        let dir = tempfile::tempdir().unwrap();
        write_manifest(
            dir.path(),
            "plugin-alt-tab",
            "plugin-alt-tab",
            Some("alt-tab"),
        );
        let profile = setup_profile(dir.path(), "default");
        write(&profile.join("core/plugin-configs/alt-tab.json"), b"\"c\"");
        write_json(
            &profile.join("os/linux/hotkeys.json"),
            &json!({"hotkeys":[{"id":"hk","key":"K","plugin_id":"alt-tab","action":"open","enabled":true}]}),
        );

        run(dir.path());
        assert!(!migration().applies(dir.path()).unwrap());
        run(dir.path());

        assert!(profile
            .join("core/plugin-configs/plugin-alt-tab.json")
            .is_file());
        let hotkeys = read_json(&profile.join("os/linux/hotkeys.json"));
        assert_eq!(hotkeys["hotkeys"][0]["plugin_id"], "plugin-alt-tab");
    }

    #[test]
    fn many_to_one_alias_is_dropped_rather_than_merging_two_plugins() {
        let dir = tempfile::tempdir().unwrap();
        write_manifest(dir.path(), "plugin-a", "plugin-a", Some("shared"));
        write_manifest(dir.path(), "plugin-b", "plugin-b", Some("shared"));
        let remap = build_remap(&dir.path().join("plugins"));
        assert!(
            !remap.contains_key("shared"),
            "an alias claimed by two plugins must not be remapped"
        );
    }

    #[test]
    fn alias_that_is_a_live_declared_id_is_never_remapped() {
        let dir = tempfile::tempdir().unwrap();
        write_manifest(dir.path(), "real", "real", None);
        write_manifest(dir.path(), "other", "other", Some("real"));
        let remap = build_remap(&dir.path().join("plugins"));
        assert!(
            !remap.contains_key("real"),
            "an alias colliding with a live id must not be remapped"
        );
    }

    #[test]
    fn rekey_is_skipped_when_the_target_id_already_exists_in_the_array() {
        let dir = tempfile::tempdir().unwrap();
        write_manifest(
            dir.path(),
            "plugin-alt-tab",
            "plugin-alt-tab",
            Some("alt-tab"),
        );
        write_json(
            &dir.path().join("plugin-registry.json"),
            &json!({"version":1,"entries":[
                {"id":"alt-tab","active":{"path":"/old","source":{"type":"release-asset"}}},
                {"id":"plugin-alt-tab","active":{"path":"/new","source":{"type":"release-asset"}}}
            ]}),
        );

        run(dir.path());

        let registry = read_json(&dir.path().join("plugin-registry.json"));
        let ids: Vec<&str> = registry["entries"]
            .as_array()
            .unwrap()
            .iter()
            .map(|e| e["id"].as_str().unwrap())
            .collect();
        assert_eq!(
            ids,
            vec!["alt-tab", "plugin-alt-tab"],
            "must not collapse two entries onto one id"
        );
    }

    #[test]
    fn each_profile_is_migrated_independently() {
        let dir = tempfile::tempdir().unwrap();
        write_manifest(
            dir.path(),
            "plugin-alt-tab",
            "plugin-alt-tab",
            Some("alt-tab"),
        );
        let default = setup_profile(dir.path(), "default");
        let work = setup_profile(dir.path(), "work");
        write(&default.join("core/plugin-configs/alt-tab.json"), b"\"d\"");
        write(&work.join("core/plugin-configs/alt-tab.json"), b"\"w\"");

        run(dir.path());

        assert_eq!(
            std::fs::read_to_string(default.join("core/plugin-configs/plugin-alt-tab.json"))
                .unwrap(),
            "\"d\""
        );
        assert_eq!(
            std::fs::read_to_string(work.join("core/plugin-configs/plugin-alt-tab.json")).unwrap(),
            "\"w\""
        );
    }
}
