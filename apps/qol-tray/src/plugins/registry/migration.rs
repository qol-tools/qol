use crate::plugins::registry::{
    load_registry, registry_path, save_registry, Entry, Registry, Slot, SlotSource,
};
use std::collections::HashMap;
use std::path::{Path, PathBuf};

const LEGACY_DEV_LINKS_RELPATH: &str = "dev/links.json";
const LEGACY_DEV_LINKS_CORRUPT_PREFIX: &str = "dev/links.json.corrupt.";

pub fn ensure_registry_initialized(
    config_dir: &Path,
    plugins_dir: &Path,
) -> Result<Registry, String> {
    if registry_path(config_dir).exists() {
        clean_up_legacy_dev_links(config_dir);
        return load_registry(config_dir);
    }
    let registry = build_from_legacy_state(config_dir, plugins_dir);
    save_registry(config_dir, &registry)?;
    clean_up_legacy_dev_links(config_dir);
    Ok(registry)
}

fn clean_up_legacy_dev_links(config_dir: &Path) {
    let path = config_dir.join(LEGACY_DEV_LINKS_RELPATH);
    if !path.exists() {
        return;
    }
    match std::fs::remove_file(&path) {
        Ok(()) => log::info!("Removed legacy {}", path.display()),
        Err(e) => log::warn!("Failed to remove legacy {}: {}", path.display(), e),
    }
}

fn build_from_legacy_state(config_dir: &Path, plugins_dir: &Path) -> Registry {
    let mut by_id: HashMap<String, Entry> = HashMap::new();

    for (id, path) in scan_installed_plugins(plugins_dir) {
        if by_id.contains_key(&id) {
            log::error!(
                "Duplicate plugin id {:?} at {}; skipping the duplicate rather than merging two plugins' state",
                id,
                path.display()
            );
            continue;
        }
        by_id.insert(
            id.clone(),
            Entry {
                id,
                active: Slot {
                    path,
                    source: SlotSource::ReleaseAsset,
                },
                fallback: None,
            },
        );
    }

    match read_legacy_dev_links(config_dir) {
        LegacyDevLinks::Parsed(links) => {
            for (id, path) in links {
                if !is_valid_dev_link_target(&path) {
                    log::warn!(
                        "Skipping dev-link during migration: id={} path={} (invalid or missing target)",
                        id,
                        path.display()
                    );
                    continue;
                }
                merge_dev_link_into(&mut by_id, id, path);
            }
        }
        LegacyDevLinks::Corrupt(reason) => {
            log::error!(
                "Legacy dev-links file is corrupt; dev-links not migrated: {}",
                reason
            );
        }
        LegacyDevLinks::Absent => {}
    }

    let mut entries: Vec<Entry> = by_id.into_values().collect();
    entries.sort_by(|a, b| a.id.cmp(&b.id));
    Registry {
        version: 1,
        entries,
    }
}

enum LegacyDevLinks {
    Parsed(HashMap<String, PathBuf>),
    Corrupt(String),
    Absent,
}

fn read_legacy_dev_links(config_dir: &Path) -> LegacyDevLinks {
    let path = config_dir.join(LEGACY_DEV_LINKS_RELPATH);
    let content = match std::fs::read_to_string(&path) {
        Ok(c) => c,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return LegacyDevLinks::Absent,
        Err(e) => {
            let reason = format!("Failed to read {}: {}", path.display(), e);
            back_up_corrupt_dev_links(config_dir, &path);
            return LegacyDevLinks::Corrupt(reason);
        }
    };
    match serde_json::from_str::<HashMap<String, PathBuf>>(&content) {
        Ok(map) => LegacyDevLinks::Parsed(map),
        Err(e) => {
            back_up_corrupt_dev_links(config_dir, &path);
            LegacyDevLinks::Corrupt(format!("Failed to parse {}: {}", path.display(), e))
        }
    }
}

fn is_valid_dev_link_target(path: &Path) -> bool {
    if !path.is_dir() {
        return false;
    }
    let manifest = path.join("plugin.toml");
    manifest.exists() && manifest_parses_and_version_valid(&manifest)
}

