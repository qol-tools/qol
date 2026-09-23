use anyhow::{Context, Result};
use std::collections::HashMap;
use std::path::{Path, PathBuf};

use crate::fs_util::{
    hotkey_files, is_safe_id, is_safe_path_component, list_profile_dirs, plugin_config_dirs,
    write_json_atomic,
};
use crate::{FileMigration, MigrationReport};

mod legacy_table;
use legacy_table::{LEGACY_ID_TO_UID, LEGACY_RENAMED_IDS};

/// Re-key all on-disk synced plugin state from the plugin `id` to its stable
/// `uid`.
///
/// Identity used to be the manifest `id`. It is now a never-renamed `uid`, so
/// the uid-keyed runtime needs the existing artifacts re-keyed: each lock entry
/// gains a `uid` field, hotkey bindings carry `plugin_uid` instead of
/// `plugin_id`, and scoped config files are named `<uid>.json`.
///
/// The `id -> uid` remap is the hardcoded table for plugins not installed
/// locally, overlaid with the uids read from installed `plugin.toml` manifests
/// (installed wins). An id absent from both is left as its own uid (`uid = id`,
/// the transitional fallback); data is never dropped.
///
/// Lock entries that collapse onto a shared uid (the renamed
/// `plugin-screen-recorder` onto `qol-shot`'s uid, say) are coalesced into one,
/// preferring the canonical entry whose `id` already equals its `uid`. Config
/// renames never overwrite an existing `<uid>.json`. The migration is
/// shape-driven, so a second pass is a no-op.
pub struct V3_19ToV3_20PluginUid;

impl V3_19ToV3_20PluginUid {
    pub fn new() -> Self {
        Self
    }

    pub fn default_for_production() -> Self {
        Self
    }
}

impl Default for V3_19ToV3_20PluginUid {
    fn default() -> Self {
        Self::new()
    }
}

impl FileMigration for V3_19ToV3_20PluginUid {
    fn name(&self) -> &'static str {
        NAME
    }

    fn applies(&self, config_dir: &Path) -> Result<bool> {
        let remap = build_remap(&config_dir.join("plugins"));
        for profile_dir in list_profile_dirs(&config_dir.join("profile"))? {
            if lock_has_entry_without_uid(&profile_dir.join("core").join("plugins.lock.json")) {
                return Ok(true);
            }
            if hotkey_files(&profile_dir)
                .iter()
                .any(|file| hotkeys_have_plugin_id_key(file))
            {
                return Ok(true);
            }
            if plugin_config_dirs(&profile_dir)
                .iter()
                .any(|dir| config_file_remaps_to_other_uid(dir, &remap))
            {
                return Ok(true);
            }
        }
        Ok(false)
    }

    fn migrate(&self, config_dir: &Path, _archive_dir: &Path) -> Result<MigrationReport> {
        let remap = build_remap(&config_dir.join("plugins"));
        let mut touched = Vec::new();

        for profile_dir in list_profile_dirs(&config_dir.join("profile"))? {
            migrate_lock(&profile_dir.join("core").join("plugins.lock.json"), &remap)?;
            for dir in plugin_config_dirs(&profile_dir) {
                rename_config_files(&dir, &remap, &mut touched)?;
            }
            for hotkeys in hotkey_files(&profile_dir) {
                migrate_hotkeys(&hotkeys, &remap)?;
            }
        }

        Ok(MigrationReport {
            name: NAME.to_string(),
            archived: touched,
        })
    }
}

const NAME: &str = "v3.19-to-v3.20-plugin-uid";

fn manifest_uid_of(manifest: &toml::Value) -> Option<String> {
    manifest
        .get("plugin")
        .and_then(|p| p.get("uid"))
        .and_then(|u| u.as_str())
        .filter(|uid| is_safe_id(uid))
        .map(|s| s.to_string())
}

fn manifest_id_of(manifest: &toml::Value, folder: &str) -> Option<String> {
    let declared = manifest
        .get("plugin")
        .and_then(|p| p.get("id"))
        .and_then(|i| i.as_str())
        .filter(|id| is_safe_id(id))
        .map(|s| s.to_string())
        .unwrap_or_else(|| folder.to_string());
    is_safe_id(&declared).then_some(declared)
}

