use super::{PluginLockEntry, PluginsLock, ProfileImportBundle, CURRENT_PROFILE_VERSION};
use crate::plugins::PluginUid;
use anyhow::Result;
use std::collections::HashMap;
use std::path::Path;

pub fn import_plugins(bundle: &ProfileImportBundle) -> Vec<PluginLockEntry> {
    if !bundle.plugins.is_empty() {
        return bundle.plugins.clone();
    }

    bundle
        .installed_plugins
        .iter()
        .map(|plugin_id| PluginLockEntry {
            uid: PluginUid::new(plugin_id.as_str()),
            id: plugin_id.clone(),
            repo_url: default_repo_url(plugin_id),
            version: String::new(),
            platforms: None,
        })
        .collect()
}

pub fn sync_plugins_lock_from_plugins<'a>(
    plugins: impl IntoIterator<Item = &'a crate::plugins::Plugin>,
) -> Result<PluginsLock> {
    sync_plugins_lock_from_plugins_with_generation(plugins).map(|(lock, _)| lock)
}

pub(crate) fn sync_plugins_lock_from_plugins_with_generation<'a>(
    plugins: impl IntoIterator<Item = &'a crate::plugins::Plugin>,
) -> Result<(PluginsLock, u64)> {
    let plugins = plugins.into_iter().collect::<Vec<_>>();
    super::storage::update_plugins_lock(|existing| {
        let cached_urls = cached_repo_urls();
        Ok(build_plugins_lock(
            plugins.iter().copied(),
            existing,
            &cached_urls,
        ))
    })
}

pub(super) fn read_plugin_version(plugin_dir: &Path) -> std::result::Result<String, ()> {
    let manifest = crate::plugins::PluginManifest::read_from_dir(plugin_dir).map_err(|_| ())?;
    Ok(manifest.plugin.version)
}

pub(super) fn sync_plugins_lock_from_imported_state(
    plugins_dir: &Path,
    requested_plugins: &[PluginLockEntry],
) -> Result<()> {
    let requested = PluginsLock {
        version: CURRENT_PROFILE_VERSION,
        plugins: requested_plugins.to_vec(),
    };
    super::storage::update_plugins_lock(|previous_lock| {
        let cached_urls = cached_repo_urls();
        let installed_plugins = load_installed_plugins_from_dir(plugins_dir);
        let repo_urls = merged_repo_url_lock(previous_lock, &requested);
        let mut lock = build_plugins_lock_with_options(
            installed_plugins.iter(),
            &repo_urls,
            &cached_urls,
            false,
        );
        preserve_import_unsupported_entries(&mut lock, previous_lock, &requested);
        Ok(lock)
    })
    .map(|_| ())
}

fn cached_repo_urls() -> HashMap<String, String> {
    let Some(cache) = crate::features::plugin_store::github::read_cache() else {
        return HashMap::new();
    };
    cache
        .plugins
        .into_iter()
        .map(|plugin| (plugin.id, plugin.repo_url))
        .collect()
}

fn resolve_repo_url(
    uid: &PluginUid,
    plugin_id: &str,
    existing_urls: &HashMap<String, String>,
    cached_urls: &HashMap<String, String>,
) -> String {
    if let Some(repo_url) = existing_urls.get(uid.as_str()) {
        return repo_url.clone();
    }
    if let Some(repo_url) = cached_urls.get(plugin_id) {
        return repo_url.clone();
    }
    default_repo_url(plugin_id)
}

fn default_repo_url(plugin_id: &str) -> String {
    format!("https://github.com/qol-tools/{}.git", plugin_id)
}

pub(super) fn build_plugins_lock<'a>(
    plugins: impl IntoIterator<Item = &'a crate::plugins::Plugin>,
    existing: &PluginsLock,
    cached_urls: &HashMap<String, String>,
) -> PluginsLock {
    build_plugins_lock_with_options(plugins, existing, cached_urls, true)
}