fn back_up_corrupt_dev_links(config_dir: &Path, original: &Path) {
    let ts = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let corrupt_path = config_dir.join(format!("{}{}.bak", LEGACY_DEV_LINKS_CORRUPT_PREFIX, ts));
    match std::fs::rename(original, &corrupt_path) {
        Ok(()) => log::info!(
            "Backed up corrupt {} to {}",
            original.display(),
            corrupt_path.display()
        ),
        Err(e) => log::error!("Failed to back up corrupt {}: {}", original.display(), e),
    }
}

fn scan_installed_plugins(plugins_dir: &Path) -> Vec<(String, PathBuf)> {
    let Ok(entries) = std::fs::read_dir(plugins_dir) else {
        return Vec::new();
    };
    let mut result = Vec::new();
    for entry in entries.filter_map(|e| e.ok()) {
        let path = entry.path();
        let name = entry.file_name().to_string_lossy().into_owned();
        if name.starts_with('.') || path.extension().is_some_and(|e| e == "backup") {
            continue;
        }
        if !path.is_dir() {
            continue;
        }
        let manifest = path.join("plugin.toml");
        if !manifest.exists() {
            continue;
        }
        let Some(id) = declared_or_folder_id(&manifest, &name) else {
            log::warn!(
                "Skipping {} during migration: manifest parse, version, or id check failed",
                path.display()
            );
            continue;
        };
        result.push((id, path));
    }
    result
}

/// The plugin's declared id (the single source of identity), falling back to
/// the folder name only when the manifest predates the id field, so an
/// upgrading user with an id-less installed plugin keeps it instead of having
/// it silently dropped. Parses leniently because the id field is required by
/// the strict manifest schema but must still be readable from older manifests.
fn declared_or_folder_id(manifest_path: &Path, folder: &str) -> Option<String> {
    let content = std::fs::read_to_string(manifest_path).ok()?;
    let value: toml::Value = toml::from_str(&content).ok()?;
    let max_version = crate::plugins::manifest::CURRENT_MANIFEST_VERSION as i64;
    let version = value
        .get("manifest_version")
        .and_then(|v| v.as_integer())
        .unwrap_or(max_version);
    if !(1..=max_version).contains(&version) {
        return None;
    }
    match value
        .get("plugin")
        .and_then(|plugin| plugin.get("id"))
        .and_then(|id| id.as_str())
    {
        Some(id) if crate::plugins::manifest::is_valid_plugin_id(id) => Some(id.to_string()),
        Some(_) => None,
        None => Some(folder.to_string()),
    }
}

fn manifest_parses_and_version_valid(manifest_path: &Path) -> bool {
    let Ok(content) = std::fs::read_to_string(manifest_path) else {
        return false;
    };
    let Ok(manifest) = toml::from_str::<crate::plugins::manifest::PluginManifest>(&content) else {
        return false;
    };
    manifest.validate_version().is_ok()
}