/// `id -> uid`, the hardcoded table overlaid with installed manifests.
/// Installed manifests win so a locally-installed plugin's current uid takes
/// precedence over the pinned literal.
fn build_remap(plugins_dir: &Path) -> HashMap<String, String> {
    let mut remap: HashMap<String, String> = LEGACY_ID_TO_UID
        .iter()
        .filter(|(id, uid)| is_safe_id(id) && is_safe_id(uid))
        .map(|(id, uid)| (id.to_string(), uid.to_string()))
        .collect();

    let Ok(read_dir) = std::fs::read_dir(plugins_dir) else {
        return remap;
    };
    let mut dirs: Vec<PathBuf> = read_dir
        .filter_map(|e| e.ok())
        .map(|e| e.path())
        .filter(|p| p.is_dir())
        .collect();
    dirs.sort();

    for dir in dirs {
        let folder = dir.file_name().and_then(|n| n.to_str()).unwrap_or_default();
        let Ok(content) = std::fs::read_to_string(dir.join("plugin.toml")) else {
            continue;
        };
        let Ok(manifest) = toml::from_str::<toml::Value>(&content) else {
            continue;
        };
        let Some(id) = manifest_id_of(&manifest, folder) else {
            continue;
        };
        let Some(uid) = manifest_uid_of(&manifest) else {
            continue;
        };
        remap.insert(id, uid);
    }
    remap
}

/// `id -> uid` with the identity fallback applied: an id absent from the remap
/// is its own uid. Never returns `None`, so no artifact is ever left unkeyed.
fn uid_for(id: &str, remap: &HashMap<String, String>) -> String {
    remap.get(id).cloned().unwrap_or_else(|| id.to_string())
}

fn read_json(path: &Path) -> Option<serde_json::Value> {
    let raw = std::fs::read_to_string(path).ok()?;
    match serde_json::from_str(&raw) {
        Ok(value) => Some(value),
        Err(error) => {
            log::warn!("[{NAME}] skipping unparseable {}: {error}", path.display());
            None
        }
    }
}

fn lock_has_entry_without_uid(path: &Path) -> bool {
    let Some(value) = read_json(path) else {
        return false;
    };
    value
        .get("plugins")
        .and_then(|v| v.as_array())
        .into_iter()
        .flatten()
        .any(|entry| entry.get("uid").and_then(|u| u.as_str()).is_none())
}

fn hotkeys_have_plugin_id_key(path: &Path) -> bool {
    let Some(value) = read_json(path) else {
        return false;
    };
    value
        .get("hotkeys")
        .and_then(|v| v.as_array())
        .into_iter()
        .flatten()
        .any(|binding| binding.get("plugin_id").is_some())
}

/// A config file is pending work only if it would actually be renamed: its uid
/// differs from its current name AND the `<uid>.json` target is not already
/// present. A file whose rename is permanently blocked by an existing target
/// must not keep `applies()` true forever.
fn config_file_remaps_to_other_uid(dir: &Path, remap: &HashMap<String, String>) -> bool {
    let Ok(read_dir) = std::fs::read_dir(dir) else {
        return false;
    };
    read_dir.filter_map(|e| e.ok()).any(|entry| {
        let path = entry.path();
        if path.extension().is_none_or(|e| e != "json") {
            return false;
        }
        let Some(stem) = path.file_stem().and_then(|s| s.to_str()) else {
            return false;
        };
        let uid = uid_for(stem, remap);
        uid != stem && is_safe_path_component(&uid) && !dir.join(format!("{uid}.json")).exists()
    })
}

fn lock_entry_id(entry: &serde_json::Value) -> Option<&str> {
    entry.get("id").and_then(|v| v.as_str())
}

/// Rank for choosing which of two lock entries sharing a uid survives. Higher
/// wins. An entry whose id already equals its uid (the transitional uid=id
/// case) is the most canonical; an entry whose id is a known rename alias is the
/// least. Ties keep the first seen.
fn canonicality_rank(id: &str, uid: &str) -> u8 {
    if id == uid {
        return 2;
    }
    if LEGACY_RENAMED_IDS.contains(&id) {
        return 0;
    }
    1
}

