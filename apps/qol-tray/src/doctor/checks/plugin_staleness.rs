use super::super::framework::{CheckCategory, CheckMeta, CheckReport, DoctorCheck, DoctorContext};
use crate::dev;
use crate::plugins::registry::{Registry, SlotSource};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

const ID: &str = "plugin_staleness";

pub(super) struct PluginStalenessCheck;

impl DoctorCheck for PluginStalenessCheck {
    fn meta(&self) -> CheckMeta {
        CheckMeta::new(ID, "Plugin staleness", CheckCategory::DevBuild)
            .group(&["dev-loop"])
            .dev_only()
    }

    fn run(&self, ctx: &DoctorContext) -> CheckReport {
        let registry = match ctx.registry() {
            Ok(registry) => registry,
            Err(error) => {
                return CheckReport::ok(format!("could not read plugin registry: {error}"));
            }
        };
        let linked = ctx.linked();
        let workspace_root = plugin_sources_dir();

        let findings = collect_findings(registry, linked, &workspace_root, &dir_has_plugin_toml);

        if findings.is_empty() {
            return CheckReport::ok("no plugin staleness detected".to_string());
        }
        CheckReport::warn(format_message(&findings), ID, Vec::new())
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum Finding {
    UnlinkedSource {
        plugin_id: String,
        sibling_path: PathBuf,
    },
    DevLinkStale {
        plugin_id: String,
        reason: String,
    },
}

pub(crate) fn collect_findings(
    registry: &Registry,
    linked: &[dev::LinkedPlugin],
    workspace_root: &Path,
    sibling_check: &dyn Fn(&Path) -> bool,
) -> Vec<Finding> {
    let mut findings = Vec::new();
    findings.extend(unlinked_source_findings(
        registry,
        workspace_root,
        sibling_check,
    ));
    findings.extend(dev_link_stale_findings(linked));
    findings
}

fn unlinked_source_findings(
    registry: &Registry,
    workspace_root: &Path,
    sibling_check: &dyn Fn(&Path) -> bool,
) -> Vec<Finding> {
    registry
        .entries
        .iter()
        .filter(|entry| matches!(entry.active.source, SlotSource::ReleaseAsset))
        .filter_map(|entry| {
            let sibling = workspace_root.join(&entry.id);
            sibling_check(&sibling).then(|| Finding::UnlinkedSource {
                plugin_id: entry.id.clone(),
                sibling_path: sibling,
            })
        })
        .collect()
}

fn dev_link_stale_findings(linked: &[dev::LinkedPlugin]) -> Vec<Finding> {
    linked
        .iter()
        .filter(|p| p.needs_rebuild)
        .map(|p| Finding::DevLinkStale {
            plugin_id: p.id.clone(),
            reason: p.rebuild_reason.clone(),
        })
        .collect()
}

fn plugin_sources_dir() -> PathBuf {
    crate::paths::repo_root_from_manifest_dir().join("plugins")
}

fn dir_has_plugin_toml(path: &Path) -> bool {
    path.is_dir() && path.join("plugin.toml").is_file()
}

fn format_message(findings: &[Finding]) -> String {
    let mut by_kind: BTreeMap<&str, Vec<String>> = BTreeMap::new();
    for finding in findings {
        match finding {
            Finding::UnlinkedSource {
                plugin_id,
                sibling_path,
            } => by_kind
                .entry("source available but not dev-linked")
                .or_default()
                .push(format!("{plugin_id} ({})", sibling_path.display())),
            Finding::DevLinkStale { plugin_id, reason } => by_kind
                .entry("dev-link rebuild required")
                .or_default()
                .push(format!("{plugin_id} ({reason})")),
        }
    }
    let parts: Vec<String> = by_kind
        .into_iter()
        .map(|(kind, items)| format!("{kind}: {}", items.join(", ")))
        .collect();
    format!("plugin staleness detected — {}", parts.join("; "))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::plugins::registry::{Entry, Slot};

    fn release_entry(id: &str) -> Entry {
        Entry {
            id: id.into(),
            active: Slot {
                path: PathBuf::from(format!("/installed/{id}")),
                source: SlotSource::ReleaseAsset,
            },
            fallback: None,
        }
    }

    fn devlink_entry(id: &str, source: &str) -> Entry {
        Entry {
            id: id.into(),
            active: Slot {
                path: PathBuf::from(source),
                source: SlotSource::DevLink {
                    origin_path: PathBuf::from(source),
                },
            },
            fallback: None,
        }
    }

    fn linked_plugin(id: &str, needs_rebuild: bool, reason: &str) -> dev::LinkedPlugin {
        dev::LinkedPlugin {
            id: id.into(),
            name: id.into(),
            source: format!("/src/{id}"),
            has_cargo: true,
            supports_platform: true,
            needs_rebuild,
            rebuild_reason: reason.into(),
            fingerprint: None,
            last_built_fingerprint: None,
            logs_muted: false,
            suppressed_log_patterns: Vec::new(),
        }
    }

    fn registry_with(entries: Vec<Entry>) -> Registry {
        Registry {
            version: registry::CURRENT_REGISTRY_VERSION,
            entries,
        }
    }

    #[test]
    fn unlinked_source_warns_only_when_sibling_present() {
        let registry = registry_with(vec![
            release_entry("plugin-with-source"),
            release_entry("plugin-without-source"),
        ]);
        let workspace = PathBuf::from("/workspace");
        let present = |p: &Path| p == Path::new("/workspace/plugin-with-source");

        let findings = collect_findings(&registry, &[], &workspace, &present);
        assert_eq!(
            findings,
            vec![Finding::UnlinkedSource {
                plugin_id: "plugin-with-source".into(),
                sibling_path: PathBuf::from("/workspace/plugin-with-source"),
            }]
        );
    }

    #[test]
    fn devlinked_entry_does_not_trigger_unlinked_warning_even_when_sibling_present() {
        let registry = registry_with(vec![devlink_entry("plugin-foo", "/workspace/plugin-foo")]);
        let workspace = PathBuf::from("/workspace");
        let always_present = |_: &Path| true;

        let findings = collect_findings(&registry, &[], &workspace, &always_present);
        assert!(findings.is_empty(), "got: {findings:?}");
    }

    #[test]
    fn dev_link_stale_warns_for_needs_rebuild_only() {
        let registry = registry_with(vec![]);
        let linked = [
            linked_plugin("plugin-stale", true, "Source changed"),
            linked_plugin("plugin-fresh", false, "Up to date"),
        ];
        let workspace = PathBuf::from("/workspace");
        let none_present = |_: &Path| false;

        let findings = collect_findings(&registry, &linked, &workspace, &none_present);
        assert_eq!(
            findings,
            vec![Finding::DevLinkStale {
                plugin_id: "plugin-stale".into(),
                reason: "Source changed".into(),
            }]
        );
    }

    #[test]
    fn combined_message_groups_findings_by_kind() {
        let findings = vec![
            Finding::UnlinkedSource {
                plugin_id: "plugin-a".into(),
                sibling_path: PathBuf::from("/ws/plugin-a"),
            },
            Finding::UnlinkedSource {
                plugin_id: "plugin-b".into(),
                sibling_path: PathBuf::from("/ws/plugin-b"),
            },
            Finding::DevLinkStale {
                plugin_id: "plugin-c".into(),
                reason: "Source changed".into(),
            },
        ];
        let message = format_message(&findings);
        assert!(
            message.contains("source available but not dev-linked: plugin-a (/ws/plugin-a), plugin-b (/ws/plugin-b)"),
            "actual: {message}"
        );
        assert!(
            message.contains("dev-link rebuild required: plugin-c (Source changed)"),
            "actual: {message}"
        );
    }

    #[test]
    fn no_findings_returns_empty() {
        let registry = registry_with(vec![release_entry("plugin-foo")]);
        let workspace = PathBuf::from("/workspace");
        let none_present = |_: &Path| false;
        assert!(collect_findings(&registry, &[], &workspace, &none_present).is_empty());
    }

    #[test]
    fn dir_has_plugin_toml_requires_both_dir_and_file() {
        let tmp = tempfile::TempDir::new().unwrap();
        let dir = tmp.path().join("plugin-x");
        std::fs::create_dir(&dir).unwrap();
        assert!(!dir_has_plugin_toml(&dir));
        std::fs::write(dir.join("plugin.toml"), b"[plugin]\n").unwrap();
        assert!(dir_has_plugin_toml(&dir));
        assert!(!dir_has_plugin_toml(&tmp.path().join("does-not-exist")));
    }
}