fn build_plugins_lock_with_options<'a>(
    plugins: impl IntoIterator<Item = &'a crate::plugins::Plugin>,
    existing: &PluginsLock,
    cached_urls: &HashMap<String, String>,
    preserve_unsupported: bool,
) -> PluginsLock {
    let existing_urls = existing_repo_urls(existing);
    let mut next = plugins
        .into_iter()
        .map(|plugin| {
            let uid = plugin.uid();
            let repo_url = resolve_repo_url(&uid, plugin.id.as_str(), &existing_urls, cached_urls);
            PluginLockEntry {
                uid,
                id: plugin.id.to_string(),
                repo_url,
                version: plugin.manifest.plugin.version.clone(),
                platforms: plugin.manifest.plugin.platforms.clone(),
            }
        })
        .collect::<Vec<_>>();
    if preserve_unsupported {
        next.extend(existing_urls.keys().filter_map(|uid_str| {
            preserved_unsupported_entry(&PluginUid::new(uid_str.as_str()), existing)
        }));
    }
    sort_and_dedup_plugins(&mut next);

    PluginsLock {
        version: CURRENT_PROFILE_VERSION,
        plugins: next,
    }
}

fn sort_and_dedup_plugins(plugins: &mut Vec<PluginLockEntry>) {
    plugins.sort_by(|left, right| left.uid.as_str().cmp(right.uid.as_str()));
    plugins.dedup_by(|left, right| left.uid == right.uid);
}

fn existing_repo_urls(existing: &PluginsLock) -> HashMap<String, String> {
    existing
        .plugins
        .iter()
        .map(|entry| (entry.uid.as_str().to_owned(), entry.repo_url.clone()))
        .collect()
}

fn preserved_unsupported_entry(uid: &PluginUid, existing: &PluginsLock) -> Option<PluginLockEntry> {
    let entry = existing.plugins.iter().find(|entry| &entry.uid == uid)?;
    if crate::plugins::manifest::supports_current_platform(&entry.platforms) {
        return None;
    }
    Some(entry.clone())
}

fn merged_repo_url_lock(previous_lock: &PluginsLock, requested_lock: &PluginsLock) -> PluginsLock {
    let mut plugins = previous_lock.plugins.clone();
    for entry in &requested_lock.plugins {
        let Some(existing) = plugins
            .iter_mut()
            .find(|existing| existing.uid == entry.uid)
        else {
            plugins.push(entry.clone());
            continue;
        };
        *existing = entry.clone();
    }
    PluginsLock {
        version: CURRENT_PROFILE_VERSION,
        plugins,
    }
}

fn preserve_import_unsupported_entries(
    lock: &mut PluginsLock,
    previous_lock: &PluginsLock,
    requested_lock: &PluginsLock,
) {
    let source = unsupported_entries_source(previous_lock, requested_lock);
    lock.plugins.extend(
        source
            .plugins
            .iter()
            .filter_map(|entry| preserved_unsupported_entry(&entry.uid, source)),
    );
    sort_and_dedup_plugins(&mut lock.plugins);
}

fn unsupported_entries_source<'a>(
    previous_lock: &'a PluginsLock,
    requested_lock: &'a PluginsLock,
) -> &'a PluginsLock {
    if !requested_lock.plugins.is_empty() {
        return requested_lock;
    }
    previous_lock
}

fn load_installed_plugins_from_dir(plugins_dir: &Path) -> Vec<crate::plugins::Plugin> {
    let Ok(entries) = std::fs::read_dir(plugins_dir) else {
        return Vec::new();
    };
    entries
        .filter_map(|entry| entry.ok())
        .filter_map(load_installed_plugin_from_entry)
        .collect()
}

fn load_installed_plugin_from_entry(entry: std::fs::DirEntry) -> Option<crate::plugins::Plugin> {
    let path = entry.path();
    let metadata = std::fs::symlink_metadata(&path).ok()?;
    if metadata.file_type().is_symlink() || !metadata.is_dir() {
        return None;
    }
    let plugin_id = entry.file_name().into_string().ok()?;
    if !crate::paths::is_safe_path_component(&plugin_id) {
        return None;
    }
    crate::plugins::PluginLoader::load_plugin_with_id(&plugin_id, &path).ok()
}