/// Add `uid = remap(id)` to every lock entry, then collapse entries that share
/// a uid into one, keeping the most canonical (see `canonicality_rank`).
/// Requires `uid` on every surviving entry afterward.
fn migrate_lock(path: &Path, remap: &HashMap<String, String>) -> Result<()> {
    if !path.is_file() {
        return Ok(());
    }
    let Some(mut value) = read_json(path) else {
        return Ok(());
    };
    let Some(entries) = value.get("plugins").and_then(|v| v.as_array()) else {
        return Ok(());
    };

    let mut order: Vec<String> = Vec::new();
    let mut chosen: HashMap<String, serde_json::Value> = HashMap::new();
    for entry in entries {
        let Some(id) = lock_entry_id(entry).map(|s| s.to_string()) else {
            log::warn!(
                "[{NAME}] {}: lock entry has no id field; dropping entry",
                path.display()
            );
            continue;
        };
        let uid = uid_for(&id, remap);
        let mut keyed = entry.clone();
        keyed["uid"] = serde_json::Value::String(uid.clone());

        let Some(existing) = chosen.get(&uid) else {
            order.push(uid.clone());
            chosen.insert(uid, keyed);
            continue;
        };
        let existing_rank = lock_entry_id(existing)
            .map(|existing_id| canonicality_rank(existing_id, &uid))
            .unwrap_or(0);
        let candidate_rank = canonicality_rank(&id, &uid);
        if candidate_rank > existing_rank {
            chosen.insert(uid.clone(), keyed);
        }
        log::warn!(
            "[{NAME}] {}: lock entry id {id:?} coalesced into existing uid {uid:?}",
            path.display()
        );
    }

    let coalesced: Vec<serde_json::Value> = order
        .into_iter()
        .filter_map(|uid| chosen.remove(&uid))
        .collect();
    value["plugins"] = serde_json::Value::Array(coalesced);
    write_json_atomic(path, &value)
}

/// Rename `plugin_id` to `plugin_uid` on each binding, with value `remap(id)`,
/// removing the old key. Idempotent: a binding already carrying `plugin_uid`
/// and no `plugin_id` is untouched.
fn migrate_hotkeys(path: &Path, remap: &HashMap<String, String>) -> Result<()> {
    if !path.is_file() {
        return Ok(());
    }
    let Some(mut value) = read_json(path) else {
        return Ok(());
    };
    let Some(bindings) = value.get_mut("hotkeys").and_then(|v| v.as_array_mut()) else {
        return Ok(());
    };

    let mut changed = false;
    bindings.retain_mut(|binding| {
        let Some(object) = binding.as_object_mut() else {
            return true;
        };
        let Some(old) = object.remove("plugin_id") else {
            return true;
        };
        let Some(id) = old.as_str() else {
            log::warn!(
                "[{NAME}] {}: hotkey binding has non-string plugin_id; dropping binding",
                path.display()
            );
            changed = true;
            return false;
        };
        let uid = uid_for(id, remap);
        object.insert("plugin_uid".to_string(), serde_json::Value::String(uid));
        changed = true;
        true
    });

    if changed {
        write_json_atomic(path, &value)?;
    }
    Ok(())
}

