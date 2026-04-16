use crate::plugins::registry::{Registry, Slot, SlotSource};
use crate::plugins::PluginId;
use std::path::PathBuf;

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
    use std::path::Path;
    use tempfile::TempDir;

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
