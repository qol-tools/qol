mod migration;

pub use migration::ensure_registry_initialized;

use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

pub const REGISTRY_FILE_NAME: &str = "plugin-registry.json";
pub const CURRENT_REGISTRY_VERSION: u32 = 1;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Registry {
    pub version: u32,
    pub entries: Vec<Entry>,
}

impl Default for Registry {
    fn default() -> Self {
        Self {
            version: CURRENT_REGISTRY_VERSION,
            entries: Vec::new(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Entry {
    pub id: String,
    pub active: Slot,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fallback: Option<Slot>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Slot {
    pub path: PathBuf,
    pub source: SlotSource,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "type", rename_all = "kebab-case")]
pub enum SlotSource {
    ReleaseAsset,
    DevLink {
        origin_path: PathBuf,
    },
    WorktreeLink {
        origin_path: PathBuf,
        branch: String,
    },
}

pub fn registry_path(config_dir: &Path) -> PathBuf {
    config_dir.join(REGISTRY_FILE_NAME)
}

pub fn lookup_active_path(config_dir: &Path, plugin_id: &str) -> Option<PathBuf> {
    let registry = load_registry(config_dir).ok()?;
    registry
        .entries
        .into_iter()
        .find(|e| e.id == plugin_id)
        .map(|e| e.active.path)
}

pub fn current_active_path(plugin_id: &str) -> Option<PathBuf> {
    let config_dir = crate::paths::shared_config_dir().ok()?;
    lookup_active_path(&config_dir, plugin_id)
}

/// Record a release-asset install in the registry per the "Plugin-store install
/// as a pointer write" transition rules. If the plugin isn't in the registry,
/// creates an entry with ReleaseAsset active. If it is and active is already
/// ReleaseAsset, replaces active. If active is DevLink/WorktreeLink, leaves
/// active untouched and writes the release asset into the fallback slot.
pub fn record_release_install(
    config_dir: &Path,
    plugin_id: &str,
    plugin_root: PathBuf,
) -> Result<(), String> {
    let mut registry = load_registry(config_dir).unwrap_or_default();
    let new_slot = Slot {
        path: plugin_root,
        source: SlotSource::ReleaseAsset,
    };
    if let Some(entry) = registry.entries.iter_mut().find(|e| e.id == plugin_id) {
        match entry.active.source {
            SlotSource::ReleaseAsset => entry.active = new_slot,
            SlotSource::DevLink { .. } | SlotSource::WorktreeLink { .. } => {
                entry.fallback = Some(new_slot);
            }
        }
    } else {
        registry.entries.push(Entry {
            id: plugin_id.to_string(),
            active: new_slot,
            fallback: None,
        });
    }
    registry.entries.sort_by(|a, b| a.id.cmp(&b.id));
    save_registry(config_dir, &registry)
}

/// Record a dev-link create per the "Dev-link as a pointer write" transition
/// rules. If no entry: create with DevLink active. If active is ReleaseAsset:
/// preserve as fallback, replace active. If active is already DevLink or
/// WorktreeLink: replace active (fallback untouched).
pub fn record_dev_link_create(
    config_dir: &Path,
    plugin_id: &str,
    dev_source: PathBuf,
) -> Result<(), String> {
    let mut registry = load_registry(config_dir).unwrap_or_default();
    let new_active = Slot {
        path: dev_source.clone(),
        source: SlotSource::DevLink {
            origin_path: dev_source,
        },
    };
    if let Some(entry) = registry.entries.iter_mut().find(|e| e.id == plugin_id) {
        let previous_active = std::mem::replace(&mut entry.active, new_active);
        if matches!(previous_active.source, SlotSource::ReleaseAsset) {
            entry.fallback = Some(previous_active);
        }
    } else {
        registry.entries.push(Entry {
            id: plugin_id.to_string(),
            active: new_active,
            fallback: None,
        });
    }
    registry.entries.sort_by(|a, b| a.id.cmp(&b.id));
    save_registry(config_dir, &registry)
}

/// Record a dev-link removal. If fallback exists, promote it to active and
/// clear the fallback slot. If no fallback, remove the entry entirely.
pub fn record_dev_link_remove(config_dir: &Path, plugin_id: &str) -> Result<(), String> {
    let mut registry = load_registry(config_dir).unwrap_or_default();
    let Some(idx) = registry.entries.iter().position(|e| e.id == plugin_id) else {
        return Ok(());
    };
    let entry = &mut registry.entries[idx];
    match entry.fallback.take() {
        Some(fallback) => {
            entry.active = fallback;
        }
        None => {
            registry.entries.remove(idx);
        }
    }
    save_registry(config_dir, &registry)
}

/// Record a release-asset uninstall per the transition rules. If active is
/// ReleaseAsset without a fallback, drops the whole entry. If active is
/// DevLink/WorktreeLink, clears the fallback slot. If active is ReleaseAsset
/// with a fallback (unexpected per install rules but handled defensively),
/// promotes the fallback to active.
pub fn record_release_uninstall(config_dir: &Path, plugin_id: &str) -> Result<(), String> {
    let mut registry = load_registry(config_dir).unwrap_or_default();
    let Some(idx) = registry.entries.iter().position(|e| e.id == plugin_id) else {
        return Ok(());
    };
    let entry = &mut registry.entries[idx];
    match entry.active.source {
        SlotSource::ReleaseAsset => match entry.fallback.take() {
            Some(fallback) => entry.active = fallback,
            None => {
                registry.entries.remove(idx);
            }
        },
        SlotSource::DevLink { .. } | SlotSource::WorktreeLink { .. } => {
            entry.fallback = None;
        }
    }
    save_registry(config_dir, &registry)
}

pub fn load_registry(config_dir: &Path) -> Result<Registry, String> {
    let path = registry_path(config_dir);
    let content = match std::fs::read_to_string(&path) {
        Ok(content) => content,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Registry::default()),
        Err(e) => return Err(format!("Failed to read {}: {}", path.display(), e)),
    };
    let registry: Registry = serde_json::from_str(&content)
        .map_err(|e| format!("Failed to parse {}: {}", path.display(), e))?;
    if registry.version > CURRENT_REGISTRY_VERSION {
        return Err(format!(
            "Registry at {} is version {}; this binary supports up to version {}. \
             Downgrade detected — refusing to read a newer format.",
            path.display(),
            registry.version,
            CURRENT_REGISTRY_VERSION
        ));
    }
    Ok(registry)
}

pub fn save_registry(config_dir: &Path, registry: &Registry) -> Result<(), String> {
    let final_path = registry_path(config_dir);
    let tmp_path = config_dir.join(format!("{}.new", REGISTRY_FILE_NAME));
    let content = serde_json::to_string_pretty(registry)
        .map_err(|e| format!("Failed to serialize registry: {}", e))?;
    std::fs::write(&tmp_path, &content)
        .map_err(|e| format!("Failed to write {}: {}", tmp_path.display(), e))?;
    std::fs::rename(&tmp_path, &final_path)
        .map_err(|e| format!("Failed to finalize {}: {}", final_path.display(), e))?;
    let _ = fsync_dir(config_dir);
    Ok(())
}

fn fsync_dir(dir: &Path) -> std::io::Result<()> {
    #[cfg(unix)]
    {
        std::fs::File::open(dir)?.sync_all()
    }
    #[cfg(not(unix))]
    {
        let _ = dir;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    fn sample_entry() -> Entry {
        Entry {
            id: "plugin-lights".to_string(),
            active: Slot {
                path: PathBuf::from("/home/user/dev/plugin-lights"),
                source: SlotSource::DevLink {
                    origin_path: PathBuf::from("/home/user/dev/plugin-lights"),
                },
            },
            fallback: Some(Slot {
                path: PathBuf::from("/home/user/.config/qol-tray/plugins/plugin-lights"),
                source: SlotSource::ReleaseAsset,
            }),
        }
    }

    #[test]
    fn default_registry_uses_current_version_and_no_entries() {
        let r = Registry::default();
        assert_eq!(r.version, CURRENT_REGISTRY_VERSION);
        assert!(r.entries.is_empty());
    }

    #[test]
    fn load_missing_file_returns_default() {
        let tmp = TempDir::new().unwrap();
        let registry = load_registry(tmp.path()).unwrap();
        assert_eq!(registry, Registry::default());
    }

    #[test]
    fn save_then_load_round_trips() {
        let tmp = TempDir::new().unwrap();
        let registry = Registry {
            version: CURRENT_REGISTRY_VERSION,
            entries: vec![sample_entry()],
        };
        save_registry(tmp.path(), &registry).unwrap();
        let loaded = load_registry(tmp.path()).unwrap();
        assert_eq!(loaded, registry);
    }

    #[test]
    fn fallback_is_omitted_when_none() {
        let entry = Entry {
            id: "plugin-x".to_string(),
            active: Slot {
                path: PathBuf::from("/x"),
                source: SlotSource::ReleaseAsset,
            },
            fallback: None,
        };
        let json = serde_json::to_string(&entry).unwrap();
        assert!(!json.contains("fallback"));
    }

    #[test]
    fn slot_source_tags_are_kebab_case() {
        let release = serde_json::to_string(&SlotSource::ReleaseAsset).unwrap();
        assert!(release.contains("\"type\":\"release-asset\""));
        let dev = serde_json::to_string(&SlotSource::DevLink {
            origin_path: PathBuf::from("/p"),
        })
        .unwrap();
        assert!(dev.contains("\"type\":\"dev-link\""));
        let wt = serde_json::to_string(&SlotSource::WorktreeLink {
            origin_path: PathBuf::from("/p"),
            branch: "feat".to_string(),
        })
        .unwrap();
        assert!(wt.contains("\"type\":\"worktree-link\""));
    }

    #[test]
    fn malformed_json_returns_error() {
        let tmp = TempDir::new().unwrap();
        std::fs::write(registry_path(tmp.path()), "{not json}").unwrap();
        let result = load_registry(tmp.path());
        assert!(result.is_err());
        let err = result.unwrap_err();
        assert!(err.contains("parse"));
    }

    #[test]
    fn record_release_install_creates_entry_when_absent() {
        let tmp = TempDir::new().unwrap();
        record_release_install(tmp.path(), "plugin-foo", PathBuf::from("/p/plugin-foo")).unwrap();
        let registry = load_registry(tmp.path()).unwrap();
        assert_eq!(registry.entries.len(), 1);
        assert_eq!(registry.entries[0].id, "plugin-foo");
        assert!(matches!(
            registry.entries[0].active.source,
            SlotSource::ReleaseAsset
        ));
        assert!(registry.entries[0].fallback.is_none());
    }

    #[test]
    fn record_release_install_replaces_active_when_release_asset() {
        let tmp = TempDir::new().unwrap();
        record_release_install(tmp.path(), "plugin-foo", PathBuf::from("/p/old")).unwrap();
        record_release_install(tmp.path(), "plugin-foo", PathBuf::from("/p/new")).unwrap();
        let registry = load_registry(tmp.path()).unwrap();
        assert_eq!(registry.entries.len(), 1);
        assert_eq!(registry.entries[0].active.path, PathBuf::from("/p/new"));
    }

    #[test]
    fn record_release_install_preserves_dev_link_writes_fallback() {
        let tmp = TempDir::new().unwrap();
        let reg = Registry {
            version: CURRENT_REGISTRY_VERSION,
            entries: vec![Entry {
                id: "plugin-foo".to_string(),
                active: Slot {
                    path: PathBuf::from("/dev/src"),
                    source: SlotSource::DevLink {
                        origin_path: PathBuf::from("/dev/src"),
                    },
                },
                fallback: None,
            }],
        };
        save_registry(tmp.path(), &reg).unwrap();

        record_release_install(tmp.path(), "plugin-foo", PathBuf::from("/p/plugin-foo")).unwrap();
        let registry = load_registry(tmp.path()).unwrap();
        assert_eq!(registry.entries.len(), 1);
        assert!(matches!(
            registry.entries[0].active.source,
            SlotSource::DevLink { .. }
        ));
        let fallback = registry.entries[0].fallback.as_ref().unwrap();
        assert!(matches!(fallback.source, SlotSource::ReleaseAsset));
        assert_eq!(fallback.path, PathBuf::from("/p/plugin-foo"));
    }

    #[test]
    fn record_release_uninstall_drops_entry_when_only_release_asset() {
        let tmp = TempDir::new().unwrap();
        record_release_install(tmp.path(), "plugin-foo", PathBuf::from("/p/plugin-foo")).unwrap();
        record_release_uninstall(tmp.path(), "plugin-foo").unwrap();
        let registry = load_registry(tmp.path()).unwrap();
        assert_eq!(registry.entries.len(), 0);
    }

    #[test]
    fn record_release_uninstall_clears_fallback_when_dev_link_active() {
        let tmp = TempDir::new().unwrap();
        let reg = Registry {
            version: CURRENT_REGISTRY_VERSION,
            entries: vec![Entry {
                id: "plugin-foo".to_string(),
                active: Slot {
                    path: PathBuf::from("/dev/src"),
                    source: SlotSource::DevLink {
                        origin_path: PathBuf::from("/dev/src"),
                    },
                },
                fallback: Some(Slot {
                    path: PathBuf::from("/p/plugin-foo"),
                    source: SlotSource::ReleaseAsset,
                }),
            }],
        };
        save_registry(tmp.path(), &reg).unwrap();

        record_release_uninstall(tmp.path(), "plugin-foo").unwrap();
        let registry = load_registry(tmp.path()).unwrap();
        assert_eq!(registry.entries.len(), 1);
        assert!(matches!(
            registry.entries[0].active.source,
            SlotSource::DevLink { .. }
        ));
        assert!(registry.entries[0].fallback.is_none());
    }

    #[test]
    fn record_release_uninstall_promotes_fallback_when_active_is_release_asset_with_fallback() {
        let tmp = TempDir::new().unwrap();
        let reg = Registry {
            version: CURRENT_REGISTRY_VERSION,
            entries: vec![Entry {
                id: "plugin-foo".to_string(),
                active: Slot {
                    path: PathBuf::from("/p/plugin-foo"),
                    source: SlotSource::ReleaseAsset,
                },
                fallback: Some(Slot {
                    path: PathBuf::from("/dev/src"),
                    source: SlotSource::DevLink {
                        origin_path: PathBuf::from("/dev/src"),
                    },
                }),
            }],
        };
        save_registry(tmp.path(), &reg).unwrap();

        record_release_uninstall(tmp.path(), "plugin-foo").unwrap();
        let registry = load_registry(tmp.path()).unwrap();
        assert_eq!(registry.entries.len(), 1);
        assert!(matches!(
            registry.entries[0].active.source,
            SlotSource::DevLink { .. }
        ));
        assert_eq!(registry.entries[0].active.path, PathBuf::from("/dev/src"));
        assert!(registry.entries[0].fallback.is_none());
    }

    #[test]
    fn record_release_uninstall_is_noop_when_entry_missing() {
        let tmp = TempDir::new().unwrap();
        record_release_uninstall(tmp.path(), "absent-plugin").unwrap();
        let registry = load_registry(tmp.path()).unwrap();
        assert_eq!(registry.entries.len(), 0);
    }

    #[test]
    fn record_dev_link_create_adds_new_entry_when_absent() {
        let tmp = TempDir::new().unwrap();
        record_dev_link_create(tmp.path(), "plugin-foo", PathBuf::from("/dev/src")).unwrap();
        let registry = load_registry(tmp.path()).unwrap();
        assert_eq!(registry.entries.len(), 1);
        assert!(matches!(
            registry.entries[0].active.source,
            SlotSource::DevLink { .. }
        ));
        assert!(registry.entries[0].fallback.is_none());
    }

    #[test]
    fn record_dev_link_create_promotes_release_asset_to_fallback() {
        let tmp = TempDir::new().unwrap();
        record_release_install(tmp.path(), "plugin-foo", PathBuf::from("/p/plugin-foo")).unwrap();
        record_dev_link_create(tmp.path(), "plugin-foo", PathBuf::from("/dev/src")).unwrap();
        let registry = load_registry(tmp.path()).unwrap();
        assert_eq!(registry.entries.len(), 1);
        assert!(matches!(
            registry.entries[0].active.source,
            SlotSource::DevLink { .. }
        ));
        let fallback = registry.entries[0].fallback.as_ref().unwrap();
        assert!(matches!(fallback.source, SlotSource::ReleaseAsset));
        assert_eq!(fallback.path, PathBuf::from("/p/plugin-foo"));
    }

    #[test]
    fn record_dev_link_create_replaces_existing_dev_link_preserves_fallback() {
        let tmp = TempDir::new().unwrap();
        record_release_install(tmp.path(), "plugin-foo", PathBuf::from("/p/plugin-foo")).unwrap();
        record_dev_link_create(tmp.path(), "plugin-foo", PathBuf::from("/dev/src-old")).unwrap();
        record_dev_link_create(tmp.path(), "plugin-foo", PathBuf::from("/dev/src-new")).unwrap();

        let registry = load_registry(tmp.path()).unwrap();
        assert_eq!(
            registry.entries[0].active.path,
            PathBuf::from("/dev/src-new")
        );
        let fallback = registry.entries[0].fallback.as_ref().unwrap();
        assert_eq!(fallback.path, PathBuf::from("/p/plugin-foo"));
    }

    #[test]
    fn record_dev_link_remove_promotes_fallback_to_active() {
        let tmp = TempDir::new().unwrap();
        record_release_install(tmp.path(), "plugin-foo", PathBuf::from("/p/plugin-foo")).unwrap();
        record_dev_link_create(tmp.path(), "plugin-foo", PathBuf::from("/dev/src")).unwrap();
        record_dev_link_remove(tmp.path(), "plugin-foo").unwrap();

        let registry = load_registry(tmp.path()).unwrap();
        assert_eq!(registry.entries.len(), 1);
        assert!(matches!(
            registry.entries[0].active.source,
            SlotSource::ReleaseAsset
        ));
        assert_eq!(
            registry.entries[0].active.path,
            PathBuf::from("/p/plugin-foo")
        );
        assert!(registry.entries[0].fallback.is_none());
    }

    #[test]
    fn record_dev_link_remove_drops_entry_when_no_fallback() {
        let tmp = TempDir::new().unwrap();
        record_dev_link_create(tmp.path(), "plugin-foo", PathBuf::from("/dev/src")).unwrap();
        record_dev_link_remove(tmp.path(), "plugin-foo").unwrap();

        let registry = load_registry(tmp.path()).unwrap();
        assert_eq!(registry.entries.len(), 0);
    }

    #[test]
    fn record_dev_link_remove_is_noop_when_entry_missing() {
        let tmp = TempDir::new().unwrap();
        record_dev_link_remove(tmp.path(), "absent").unwrap();
        let registry = load_registry(tmp.path()).unwrap();
        assert_eq!(registry.entries.len(), 0);
    }

    #[test]
    fn rejects_registry_with_future_version() {
        let tmp = TempDir::new().unwrap();
        let future = Registry {
            version: CURRENT_REGISTRY_VERSION + 1,
            entries: vec![],
        };
        let content = serde_json::to_string_pretty(&future).unwrap();
        std::fs::write(registry_path(tmp.path()), content).unwrap();
        let result = load_registry(tmp.path());
        assert!(result.is_err());
        let err = result.unwrap_err();
        assert!(err.contains("Downgrade"));
    }
}
