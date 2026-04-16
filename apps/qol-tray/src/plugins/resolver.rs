use crate::file_io;
use crate::plugins::registry::{Registry, Slot, SlotSource};
use crate::plugins::PluginId;
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

#[derive(Debug, Clone)]
pub struct ResolvedPlugin {
    pub id: PluginId,
    pub path: PathBuf,
    pub source: PluginSource,
    pub resolved_from: ResolutionOrigin,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PluginSource {
    Installed,
    DevLinked,
}

impl PluginSource {
    pub fn is_live_source(&self) -> bool {
        match self {
            Self::DevLinked => true,
            Self::Installed => false,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ResolutionOrigin {
    Active,
    Fallback,
}

#[derive(Debug, Clone)]
pub struct SlotFailure {
    pub path: PathBuf,
    pub reason: String,
}

#[derive(Debug, Clone)]
pub struct PluginUnavailable {
    pub id: String,
    pub active: SlotFailure,
    pub fallback: Option<SlotFailure>,
}

#[derive(Debug, Default)]
pub struct ResolutionReport {
    pub plugins: Vec<ResolvedPlugin>,
    pub unavailable: Vec<PluginUnavailable>,
}

pub fn resolve_all(
    plugins_dir: &Path,
    dev_links: &HashMap<String, PathBuf>,
) -> Vec<ResolvedPlugin> {
    let dev_link_targets: HashSet<PathBuf> = dev_links
        .values()
        .map(|p| file_io::canonical_or_original(p))
        .collect();
    let mut resolved = scan_installed(plugins_dir, &dev_link_targets);
    apply_dev_links(&mut resolved, dev_links);
    let mut result: Vec<_> = resolved.into_values().collect();
    result.sort_by(|a, b| a.id.as_str().cmp(b.id.as_str()));
    result
}

fn scan_installed(
    plugins_dir: &Path,
    dev_link_targets: &HashSet<PathBuf>,
) -> HashMap<PluginId, ResolvedPlugin> {
    let mut resolved = HashMap::new();
    let Ok(entries) = std::fs::read_dir(plugins_dir) else {
        return resolved;
    };
    for entry in entries.filter_map(|e| e.ok()) {
        let path = entry.path();
        let raw_id = entry.file_name().to_string_lossy().into_owned();
        if should_skip(&raw_id, &path, dev_link_targets) {
            continue;
        }
        let id = PluginId::new(raw_id);
        resolved.insert(
            id.clone(),
            ResolvedPlugin {
                id,
                path,
                source: PluginSource::Installed,
                resolved_from: ResolutionOrigin::Active,
            },
        );
    }
    resolved
}

fn should_skip(id: &str, path: &Path, dev_link_targets: &HashSet<PathBuf>) -> bool {
    if dev_link_targets.contains(&file_io::canonical_or_original(path)) {
        return true;
    }
    if id.starts_with('.') {
        return true;
    }
    if path.extension().is_some_and(|ext| ext == "backup") {
        return true;
    }
    let Ok(metadata) = std::fs::symlink_metadata(path) else {
        return true;
    };
    if metadata.file_type().is_symlink() {
        log::warn!("Skipping symlink in plugins dir: {}", id);
        return true;
    }
    !metadata.is_dir()
}

fn apply_dev_links(
    resolved: &mut HashMap<PluginId, ResolvedPlugin>,
    dev_links: &HashMap<String, PathBuf>,
) {
    for (raw_id, path) in dev_links {
        let id = PluginId::new(raw_id);
        if resolved.contains_key(&id) {
            log::info!("Dev-link overrides installed plugin: {}", raw_id);
        }
        resolved.insert(
            id.clone(),
            ResolvedPlugin {
                id,
                path: path.clone(),
                source: PluginSource::DevLinked,
                resolved_from: ResolutionOrigin::Active,
            },
        );
    }
}

pub fn resolve_from_registry(registry: &Registry) -> ResolutionReport {
    let mut report = ResolutionReport::default();
    for entry in &registry.entries {
        match validate_slot(&entry.active) {
            Ok(()) => report.plugins.push(resolved_from_slot(
                &entry.id,
                &entry.active,
                ResolutionOrigin::Active,
            )),
            Err(active_reason) => {
                let active_fail = SlotFailure {
                    path: entry.active.path.clone(),
                    reason: active_reason.clone(),
                };
                match entry.fallback.as_ref().map(|s| (s, validate_slot(s))) {
                    Some((slot, Ok(()))) => {
                        log::warn!(
                            "Plugin {} active slot invalid ({}); falling back to {}",
                            entry.id,
                            active_reason,
                            slot.path.display()
                        );
                        report.plugins.push(resolved_from_slot(
                            &entry.id,
                            slot,
                            ResolutionOrigin::Fallback,
                        ));
                    }
                    Some((slot, Err(fallback_reason))) => {
                        report.unavailable.push(PluginUnavailable {
                            id: entry.id.clone(),
                            active: active_fail,
                            fallback: Some(SlotFailure {
                                path: slot.path.clone(),
                                reason: fallback_reason,
                            }),
                        });
                    }
                    None => {
                        report.unavailable.push(PluginUnavailable {
                            id: entry.id.clone(),
                            active: active_fail,
                            fallback: None,
                        });
                    }
                }
            }
        }
    }
    report
        .plugins
        .sort_by(|a, b| a.id.as_str().cmp(b.id.as_str()));
    report.unavailable.sort_by(|a, b| a.id.cmp(&b.id));
    report
}

fn resolved_from_slot(id: &str, slot: &Slot, origin: ResolutionOrigin) -> ResolvedPlugin {
    ResolvedPlugin {
        id: PluginId::new(id.to_string()),
        path: slot.path.clone(),
        source: slot_source_to_plugin_source(&slot.source),
        resolved_from: origin,
    }
}

fn slot_source_to_plugin_source(source: &SlotSource) -> PluginSource {
    match source {
        SlotSource::ReleaseAsset => PluginSource::Installed,
        SlotSource::DevLink { .. } | SlotSource::WorktreeLink { .. } => PluginSource::DevLinked,
    }
}

fn validate_slot(slot: &Slot) -> Result<(), String> {
    if !slot.path.is_dir() {
        return Err(format!("not a directory: {}", slot.path.display()));
    }
    let manifest_path = slot.path.join("plugin.toml");
    let content = std::fs::read_to_string(&manifest_path)
        .map_err(|e| format!("failed to read {}: {}", manifest_path.display(), e))?;
    let manifest: crate::plugins::manifest::PluginManifest = toml::from_str(&content)
        .map_err(|e| format!("failed to parse {}: {}", manifest_path.display(), e))?;
    manifest.validate_version().map_err(|e| {
        format!(
            "manifest_version check failed for {}: {}",
            manifest_path.display(),
            e
        )
    })?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::plugins::registry::{Entry, Registry, Slot, SlotSource};
    use std::fs;
    use tempfile::TempDir;

    #[test]
    fn installed_plugin_resolved_from_dir() {
        let tmp = TempDir::new().unwrap();
        let plugin_dir = tmp.path().join("foo");
        fs::create_dir(&plugin_dir).unwrap();

        let result = resolve_all(tmp.path(), &HashMap::new());

        assert_eq!(result.len(), 1);
        assert_eq!(result[0].id.as_str(), "foo");
        assert_eq!(result[0].source, PluginSource::Installed);
    }

    #[test]
    fn dev_link_added_when_not_installed() {
        let tmp = TempDir::new().unwrap();
        let dev_path = tmp.path().join("dev-src");
        fs::create_dir(&dev_path).unwrap();

        let dev_links = HashMap::from([("foo".to_string(), dev_path.clone())]);
        let result = resolve_all(tmp.path(), &dev_links);

        assert_eq!(result.len(), 1);
        assert_eq!(result[0].id.as_str(), "foo");
        assert_eq!(result[0].path, dev_path);
        assert_eq!(result[0].source, PluginSource::DevLinked);
    }

    #[test]
    fn dev_link_overrides_installed() {
        let tmp = TempDir::new().unwrap();
        let installed = tmp.path().join("foo");
        fs::create_dir(&installed).unwrap();
        let dev_path = tmp.path().join("dev-foo");
        fs::create_dir(&dev_path).unwrap();

        let dev_links = HashMap::from([("foo".to_string(), dev_path.clone())]);
        let result = resolve_all(tmp.path(), &dev_links);

        assert_eq!(result.len(), 1);
        assert_eq!(result[0].path, dev_path);
        assert_eq!(result[0].source, PluginSource::DevLinked);
    }

    #[test]
    fn skips_backup_dirs() {
        let tmp = TempDir::new().unwrap();
        let backup = tmp.path().join("foo.backup");
        fs::create_dir(&backup).unwrap();

        let result = resolve_all(tmp.path(), &HashMap::new());
        assert!(result.is_empty());
    }

    #[test]
    fn skips_hidden_dirs() {
        let tmp = TempDir::new().unwrap();
        fs::create_dir(tmp.path().join(".hidden")).unwrap();

        let result = resolve_all(tmp.path(), &HashMap::new());
        assert!(result.is_empty());
    }

    #[cfg(unix)]
    #[test]
    fn skips_symlinks() {
        let tmp = TempDir::new().unwrap();
        let target = tmp.path().join("real");
        fs::create_dir(&target).unwrap();
        std::os::unix::fs::symlink(&target, tmp.path().join("link")).unwrap();

        let result = resolve_all(tmp.path(), &HashMap::new());
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].id.as_str(), "real");
    }

    #[test]
    fn skips_files() {
        let tmp = TempDir::new().unwrap();
        fs::write(tmp.path().join("not-a-dir"), "").unwrap();

        let result = resolve_all(tmp.path(), &HashMap::new());
        assert!(result.is_empty());
    }

    #[test]
    fn results_sorted_by_id() {
        let tmp = TempDir::new().unwrap();
        fs::create_dir(tmp.path().join("z-plugin")).unwrap();
        fs::create_dir(tmp.path().join("a-plugin")).unwrap();

        let result = resolve_all(tmp.path(), &HashMap::new());
        assert_eq!(result[0].id.as_str(), "a-plugin");
        assert_eq!(result[1].id.as_str(), "z-plugin");
    }

    #[test]
    fn mixed_installed_and_dev_linked() {
        let tmp = TempDir::new().unwrap();
        fs::create_dir(tmp.path().join("installed-only")).unwrap();
        let dev_path = tmp.path().join("dev-src");
        fs::create_dir(&dev_path).unwrap();

        let dev_links = HashMap::from([("dev-only".to_string(), dev_path)]);
        let result = resolve_all(tmp.path(), &dev_links);

        assert_eq!(result.len(), 2);
        let ids: Vec<&str> = result.iter().map(|r| r.id.as_str()).collect();
        assert!(ids.contains(&"installed-only"));
        assert!(ids.contains(&"dev-only"));
    }

    fn write_valid_manifest(plugin_dir: &Path, name: &str) {
        fs::write(
            plugin_dir.join("plugin.toml"),
            format!(
                "[plugin]\nname = \"{}\"\ndescription = \"\"\nversion = \"1.0.0\"\n\n[menu]\nlabel = \"Test\"\nitems = []\n",
                name
            ),
        )
        .unwrap();
    }

    fn installed_entry(id: &str, path: &Path) -> Entry {
        Entry {
            id: id.to_string(),
            active: Slot {
                path: path.to_path_buf(),
                source: SlotSource::ReleaseAsset,
            },
            fallback: None,
        }
    }

    #[test]
    fn resolve_from_registry_valid_active() {
        let tmp = TempDir::new().unwrap();
        let plugin_dir = tmp.path().join("plugin-foo");
        fs::create_dir(&plugin_dir).unwrap();
        write_valid_manifest(&plugin_dir, "plugin-foo");

        let registry = Registry {
            version: 1,
            entries: vec![installed_entry("plugin-foo", &plugin_dir)],
        };
        let report = resolve_from_registry(&registry);

        assert_eq!(report.plugins.len(), 1);
        assert_eq!(report.unavailable.len(), 0);
        assert_eq!(report.plugins[0].id.as_str(), "plugin-foo");
        assert_eq!(report.plugins[0].resolved_from, ResolutionOrigin::Active);
        assert_eq!(report.plugins[0].source, PluginSource::Installed);
    }

    #[test]
    fn resolve_from_registry_falls_back_when_active_missing() {
        let tmp = TempDir::new().unwrap();
        let fallback_dir = tmp.path().join("plugin-foo");
        fs::create_dir(&fallback_dir).unwrap();
        write_valid_manifest(&fallback_dir, "plugin-foo");

        let entry = Entry {
            id: "plugin-foo".to_string(),
            active: Slot {
                path: tmp.path().join("missing-dev-src"),
                source: SlotSource::DevLink {
                    origin_path: tmp.path().join("missing-dev-src"),
                },
            },
            fallback: Some(Slot {
                path: fallback_dir,
                source: SlotSource::ReleaseAsset,
            }),
        };
        let registry = Registry {
            version: 1,
            entries: vec![entry],
        };
        let report = resolve_from_registry(&registry);

        assert_eq!(report.plugins.len(), 1);
        assert_eq!(report.unavailable.len(), 0);
        assert_eq!(report.plugins[0].resolved_from, ResolutionOrigin::Fallback);
        assert_eq!(report.plugins[0].source, PluginSource::Installed);
    }

    #[test]
    fn resolve_from_registry_unavailable_when_both_missing() {
        let tmp = TempDir::new().unwrap();

        let entry = Entry {
            id: "plugin-foo".to_string(),
            active: Slot {
                path: tmp.path().join("active-missing"),
                source: SlotSource::DevLink {
                    origin_path: tmp.path().join("active-missing"),
                },
            },
            fallback: Some(Slot {
                path: tmp.path().join("fallback-missing"),
                source: SlotSource::ReleaseAsset,
            }),
        };
        let registry = Registry {
            version: 1,
            entries: vec![entry],
        };
        let report = resolve_from_registry(&registry);

        assert_eq!(report.plugins.len(), 0);
        assert_eq!(report.unavailable.len(), 1);
        let u = &report.unavailable[0];
        assert_eq!(u.id, "plugin-foo");
        assert!(u.active.reason.contains("not a directory"));
        assert!(u.fallback.is_some());
    }

    #[test]
    fn resolve_from_registry_unavailable_when_no_fallback() {
        let tmp = TempDir::new().unwrap();

        let entry = Entry {
            id: "plugin-foo".to_string(),
            active: Slot {
                path: tmp.path().join("gone"),
                source: SlotSource::ReleaseAsset,
            },
            fallback: None,
        };
        let registry = Registry {
            version: 1,
            entries: vec![entry],
        };
        let report = resolve_from_registry(&registry);

        assert_eq!(report.plugins.len(), 0);
        assert_eq!(report.unavailable.len(), 1);
        assert!(report.unavailable[0].fallback.is_none());
    }

    #[test]
    fn resolve_from_registry_rejects_invalid_manifest() {
        let tmp = TempDir::new().unwrap();
        let plugin_dir = tmp.path().join("plugin-foo");
        fs::create_dir(&plugin_dir).unwrap();
        fs::write(plugin_dir.join("plugin.toml"), "not valid toml {{{").unwrap();

        let registry = Registry {
            version: 1,
            entries: vec![installed_entry("plugin-foo", &plugin_dir)],
        };
        let report = resolve_from_registry(&registry);

        assert_eq!(report.plugins.len(), 0);
        assert_eq!(report.unavailable.len(), 1);
        assert!(report.unavailable[0].active.reason.contains("parse"));
    }

    #[test]
    fn resolve_from_registry_sorts_results() {
        let tmp = TempDir::new().unwrap();
        let z_dir = tmp.path().join("z-plugin");
        fs::create_dir(&z_dir).unwrap();
        write_valid_manifest(&z_dir, "z-plugin");
        let a_dir = tmp.path().join("a-plugin");
        fs::create_dir(&a_dir).unwrap();
        write_valid_manifest(&a_dir, "a-plugin");

        let registry = Registry {
            version: 1,
            entries: vec![
                installed_entry("z-plugin", &z_dir),
                installed_entry("a-plugin", &a_dir),
            ],
        };
        let report = resolve_from_registry(&registry);

        assert_eq!(report.plugins.len(), 2);
        assert_eq!(report.plugins[0].id.as_str(), "a-plugin");
        assert_eq!(report.plugins[1].id.as_str(), "z-plugin");
    }
}
