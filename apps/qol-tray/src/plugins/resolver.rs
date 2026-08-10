use crate::plugins::registry::{Entry, Registry, Slot, SlotSource};
use crate::plugins::PluginId;
#[cfg(feature = "dev")]
use std::collections::HashMap;
use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

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

pub(crate) fn resolve_effective_registry(
    registry: &Registry,
    config_dir: &Path,
) -> ResolutionReport {
    let mut effective = registry.clone();
    apply_worktree_override(&mut effective, config_dir);
    resolve_from_registry(&effective)
}

pub fn resolve_from_registry(registry: &Registry) -> ResolutionReport {
    let mut report = ResolutionReport::default();
    let entry_counts = registry
        .entries
        .iter()
        .filter(|entry| !crate::plugins::is_reserved_plugin_id(&entry.id))
        .fold(BTreeMap::<&str, usize>::new(), |mut counts, entry| {
            *counts.entry(entry.id.as_str()).or_default() += 1;
            counts
        });
    let mut reported_duplicates = BTreeSet::new();
    for entry in &registry.entries {
        if crate::plugins::is_reserved_plugin_id(&entry.id) {
            log::warn!(
                "Refusing to resolve reserved plugin id from registry: {}",
                entry.id
            );
            continue;
        }
        if entry_counts
            .get(entry.id.as_str())
            .copied()
            .unwrap_or_default()
            > 1
        {
            if reported_duplicates.insert(entry.id.as_str()) {
                report.unavailable.push(PluginUnavailable {
                    id: entry.id.clone(),
                    active: SlotFailure {
                        path: entry.active.path.clone(),
                        reason: "duplicate registry entries share this plugin id".to_string(),
                    },
                    fallback: None,
                });
            }
            continue;
        }
        if !crate::plugins::manifest::is_valid_plugin_id(&entry.id) {
            report.unavailable.push(PluginUnavailable {
                id: entry.id.clone(),
                active: SlotFailure {
                    path: entry.active.path.clone(),
                    reason: "registry entry has an invalid plugin id".to_string(),
                },
                fallback: None,
            });
            continue;
        }
        let (active, fallback) = visible_slots(entry);
        let Some(active) = active else {
            continue;
        };
        match validate_slot(&entry.id, active) {
            Ok(()) => report.plugins.push(resolved_from_slot(
                &entry.id,
                active,
                ResolutionOrigin::Active,
                None,
            )),
            Err(active_reason) => {
                let active_fail = SlotFailure {
                    path: active.path.clone(),
                    reason: active_reason.clone(),
                };
                match fallback.map(|s| (s, validate_slot(&entry.id, s))) {
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

#[cfg(feature = "dev")]
fn apply_worktree_override(registry: &mut Registry, config_dir: &Path) {
    let Some(branch) = crate::dev::get_active_worktree_branch(config_dir) else {
        return;
    };
    let base_links: HashMap<String, PathBuf> = registry
        .entries
        .iter()
        .filter(|entry| {
            matches!(
                entry.active.source,
                SlotSource::DevLink { .. } | SlotSource::WorktreeLink { .. }
            )
        })
        .map(|entry| (entry.id.clone(), entry.active.path.clone()))
        .collect();
    if base_links.is_empty() {
        return;
    }
    let resolved = crate::dev::resolve_worktree_paths(&base_links, Some(&branch));
    let mut detached = Vec::new();
    for entry in &mut registry.entries {
        if let Some(new_path) = resolved.get(&entry.id) {
            override_entry_to_worktree(entry, new_path);
        } else if base_links.contains_key(&entry.id) {
            detached.push(entry.id.clone());
        }
    }
    if detached.is_empty() {
        return;
    }
    let mut removed = Vec::new();
    for entry in &mut registry.entries {
        if !detached.contains(&entry.id) {
            continue;
        }
        if let Some(fallback) = entry.fallback.take() {
            log::info!(
                "[worktree] detaching dev link for {}: promoting installed fallback",
                entry.id
            );
            entry.active = fallback;
        } else {
            removed.push(entry.id.clone());
        }
    }
    if !removed.is_empty() {
        log::info!(
            "[worktree] detaching dev links outside the active selection: {}",
            removed.join(", ")
        );
        registry
            .entries
            .retain(|entry| !removed.contains(&entry.id));
    }
}

/// Repoint an entry's active slot at the worktree, making the pre-override slot
/// the fallback so a plugin whose binary was not built in the worktree resolves
/// to its main-clone build. The override only runs on dev-linked actives, so the
/// pre-override slot is always the main-clone build; it replaces any stale
/// installed-release fallback (which would be protocol-incompatible with a
/// freshly built host).
#[cfg(feature = "dev")]
fn override_entry_to_worktree(entry: &mut Entry, new_path: &Path) {
    if *new_path == entry.active.path {
        return;
    }
    log::info!(
        "[worktree] overriding plugin={} with active worktree slot",
        entry.id
    );
    entry.fallback = Some(entry.active.clone());
    entry.active.path = new_path.to_path_buf();
}

#[cfg(not(feature = "dev"))]
fn apply_worktree_override(_registry: &mut Registry, _config_dir: &Path) {}

fn visible_slots(entry: &Entry) -> (Option<&Slot>, Option<&Slot>) {
    let active = is_visible_slot(&entry.active).then_some(&entry.active);
    let fallback = entry.fallback.as_ref().filter(|slot| is_visible_slot(slot));
    match (active, fallback) {
        (Some(active), fallback) => (Some(active), fallback),
        (None, Some(promoted)) => (Some(promoted), None),
        (None, None) => (None, None),
    }
}

#[cfg(feature = "dev")]
fn is_visible_slot(_slot: &Slot) -> bool {
    true
}

#[cfg(not(feature = "dev"))]
fn is_visible_slot(slot: &Slot) -> bool {
    matches!(slot.source, SlotSource::ReleaseAsset)
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

    fn write_valid_manifest(plugin_dir: &Path, id: &str) {
        fs::write(
            plugin_dir.join("plugin.toml"),
            format!(
                "[plugin]\nid = \"{id}\"\nname = \"{id}\"\ndescription = \"\"\nversion = \"1.0.0\"\n\n[menu]\nlabel = \"Test\"\nitems = []\n",
            ),
        )
        .unwrap();
    }

    fn write_runtime_manifest(plugin_dir: &Path, id: &str, command: &str) {
        fs::write(
            plugin_dir.join("plugin.toml"),
            format!(
                "[plugin]\nid = \"{id}\"\nname = \"{id}\"\ndescription = \"\"\nversion = \"1.0.0\"\n\n[menu]\nlabel = \"Test\"\nitems = []\n\n[runtime]\ncommand = \"{command}\"\n",
            ),
        )
        .unwrap();
    }

    fn write_daemon_manifest(plugin_dir: &Path, id: &str, command: &str) {
        fs::write(
            plugin_dir.join("plugin.toml"),
            format!(
                "[plugin]\nid = \"{id}\"\nname = \"{id}\"\ndescription = \"\"\nversion = \"1.0.0\"\n\n[menu]\nlabel = \"Test\"\nitems = []\n\n[daemon]\nenabled = true\ncommand = \"{command}\"\n",
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
    fn resolve_from_registry_skips_reserved_ids() {
        let tmp = TempDir::new().unwrap();
        let plugin_dir = tmp.path().join("plugin-template");
        fs::create_dir(&plugin_dir).unwrap();
        write_valid_manifest(&plugin_dir, "plugin-template");

        let registry = Registry {
            version: 1,
            entries: vec![installed_entry("plugin-template", &plugin_dir)],
        };
        let report = resolve_from_registry(&registry);

        assert!(
            report.plugins.is_empty(),
            "reserved id must never resolve: {:?}",
            report.plugins
        );
        assert!(report.unavailable.is_empty());
    }

    #[test]
    fn resolve_from_registry_rejects_duplicate_and_unsafe_ids_before_loading() {
        let registry = Registry {
            version: 1,
            entries: vec![
                installed_entry("plugin-duplicate", Path::new("/first")),
                installed_entry("plugin-duplicate", Path::new("/second")),
                installed_entry("../unsafe", Path::new("/unsafe")),
            ],
        };

        let report = resolve_from_registry(&registry);

        assert!(report.plugins.is_empty());
        assert_eq!(report.unavailable.len(), 2);
        assert_eq!(report.unavailable[0].id, "../unsafe");
        assert!(report.unavailable[0]
            .active
            .reason
            .contains("invalid plugin id"));
        assert_eq!(report.unavailable[1].id, "plugin-duplicate");
        assert!(report.unavailable[1]
            .active
            .reason
            .contains("duplicate registry entries"));
    }

    #[cfg(feature = "dev")]
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

    #[cfg(feature = "dev")]
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

    #[cfg(not(feature = "dev"))]
    #[test]
    fn prod_promotes_release_fallback_when_active_is_dev_link() {
        let tmp = TempDir::new().unwrap();
        let fallback_dir = tmp.path().join("plugin-foo");
        fs::create_dir(&fallback_dir).unwrap();
        write_valid_manifest(&fallback_dir, "plugin-foo");

        let entry = Entry {
            id: "plugin-foo".to_string(),
            active: Slot {
                path: tmp.path().join("any-dev-src"),
                source: SlotSource::DevLink {
                    origin_path: tmp.path().join("any-dev-src"),
                },
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

        assert_eq!(report.plugins.len(), 1);
        assert_eq!(report.unavailable.len(), 0);
        assert_eq!(
            report.plugins[0].resolved_from,
            ResolutionOrigin::Active,
            "prod must surface release-asset as active, not fallback",
        );
        assert!(
            report.plugins[0].active_failure.is_none(),
            "prod must not emit a FALLBACK chip when dev slot was filtered",
        );
        assert_eq!(report.plugins[0].path, fallback_dir);
    }

    #[cfg(not(feature = "dev"))]
    #[test]
    fn prod_drops_entry_when_only_slot_is_dev_link() {
        let tmp = TempDir::new().unwrap();
        let entry = Entry {
            id: "plugin-foo".to_string(),
            active: Slot {
                path: tmp.path().join("dev-src"),
                source: SlotSource::DevLink {
                    origin_path: tmp.path().join("dev-src"),
                },
            },
            fallback: None,
        };
        let registry = Registry {
            version: 1,
            entries: vec![entry],
        };
        let report = resolve_from_registry(&registry);

        assert!(report.plugins.is_empty(), "no visible slots, no plugin");
        assert!(
            report.unavailable.is_empty(),
            "dev-only entries do not appear as broken in prod, they simply do not exist",
        );
    }

    #[cfg(not(feature = "dev"))]
    #[test]
    fn prod_ignores_worktree_link_active_with_release_fallback() {
        let tmp = TempDir::new().unwrap();
        let release_dir = tmp.path().join("plugin-foo");
        fs::create_dir(&release_dir).unwrap();
        write_valid_manifest(&release_dir, "plugin-foo");

        let entry = Entry {
            id: "plugin-foo".to_string(),
            active: Slot {
                path: tmp.path().join("worktree-src"),
                source: SlotSource::WorktreeLink {
                    origin_path: tmp.path().join("worktree-src"),
                    branch: "feature-x".to_string(),
                },
            },
            fallback: Some(Slot {
                path: release_dir.clone(),
                source: SlotSource::ReleaseAsset,
            }),
        };
        let registry = Registry {
            version: 1,
            entries: vec![entry],
        };
        let report = resolve_from_registry(&registry);

        assert_eq!(report.plugins.len(), 1);
        assert_eq!(report.plugins[0].resolved_from, ResolutionOrigin::Active);
        assert_eq!(report.plugins[0].path, release_dir);
    }

    #[cfg(not(feature = "dev"))]
    #[test]
    fn prod_filters_dev_fallback_when_release_active_fails() {
        let tmp = TempDir::new().unwrap();
        let entry = Entry {
            id: "plugin-foo".to_string(),
            active: Slot {
                path: tmp.path().join("missing-release"),
                source: SlotSource::ReleaseAsset,
            },
            fallback: Some(Slot {
                path: tmp.path().join("dev-src"),
                source: SlotSource::DevLink {
                    origin_path: tmp.path().join("dev-src"),
                },
            }),
        };
        let registry = Registry {
            version: 1,
            entries: vec![entry],
        };
        let report = resolve_from_registry(&registry);

        assert_eq!(report.plugins.len(), 0);
        assert_eq!(report.unavailable.len(), 1);
        assert!(
            report.unavailable[0].fallback.is_none(),
            "dev-link fallback must be invisible in prod",
        );
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

    #[cfg(feature = "dev")]
    fn dev_linked_entry(active: &str, fallback: Option<&str>) -> Entry {
        Entry {
            id: "plugin-foo".to_string(),
            active: Slot {
                path: PathBuf::from(active),
                source: SlotSource::DevLink {
                    origin_path: PathBuf::from(active),
                },
            },
            fallback: fallback.map(|path| Slot {
                path: PathBuf::from(path),
                source: SlotSource::ReleaseAsset,
            }),
        }
    }

    #[cfg(feature = "dev")]
    #[test]
    fn worktree_override_preserves_the_main_clone_as_fallback() {
        let cases = [
            (
                "missing fallback is filled from prior active",
                None,
                "/wt",
                "/wt",
                Some("/main"),
            ),
            (
                "stale installed fallback is replaced by main clone",
                Some("/rel"),
                "/wt",
                "/wt",
                Some("/main"),
            ),
            ("no-op when path is unchanged", None, "/main", "/main", None),
        ];
        for (label, initial_fallback, new_path, want_active, want_fallback) in cases {
            let mut entry = dev_linked_entry("/main", initial_fallback);
            override_entry_to_worktree(&mut entry, &PathBuf::from(new_path));
            assert_eq!(
                entry.active.path,
                PathBuf::from(want_active),
                "active path: {label}"
            );
            assert_eq!(
                entry.fallback.as_ref().map(|slot| slot.path.clone()),
                want_fallback.map(PathBuf::from),
                "fallback path: {label}"
            );
        }
    }

    #[cfg(feature = "dev")]
    #[test]
    fn worktree_fallback_keeps_the_original_dev_link_source() {
        let mut entry = dev_linked_entry("/main", None);
        override_entry_to_worktree(&mut entry, &PathBuf::from("/wt"));
        let fallback = entry.fallback.expect("fallback populated");

        assert!(
            matches!(fallback.source, SlotSource::DevLink { .. }),
            "fallback keeps the original dev-link source so resolution targets the main clone"
        );
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

    #[cfg(feature = "dev")]
    #[test]
    fn apply_worktree_override_keeps_links_without_selection() {
        let repo = crate::test_support::GitRepo::new();
        let feat = repo.add_worktree("feat");
        let link = repo.plugin(&feat, "plugin-foo");
        let registry = Registry {
            version: 1,
            entries: vec![dev_linked_entry(link.to_str().unwrap(), None)],
        };
        let config_dir = TempDir::new().unwrap();

        let mut effective = registry.clone();
        apply_worktree_override(&mut effective, config_dir.path());

        assert_eq!(
            effective.entries.len(),
            1,
            "no selection must keep links as-is"
        );
        assert_eq!(effective.entries[0].active.path, link);
    }

    #[cfg(feature = "dev")]
    #[test]
    fn apply_worktree_override_keeps_link_on_its_own_branch() {
        let repo = crate::test_support::GitRepo::new();
        let feat = repo.add_worktree("feat");
        let link = repo.plugin(&feat, "plugin-foo");
        let registry = Registry {
            version: 1,
            entries: vec![dev_linked_entry(link.to_str().unwrap(), None)],
        };
        let config_dir = TempDir::new().unwrap();
        qol_dev_build::tray::set_active_worktree_marker(config_dir.path(), Some("feat")).unwrap();

        let mut effective = registry.clone();
        apply_worktree_override(&mut effective, config_dir.path());

        assert_eq!(effective.entries.len(), 1);
        assert_eq!(effective.entries[0].active.path, link);
    }

    #[cfg(feature = "dev")]
    #[test]
    fn apply_worktree_override_detaches_link_without_fallback() {
        let repo = crate::test_support::GitRepo::new();
        let feat = repo.add_worktree("feat");
        repo.add_worktree("other");
        let link = repo.plugin(&feat, "plugin-foo");
        let registry = Registry {
            version: 1,
            entries: vec![dev_linked_entry(link.to_str().unwrap(), None)],
        };
        let config_dir = TempDir::new().unwrap();
        qol_dev_build::tray::set_active_worktree_marker(config_dir.path(), Some("other")).unwrap();

        let mut effective = registry.clone();
        apply_worktree_override(&mut effective, config_dir.path());

        assert_eq!(
            effective.entries.len(),
            0,
            "link whose only home is a non-selected worktree leaves the session"
        );
    }

    #[cfg(feature = "dev")]
    #[test]
    fn apply_worktree_override_promotes_release_fallback() {
        let repo = crate::test_support::GitRepo::new();
        let feat = repo.add_worktree("feat");
        repo.add_worktree("other");
        let link = repo.plugin(&feat, "plugin-foo");
        let release = repo.root.join("plugins").join("plugin-foo");
        let registry = Registry {
            version: 1,
            entries: vec![dev_linked_entry(
                link.to_str().unwrap(),
                Some(release.to_str().unwrap()),
            )],
        };
        let config_dir = TempDir::new().unwrap();
        qol_dev_build::tray::set_active_worktree_marker(config_dir.path(), Some("other")).unwrap();

        let mut effective = registry.clone();
        apply_worktree_override(&mut effective, config_dir.path());

        assert_eq!(
            effective.entries.len(),
            1,
            "installed fallback must survive"
        );
        assert_eq!(effective.entries[0].active.path, release);
        assert!(
            matches!(effective.entries[0].active.source, SlotSource::ReleaseAsset),
            "promoted active must be the release slot"
        );
    }
}