fn merge_dev_link_into(by_id: &mut HashMap<String, Entry>, id: String, path: PathBuf) {
    let dev_slot = Slot {
        path: path.clone(),
        source: SlotSource::DevLink { origin_path: path },
    };
    if let Some(existing) = by_id.remove(&id) {
        by_id.insert(
            id.clone(),
            Entry {
                id,
                active: dev_slot,
                fallback: Some(existing.active),
            },
        );
    } else {
        by_id.insert(
            id.clone(),
            Entry {
                id,
                active: dev_slot,
                fallback: None,
            },
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::TempDir;

    fn valid_manifest(id: &str) -> String {
        format!(
            "[plugin]\nid = \"{id}\"\nname = \"{id}\"\ndescription = \"\"\nversion = \"1.0.0\"\n\n[menu]\nlabel = \"Test\"\nitems = []\n",
        )
    }

    fn make_plugin_dir(plugins_dir: &Path, id: &str) -> PathBuf {
        let dir = plugins_dir.join(id);
        fs::create_dir_all(&dir).unwrap();
        fs::write(dir.join("plugin.toml"), valid_manifest(id)).unwrap();
        dir
    }

    fn write_legacy_dev_links(config_dir: &Path, links: &HashMap<String, PathBuf>) {
        let dev_dir = config_dir.join("dev");
        fs::create_dir_all(&dev_dir).unwrap();
        let json = serde_json::to_string_pretty(links).unwrap();
        fs::write(config_dir.join(LEGACY_DEV_LINKS_RELPATH), json).unwrap();
    }

    #[test]
    fn builds_registry_from_installed_plugins_only() {
        let tmp = TempDir::new().unwrap();
        let plugins_dir = tmp.path().join("plugins");
        fs::create_dir(&plugins_dir).unwrap();
        make_plugin_dir(&plugins_dir, "plugin-a");
        make_plugin_dir(&plugins_dir, "plugin-b");

        let registry = ensure_registry_initialized(tmp.path(), &plugins_dir).unwrap();
        assert_eq!(registry.version, 1);
        assert_eq!(registry.entries.len(), 2);
        assert!(registry
            .entries
            .iter()
            .all(|e| matches!(e.active.source, SlotSource::ReleaseAsset)));
        assert!(registry.entries.iter().all(|e| e.fallback.is_none()));
    }

    #[test]
    fn pairs_dev_link_with_installed_as_fallback() {
        let tmp = TempDir::new().unwrap();
        let plugins_dir = tmp.path().join("plugins");
        fs::create_dir(&plugins_dir).unwrap();
        let installed = make_plugin_dir(&plugins_dir, "plugin-foo");
        let dev_src = tmp.path().join("dev-src");
        make_plugin_dir(tmp.path(), "dev-src");

        let mut links = HashMap::new();
        links.insert("plugin-foo".to_string(), dev_src.clone());
        write_legacy_dev_links(tmp.path(), &links);

        let registry = ensure_registry_initialized(tmp.path(), &plugins_dir).unwrap();
        let entry = registry
            .entries
            .iter()
            .find(|e| e.id == "plugin-foo")
            .unwrap();

        assert!(matches!(entry.active.source, SlotSource::DevLink { .. }));
        assert_eq!(entry.active.path, dev_src);
        let fallback = entry.fallback.as_ref().unwrap();
        assert!(matches!(fallback.source, SlotSource::ReleaseAsset));
        assert_eq!(fallback.path, installed);
    }

    #[test]
    fn dev_link_without_install_has_no_fallback() {
        let tmp = TempDir::new().unwrap();
        let plugins_dir = tmp.path().join("plugins");
        fs::create_dir(&plugins_dir).unwrap();
        let dev_src = make_plugin_dir(tmp.path(), "dev-only");

        let mut links = HashMap::new();
        links.insert("dev-only".to_string(), dev_src);
        write_legacy_dev_links(tmp.path(), &links);

        let registry = ensure_registry_initialized(tmp.path(), &plugins_dir).unwrap();
        let entry = registry
            .entries
            .iter()
            .find(|e| e.id == "dev-only")
            .unwrap();

        assert!(matches!(entry.active.source, SlotSource::DevLink { .. }));
        assert!(entry.fallback.is_none());
    }

    #[test]
    fn legacy_file_removed_after_first_migration() {
        let tmp = TempDir::new().unwrap();
        let plugins_dir = tmp.path().join("plugins");
        fs::create_dir(&plugins_dir).unwrap();
        write_legacy_dev_links(tmp.path(), &HashMap::new());

        let _ = ensure_registry_initialized(tmp.path(), &plugins_dir).unwrap();
        assert!(!tmp.path().join(LEGACY_DEV_LINKS_RELPATH).exists());
    }

    #[test]
    fn does_not_rebuild_registry_once_it_exists_even_if_legacy_reappears() {
        let tmp = TempDir::new().unwrap();
        let plugins_dir = tmp.path().join("plugins");
        fs::create_dir(&plugins_dir).unwrap();

        let first = ensure_registry_initialized(tmp.path(), &plugins_dir).unwrap();
        assert_eq!(first.entries.len(), 0);

        let dev_src = make_plugin_dir(tmp.path(), "new-dev-link");
        let mut links = HashMap::new();
        links.insert("new-dev-link".to_string(), dev_src);
        write_legacy_dev_links(tmp.path(), &links);

        let second = ensure_registry_initialized(tmp.path(), &plugins_dir).unwrap();
        assert_eq!(second.entries.len(), 0);
        assert!(!tmp.path().join(LEGACY_DEV_LINKS_RELPATH).exists());
    }

    #[test]
    fn skips_dev_link_with_missing_target() {
        let tmp = TempDir::new().unwrap();
        let plugins_dir = tmp.path().join("plugins");
        fs::create_dir(&plugins_dir).unwrap();

        let mut links = HashMap::new();
        links.insert("stale".to_string(), tmp.path().join("nonexistent-dev-src"));
        write_legacy_dev_links(tmp.path(), &links);

        let registry = ensure_registry_initialized(tmp.path(), &plugins_dir).unwrap();
        assert_eq!(registry.entries.len(), 0);
    }

    #[test]
    fn skips_dev_link_with_invalid_manifest() {
        let tmp = TempDir::new().unwrap();
        let plugins_dir = tmp.path().join("plugins");
        fs::create_dir(&plugins_dir).unwrap();

        let dev_src = tmp.path().join("bad-dev-src");
        fs::create_dir(&dev_src).unwrap();
        fs::write(dev_src.join("plugin.toml"), "not valid toml {{{").unwrap();

        let mut links = HashMap::new();
        links.insert("bad".to_string(), dev_src);
        write_legacy_dev_links(tmp.path(), &links);

        let registry = ensure_registry_initialized(tmp.path(), &plugins_dir).unwrap();
        assert_eq!(registry.entries.len(), 0);
    }

    #[test]
    fn stale_dev_link_leaves_installed_fallback_as_sole_release_entry() {
        let tmp = TempDir::new().unwrap();
        let plugins_dir = tmp.path().join("plugins");
        fs::create_dir(&plugins_dir).unwrap();
        let installed = make_plugin_dir(&plugins_dir, "plugin-foo");

        let mut links = HashMap::new();
        links.insert(
            "plugin-foo".to_string(),
            tmp.path().join("nonexistent-dev-src"),
        );
        write_legacy_dev_links(tmp.path(), &links);

        let registry = ensure_registry_initialized(tmp.path(), &plugins_dir).unwrap();
        assert_eq!(registry.entries.len(), 1);
        let entry = &registry.entries[0];
        assert!(matches!(entry.active.source, SlotSource::ReleaseAsset));
        assert_eq!(entry.active.path, installed);
        assert!(entry.fallback.is_none());
    }

    #[test]
    fn corrupt_dev_links_are_backed_up_and_migration_continues() {
        let tmp = TempDir::new().unwrap();
        let plugins_dir = tmp.path().join("plugins");
        fs::create_dir(&plugins_dir).unwrap();
        make_plugin_dir(&plugins_dir, "installed");

        let dev_dir = tmp.path().join("dev");
        fs::create_dir_all(&dev_dir).unwrap();
        fs::write(tmp.path().join(LEGACY_DEV_LINKS_RELPATH), "{{{ not json").unwrap();

        let registry = ensure_registry_initialized(tmp.path(), &plugins_dir).unwrap();
        assert_eq!(registry.entries.len(), 1);
        assert_eq!(registry.entries[0].id, "installed");

        let corrupt_present = fs::read_dir(&dev_dir)
            .unwrap()
            .filter_map(|e| e.ok())
            .any(|e| {
                e.file_name()
                    .to_string_lossy()
                    .starts_with("links.json.corrupt.")
            });
        assert!(corrupt_present);
        assert!(!tmp.path().join(LEGACY_DEV_LINKS_RELPATH).exists());
    }

    #[test]
    fn skips_entries_with_invalid_manifest() {
        let tmp = TempDir::new().unwrap();
        let plugins_dir = tmp.path().join("plugins");
        fs::create_dir(&plugins_dir).unwrap();

        let bad = plugins_dir.join("broken-plugin");
        fs::create_dir(&bad).unwrap();
        fs::write(bad.join("plugin.toml"), "not valid toml {{{").unwrap();

        make_plugin_dir(&plugins_dir, "good-plugin");

        let registry = ensure_registry_initialized(tmp.path(), &plugins_dir).unwrap();
        assert_eq!(registry.entries.len(), 1);
        assert_eq!(registry.entries[0].id, "good-plugin");
    }

    #[test]
    fn skips_dirs_without_manifest_and_backup_dirs() {
        let tmp = TempDir::new().unwrap();
        let plugins_dir = tmp.path().join("plugins");
        fs::create_dir(&plugins_dir).unwrap();

        fs::create_dir(plugins_dir.join("no-manifest")).unwrap();
        fs::create_dir(plugins_dir.join("foo.backup")).unwrap();
        make_plugin_dir(&plugins_dir, "real-plugin");

        let registry = ensure_registry_initialized(tmp.path(), &plugins_dir).unwrap();
        assert_eq!(registry.entries.len(), 1);
        assert_eq!(registry.entries[0].id, "real-plugin");
    }

    #[test]
    fn idempotent_when_registry_already_exists() {
        let tmp = TempDir::new().unwrap();
        let plugins_dir = tmp.path().join("plugins");
        fs::create_dir(&plugins_dir).unwrap();
        make_plugin_dir(&plugins_dir, "plugin-a");

        let first = ensure_registry_initialized(tmp.path(), &plugins_dir).unwrap();
        make_plugin_dir(&plugins_dir, "plugin-b");
        let second = ensure_registry_initialized(tmp.path(), &plugins_dir).unwrap();

        assert_eq!(first, second);
        assert_eq!(second.entries.len(), 1);
        assert_eq!(second.entries[0].id, "plugin-a");
    }

    fn make_plugin_dir_with_manifest(plugins_dir: &Path, folder: &str, manifest: &str) -> PathBuf {
        let dir = plugins_dir.join(folder);
        fs::create_dir_all(&dir).unwrap();
        fs::write(dir.join("plugin.toml"), manifest).unwrap();
        dir
    }

    #[test]
    fn registry_id_comes_from_the_manifest_not_the_folder() {
        let tmp = TempDir::new().unwrap();
        let plugins_dir = tmp.path().join("plugins");
        fs::create_dir(&plugins_dir).unwrap();
        make_plugin_dir_with_manifest(
            &plugins_dir,
            "renamed-folder",
            &valid_manifest("declared-id"),
        );

        let registry = ensure_registry_initialized(tmp.path(), &plugins_dir).unwrap();
        assert_eq!(registry.entries.len(), 1);
        assert_eq!(
            registry.entries[0].id, "declared-id",
            "identity must follow the manifest so folders are free to rename"
        );
    }

    #[test]
    fn duplicate_declared_ids_keep_one_entry_and_reject_the_rest() {
        let tmp = TempDir::new().unwrap();
        let plugins_dir = tmp.path().join("plugins");
        fs::create_dir(&plugins_dir).unwrap();
        make_plugin_dir_with_manifest(&plugins_dir, "folder-a", &valid_manifest("dup"));
        make_plugin_dir_with_manifest(&plugins_dir, "folder-b", &valid_manifest("dup"));

        let registry = ensure_registry_initialized(tmp.path(), &plugins_dir).unwrap();
        assert_eq!(
            registry.entries.len(),
            1,
            "two plugins declaring the same id must not silently merge"
        );
        assert_eq!(registry.entries[0].id, "dup");
    }

    #[test]
    fn id_less_manifest_falls_back_to_folder_name() {
        let tmp = TempDir::new().unwrap();
        let plugins_dir = tmp.path().join("plugins");
        fs::create_dir(&plugins_dir).unwrap();
        let legacy = "[plugin]\nname = \"Legacy\"\ndescription = \"\"\nversion = \"1.0.0\"\n\n[menu]\nlabel = \"L\"\nitems = []\n";
        make_plugin_dir_with_manifest(&plugins_dir, "legacy-plugin", legacy);

        let registry = ensure_registry_initialized(tmp.path(), &plugins_dir).unwrap();
        assert_eq!(registry.entries.len(), 1);
        assert_eq!(
            registry.entries[0].id, "legacy-plugin",
            "an id-less installed plugin keeps its folder identity instead of being dropped"
        );
    }
}
