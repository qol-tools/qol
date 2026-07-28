mod migration;

pub use migration::ensure_registry_initialized;
#[cfg(feature = "dev")]
pub(crate) use migration::legacy_dev_links_path;
pub use qol_dev_build::registry::{
    dev_linked_paths, load_registry, registry_path, save_registry, Entry, Registry, Slot,
    SlotSource, CURRENT_REGISTRY_VERSION, REGISTRY_FILE_NAME,
};

use std::path::{Path, PathBuf};

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

pub fn record_release_install(
    config_dir: &Path,
    plugin_id: &str,
    plugin_root: PathBuf,
) -> Result<(), String> {
    let mut registry = load_registry(config_dir)?;
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

pub fn record_dev_link_create(
    config_dir: &Path,
    plugin_id: &str,
    dev_source: PathBuf,
) -> Result<(), String> {
    let mut registry = load_registry(config_dir)?;
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

pub fn record_dev_link_remove(config_dir: &Path, plugin_id: &str) -> Result<(), String> {
    let mut registry = load_registry(config_dir)?;
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

pub fn record_release_uninstall(config_dir: &Path, plugin_id: &str) -> Result<(), String> {
    let mut registry = load_registry(config_dir)?;
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

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

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
    fn mutations_refuse_to_clobber_a_newer_version_registry() {
        type Mutation = Box<dyn Fn(&Path) -> Result<(), String>>;
        let mutations: [(&str, Mutation); 4] = [
            (
                "record_release_install",
                Box::new(|d| record_release_install(d, "plugin-foo", PathBuf::from("/p/foo"))),
            ),
            (
                "record_dev_link_create",
                Box::new(|d| record_dev_link_create(d, "plugin-foo", PathBuf::from("/dev/src"))),
            ),
            (
                "record_dev_link_remove",
                Box::new(|d| record_dev_link_remove(d, "plugin-foo")),
            ),
            (
                "record_release_uninstall",
                Box::new(|d| record_release_uninstall(d, "plugin-foo")),
            ),
        ];
        for (name, mutate) in mutations {
            let tmp = TempDir::new().unwrap();
            let newer = Registry {
                version: CURRENT_REGISTRY_VERSION + 1,
                entries: vec![Entry {
                    id: "plugin-lights".to_string(),
                    active: Slot {
                        path: PathBuf::from("/home/user/dev/plugin-lights"),
                        source: SlotSource::DevLink {
                            origin_path: PathBuf::from("/home/user/dev/plugin-lights"),
                        },
                    },
                    fallback: None,
                }],
            };
            let content = serde_json::to_string_pretty(&newer).unwrap();
            std::fs::write(registry_path(tmp.path()), &content).unwrap();

            let result = mutate(tmp.path());
            assert!(
                result.is_err(),
                "{name}: must refuse a newer-version registry"
            );
            let on_disk = std::fs::read_to_string(registry_path(tmp.path())).unwrap();
            assert_eq!(
                on_disk, content,
                "{name}: newer-version registry must be left untouched"
            );
        }
    }
}