/// Rename `<id>.json` config files to `<uid>.json`. If `<uid>.json` already
/// exists (two ids onto one uid), the existing target wins: log and skip,
/// never merge or overwrite.
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
        let uid = uid_for(stem, remap);
        if uid == stem {
            continue;
        }
        if !is_safe_path_component(&uid) {
            continue;
        }
        let dst = dir.join(format!("{uid}.json"));
        if dst.exists() {
            log::warn!(
                "[{NAME}] {}: config rename to {} skipped; target already present",
                src.display(),
                dst.display()
            );
            continue;
        }
        std::fs::rename(&src, &dst)
            .with_context(|| format!("renaming {} to {}", src.display(), dst.display()))?;
        touched.push(dst);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn migration() -> V3_19ToV3_20PluginUid {
        V3_19ToV3_20PluginUid::new()
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

    fn read_back(path: &Path) -> serde_json::Value {
        serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap()
    }

    fn write_manifest(config_dir: &Path, folder: &str, id: &str, uid: Option<&str>) {
        let uid_line = uid.map(|u| format!("uid = \"{u}\"\n")).unwrap_or_default();
        let toml = format!(
            "[plugin]\nid = \"{id}\"\n{uid_line}name = \"x\"\ndescription = \"\"\nversion = \"1.0.0\"\n\n[menu]\nlabel = \"x\"\nitems = []\n"
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

    const ALT_TAB_UID: &str = "a7f48ac7-3cd5-4402-a1fe-d517fbce0fd6";
    const QOL_SHOT_UID: &str = "e8208e3e-58b3-4f8c-ad4b-ddbecafa3375";

    #[test]
    fn lock_entry_for_installed_plugin_gets_its_manifest_uid() {
        let dir = tempfile::tempdir().unwrap();
        write_manifest(
            dir.path(),
            "plugin-alt-tab",
            "plugin-alt-tab",
            Some(ALT_TAB_UID),
        );
        let profile = setup_profile(dir.path(), "default");
        write_json(
            &profile.join("core").join("plugins.lock.json"),
            &json!({"version":1,"plugins":[
                {"id":"plugin-alt-tab","repo_url":"x","version":"1.0.0"}
            ]}),
        );

        assert!(migration().applies(dir.path()).unwrap());
        run(dir.path());

        let lock = read_back(&profile.join("core").join("plugins.lock.json"));
        assert_eq!(lock["plugins"][0]["uid"], ALT_TAB_UID);
        assert_eq!(
            lock["plugins"][0]["id"], "plugin-alt-tab",
            "id stays as-is alongside the added uid"
        );
    }

    #[test]
    fn screen_recorder_lock_entry_rekeys_to_qol_shot_uid_via_table_when_not_installed() {
        let dir = tempfile::tempdir().unwrap();
        let profile = setup_profile(dir.path(), "default");
        write_json(
            &profile.join("core").join("plugins.lock.json"),
            &json!({"version":1,"plugins":[
                {"id":"plugin-screen-recorder","repo_url":"x","version":"1.0.0","platforms":["linux"]}
            ]}),
        );

        run(dir.path());

        let lock = read_back(&profile.join("core").join("plugins.lock.json"));
        let plugins = lock["plugins"].as_array().unwrap();
        assert_eq!(plugins.len(), 1);
        assert_eq!(plugins[0]["uid"], QOL_SHOT_UID);
        assert_eq!(plugins[0]["id"], "plugin-screen-recorder");
        assert_eq!(plugins[0]["platforms"][0], "linux");
    }

    #[test]
    fn screen_recorder_and_qol_shot_lock_entries_coalesce_to_one_canonical_entry() {
        let dir = tempfile::tempdir().unwrap();
        let profile = setup_profile(dir.path(), "default");
        write_json(
            &profile.join("core").join("plugins.lock.json"),
            &json!({"version":1,"plugins":[
                {"id":"plugin-screen-recorder","repo_url":"old","version":"0.9.0","platforms":["linux"]},
                {"id":"qol-shot","repo_url":"new","version":"1.2.0"}
            ]}),
        );

        run(dir.path());

        let lock = read_back(&profile.join("core").join("plugins.lock.json"));
        let plugins = lock["plugins"].as_array().unwrap();
        assert_eq!(
            plugins.len(),
            1,
            "two ids onto one uid collapse to one entry"
        );
        assert_eq!(plugins[0]["uid"], QOL_SHOT_UID);
        assert_eq!(
            plugins[0]["id"], "qol-shot",
            "the canonical entry (id == uid-source) is kept, not the stale alias"
        );
        assert_eq!(plugins[0]["repo_url"], "new");
    }

    #[test]
    fn hotkey_plugin_id_becomes_plugin_uid_with_mapped_value() {
        let dir = tempfile::tempdir().unwrap();
        write_manifest(
            dir.path(),
            "plugin-alt-tab",
            "plugin-alt-tab",
            Some(ALT_TAB_UID),
        );
        let profile = setup_profile(dir.path(), "default");
        write_json(
            &profile.join("os").join("linux").join("hotkeys.json"),
            &json!({"hotkeys":[
                {"id":"hk-1","key":"Super+Tab","plugin_id":"plugin-alt-tab","action":"open","enabled":true}
            ]}),
        );

        assert!(migration().applies(dir.path()).unwrap());
        run(dir.path());

        let hotkeys = read_back(&profile.join("os").join("linux").join("hotkeys.json"));
        let binding = &hotkeys["hotkeys"][0];
        assert_eq!(binding["plugin_uid"], ALT_TAB_UID);
        assert!(
            binding.get("plugin_id").is_none(),
            "the old plugin_id key must be removed"
        );
        assert_eq!(binding["id"], "hk-1", "the binding's own id is untouched");
    }

    #[test]
    fn scoped_config_filenames_are_renamed_to_uid_across_every_bucket() {
        let dir = tempfile::tempdir().unwrap();
        write_manifest(
            dir.path(),
            "plugin-alt-tab",
            "plugin-alt-tab",
            Some(ALT_TAB_UID),
        );
        let profile = setup_profile(dir.path(), "default");
        write(
            &profile.join("core/plugin-configs/plugin-alt-tab.json"),
            b"\"c\"",
        );
        write(
            &profile.join("device/plugin-configs/plugin-alt-tab.json"),
            b"\"d\"",
        );
        write(
            &profile.join("os/linux/plugin-configs/plugin-alt-tab.json"),
            b"\"l\"",
        );
        write(
            &profile.join("os/macos/plugin-configs/plugin-alt-tab.json"),
            b"\"m\"",
        );

        assert!(migration().applies(dir.path()).unwrap());
        run(dir.path());

        for (scope, expected) in [
            ("core/plugin-configs", "\"c\""),
            ("device/plugin-configs", "\"d\""),
            ("os/linux/plugin-configs", "\"l\""),
            ("os/macos/plugin-configs", "\"m\""),
        ] {
            let renamed = profile.join(scope).join(format!("{ALT_TAB_UID}.json"));
            assert!(renamed.is_file(), "{scope} should be renamed to uid");
            assert_eq!(std::fs::read_to_string(&renamed).unwrap(), expected);
            assert!(
                !profile.join(scope).join("plugin-alt-tab.json").exists(),
                "{scope} id-named file removed"
            );
        }
    }

    #[test]
    fn config_rename_skips_when_uid_target_already_exists() {
        let dir = tempfile::tempdir().unwrap();
        let profile = setup_profile(dir.path(), "default");
        let configs = profile.join("core/plugin-configs");
        write(&configs.join("plugin-screen-recorder.json"), b"\"stale\"");
        write(
            &configs.join(format!("{QOL_SHOT_UID}.json")),
            b"\"canonical\"",
        );

        run(dir.path());

        assert_eq!(
            std::fs::read_to_string(configs.join(format!("{QOL_SHOT_UID}.json"))).unwrap(),
            "\"canonical\"",
            "existing uid target must not be overwritten"
        );
        assert!(
            configs.join("plugin-screen-recorder.json").is_file(),
            "the source is left in place when the rename is skipped"
        );
    }

    #[test]
    fn migration_is_idempotent_on_a_second_pass() {
        let dir = tempfile::tempdir().unwrap();
        write_manifest(
            dir.path(),
            "plugin-alt-tab",
            "plugin-alt-tab",
            Some(ALT_TAB_UID),
        );
        let profile = setup_profile(dir.path(), "default");
        write_json(
            &profile.join("core").join("plugins.lock.json"),
            &json!({"version":1,"plugins":[
                {"id":"plugin-alt-tab","repo_url":"x","version":"1.0.0"}
            ]}),
        );
        write_json(
            &profile.join("os/linux/hotkeys.json"),
            &json!({"hotkeys":[
                {"id":"hk","key":"K","plugin_id":"plugin-alt-tab","action":"open","enabled":true}
            ]}),
        );
        write(
            &profile.join("core/plugin-configs/plugin-alt-tab.json"),
            b"\"c\"",
        );

        run(dir.path());
        assert!(
            !migration().applies(dir.path()).unwrap(),
            "after one pass the migration must report nothing left to do"
        );
        run(dir.path());

        let lock = read_back(&profile.join("core").join("plugins.lock.json"));
        assert_eq!(lock["plugins"][0]["uid"], ALT_TAB_UID);
        assert_eq!(lock["plugins"].as_array().unwrap().len(), 1);
        let hotkeys = read_back(&profile.join("os/linux/hotkeys.json"));
        assert_eq!(hotkeys["hotkeys"][0]["plugin_uid"], ALT_TAB_UID);
        assert!(profile
            .join("core/plugin-configs")
            .join(format!("{ALT_TAB_UID}.json"))
            .is_file());
    }

    #[test]
    fn unknown_id_falls_back_to_uid_equals_id_and_drops_no_data() {
        let dir = tempfile::tempdir().unwrap();
        let profile = setup_profile(dir.path(), "default");
        write_json(
            &profile.join("core").join("plugins.lock.json"),
            &json!({"version":1,"plugins":[
                {"id":"plugin-unknown","repo_url":"x","version":"1.0.0"}
            ]}),
        );
        write_json(
            &profile.join("os/linux/hotkeys.json"),
            &json!({"hotkeys":[
                {"id":"hk","key":"K","plugin_id":"plugin-unknown","action":"open","enabled":true}
            ]}),
        );
        write(
            &profile.join("core/plugin-configs/plugin-unknown.json"),
            b"\"u\"",
        );

        run(dir.path());

        let lock = read_back(&profile.join("core").join("plugins.lock.json"));
        assert_eq!(
            lock["plugins"][0]["uid"], "plugin-unknown",
            "unmapped id becomes its own uid; the lock entry is never left unkeyed"
        );
        let hotkeys = read_back(&profile.join("os/linux/hotkeys.json"));
        assert_eq!(hotkeys["hotkeys"][0]["plugin_uid"], "plugin-unknown");
        assert!(
            profile
                .join("core/plugin-configs/plugin-unknown.json")
                .is_file(),
            "an id-named config that maps to itself is left in place, not deleted"
        );
    }

    #[test]
    fn applies_is_false_on_already_uid_keyed_state() {
        let dir = tempfile::tempdir().unwrap();
        let profile = setup_profile(dir.path(), "default");
        write_json(
            &profile.join("core").join("plugins.lock.json"),
            &json!({"version":1,"plugins":[
                {"uid":ALT_TAB_UID,"id":"plugin-alt-tab","repo_url":"x","version":"1.0.0"}
            ]}),
        );
        write_json(
            &profile.join("os/linux/hotkeys.json"),
            &json!({"hotkeys":[
                {"id":"hk","key":"K","plugin_uid":ALT_TAB_UID,"action":"open","enabled":true}
            ]}),
        );
        write(
            &profile.join(format!("core/plugin-configs/{ALT_TAB_UID}.json")),
            b"\"c\"",
        );

        assert!(!migration().applies(dir.path()).unwrap());
    }

    #[test]
    fn installed_manifest_uid_wins_over_legacy_table() {
        let dir = tempfile::tempdir().unwrap();
        write_manifest(
            dir.path(),
            "plugin-alt-tab",
            "plugin-alt-tab",
            Some("override-uid"),
        );
        let remap = build_remap(&dir.path().join("plugins"));
        assert_eq!(
            remap.get("plugin-alt-tab"),
            Some(&"override-uid".to_string()),
            "a locally-installed manifest uid overrides the pinned table literal"
        );
    }

    #[test]
    fn each_profile_is_migrated_independently() {
        let dir = tempfile::tempdir().unwrap();
        write_manifest(
            dir.path(),
            "plugin-alt-tab",
            "plugin-alt-tab",
            Some(ALT_TAB_UID),
        );
        let default = setup_profile(dir.path(), "default");
        let work = setup_profile(dir.path(), "work");
        write(
            &default.join("core/plugin-configs/plugin-alt-tab.json"),
            b"\"d\"",
        );
        write(
            &work.join("core/plugin-configs/plugin-alt-tab.json"),
            b"\"w\"",
        );

        run(dir.path());

        assert_eq!(
            std::fs::read_to_string(
                default.join(format!("core/plugin-configs/{ALT_TAB_UID}.json"))
            )
            .unwrap(),
            "\"d\""
        );
        assert_eq!(
            std::fs::read_to_string(work.join(format!("core/plugin-configs/{ALT_TAB_UID}.json")))
                .unwrap(),
            "\"w\""
        );
    }

    fn fixture_root() -> PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR")).join("fixtures/v3_19_to_v3_20_plugin_uid")
    }

    fn copy_tree(src: &Path, dst: &Path) {
        std::fs::create_dir_all(dst).unwrap();
        for entry in std::fs::read_dir(src).unwrap() {
            let entry = entry.unwrap();
            let from = entry.path();
            let to = dst.join(entry.file_name());
            if entry.file_type().unwrap().is_dir() {
                copy_tree(&from, &to);
                continue;
            }
            std::fs::copy(&from, &to).unwrap();
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
    fn malformed_hotkey_binding_is_dropped_and_valid_binding_is_kept() {
        let dir = tempfile::tempdir().unwrap();
        write_manifest(
            dir.path(),
            "plugin-alt-tab",
            "plugin-alt-tab",
            Some(ALT_TAB_UID),
        );
        let profile = setup_profile(dir.path(), "default");
        write_json(
            &profile.join("os").join("linux").join("hotkeys.json"),
            &json!({"hotkeys":[
                {"id":"hk-1","key":"Super+Tab","plugin_id":"plugin-alt-tab","action":"open","enabled":true},
                {"id":"hk-2","key":"Super+Q","plugin_id":42,"action":"close","enabled":true}
            ]}),
        );

        assert!(migration().applies(dir.path()).unwrap());
        run(dir.path());

        let hotkeys = read_back(&profile.join("os").join("linux").join("hotkeys.json"));
        let bindings = hotkeys["hotkeys"].as_array().unwrap();
        assert_eq!(bindings.len(), 1, "malformed binding must be dropped");
        let surviving = &bindings[0];
        assert_eq!(
            surviving["plugin_uid"], ALT_TAB_UID,
            "valid binding maps plugin_id to plugin_uid"
        );
        assert!(
            surviving.get("plugin_id").is_none(),
            "plugin_id key must be removed from the surviving binding"
        );
        assert_eq!(surviving["id"], "hk-1");

        assert!(
            !migration().applies(dir.path()).unwrap(),
            "second applies() must be false after migration"
        );
    }

    #[test]
    fn malformed_hotkey_binding_with_array_plugin_id_is_dropped() {
        let dir = tempfile::tempdir().unwrap();
        let profile = setup_profile(dir.path(), "default");
        write_json(
            &profile.join("os").join("linux").join("hotkeys.json"),
            &json!({"hotkeys":[
                {"id":"hk-1","key":"K","plugin_id":["not","a","string"],"action":"open","enabled":true}
            ]}),
        );

        run(dir.path());

        let hotkeys = read_back(&profile.join("os").join("linux").join("hotkeys.json"));
        let bindings = hotkeys["hotkeys"].as_array().unwrap();
        assert_eq!(
            bindings.len(),
            0,
            "binding with array plugin_id must be dropped"
        );
    }

    #[test]
    fn before_fixture_migrates_into_the_after_fixture_shape() {
        let dir = tempfile::tempdir().unwrap();
        let work = dir.path().join("config");
        copy_tree(&fixture_root().join("before"), &work);

        assert!(
            migration().applies(&work).unwrap(),
            "the before fixture is id-keyed and must report work to do"
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
            let expected_value: serde_json::Value =
                serde_json::from_str(&std::fs::read_to_string(expected_abs).unwrap()).unwrap();
            assert_eq!(
                produced_value,
                expected_value,
                "content mismatch for {}",
                rel.display()
            );
        }
    }
}
