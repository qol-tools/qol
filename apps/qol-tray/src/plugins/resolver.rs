use crate::plugins::registry::{Registry, Slot, SlotSource};
use crate::plugins::PluginId;
use std::path::PathBuf;

#[derive(Debug, Clone)]
pub struct ResolvedPlugin {
    pub id: PluginId,
    pub path: PathBuf,
    pub source: PluginSource,
    pub resolved_from: ResolutionOrigin,
    pub active_failure: Option<SlotFailure>,
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

#[derive(Debug, Clone, Default)]
pub struct ResolutionReport {
    pub plugins: Vec<ResolvedPlugin>,
    pub unavailable: Vec<PluginUnavailable>,
}

pub fn resolve_from_registry(registry: &Registry) -> ResolutionReport {
    let mut report = ResolutionReport::default();
    for entry in &registry.entries {
        match validate_slot(&entry.id, &entry.active) {
            Ok(()) => report.plugins.push(resolved_from_slot(
                &entry.id,
                &entry.active,
                ResolutionOrigin::Active,
                None,
            )),
            Err(active_reason) => {
                let active_fail = SlotFailure {
                    path: entry.active.path.clone(),
                    reason: active_reason.clone(),
                };
                match entry
                    .fallback
                    .as_ref()
                    .map(|s| (s, validate_slot(&entry.id, s)))
                {
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
                            Some(active_fail),
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

fn resolved_from_slot(
    id: &str,
    slot: &Slot,
    origin: ResolutionOrigin,
    active_failure: Option<SlotFailure>,
) -> ResolvedPlugin {
    ResolvedPlugin {
        id: PluginId::new(id.to_string()),
        path: slot.path.clone(),
        source: slot_source_to_plugin_source(&slot.source),
        resolved_from: origin,
        active_failure,
    }
}

fn slot_source_to_plugin_source(source: &SlotSource) -> PluginSource {
    match source {
        SlotSource::ReleaseAsset => PluginSource::Installed,
        SlotSource::DevLink { .. } | SlotSource::WorktreeLink { .. } => PluginSource::DevLinked,
    }
}

fn validate_slot(plugin_id: &str, slot: &Slot) -> Result<(), String> {
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
    if !manifest.plugin.supports_current_platform() {
        return Ok(());
    }
    let plugin_source = slot_source_to_plugin_source(&slot.source);
    crate::plugins::validate_execution_contract_for_source(
        plugin_id,
        &manifest,
        &slot.path,
        Some(&plugin_source),
    )
    .map_err(|e| format!("binary contract for {}: {}", manifest_path.display(), e))?;
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

    fn write_runtime_manifest(plugin_dir: &Path, name: &str, command: &str) {
        fs::write(
            plugin_dir.join("plugin.toml"),
            format!(
                "[plugin]\nname = \"{name}\"\ndescription = \"\"\nversion = \"1.0.0\"\n\n[menu]\nlabel = \"Test\"\nitems = []\n\n[runtime]\ncommand = \"{command}\"\n",
            ),
        )
        .unwrap();
    }

    fn write_daemon_manifest(plugin_dir: &Path, name: &str, command: &str) {
        fs::write(
            plugin_dir.join("plugin.toml"),
            format!(
                "[plugin]\nname = \"{name}\"\ndescription = \"\"\nversion = \"1.0.0\"\n\n[menu]\nlabel = \"Test\"\nitems = []\n\n[daemon]\nenabled = true\ncommand = \"{command}\"\n",
            ),
        )
        .unwrap();
    }

    fn write_executable(path: &Path) {
        fs::write(path, "#!/bin/sh\nexit 0\n").unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mut perms = fs::metadata(path).unwrap().permissions();
            perms.set_mode(0o755);
            fs::set_permissions(path, perms).unwrap();
        }
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
        assert!(report.plugins[0].active_failure.is_none());
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
        let failure = report.plugins[0]
            .active_failure
            .as_ref()
            .expect("fallback resolution carries active failure");
        assert_eq!(failure.path, tmp.path().join("missing-dev-src"));
        assert!(failure.reason.contains("not a directory"));
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
    fn falls_back_when_active_binary_missing_and_fallback_binary_present() {
        let tmp = TempDir::new().unwrap();
        let active_dir = tmp.path().join("active");
        let fallback_dir = tmp.path().join("fallback");
        fs::create_dir(&active_dir).unwrap();
        fs::create_dir(&fallback_dir).unwrap();
        write_runtime_manifest(&active_dir, "plugin-foo", "plugin-foo");
        write_runtime_manifest(&fallback_dir, "plugin-foo", "plugin-foo");
        write_executable(&fallback_dir.join("plugin-foo"));

        let entry = Entry {
            id: "plugin-foo".to_string(),
            active: Slot {
                path: active_dir.clone(),
                source: SlotSource::ReleaseAsset,
            },
            fallback: Some(Slot {
                path: fallback_dir.clone(),
                source: SlotSource::ReleaseAsset,
            }),
        };
        let registry = Registry {
            version: 1,
            entries: vec![entry],
        };
        let report = resolve_from_registry(&registry);

        assert_eq!(report.plugins.len(), 1, "fallback chosen");
        assert_eq!(report.unavailable.len(), 0);
        assert_eq!(report.plugins[0].resolved_from, ResolutionOrigin::Fallback);
        assert_eq!(report.plugins[0].path, fallback_dir);
        let failure = report.plugins[0]
            .active_failure
            .as_ref()
            .expect("active failure recorded");
        assert_eq!(failure.path, active_dir);
        assert!(
            failure.reason.contains("binary"),
            "failure should describe binary contract: {}",
            failure.reason
        );
    }

    #[test]
    fn unavailable_when_active_and_fallback_binaries_missing() {
        let tmp = TempDir::new().unwrap();
        let active_dir = tmp.path().join("active");
        let fallback_dir = tmp.path().join("fallback");
        fs::create_dir(&active_dir).unwrap();
        fs::create_dir(&fallback_dir).unwrap();
        write_runtime_manifest(&active_dir, "plugin-foo", "plugin-foo");
        write_runtime_manifest(&fallback_dir, "plugin-foo", "plugin-foo");

        let entry = Entry {
            id: "plugin-foo".to_string(),
            active: Slot {
                path: active_dir,
                source: SlotSource::ReleaseAsset,
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

        assert_eq!(report.plugins.len(), 0);
        assert_eq!(report.unavailable.len(), 1);
        let u = &report.unavailable[0];
        assert_eq!(u.id, "plugin-foo");
        assert!(u.active.reason.contains("binary"), "{}", u.active.reason);
        let fallback = u.fallback.as_ref().expect("fallback failure recorded");
        assert!(fallback.reason.contains("binary"), "{}", fallback.reason);
    }

    #[test]
    fn falls_back_when_active_daemon_binary_missing_and_fallback_present() {
        let tmp = TempDir::new().unwrap();
        let active_dir = tmp.path().join("active");
        let fallback_dir = tmp.path().join("fallback");
        fs::create_dir(&active_dir).unwrap();
        fs::create_dir(&fallback_dir).unwrap();
        write_daemon_manifest(&active_dir, "plugin-foo", "plugin-foo");
        write_daemon_manifest(&fallback_dir, "plugin-foo", "plugin-foo");
        write_executable(&fallback_dir.join("plugin-foo"));

        let entry = Entry {
            id: "plugin-foo".to_string(),
            active: Slot {
                path: active_dir.clone(),
                source: SlotSource::ReleaseAsset,
            },
            fallback: Some(Slot {
                path: fallback_dir.clone(),
                source: SlotSource::ReleaseAsset,
            }),
        };
        let registry = Registry {
            version: 1,
            entries: vec![entry],
        };
        let report = resolve_from_registry(&registry);

        assert_eq!(report.plugins.len(), 1, "fallback chosen");
        assert_eq!(report.unavailable.len(), 0);
        assert_eq!(report.plugins[0].resolved_from, ResolutionOrigin::Fallback);
        assert_eq!(report.plugins[0].path, fallback_dir);
        let failure = report.plugins[0]
            .active_failure
            .as_ref()
            .expect("active failure recorded");
        assert_eq!(failure.path, active_dir);
        assert!(
            failure.reason.contains("daemon.command"),
            "failure should describe daemon.command contract: {}",
            failure.reason
        );
    }

    #[test]
    fn unavailable_when_active_and_fallback_daemon_binaries_missing() {
        let tmp = TempDir::new().unwrap();
        let active_dir = tmp.path().join("active");
        let fallback_dir = tmp.path().join("fallback");
        fs::create_dir(&active_dir).unwrap();
        fs::create_dir(&fallback_dir).unwrap();
        write_daemon_manifest(&active_dir, "plugin-foo", "plugin-foo");
        write_daemon_manifest(&fallback_dir, "plugin-foo", "plugin-foo");

        let entry = Entry {
            id: "plugin-foo".to_string(),
            active: Slot {
                path: active_dir,
                source: SlotSource::ReleaseAsset,
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

        assert_eq!(report.plugins.len(), 0);
        assert_eq!(report.unavailable.len(), 1);
        let u = &report.unavailable[0];
        assert!(
            u.active.reason.contains("daemon.command"),
            "{}",
            u.active.reason
        );
        let fallback = u.fallback.as_ref().expect("fallback failure recorded");
        assert!(
            fallback.reason.contains("daemon.command"),
            "{}",
            fallback.reason
        );
    }

    #[test]
    fn unavailable_when_active_binary_missing_no_fallback() {
        let tmp = TempDir::new().unwrap();
        let active_dir = tmp.path().join("active");
        fs::create_dir(&active_dir).unwrap();
        write_runtime_manifest(&active_dir, "plugin-foo", "plugin-foo");

        let entry = Entry {
            id: "plugin-foo".to_string(),
            active: Slot {
                path: active_dir,
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
        assert!(report.unavailable[0].active.reason.contains("binary"));
    }

    #[cfg(feature = "dev")]
    #[test]
    fn dev_link_active_with_target_debug_binary_resolves() {
        let tmp = TempDir::new().unwrap();
        let dev_dir = tmp.path().join("plugin-foo-src");
        fs::create_dir_all(dev_dir.join("target").join("debug")).unwrap();
        write_runtime_manifest(&dev_dir, "plugin-foo", "plugin-foo");
        write_executable(&dev_dir.join("target").join("debug").join("plugin-foo"));

        let entry = Entry {
            id: "plugin-foo".to_string(),
            active: Slot {
                path: dev_dir.clone(),
                source: SlotSource::DevLink {
                    origin_path: dev_dir.clone(),
                },
            },
            fallback: None,
        };
        let registry = Registry {
            version: 1,
            entries: vec![entry],
        };
        let report = resolve_from_registry(&registry);

        assert_eq!(report.plugins.len(), 1);
        assert_eq!(report.unavailable.len(), 0);
        assert_eq!(report.plugins[0].resolved_from, ResolutionOrigin::Active);
        assert_eq!(report.plugins[0].source, PluginSource::DevLinked);
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
