use std::collections::HashMap;
use std::path::{Path, PathBuf};

#[cfg(feature = "dev")]
pub(super) fn migrate_symlinks_to_registry(plugins_dir: &Path) {
    let Some(config_dir) = plugins_dir.parent() else {
        return;
    };

    let dev_links_path = config_dir.join("dev/links.json");
    if dev_links_path.exists() {
        return;
    }

    let migrated = migrated_symlinks(plugins_dir);
    if migrated.is_empty() {
        restore_plugin_backups(plugins_dir);
        return;
    }

    write_dev_links(&dev_links_path, &migrated);
    restore_plugin_backups(plugins_dir);
}

#[cfg(not(feature = "dev"))]
pub(super) fn migrate_symlinks_to_registry(_plugins_dir: &Path) {}

#[cfg(feature = "dev")]
fn migrated_symlinks(plugins_dir: &Path) -> HashMap<String, PathBuf> {
    let Ok(entries) = std::fs::read_dir(plugins_dir) else {
        return HashMap::new();
    };

    let mut migrated = HashMap::new();
    for entry in entries.filter_map(|entry| entry.ok()) {
        migrate_symlink_entry(plugins_dir, entry, &mut migrated);
    }
    migrated
}

#[cfg(feature = "dev")]
fn migrate_symlink_entry(
    plugins_dir: &Path,
    entry: std::fs::DirEntry,
    migrated: &mut HashMap<String, PathBuf>,
) {
    let path = entry.path();
    let Ok(metadata) = std::fs::symlink_metadata(&path) else {
        return;
    };
    if !metadata.file_type().is_symlink() {
        return;
    }

    let Ok(target) = std::fs::read_link(&path) else {
        return;
    };

    let id = entry.file_name().to_string_lossy().into_owned();
    let abs_target = absolute_target(plugins_dir, target);
    log::info!("Migrating symlink to dev-link: {} -> {:?}", id, abs_target);
    migrated.insert(id, abs_target);
    let _ = std::fs::remove_file(&path);
}

#[cfg(feature = "dev")]
fn absolute_target(plugins_dir: &Path, target: PathBuf) -> PathBuf {
    if !target.is_relative() {
        return target;
    }
    plugins_dir.join(&target).canonicalize().unwrap_or(target)
}

#[cfg(feature = "dev")]
fn write_dev_links(dev_links_path: &Path, migrated: &HashMap<String, PathBuf>) {
    if let Some(parent) = dev_links_path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    let Ok(content) = serde_json::to_string_pretty(migrated) else {
        return;
    };
    let _ = std::fs::write(dev_links_path, content);
    log::info!("Migrated {} symlinks to dev/links.json", migrated.len());
}

#[cfg(feature = "dev")]
fn restore_plugin_backups(plugins_dir: &Path) {
    let Ok(entries) = std::fs::read_dir(plugins_dir) else {
        return;
    };

    for entry in entries.filter_map(|entry| entry.ok()) {
        restore_backup_entry(entry.path());
    }
}

#[cfg(feature = "dev")]
fn restore_backup_entry(path: PathBuf) {
    if path.extension().is_none_or(|ext| ext != "backup") {
        return;
    }

    let restored_name = path.with_extension("");
    if !restored_name.exists() {
        log::info!("Restoring backup: {:?}", path);
        let _ = std::fs::rename(&path, &restored_name);
        return;
    }

    log::info!("Removing orphan backup: {:?}", path);
    let _ = std::fs::remove_dir_all(&path);
}
