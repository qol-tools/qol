use crate::plugins::registry::{
    load_registry, registry_path, save_registry, Entry, Registry, Slot, SlotSource,
};
use std::path::{Path, PathBuf};

const LEGACY_DEV_LINKS_RELPATH: &str = "dev/links.json";

pub fn ensure_registry_initialized(
    config_dir: &Path,
    plugins_dir: &Path,
) -> Result<Registry, String> {
    clean_up_legacy_dev_links(config_dir);
    if registry_path(config_dir).exists() {
        return load_registry(config_dir);
    }
    let registry = build_from_installed_plugins(plugins_dir);
    save_registry(config_dir, &registry)?;
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

fn build_from_installed_plugins(plugins_dir: &Path) -> Registry {
    let mut entries: Vec<Entry> = scan_installed_plugins(plugins_dir)
        .into_iter()
        .map(|(id, path)| Entry {
            id,
            active: Slot {
                path,
                source: SlotSource::ReleaseAsset,
            },
            fallback: None,
        })
        .collect();
    entries.sort_by(|a, b| a.id.cmp(&b.id));
    Registry {
        version: 1,
        entries,
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
        if !manifest_parses_and_version_valid(&manifest) {
            log::warn!(
                "Skipping {} during initial registry build: manifest parse or version check failed",
                path.display()
            );
            continue;
        }
        result.push((name, path));
    }
    result
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

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::TempDir;

    fn valid_manifest(name: &str) -> String {
        format!(
            "[plugin]\nname = \"{}\"\ndescription = \"\"\nversion = \"1.0.0\"\n\n[menu]\nlabel = \"Test\"\nitems = []\n",
            name
        )
    }

    fn make_plugin_dir(plugins_dir: &Path, id: &str) -> PathBuf {
        let dir = plugins_dir.join(id);
        fs::create_dir_all(&dir).unwrap();
        fs::write(dir.join("plugin.toml"), valid_manifest(id)).unwrap();
        dir
    }

    #[test]
    fn builds_registry_from_installed_plugins() {
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
    fn legacy_dev_links_file_is_removed_on_boot() {
        let tmp = TempDir::new().unwrap();
        let plugins_dir = tmp.path().join("plugins");
        fs::create_dir(&plugins_dir).unwrap();
        fs::create_dir_all(tmp.path().join("dev")).unwrap();
        fs::write(tmp.path().join(LEGACY_DEV_LINKS_RELPATH), "{}").unwrap();

        let _ = ensure_registry_initialized(tmp.path(), &plugins_dir).unwrap();
        assert!(!tmp.path().join(LEGACY_DEV_LINKS_RELPATH).exists());
    }

    #[test]
    fn does_not_rescan_plugins_dir_after_registry_exists() {
        let tmp = TempDir::new().unwrap();
        let plugins_dir = tmp.path().join("plugins");
        fs::create_dir(&plugins_dir).unwrap();

        let initial = ensure_registry_initialized(tmp.path(), &plugins_dir).unwrap();
        assert_eq!(initial.entries.len(), 0);

        make_plugin_dir(&plugins_dir, "dropped-in");

        let second = ensure_registry_initialized(tmp.path(), &plugins_dir).unwrap();
        assert_eq!(second.entries.len(), 0);
    }
}
